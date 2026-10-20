//! Listowanie, wyszukiwanie, podgląd zawartości, rozmiary folderów, skróty, obserwacja zmian.

use std::collections::HashSet;
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_READONLY: u32 = 0x1;
const ATTR_REPARSE: u32 = 0x400;
const ATTR_OFFLINE: u32 = 0x1000;
const ATTR_RECALL: u32 = 0x400000 | 0x40000;

#[derive(Serialize, Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub modified: i64,
    pub created: i64,
    pub ext: String,
    pub hidden: bool,
    pub system: bool,
    pub readonly: bool,
    pub link: bool,
    /// Plik tylko w chmurze (OneDrive „na żądanie") – nie czytamy go, żeby nie wymuszać pobrania.
    pub cloud: bool,
}

fn ms(t: std::io::Result<SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn entry_from(path: &Path, md: &std::fs::Metadata) -> Entry {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let attrs = md.file_attributes();
    let dir = md.is_dir();
    let ext = if dir {
        String::new()
    } else {
        path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
    };
    Entry {
        name,
        path: path.to_string_lossy().to_string(),
        dir,
        size: if dir { 0 } else { md.len() },
        modified: ms(md.modified()),
        created: ms(md.created()),
        ext,
        hidden: attrs & ATTR_HIDDEN != 0,
        system: attrs & ATTR_SYSTEM != 0,
        readonly: attrs & ATTR_READONLY != 0,
        link: attrs & ATTR_REPARSE != 0 && md.file_type().is_symlink(),
        cloud: attrs & (ATTR_OFFLINE | ATTR_RECALL) != 0,
    }
}

pub fn list_dir(path: &str) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(path).map_err(|e| io_err(&e))?;
    let mut out = Vec::with_capacity(256);
    for de in rd.flatten() {
        let p = de.path();
        // DirEntry::metadata na Windows korzysta z danych FindNextFile – bez dodatkowego otwierania pliku.
        let md = match de.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mut e = entry_from(&p, &md);
        if e.link {
            if let Ok(t) = std::fs::metadata(&p) {
                e.dir = t.is_dir();
            }
        }
        out.push(e);
    }
    Ok(out)
}

pub fn stat(path: &str) -> Result<Entry, String> {
    let p = Path::new(path);
    let md = std::fs::metadata(p).map_err(|e| io_err(&e))?;
    Ok(entry_from(p, &md))
}

pub fn io_err(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(2) | Some(3) => "Nie znaleziono ścieżki".into(),
        Some(5) => "Brak dostępu".into(),
        Some(21) => "Urządzenie nie jest gotowe".into(),
        Some(32) => "Plik jest używany przez inny proces".into(),
        Some(53) | Some(67) => "Ścieżka sieciowa jest niedostępna".into(),
        Some(112) => "Za mało miejsca na dysku".into(),
        Some(123) => "Nieprawidłowa nazwa".into(),
        Some(145) => "Folder nie jest pusty".into(),
        Some(183) => "Taki element już istnieje".into(),
        Some(206) => "Za długa nazwa lub ścieżka".into(),
        _ => e.to_string(),
    }
}

// ---------- Wyszukiwanie ----------

pub static SEARCH_GEN: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Clone)]
pub struct SearchBatch {
    pub id: u64,
    pub items: Vec<Entry>,
    pub done: bool,
    pub scanned: u64,
}

fn wildcard(pat: &str, s: &str) -> bool {
    let (p, s): (Vec<char>, Vec<char>) = (pat.chars().collect(), s.chars().collect());
    let (mut pi, mut si, mut star, mut mark) = (0usize, 0usize, None, 0usize);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn matcher(query: &str) -> impl Fn(&str) -> bool {
    let q = query.trim().to_lowercase();
    let terms: Vec<String> = q.split_whitespace().map(|s| s.to_string()).collect();
    move |name: &str| {
        let n = name.to_lowercase();
        terms.iter().all(|t| {
            if t.contains('*') || t.contains('?') {
                wildcard(t, &n)
            } else {
                n.contains(t.as_str())
            }
        })
    }
}

pub fn search(app: AppHandle, root: String, query: String, include_hidden: bool) -> u64 {
    let id = SEARCH_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        let m = matcher(&query);
        let mut stack = vec![PathBuf::from(&root)];
        let mut batch = Vec::new();
        let mut last = Instant::now();
        let mut scanned = 0u64;
        let mut found = 0usize;
        while let Some(dir) = stack.pop() {
            if SEARCH_GEN.load(Ordering::SeqCst) != id {
                return;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for de in rd.flatten() {
                scanned += 1;
                let Ok(md) = de.metadata() else { continue };
                let attrs = md.file_attributes();
                let hidden = attrs & (ATTR_HIDDEN | ATTR_SYSTEM) != 0;
                if hidden && !include_hidden {
                    continue;
                }
                let name = de.file_name().to_string_lossy().to_string();
                if md.is_dir() && attrs & ATTR_REPARSE == 0 {
                    stack.push(de.path());
                }
                if m(&name) {
                    batch.push(entry_from(&de.path(), &md));
                    found += 1;
                }
            }
            if !batch.is_empty() && last.elapsed() > Duration::from_millis(120) {
                let _ = app.emit("search", SearchBatch { id, items: std::mem::take(&mut batch), done: false, scanned });
                last = Instant::now();
            }
            if found > 20000 {
                break;
            }
        }
        let _ = app.emit("search", SearchBatch { id, items: batch, done: true, scanned });
    });
    id
}

