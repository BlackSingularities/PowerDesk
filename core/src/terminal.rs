//! Wbudowany terminal (ConPTY): PowerShell w bieżącym folderze, wyjście strumieniowane do xterm.js.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use crate::events::{AppHandle, Emitter};

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

static SESSIONS: Lazy<Mutex<HashMap<u64, Session>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Serialize, Clone)]
struct Data {
    id: u64,
    data: String,
}

/// PowerShell 7, jeśli jest; inaczej wbudowany Windows PowerShell.
fn shell() -> String {
    let pwsh = ["ProgramFiles", "ProgramW6432"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .map(|d| std::path::Path::new(&d).join("PowerShell").join("7").join("pwsh.exe"))
        .find(|p| p.exists());
    pwsh.map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "powershell.exe".into())
}

pub fn start(app: AppHandle, cwd: &str, cols: u16, rows: u16) -> Result<u64, String> {
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;
    let mut cmd = CommandBuilder::new(shell());
    cmd.arg("-NoLogo");
    if std::path::Path::new(cwd).is_dir() {
        cmd.cwd(cwd);
    }
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    SESSIONS.lock().insert(id, Session { master: pair.master, writer, child });

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        // Niepełne znaki UTF-8 na granicy odczytu przenosimy do następnej porcji.
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    let valid = match std::str::from_utf8(&pending) {
                        Ok(_) => pending.len(),
                        Err(e) => e.valid_up_to(),
                    };
                    if valid > 0 {
                        let data = String::from_utf8_lossy(&pending[..valid]).to_string();
                        pending.drain(..valid);
                        let _ = app.emit("term-data", Data { id, data });
                    }
                }
            }
        }
        SESSIONS.lock().remove(&id);
        let _ = app.emit("term-exit", id);
    });
    Ok(id)
}

pub fn write(id: u64, data: &str) -> Result<(), String> {
    let mut s = SESSIONS.lock();
    let sess = s.get_mut(&id).ok_or(crate::lang::tr("Terminal został zamknięty"))?;
    sess.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
    sess.writer.flush().map_err(|e| e.to_string())
}

pub fn resize(id: u64, cols: u16, rows: u16) {
    if let Some(s) = SESSIONS.lock().get(&id) {
        let _ = s.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
    }
}

pub fn kill(id: u64) {
    if let Some(mut s) = SESSIONS.lock().remove(&id) {
        let _ = s.child.kill();
    }
}

/// Przechodzi do folderu (gdy w aplikacji zmieni się bieżący folder).
pub fn cd(id: u64, path: &str) -> Result<(), String> {
    let p = path.replace('\'', "''");
    write(id, &crate::trf!("Set-Location -LiteralPath '{p}'\r"))
}
