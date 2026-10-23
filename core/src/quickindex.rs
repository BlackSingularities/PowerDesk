//! Szybki indeks plików do wyszukiwania w menu Start: nazwy z folderów użytkownika (Pulpit,
//! Dokumenty, Pobrane, Obrazy, Muzyka, Wideo, OneDrive) w pamięci, budowany w tle i co 10 minut
//! od nowa. Nie zależy od usługi Windows Search (bez Eksploratora bywa wyłączona).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::RwLock;

/// Plik w indeksie: ścieżka, nazwa (do wyświetlenia) i nazwa uproszczona (małe litery, bez polskich znaków).
#[derive(Clone, Debug)]
pub struct Item {
    pub path: String,
    pub name: String,
    pub folded: String,
    pub dir: bool,
    pub modified: i64,
}

const LIMIT: usize = 200_000;
const SKIP: [&str; 8] = ["node_modules", ".git", "target", "$recycle.bin", "appdata", ".cache", "__pycache__", ".venv"];

static INDEX: RwLock<Vec<Item>> = RwLock::new(Vec::new());
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Małe litery bez polskich znaków – wspólne dla zapytania i nazw.
pub fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ą' | 'Ą' => 'a',
            'ć' | 'Ć' => 'c',
            'ę' | 'Ę' => 'e',
            'ł' | 'Ł' => 'l',
            'ń' | 'Ń' => 'n',
            'ó' | 'Ó' => 'o',
            'ś' | 'Ś' => 's',
            'ź' | 'Ź' | 'ż' | 'Ż' => 'z',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

fn roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [dirs::desktop_dir(), dirs::document_dir(), dirs::download_dir(), dirs::picture_dir(), dirs::audio_dir(), dirs::video_dir()].into_iter().flatten().collect();
    if let Ok(od) = std::env::var("OneDrive") {
        v.push(PathBuf::from(od));
    }
    v.sort();
    v.dedup();
    v
}

fn build() -> Vec<Item> {
    let mut out = Vec::new();
    let mut stack = roots();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && name != ".config" {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let dir = meta.is_dir();
            if dir && SKIP.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            let path = e.path();
            out.push(Item { path: path.to_string_lossy().to_string(), folded: fold(&name), name, dir, modified });
            if out.len() >= LIMIT {
                return out;
            }
            if dir {
                stack.push(path);
            }
        }
    }
    out
}

/// Indeksowanie w tle: od razu i co 10 minut. Drugie wywołanie – nic.
pub fn start() {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        let items = build();
        *INDEX.write() = items;
        std::thread::sleep(std::time::Duration::from_secs(600));
    });
}

/// Pliki i foldery pasujące do wszystkich słów zapytania; najpierw zaczynające się od zapytania,
/// potem najnowsze. Do `limit` wyników.
pub fn search(query: &str, limit: usize) -> Vec<Item> {
    let q = fold(query.trim());
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let idx = INDEX.read();
    let mut hits: Vec<&Item> = idx.iter().filter(|i| words.iter().all(|w| i.folded.contains(w))).collect();
    hits.sort_by(|a, b| b.folded.starts_with(&q).cmp(&a.folded.starts_with(&q)).then(b.modified.cmp(&a.modified)));
    hits.into_iter().take(limit).cloned().collect()
}

/// Ostatnio używane pliki z listy Windows (`Recent` – skróty .lnk), najnowsze pierwsze.
/// Zwraca (nazwa bez .lnk, ścieżka skrótu).
pub fn recent(limit: usize) -> Vec<(String, String)> {
    let Ok(appdata) = std::env::var("APPDATA") else { return Vec::new() };
    let dir = PathBuf::from(appdata).join(r"Microsoft\Windows\Recent");
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, String, String)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
        .filter_map(|e| {
            let t = e.metadata().ok()?.modified().ok()?;
            let name = e.path().file_stem()?.to_string_lossy().to_string();
            Some((t, name, e.path().to_string_lossy().to_string()))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().take(limit).map(|(_, n, p)| (n, p)).collect()
}
