//! Aktualizacje PowerDesk z portfolio redzikowski.com – serwer podaje wydania w formacie
//! GitHub API, a pliki pobiera z repozytorium sam, więc program nie potrzebuje żadnego klucza.
//!
//! Wydanie zawiera `PowerDesk-<wersja>.exe` i `SHA256SUMS.txt`. Nowy plik pobieramy do TEMP,
//! sprawdzamy sumę, a skrypt po zamknięciu PowerDesk podmienia .exe i uruchamia go ponownie.

use std::io::{Read, Write};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

const FEED: &str = "https://redzikowski.com/api/github/repos/BlackSingularities/OpenFilePowerWin/releases/latest";
const UA: &str = "PowerDesk";

/// Wersja tego programu (CI podaje numer wydania przy kompilacji).
pub fn current() -> &'static str {
    option_env!("POWERDESK_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join("PowerDesk")
}

fn get(url: &str, accept: &str) -> Result<ureq::Response, String> {
    ureq::get(url)
        .set("User-Agent", UA)
        .set("Accept", accept)
        .timeout(std::time::Duration::from_secs(30))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "Brak wydań do pobrania".to_string(),
            e => e.to_string(),
        })
}

/// Nowa wersja do pobrania.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    asset: String,
    name: String,
    sums: Option<String>,
}

fn parse(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Czy `a` jest nowsza od `b` (MAJOR.MINOR.PATCH).
pub fn newer(a: &str, b: &str) -> bool {
    parse(a) > parse(b)
}

/// Najnowsze wydanie, jeśli nowsze od tej wersji i ma plik PowerDesk.
pub fn check() -> Result<Option<Release>, String> {
    let r: serde_json::Value = get(FEED, "application/vnd.github+json")?
        .into_json()
        .map_err(|e| e.to_string())?;
    let version = r["tag_name"].as_str().unwrap_or_default().trim_start_matches('v').to_string();
    if version.is_empty() || !newer(&version, current()) {
        return Ok(None);
    }
    let assets = r["assets"].as_array().cloned().unwrap_or_default();
    let find = |pred: &dyn Fn(&str) -> bool| assets.iter().find(|a| a["name"].as_str().is_some_and(|n| pred(&n.to_lowercase()))).cloned();
    let Some(exe) = find(&|n| n.starts_with("powerdesk-") && n.ends_with(".exe")) else { return Ok(None) };
    Ok(Some(Release {
        version,
        notes: r["body"].as_str().unwrap_or_default().to_string(),
        asset: exe["browser_download_url"].as_str().unwrap_or_default().to_string(),
        name: exe["name"].as_str().unwrap_or("PowerDesk.exe").to_string(),
        sums: find(&|n| n == "sha256sums.txt").and_then(|a| a["browser_download_url"].as_str().map(str::to_string)),
    }))
}

/// Pobiera nowy plik i sprawdza sumę SHA-256. `progress(pobrane, całość)`.
pub fn download(rel: &Release, progress: impl Fn(u64, u64)) -> Result<PathBuf, String> {
    let expected = match &rel.sums {
        Some(url) => get(url, "application/octet-stream")?
            .into_string()
            .map_err(|e| e.to_string())?
            .lines()
            .find(|l| l.trim_end().ends_with(&rel.name))
            .and_then(|l| l.split_whitespace().next())
            .map(|h| h.to_lowercase()),
        None => None,
    };
    // Bez sumy nie podmieniamy programu – to jest powłoka systemu.
    let expected = expected.ok_or("Wydanie nie ma sumy kontrolnej pliku PowerDesk")?;
    let resp = get(&rel.asset, "application/octet-stream")?;
    let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let folder = dir().join("update");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(&rel.name);
    let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut rd = resp.into_reader();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = rd.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        h.update(&buf[..n]);
        done += n as u64;
        progress(done, total.max(done));
    }
    drop(f);
    if format!("{:x}", h.finalize()) != expected {
        let _ = std::fs::remove_file(&path);
        return Err("Suma kontrolna pobranego pliku się nie zgadza – aktualizacja przerwana".into());
    }
    Ok(path)
}

/// Znacznik podmiany – strażnik powłoki czeka, aż zniknie, zanim uruchomi PowerDesk ponownie.
pub fn marker() -> PathBuf {
    dir().join("updating.txt")
}

/// Skrypt: czeka na koniec tego procesu, podmienia .exe (stary zostaje jako .old) i uruchamia
/// nowy (strażnik w tym czasie tylko czeka na zdjęcie znacznika).
/// Po wywołaniu program powinien się zakończyć.
pub fn apply(new_exe: &std::path::Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let me_s = me.to_string_lossy();
    let old = format!("{me_s}.old");
    let pid = std::process::id();
    let mark = marker();
    std::fs::write(&mark, pid.to_string()).map_err(|e| e.to_string())?;
    let start = format!("start \"\" \"{me_s}\" {}", crate::autostart::FLAG);
    let script = format!(
        "@echo off\r\n:wait\r\ntasklist /fi \"PID eq {pid}\" | find \"{pid}\" >nul && (timeout /t 1 /nobreak >nul & goto wait)\r\n\
         del /f /q \"{old}\" 2>nul\r\nmove /y \"{me_s}\" \"{old}\" >nul\r\ncopy /y \"{new}\" \"{me_s}\" >nul || move /y \"{old}\" \"{me_s}\" >nul\r\n\
         del /f /q \"{mark}\"\r\n{start}\r\n",
        new = new_exe.to_string_lossy(),
        mark = mark.to_string_lossy(),
    );
    let bat = dir().join("update").join("apply.cmd");
    std::fs::write(&bat, script).map_err(|e| e.to_string())?;
    std::process::Command::new("cmd").arg("/c").arg(&bat).creation_flags(0x0800_0000).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Dla strażnika: trwa podmiana – czekamy do 60 s, aż skrypt skończy.
pub fn wait_for_apply() {
    let mark = marker();
    for _ in 0..120 {
        if !mark.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let _ = std::fs::remove_file(mark);
}
