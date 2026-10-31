//! Aktualizacje PowerDesk z portfolio redzikowski.com – serwer podaje wydania w formacie
//! GitHub API, a pliki pobiera z repozytorium sam, więc program nie potrzebuje żadnego klucza.
//!
//! Wydanie zawiera jeden plik – instalator `PowerDesk-Setup-<wersja>.exe` (ten sam program),
//! a jego sumę SHA-256 w opisie wydania (starsze wydania: `SHA256SUMS.txt`). Instalator pobieramy
//! do TEMP, sprawdzamy sumę i uruchamiamy z `--install --update` w miejscu instalacji (Program
//! Files – z UAC). Potem skrypt po zamknięciu PowerDesk uruchamia zainstalowaną kopię.

use std::io::{Read, Write};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

const FEED: &str = "https://redzikowski.com/api/github/repos/BlackSingularities/PowerDesk/releases/latest";
/// Zapas, gdy portfolio nie odpowiada – publiczne API GitHuba (bez klucza, limit 60 zapytań/h).
const FEED_GITHUB: &str = "https://api.github.com/repos/BlackSingularities/PowerDesk/releases/latest";
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
            ureq::Error::Status(404, _) => crate::lang::tr("Brak wydań do pobrania").to_string(),
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

/// Suma SHA-256 pliku `name` z opisu wydania: linia z nazwą pliku albo pierwsza 64-znakowa
/// liczba szesnastkowa.
fn sum_in_notes(notes: &str, name: &str) -> Option<String> {
    let hex = |w: &str| w.len() == 64 && w.chars().all(|c| c.is_ascii_hexdigit());
    let words = |l: &str| l.split(|c: char| !c.is_ascii_alphanumeric()).map(str::to_string).collect::<Vec<_>>();
    notes
        .lines()
        .find(|l| l.contains(name))
        .and_then(|l| words(l).into_iter().find(|w| hex(w)))
        .or_else(|| notes.lines().flat_map(words).find(|w| hex(w)))
        .map(|h| h.to_lowercase())
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
    let feed = |url: &str| -> Result<serde_json::Value, String> { get(url, "application/vnd.github+json")?.into_json().map_err(|e| e.to_string()) };
    let r = feed(FEED).or_else(|_| feed(FEED_GITHUB))?;
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
        None => sum_in_notes(&rel.notes, &rel.name),
    };
    // Bez sumy nie podmieniamy programu – to jest powłoka systemu.
    let expected = expected.ok_or(crate::lang::tr("Wydanie nie ma sumy kontrolnej pliku PowerDesk"))?;
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
        return Err(crate::lang::tr("Suma kontrolna pobranego pliku się nie zgadza – aktualizacja przerwana").into());
    }
    Ok(path)
}

/// Znacznik podmiany – strażnik powłoki czeka, aż zniknie, zanim uruchomi PowerDesk ponownie.
pub fn marker() -> PathBuf {
    dir().join("updating.txt")
}

/// Zainstalowany PowerDesk: pobrany instalator instaluje nową wersję w tym samym miejscu
/// (dla wszystkich użytkowników – z UAC) i czeka, aż skończy. Potem skrypt po zamknięciu tego
/// procesu uruchamia zainstalowaną kopię (strażnik powłoki w tym czasie czeka na znacznik).
/// Po wywołaniu program powinien się zakończyć.
pub fn apply_installer(setup: &std::path::Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // Bez instalacji (np. kopia z budowania) – zwykły instalator, który sam zaproponuje instalację.
    let Some((exe, all)) = crate::install::installed() else {
        std::process::Command::new(setup).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    };
    let args = format!("{} {} {}", crate::install::INSTALL_FLAG, crate::install::UPDATE_FLAG, if all { crate::install::ALL_USERS_FLAG } else { "" });
    let elevated = all && !crate::install::is_admin();
    match crate::install::run_wait(setup, args.trim(), elevated)? {
        0 => {}
        code => return Err(crate::trf!("Instalator zakończył się błędem ({code})")),
    }
    let pid = std::process::id();
    let mark = marker();
    std::fs::write(&mark, pid.to_string()).map_err(|e| e.to_string())?;
    let script = format!(
        "@echo off\r\n:wait\r\ntasklist /fi \"PID eq {pid}\" | find \"{pid}\" >nul && (timeout /t 1 /nobreak >nul & goto wait)\r\ndel /f /q \"{mark}\"\r\nstart \"\" \"{exe}\" {flag}\r\n",
        mark = mark.to_string_lossy(),
        exe = exe.to_string_lossy(),
        flag = crate::autostart::FLAG,
    );
    let bat = dir().join("update").join("restart.cmd");
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