pub fn cancel_search() {
    SEARCH_GEN.fetch_add(1, Ordering::SeqCst);
}

// ---------- Rozmiar folderu / skrót ----------

#[derive(Serialize)]
pub struct DirSize {
    pub size: u64,
    pub files: u64,
    pub dirs: u64,
}

pub fn dir_size(path: &str) -> DirSize {
    let mut r = DirSize { size: 0, files: 0, dirs: 0 };
    let mut stack = vec![PathBuf::from(path)];
    let started = Instant::now();
    while let Some(d) = stack.pop() {
        if started.elapsed() > Duration::from_secs(120) {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for de in rd.flatten() {
            let Ok(md) = de.metadata() else { continue };
            if md.is_dir() {
                r.dirs += 1;
                if md.file_attributes() & ATTR_REPARSE == 0 {
                    stack.push(de.path());
                }
            } else {
                r.files += 1;
                r.size += md.len();
            }
        }
    }
    r
}

pub fn sha256(path: &str) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| io_err(&e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| io_err(&e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

// ---------- Podgląd zawartości ----------

#[derive(Serialize)]
pub struct TextPreview {
    pub text: String,
    pub truncated: bool,
    pub kind: String,
}

pub fn read_text(path: &str, limit: usize) -> Result<TextPreview, String> {
    let f = std::fs::File::open(path).map_err(|e| io_err(&e))?;
    let mut buf = Vec::new();
    f.take(limit as u64 + 1).read_to_end(&mut buf).map_err(|e| io_err(&e))?;
    let truncated = buf.len() > limit;
    buf.truncate(limit);
    let text = if buf.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = buf[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    } else {
        let b = buf.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&buf);
        match std::str::from_utf8(b) {
            Ok(s) => s.to_string(),
            Err(e) if e.valid_up_to() + 4 >= b.len() => String::from_utf8_lossy(&b[..e.valid_up_to()]).to_string(),
            Err(_) => b.iter().map(|&c| cp1250(c)).collect(),
        }
    };
    let binary = text.chars().take(4096).filter(|c| *c == '\0').count() > 4;
    Ok(TextPreview { text: if binary { String::new() } else { text }, truncated, kind: if binary { "binary".into() } else { "text".into() } })
}

fn cp1250(c: u8) -> char {
    const HI: &str = "€\u{81}‚\u{83}„…†‡\u{88}‰Š‹ŚŤŽŹ\u{90}‘’“”•–—\u{98}™š›śťžź ˇ˘Ł¤Ą¦§¨©Ş«¬\u{ad}®Ż°±˛ł´µ¶·¸ąş»Ľ˝ľżŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢßŕáâăäĺćçčéęëěíîďđńňóôőö÷řůúűüýţ˙";
    if c < 0x80 {
        c as char
    } else {
        HI.chars().nth((c - 0x80) as usize).unwrap_or('?')
    }
}

/// Tekst z dokumentów Office/OpenDocument (docx, xlsx, pptx, odt, ods, odp, epub).
pub fn office_text(path: &str, limit: usize) -> Result<TextPreview, String> {
    let f = std::fs::File::open(path).map_err(|e| io_err(&e))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|_| "To nie jest prawidłowy dokument".to_string())?;
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut names: Vec<String> = match ext.as_str() {
        "docx" | "docm" | "dotx" => vec!["word/document.xml".into()],
        "xlsx" | "xlsm" => vec!["xl/sharedStrings.xml".into()],
        "pptx" | "ppsx" => {
            let mut v: Vec<String> = z.file_names().filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml")).map(|s| s.to_string()).collect();
            v.sort_by_key(|n| n.trim_start_matches("ppt/slides/slide").trim_end_matches(".xml").parse::<u32>().unwrap_or(0));
            v
        }
        "epub" => {
            let mut v: Vec<String> = z.file_names().filter(|n| n.ends_with(".xhtml") || n.ends_with(".html")).map(|s| s.to_string()).collect();
            v.sort();
            v
        }
        _ => vec!["content.xml".into()],
    };
    names.truncate(200);
    let mut text = String::new();
    for n in names {
        let Ok(mut file) = z.by_name(&n) else { continue };
        let mut xml = String::new();
        if file.read_to_string(&mut xml).is_err() {
            continue;
        }
        xml_text(&xml, &mut text, limit);
        if ext == "pptx" {
            text.push_str("\n\n―――\n\n");
        }
        if text.len() >= limit {
            break;
        }
    }
    let truncated = text.len() >= limit;
    Ok(TextPreview { text, truncated, kind: "document".into() })
}

fn xml_text(xml: &str, out: &mut String, limit: usize) {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    loop {
        match r.read_event() {
            Ok(Event::Text(t)) => {
                if let Ok(s) = t.unescape() {
                    out.push_str(&s);
                }
            }
            Ok(Event::End(e)) => {
                let n = e.local_name();
                let n = n.as_ref();
                if matches!(n, b"p" | b"si" | b"h" | b"tr" | b"br" | b"div" | b"li" | b"h1" | b"h2" | b"h3") {
                    out.push('\n');
                } else if matches!(n, b"tc" | b"table-cell") {
                    out.push('\t');
                }
            }
            Ok(Event::Empty(e)) => {
                let n = e.local_name();
                match n.as_ref() {
                    b"tab" => out.push('\t'),
                    b"br" | b"line-break" => out.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        if out.len() >= limit {
            break;
        }
    }
}

/// Spis zawartości archiwum ZIP bez rozpakowywania.
#[derive(Serialize)]
pub struct ZipItem {
    pub name: String,
    pub size: u64,
    pub packed: u64,
    pub dir: bool,
}

pub fn zip_list(path: &str) -> Result<Vec<ZipItem>, String> {
    let f = std::fs::File::open(path).map_err(|e| io_err(&e))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|_| "Nieobsługiwane archiwum".to_string())?;
    let mut out = Vec::new();
    for i in 0..z.len().min(5000) {
        if let Ok(f) = z.by_index_raw(i) {
            out.push(ZipItem { name: f.name().to_string(), size: f.size(), packed: f.compressed_size(), dir: f.is_dir() });
        }
    }
    Ok(out)
}

// ---------- Obserwacja zmian ----------

struct WatchState {
    watcher: Option<RecommendedWatcher>,
    watched: HashSet<PathBuf>,
}

static WATCH: Lazy<Mutex<WatchState>> = Lazy::new(|| Mutex::new(WatchState { watcher: None, watched: HashSet::new() }));
static PENDING: Lazy<Mutex<(HashSet<String>, Option<Instant>)>> = Lazy::new(|| Mutex::new((HashSet::new(), None)));

pub fn init_watcher(app: AppHandle) {
    let app2 = app.clone();
    let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            let mut p = PENDING.lock();
            for path in ev.paths {
                crate::shell::invalidate(&path.to_string_lossy());
                if let Some(parent) = path.parent() {
                    p.0.insert(parent.to_string_lossy().to_string());
                }
            }
            p.1.get_or_insert_with(Instant::now);
        }
    })
    .ok();
    WATCH.lock().watcher = w;
    // Zdarzenia zbieramy i wysyłamy paczką co 250 ms – kopiowanie tysięcy plików nie zalewa UI.
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(250));
        let dirs: Vec<String> = {
            let mut p = PENDING.lock();
            match p.1 {
                Some(t) if t.elapsed() > Duration::from_millis(200) => {
                    p.1 = None;
                    p.0.drain().collect()
                }
                _ => continue,
            }
        };
        if !dirs.is_empty() {
            let _ = app2.emit("fs-changed", dirs);
        }
    });
    let _ = app;
}

