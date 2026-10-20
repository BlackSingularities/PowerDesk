//! Operacje na plikach w tle: kopiowanie, przenoszenie, usuwanie (kosz lub trwale).
//! Postęp, prędkość, anulowanie, rozwiązywanie konfliktów i cofanie (Ctrl+Z).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::fsx::io_err;

#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    Replace,
    Skip,
    Rename,
    /// Zastąp tylko, gdy źródło jest nowsze.
    Newer,
}

#[derive(Serialize, Clone)]
pub struct Progress {
    pub id: u64,
    pub kind: String,
    pub title: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub done_items: u64,
    pub total_items: u64,
    pub current: String,
    pub speed: f64,
    pub state: String, // running | done | error | cancelled
    pub errors: Vec<String>,
    pub dest: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Undo {
    Moved(Vec<(String, String)>),
    Created(Vec<String>),
    Renamed(String, String),
}

static NEXT: AtomicU64 = AtomicU64::new(1);
static CANCEL: Lazy<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static UNDO: Lazy<Mutex<Vec<Undo>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub fn push_undo(u: Undo) {
    let mut s = UNDO.lock();
    s.push(u);
    if s.len() > 50 {
        s.remove(0);
    }
}

pub fn cancel(id: u64) {
    if let Some(f) = CANCEL.lock().get(&id) {
        f.store(true, Ordering::SeqCst);
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Wolna nazwa w stylu Eksploratora: „plik (2).txt".
pub fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let parent = p.parent().unwrap_or(Path::new(""));
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let is_dir = p.is_dir();
    for i in 2..10000 {
        let n = if is_dir { format!("{}{} ({i})", stem, ext) } else { format!("{stem} ({i}){ext}") };
        let c = parent.join(n);
        if !c.exists() {
            return c;
        }
    }
    p.to_path_buf()
}

#[derive(Serialize)]
pub struct ConflictInfo {
    pub name: String,
    pub src: String,
    pub dst: String,
    pub src_size: u64,
    pub dst_size: u64,
    pub src_modified: i64,
    pub dst_modified: i64,
    pub dir: bool,
}

pub fn check_conflicts(sources: &[String], dest: &str) -> Vec<ConflictInfo> {
    let mut out = Vec::new();
    for s in sources {
        let sp = Path::new(s);
        let dp = Path::new(dest).join(file_name(sp));
        if sp == dp {
            continue;
        }
        if let (Ok(a), Ok(b)) = (std::fs::metadata(sp), std::fs::metadata(&dp)) {
            let ea = crate::fsx::entry_from(sp, &a);
            let eb = crate::fsx::entry_from(&dp, &b);
            out.push(ConflictInfo {
                name: ea.name.clone(),
                src: s.clone(),
                dst: eb.path,
                src_size: ea.size,
                dst_size: eb.size,
                src_modified: ea.modified,
                dst_modified: eb.modified,
                dir: ea.dir,
            });
        }
    }
    out
}

struct Ctx {
    app: AppHandle,
    p: Progress,
    cancel: Arc<AtomicBool>,
    last_emit: Instant,
    started: Instant,
    conflict: Conflict,
    undo: Vec<(String, String)>,
    created: Vec<String>,
}

impl Ctx {
    fn emit(&mut self, force: bool) {
        if force || self.last_emit.elapsed() > Duration::from_millis(120) {
            let secs = self.started.elapsed().as_secs_f64().max(0.001);
            self.p.speed = self.p.done_bytes as f64 / secs;
            let _ = self.app.emit("op-progress", self.p.clone());
            self.last_emit = Instant::now();
        }
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    fn err(&mut self, msg: String) {
        if self.p.errors.len() < 200 {
            self.p.errors.push(msg);
        }
    }
}

fn measure(paths: &[PathBuf]) -> (u64, u64) {
    let (mut bytes, mut items) = (0u64, 0u64);
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = stack.pop() {
        let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
        items += 1;
        if md.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|d| d.path()));
            }
        } else {
            bytes += md.len();
        }
    }
    (bytes, items)
}

fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| -> String {
        let s = p.to_string_lossy().to_lowercase();
        if let Some(rest) = s.strip_prefix(r"\\") {
            let parts: Vec<&str> = rest.split('\\').take(2).collect();
            parts.join("\\")
        } else {
            s.chars().take(2).collect()
        }
    };
    root(a) == root(b)
}

