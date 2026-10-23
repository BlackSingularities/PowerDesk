//! Zdjęcie dnia z Binga na tapetę: pobierane raz dziennie do `%LOCALAPPDATA%\PowerDesk\bing.jpg`.

use std::io::Read;
use std::path::PathBuf;

fn dir() -> PathBuf {
    crate::log::dir()
}

/// Ścieżka dzisiejszego zdjęcia – pobiera je, jeśli jeszcze nie ma (wołać z wątku w tle).
/// Bez internetu zostaje zdjęcie z poprzedniego dnia, jeśli jest.
pub fn today() -> Option<PathBuf> {
    let file = dir().join("bing.jpg");
    let stamp = dir().join("bing.txt");
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    if file.is_file() && std::fs::read_to_string(&stamp).is_ok_and(|d| d.trim() == date) {
        return Some(file);
    }
    match fetch(&file) {
        Ok(()) => {
            let _ = std::fs::write(&stamp, &date);
            Some(file)
        }
        Err(e) => {
            crate::log::error(format!("tapeta Bing: {e}"));
            file.is_file().then_some(file)
        }
    }
}

fn fetch(file: &PathBuf) -> Result<(), String> {
    let meta: serde_json::Value = ureq::get("https://www.bing.com/HPImageArchive.aspx?format=js&idx=0&n=1&mkt=pl-PL")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    let url = meta["images"][0]["url"].as_str().ok_or("brak zdjęcia w odpowiedzi")?;
    // Pełna rozdzielczość (UHD), gdy jest – adres z „1920x1080” zamieniony na „UHD”.
    let full = format!("https://www.bing.com{}", url.replace("1920x1080", "UHD"));
    let resp = ureq::get(&full).timeout(std::time::Duration::from_secs(60)).call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    resp.into_reader().take(40 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(dir());
    std::fs::write(file, buf).map_err(|e| e.to_string())
}