pub fn watch_dirs(paths: Vec<String>) {
    let mut st = WATCH.lock();
    let want: HashSet<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let old = std::mem::take(&mut st.watched);
    let Some(w) = st.watcher.as_mut() else { return };
    for p in old.difference(&want) {
        let _ = w.unwatch(p);
    }
    let mut now = HashSet::new();
    for p in want {
        if old.contains(&p) || w.watch(&p, RecursiveMode::NonRecursive).is_ok() {
            now.insert(p);
        }
    }
    st.watched = now;
}

/// Ostatnio zmienione pliki z podanych folderów (bez zagłębiania się dalej niż 2 poziomy).
pub fn recent_files(dirs: &[String], days: u64, limit: usize) -> Vec<Entry> {
    let since = SystemTime::now() - Duration::from_secs(days * 86_400);
    let mut out = Vec::new();
    fn walk(p: &Path, depth: u32, since: SystemTime, out: &mut Vec<Entry>) {
        let Ok(rd) = std::fs::read_dir(p) else { return };
        for de in rd.flatten() {
            let Ok(md) = de.metadata() else { continue };
            let attrs = md.file_attributes();
            if attrs & (ATTR_HIDDEN | ATTR_SYSTEM) != 0 {
                continue;
            }
            if md.is_dir() {
                if depth > 0 && attrs & ATTR_REPARSE == 0 {
                    walk(&de.path(), depth - 1, since, out);
                }
            } else if md.modified().map(|t| t >= since).unwrap_or(false) {
                let name = de.file_name().to_string_lossy().to_lowercase();
                if name == "desktop.ini" || name.ends_with(".lnk") || name.ends_with(".tmp") || name.starts_with("~$") {
                    continue;
                }
                out.push(entry_from(&de.path(), &md));
            }
        }
    }
    for d in dirs {
        walk(Path::new(d), 1, since, &mut out);
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.dedup_by(|a, b| a.path == b.path);
    out.truncate(limit);
    out
}
