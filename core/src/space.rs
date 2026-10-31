//! Zajętość dysku (drzewo do mapy kafelkowej) i bezpieczne porządki: puste foldery i śmieci.

use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use crate::events::{AppHandle, Emitter};

const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_REPARSE: u32 = 0x400;

static GEN: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Clone)]
pub struct Node {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub files: u64,
    pub dir: bool,
    pub children: Vec<Node>,
}

#[derive(Serialize, Clone)]
struct ScanProgress {
    files: u64,
    bytes: u64,
    current: String,
}

struct Ctx<'a> {
    app: &'a AppHandle,
    id: u64,
    files: u64,
    bytes: u64,
    last: Instant,
}

/// Rekurencyjnie liczy rozmiary. Zachowuje szczegóły do `depth` poziomów, głębiej tylko sumy.
fn walk(p: &Path, depth: u32, ctx: &mut Ctx) -> Option<Node> {
    if GEN.load(Ordering::SeqCst) != ctx.id {
        return None;
    }
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string());
    let mut node = Node { name, path: p.to_string_lossy().to_string(), size: 0, files: 0, dir: true, children: Vec::new() };
    let Ok(rd) = std::fs::read_dir(p) else { return Some(node) };
    let mut small_files = 0u64;
    let mut small_count = 0u64;
    for de in rd.flatten() {
        let Ok(md) = de.metadata() else { continue };
        let attrs = md.file_attributes();
        if md.is_dir() {
            if attrs & ATTR_REPARSE != 0 {
                continue;
            }
            if let Some(c) = walk(&de.path(), depth.saturating_sub(1), ctx) {
                node.size += c.size;
                node.files += c.files;
                if depth > 0 {
                    node.children.push(c);
                }
            } else {
                return None;
            }
        } else {
            let len = md.len();
            node.size += len;
            node.files += 1;
            ctx.files += 1;
            ctx.bytes += len;
            // Duże pliki pokazujemy osobno, drobnicę zbieramy w jeden kafelek.
            if depth > 0 && len >= 1 << 20 {
                node.children.push(Node { name: de.file_name().to_string_lossy().to_string(), path: de.path().to_string_lossy().to_string(), size: len, files: 1, dir: false, children: Vec::new() });
            } else {
                small_files += len;
                small_count += 1;
            }
        }
    }
    if depth > 0 && small_count > 0 {
        node.children.push(Node { name: crate::trf!("{small_count} mniejszych plików"), path: String::new(), size: small_files, files: small_count, dir: false, children: Vec::new() });
    }
    node.children.sort_by(|a, b| b.size.cmp(&a.size));
    node.children.truncate(80);
    if ctx.last.elapsed() > Duration::from_millis(200) {
        let _ = ctx.app.emit("space-progress", ScanProgress { files: ctx.files, bytes: ctx.bytes, current: node.path.clone() });
        ctx.last = Instant::now();
    }
    Some(node)
}

pub fn scan(app: &AppHandle, root: &str) -> Result<Node, String> {
    let id = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let mut ctx = Ctx { app, id, files: 0, bytes: 0, last: Instant::now() };
    walk(Path::new(root), 4, &mut ctx).ok_or_else(|| crate::lang::tr("Przerwano").to_string())
}

pub fn cancel() {
    GEN.fetch_add(1, Ordering::SeqCst);
}

// ---------- Porządki ----------

#[derive(Serialize, Clone)]
pub struct Junk {
    pub path: String,
    pub size: u64,
    pub kind: &'static str,
    pub reason: String,
    pub modified: i64,
}

/// Foldery, do których nie zaglądamy: systemowe, programów i repozytoriów – tam „puste” często coś znaczy.
fn skip_dir(name: &str, attrs: u32) -> bool {
    let n = name.to_lowercase();
    attrs & (ATTR_SYSTEM | ATTR_REPARSE) != 0
        || matches!(n.as_str(), "windows" | "program files" | "program files (x86)" | "programdata" | "$recycle.bin" | "system volume information" | "appdata" | ".git" | ".svn" | ".hg" | "node_modules" | ".venv" | "venv" | "target" | ".idea" | ".vscode")
}

