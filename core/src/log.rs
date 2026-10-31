//! Dziennik PowerDesk: `%LOCALAPPDATA%\PowerDesk\log.txt` (do 1 MB, poprzedni jako log.old.txt).
//! Start, tryb pracy, błędy i awarie (panika z miejscem i śladem stosu) – do „Zgłoś problem”.

use std::io::Write;
use std::path::PathBuf;

use parking_lot::Mutex;

const MAX: u64 = 1024 * 1024;
static LOCK: Mutex<()> = Mutex::new(());

pub fn dir() -> PathBuf {
    std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir()).join("PowerDesk")
}

pub fn file() -> PathBuf {
    dir().join("log.txt")
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

fn write(level: &str, msg: &str) {
    let _g = LOCK.lock();
    let f = file();
    let _ = std::fs::create_dir_all(dir());
    if std::fs::metadata(&f).is_ok_and(|m| m.len() > MAX) {
        let _ = std::fs::rename(&f, dir().join("log.old.txt"));
    }
    if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(&f) {
        let _ = writeln!(out, "{} [{level}] {} {msg}", now(), std::process::id());
    }
}

pub fn info(msg: impl AsRef<str>) {
    write("info", msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    write(crate::lang::tr("błąd"), msg.as_ref());
}

/// Panika (awaria wątku) trafia do dziennika ze śladem stosu, zanim program się zakończy.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        write("AWARIA", &format!("{info}\n{bt}"));
        prev(info);
    }));
}

/// Ostatnie `n` wierszy dziennika (do raportu).
pub fn tail(n: usize) -> String {
    let text = std::fs::read_to_string(file()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Raport do zgłoszenia problemu: wersja, system, tryb pracy, końcówka dziennika.
pub fn report(version: &str) -> String {
    let info = crate::sysmon::info();
    crate::trf!(
        "PowerDesk {version}\n{} · {}\nProcesor: {} ({} wątków), RAM: {} MB\nTryb powłoki: {} · zainstalowany: {}\n\n--- dziennik (ostatnie 200 wierszy) ---\n{}",
        info.os,
        info.host,
        info.cpu,
        info.threads,
        info.mem_total / 1024 / 1024,
        if crate::shellmode::active() { "tak" } else { "nie" },
        if crate::install::running_installed() { "tak" } else { "nie" },
        tail(200)
    )
}
