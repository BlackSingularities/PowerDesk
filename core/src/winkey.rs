//! Win+E otwiera PowerDesk zamiast Eksploratora: polecenie „opennewwindow” obiektu
//! „Ten komputer” nadpisane w HKCU (tylko dla bieżącego użytkownika, bez uprawnień administratora).
//! Usunięcie naszego klucza przywraca Eksploratora.

use std::os::windows::process::CommandExt;
use std::process::Command;

const CLSID: &str = r"HKCU\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
const NO_WINDOW: u32 = 0x0800_0000;

fn command_key() -> String {
    format!(r"{CLSID}\shell\opennewwindow\command")
}

fn reg(args: &[&str]) -> Result<String, String> {
    let out = Command::new("reg").args(args).creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn exe() -> Result<String, String> {
    std::env::current_exe().map(|p| p.to_string_lossy().to_string()).map_err(|e| e.to_string())
}

/// Win+E prowadzi do tego programu (do bieżącego pliku .exe).
pub fn enabled() -> bool {
    let Ok(exe) = exe() else { return false };
    reg(&["query", &command_key(), "/ve"]).is_ok_and(|o| o.to_lowercase().contains(&exe.to_lowercase()))
}

/// Włącza albo wyłącza przejęcie Win+E.
pub fn set(on: bool) -> Result<(), String> {
    if on {
        let key = command_key();
        reg(&["add", &key, "/ve", "/d", &format!("\"{}\"", exe()?), "/f"])?;
        // Pusty DelegateExecute każe powłoce uruchomić polecenie zamiast własnego obiektu COM.
        reg(&["add", &key, "/v", "DelegateExecute", "/d", "", "/f"])?;
    } else {
        // Tylko nasza gałąź – reszty nadpisań tego obiektu (gdyby ktoś je miał) nie ruszamy.
        let ours = format!(r"{CLSID}\shell\opennewwindow");
        if reg(&["query", &ours]).is_ok() {
            reg(&["delete", &ours, "/f"])?;
        }
    }
    Ok(())
}