pub fn start(app: AppHandle, kind: String, sources: Vec<String>, dest: String, conflict: Conflict) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let flag = Arc::new(AtomicBool::new(false));
    CANCEL.lock().insert(id, flag.clone());
    std::thread::spawn(move || {
        let srcs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
        let title = match kind.as_str() {
            "copy" => format!("Kopiowanie {} elem.", srcs.len()),
            "move" => format!("Przenoszenie {} elem.", srcs.len()),
            "trash" => format!("Do kosza: {} elem.", srcs.len()),
            _ => format!("Usuwanie {} elem.", srcs.len()),
        };
        let mut ctx = Ctx {
            app: app.clone(),
            p: Progress {
                id,
                kind: kind.clone(),
                title,
                done_bytes: 0,
                total_bytes: 0,
                done_items: 0,
                total_items: srcs.len() as u64,
                current: String::new(),
                speed: 0.0,
                state: "running".into(),
                errors: vec![],
                dest: dest.clone(),
            },
            cancel: flag,
            last_emit: Instant::now() - Duration::from_secs(1),
            started: Instant::now(),
            conflict,
            undo: vec![],
            created: vec![],
        };
        ctx.emit(true);
        match kind.as_str() {
            "trash" => {
                for s in &srcs {
                    if ctx.cancelled() {
                        break;
                    }
                    ctx.p.current = file_name(s);
                    if let Err(e) = trash::delete(s) {
                        ctx.err(format!("{}: {e}", file_name(s)));
                    }
                    ctx.p.done_items += 1;
                    ctx.emit(false);
                }
            }
            "delete" => {
                let (b, n) = measure(&srcs);
                ctx.p.total_bytes = b;
                ctx.p.total_items = n;
                for s in &srcs {
                    if ctx.cancelled() {
                        break;
                    }
                    delete_tree(&mut ctx, s);
                }
            }
            "copy" | "move" => {
                let dst_dir = PathBuf::from(&dest);
                let is_move = kind == "move";
                let fast: Vec<bool> = srcs.iter().map(|s| is_move && same_volume(s, &dst_dir)).collect();
                let slow: Vec<PathBuf> = srcs.iter().zip(&fast).filter(|(_, f)| !**f).map(|(s, _)| s.clone()).collect();
                let (b, n) = measure(&slow);
                ctx.p.total_bytes = b;
                ctx.p.total_items = n + fast.iter().filter(|f| **f).count() as u64;
                for (s, f) in srcs.iter().zip(fast) {
                    if ctx.cancelled() {
                        break;
                    }
                    let mut target = dst_dir.join(file_name(s));
                    if target == *s {
                        if is_move {
                            continue;
                        }
                        target = unique_path(&target);
                    } else if dst_dir.starts_with(s) {
                        ctx.err(format!("{}: nie można skopiować folderu do jego wnętrza", file_name(s)));
                        continue;
                    }
                    if f {
                        // Ten sam wolumin: zmiana nazwy jest natychmiastowa.
                        if target.exists() {
                            match ctx.conflict {
                                Conflict::Skip => {
                                    ctx.p.done_items += 1;
                                    continue;
                                }
                                Conflict::Rename => target = unique_path(&target),
                                Conflict::Newer if !newer(s, &target) => {
                                    ctx.p.done_items += 1;
                                    continue;
                                }
                                _ => {
                                    if target.is_dir() && s.is_dir() {
                                        // Scalanie folderów – zwykła ścieżka kopiowania.
                                        copy_tree(&mut ctx, s, &target, true);
                                        continue;
                                    }
                                    let _ = trash::delete(&target);
                                }
                            }
                        }
                        ctx.p.current = file_name(s);
                        match std::fs::rename(s, &target) {
                            Ok(_) => ctx.undo.push((target.to_string_lossy().to_string(), s.to_string_lossy().to_string())),
                            Err(e) => ctx.err(format!("{}: {}", file_name(s), io_err(&e))),
                        }
                        ctx.p.done_items += 1;
                        ctx.emit(false);
                    } else {
                        let ok = copy_tree(&mut ctx, s, &target, is_move);
                        if ok && !is_move {
                            ctx.created.push(target.to_string_lossy().to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        if !ctx.undo.is_empty() {
            push_undo(Undo::Moved(std::mem::take(&mut ctx.undo)));
        }
        if !ctx.created.is_empty() {
            push_undo(Undo::Created(std::mem::take(&mut ctx.created)));
        }
        ctx.p.current.clear();
        ctx.p.state = if ctx.cancelled() {
            "cancelled".into()
        } else if ctx.p.errors.is_empty() {
            "done".into()
        } else {
            "error".into()
        };
        ctx.emit(true);
        CANCEL.lock().remove(&id);
    });
    id
}

fn newer(a: &Path, b: &Path) -> bool {
    let m = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (m(a), m(b)) {
        (Some(x), Some(y)) => x > y + Duration::from_secs(2),
        _ => true,
    }
}

fn delete_tree(ctx: &mut Ctx, p: &Path) {
    let Ok(md) = std::fs::symlink_metadata(p) else { return };
    ctx.p.current = file_name(p);
    if md.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for d in rd.flatten() {
                if ctx.cancelled() {
                    return;
                }
                delete_tree(ctx, &d.path());
            }
        }
        if let Err(e) = std::fs::remove_dir(p) {
            ctx.err(format!("{}: {}", file_name(p), io_err(&e)));
        }
    } else {
        clear_readonly(p, &md);
        match std::fs::remove_file(p) {
            Ok(_) => ctx.p.done_bytes += md.len(),
            Err(e) => ctx.err(format!("{}: {}", file_name(p), io_err(&e))),
        }
    }
    ctx.p.done_items += 1;
    ctx.emit(false);
}

fn clear_readonly(p: &Path, md: &std::fs::Metadata) {
    let mut perm = md.permissions();
    if perm.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = std::fs::set_permissions(p, perm);
    }
}

/// Zwraca true, gdy całe drzewo skopiowano bez błędów.
fn copy_tree(ctx: &mut Ctx, src: &Path, dst: &Path, remove_src: bool) -> bool {
    if ctx.cancelled() {
        return false;
    }
    let Ok(md) = std::fs::metadata(src) else {
        ctx.err(format!("{}: nie można odczytać", file_name(src)));
        return false;
    };
    if md.is_dir() {
        let mut dst = dst.to_path_buf();
        if dst.exists() && !dst.is_dir() {
            dst = unique_path(&dst);
        }
        if let Err(e) = std::fs::create_dir_all(&dst) {
            ctx.err(format!("{}: {}", file_name(&dst), io_err(&e)));
            return false;
        }
        ctx.p.done_items += 1;
        let mut ok = true;
        if let Ok(rd) = std::fs::read_dir(src) {
            let children: Vec<PathBuf> = rd.flatten().map(|d| d.path()).collect();
            for c in children {
                ok &= copy_tree(ctx, &c, &dst.join(file_name(&c)), remove_src);
            }
        }
        if ok && remove_src && !ctx.cancelled() {
            let _ = std::fs::remove_dir(src);
        }
        return ok;
    }
    let mut dst = dst.to_path_buf();
    if dst.exists() {
        match ctx.conflict {
            Conflict::Skip => {
                ctx.p.done_bytes += md.len();
                ctx.p.done_items += 1;
                return true;
            }
            Conflict::Rename => dst = unique_path(&dst),
            Conflict::Newer if !newer(src, &dst) => {
                ctx.p.done_bytes += md.len();
                ctx.p.done_items += 1;
                return true;
            }
            _ => {}
        }
    }
    ctx.p.current = file_name(src);
    match copy_file(ctx, src, &dst, &md) {
        Ok(_) => {
            ctx.p.done_items += 1;
            if remove_src {
                clear_readonly(src, &md);
                if let Err(e) = std::fs::remove_file(src) {
                    ctx.err(format!("{}: skopiowano, ale nie usunięto źródła ({})", file_name(src), io_err(&e)));
                }
            }
            true
        }
        Err(e) => {
            ctx.err(format!("{}: {e}", file_name(src)));
            false
        }
    }
}

/// Kopiowanie do pliku tymczasowego i atomowa zamiana – przerwane kopiowanie nie zostawia połówek.
fn copy_file(ctx: &mut Ctx, src: &Path, dst: &Path, md: &std::fs::Metadata) -> Result<(), String> {
    let tmp = dst.with_file_name(format!(".{}.ofp-part", file_name(dst)));
    let res = (|| -> Result<(), String> {
        let mut r = std::fs::File::open(src).map_err(|e| io_err(&e))?;
        let mut w = std::fs::File::create(&tmp).map_err(|e| io_err(&e))?;
        let _ = w.set_len(md.len());
        let mut buf = vec![0u8; 4 << 20];
        loop {
            if ctx.cancelled() {
                return Err("anulowano".into());
            }
            let n = r.read(&mut buf).map_err(|e| io_err(&e))?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n]).map_err(|e| io_err(&e))?;
            ctx.p.done_bytes += n as u64;
            ctx.emit(false);
        }
        if let Ok(m) = md.modified() {
            let _ = w.set_modified(m);
        }
        drop(w);
        if dst.exists() {
            if let Ok(dm) = std::fs::metadata(dst) {
                clear_readonly(dst, &dm);
            }
            std::fs::remove_file(dst).map_err(|e| io_err(&e))?;
        }
        std::fs::rename(&tmp, dst).map_err(|e| io_err(&e))?;
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

pub fn undo_last() -> Result<String, String> {
    let Some(u) = UNDO.lock().pop() else { return Err("Nie ma nic do cofnięcia".into()) };
    match u {
        Undo::Moved(pairs) => {
            let mut n = 0;
            for (now, was) in pairs.iter().rev() {
                if std::fs::rename(now, was).is_ok() {
                    n += 1;
                }
            }
            Ok(format!("Cofnięto przeniesienie ({n})"))
        }
        Undo::Created(paths) => {
            let mut n = 0;
            for p in &paths {
                if trash::delete(p).is_ok() {
                    n += 1;
                }
            }
            Ok(format!("Cofnięto utworzenie ({n}) – elementy są w koszu"))
        }
        Undo::Renamed(now, was) => {
            std::fs::rename(&now, &was).map_err(|e| io_err(&e))?;
            Ok("Cofnięto zmianę nazwy".into())
        }
    }
}