fn age_days(md: &std::fs::Metadata) -> u64 {
    md.modified().ok().and_then(|t| SystemTime::now().duration_since(t).ok()).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

fn ms(md: &std::fs::Metadata) -> i64 {
    md.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Czy plik to pewny śmieć (i dlaczego). Tylko wzorce, które na pewno da się odtworzyć albo są pozostałością.
fn junk_reason(name: &str, md: &std::fs::Metadata) -> Option<String> {
    let n = name.to_lowercase();
    let days = age_days(md);
    if n == "thumbs.db" || n == "ehthumbs.db" {
        return Some(crate::lang::tr("Pamięć podręczna miniatur Windows – odtworzy się sama").into());
    }
    if n == ".ds_store" || (n.starts_with("._") && md.len() <= 4096) {
        return Some(crate::lang::tr("Pozostałość po macOS").into());
    }
    if n.starts_with("~$") && md.len() < 1024 && days >= 1 {
        return Some(crate::lang::tr("Plik blokady Office po zamkniętym dokumencie").into());
    }
    if (n.ends_with(".tmp") || n.ends_with(".temp")) && days >= 7 {
        return Some(crate::trf!("Plik tymczasowy sprzed {days} dni"));
    }
    if (n.ends_with(".crdownload") || n.ends_with(".part") || n.ends_with(".partial") || n.ends_with(".download")) && days >= 7 {
        return Some(crate::trf!("Nieukończone pobieranie sprzed {days} dni"));
    }
    if md.len() == 0 && days >= 30 && !n.starts_with('.') && !n.ends_with(".ini") && !n.ends_with(".gitkeep") && n != "__init__.py" {
        return Some(crate::trf!("Pusty plik (0 B) sprzed {days} dni"));
    }
    None
}

/// Zwraca true, jeśli folder (po pominięciu śmieci) jest pusty.
fn find(p: &Path, depth: u32, out: &mut Vec<Junk>, id: u64, deadline: Instant) -> bool {
    if GEN.load(Ordering::SeqCst) != id || Instant::now() > deadline {
        return false;
    }
    let Ok(rd) = std::fs::read_dir(p) else { return false };
    let mut empty = true;
    for de in rd.flatten() {
        let Ok(md) = de.metadata() else {
            empty = false;
            continue;
        };
        let name = de.file_name().to_string_lossy().to_string();
        let attrs = md.file_attributes();
        if md.is_dir() {
            if skip_dir(&name, attrs) || attrs & ATTR_HIDDEN != 0 {
                empty = false;
                continue;
            }
            let child_empty = find(&de.path(), depth + 1, out, id, deadline);
            if child_empty && age_days(&md) >= 1 {
                // Zgłaszamy tylko najwyższy pusty folder – jego podfoldery znikną razem z nim.
                let path = de.path().to_string_lossy().to_string();
                let pre = format!("{path}\\");
                out.retain(|j| !(j.kind == "empty" && j.path.starts_with(&pre)));
                out.push(Junk { path, size: 0, kind: "empty", reason: crate::lang::tr("Pusty folder (także w podfolderach)").into(), modified: ms(&md) });
            } else {
                empty = false;
            }
        } else if let Some(reason) = junk_reason(&name, &md) {
            out.push(Junk { path: de.path().to_string_lossy().to_string(), size: md.len(), kind: "junk", reason, modified: ms(&md) });
        } else {
            empty = false;
        }
    }
    empty && depth > 0
}

pub fn cleanup_scan(root: &str, include_temp: bool) -> Vec<Junk> {
    let id = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut out = Vec::new();
    find(Path::new(root), 0, &mut out, id, deadline);
    if include_temp {
        // Folder TEMP użytkownika: tylko elementy nieruszane od tygodnia (te używane teraz zostają).
        let temp: PathBuf = std::env::temp_dir();
        if let Ok(rd) = std::fs::read_dir(&temp) {
            for de in rd.flatten() {
                let Ok(md) = de.metadata() else { continue };
                if age_days(&md) < 7 || de.file_name().to_string_lossy().eq_ignore_ascii_case("OpenFilePower") {
                    continue;
                }
                let size = if md.is_dir() { crate::fsx::dir_size(&de.path().to_string_lossy()).size } else { md.len() };
                out.push(Junk { path: de.path().to_string_lossy().to_string(), size, kind: "temp", reason: crate::trf!("Folder TEMP – nieużywane od {} dni", age_days(&md)), modified: ms(&md) });
            }
        }
    }
    out
}
