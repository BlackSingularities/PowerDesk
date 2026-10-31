//! Logika synchronizacji niezależna od UI: skanowanie, planowanie, wykonanie akcji.

use std::collections::{BTreeMap, BTreeSet};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Cel = dokładna kopia źródła (usunięcia też są przenoszone).
    Mirror,
    /// Zmiany z obu stron, konflikty zachowują obie wersje.
    TwoWay,
    /// Tylko dopisywanie i aktualizacja – nic nie jest usuwane z celu.
    Backup,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub dir: bool,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileState {
    pub dir: bool,
    pub size: u64,
    /// Czas modyfikacji po stronie źródła i celu przy ostatniej udanej synchronizacji.
    pub ms: i64,
    pub mt: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    MkdirTarget(String),
    MkdirSource(String),
    CopyToTarget(String, u64),
    CopyToSource(String, u64),
    DeleteTarget(String),
    DeleteSource(String),
    Conflict { rel: String, source_wins: bool, size: u64 },
}

impl Action {
    pub fn rel(&self) -> Option<&str> {
        Some(match self {
            Action::MkdirTarget(r) | Action::MkdirSource(r) | Action::CopyToTarget(r, _) | Action::CopyToSource(r, _) | Action::DeleteTarget(r) | Action::DeleteSource(r) => r,
            Action::Conflict { rel, .. } => rel,
        })
    }
    pub fn label(&self) -> String {
        match self {
            Action::MkdirTarget(r) => format!("+ folder → {r}"),
            Action::MkdirSource(r) => format!("+ folder ← {r}"),
            Action::CopyToTarget(r, _) => format!("→ {r}"),
            Action::CopyToSource(r, _) => format!("← {r}"),
            Action::DeleteTarget(r) => format!("✕ cel: {r}"),
            Action::DeleteSource(r) => format!("✕ źródło: {r}"),
            Action::Conflict { rel, source_wins, .. } => format!("⚠ konflikt: {rel} (wygrywa {})", if *source_wins { "źródło" } else { "cel" }),
        }
    }
}

pub struct Plan {
    pub actions: Vec<Action>,
    /// Stan bazowy dla elementów, które już są zgodne.
    pub new_base: BTreeMap<String, FileState>,
}

pub fn norm(p: &str) -> String {
    p.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

pub struct Excludes {
    names: Vec<String>,
    paths: Vec<String>,
}

const DEFAULT_EXCLUDES: &[&str] = &[
    ".ofp-versions",
    "*.ofp-part",
    "thumbs.db",
    "desktop.ini",
    "~$*",
    ".~lock.*",
    "$recycle.bin",
    "system volume information",
    "*.tmp",
    ".ds_store",
];

impl Excludes {
    pub fn new(user: &[String]) -> Self {
        let mut names = Vec::new();
        let mut paths = Vec::new();
        for p in DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).chain(user.iter().map(|s| s.trim().to_lowercase())) {
            if p.is_empty() {
                continue;
            }
            if p.contains('\\') || p.contains('/') {
                paths.push(norm(&p).trim_start_matches('\\').to_string());
            } else {
                names.push(p);
            }
        }
        Excludes { names, paths }
    }
    pub fn excluded(&self, rel: &str, name: &str) -> bool {
        let n = name.to_lowercase();
        if self.names.iter().any(|p| wild(p, &n)) {
            return true;
        }
        let r = rel.to_lowercase();
        self.paths.iter().any(|p| r == *p || r.starts_with(&format!("{p}\\")) || wild(p, &r))
    }
}

fn wild(p: &str, s: &str) -> bool {
    if !p.contains('*') && !p.contains('?') {
        return p == s;
    }
    let (p, s): (Vec<char>, Vec<char>) = (p.chars().collect(), s.chars().collect());
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

fn mtime(md: &std::fs::Metadata) -> i64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn scan(root: &Path, ex: &Excludes, hidden: bool, cancel: &AtomicBool) -> std::io::Result<BTreeMap<String, Item>> {
    if !root.is_dir() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "folder nie istnieje"));
    }
    // Sprawdzenie odczytu – odłączony dysk ma zwrócić błąd, a nie „pusty folder".
    std::fs::read_dir(root)?;
    let mut out = BTreeMap::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for de in rd.flatten() {
            let name = de.file_name().to_string_lossy().to_string();
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}\\{name}") };
            if ex.excluded(&rel, &name) {
                continue;
            }
            let Ok(md) = de.metadata() else { continue };
            let attrs = md.file_attributes();
            if !hidden && attrs & 0x6 != 0 {
                continue;
            }
            if md.is_dir() {
                out.insert(rel.clone(), Item { dir: true, size: 0, mtime: 0 });
                if attrs & 0x400 == 0 {
                    stack.push((de.path(), rel));
                }
            } else {
                out.insert(rel, Item { dir: false, size: md.len(), mtime: mtime(&md) });
            }
        }
    }
    Ok(out)
}

/// FAT/exFAT zapisuje czas z dokładnością 2 s – tolerancja zapobiega wiecznemu kopiowaniu.
fn same_time(a: i64, b: i64) -> bool {
    (a - b).abs() <= 2000
}

fn same(a: &Item, b: &Item) -> bool {
    a.dir == b.dir && (a.dir || (a.size == b.size && same_time(a.mtime, b.mtime)))
}

fn matches_base(i: &Item, size: u64, t: i64, dir: bool) -> bool {
    i.dir == dir && (dir || (i.size == size && same_time(i.mtime, t)))
}

pub fn plan(mode: Mode, src: &BTreeMap<String, Item>, dst: &BTreeMap<String, Item>, base: &BTreeMap<String, FileState>) -> Plan {
    let keys: BTreeSet<&String> = src.keys().chain(dst.keys()).collect();
    let mut mk = Vec::new();
    let mut copies = Vec::new();
    let mut dels = Vec::new();
    let mut new_base = BTreeMap::new();
    let state = |s: &Item, d: &Item| FileState { dir: s.dir, size: s.size, ms: s.mtime, mt: d.mtime };

    // Usunięcie folderu po jednej stronie przenosimy tylko, gdy nic w nim nie zmieniono po drugiej.
    let mut blocked_src_dirs: BTreeSet<String> = BTreeSet::new();
    let mut blocked_dst_dirs: BTreeSet<String> = BTreeSet::new();

    for k in keys {
        let s = src.get(k);
        let d = dst.get(k);
        let b = base.get(k);
        match (s, d) {
            (Some(s), Some(d)) => {
                if same(s, d) {
                    new_base.insert(k.clone(), state(s, d));
                    continue;
                }
                if s.dir != d.dir {
                    // Plik zamienił się w folder (lub odwrotnie) – traktujemy źródło jako ważniejsze, chyba że TwoWay i zmienił się cel.
                    let t_changed = b.map(|b| !matches_base(d, b.size, b.mt, b.dir)).unwrap_or(true);
                    let s_changed = b.map(|b| !matches_base(s, b.size, b.ms, b.dir)).unwrap_or(true);
                    if mode == Mode::TwoWay && t_changed && !s_changed {
                        dels.push(Action::DeleteSource(k.clone()));
                        copies.push(if d.dir { Action::MkdirSource(k.clone()) } else { Action::CopyToSource(k.clone(), d.size) });
                    } else {
                        dels.push(Action::DeleteTarget(k.clone()));
                        copies.push(if s.dir { Action::MkdirTarget(k.clone()) } else { Action::CopyToTarget(k.clone(), s.size) });
                    }
                    continue;
                }
                match mode {
                    Mode::Mirror | Mode::Backup => copies.push(Action::CopyToTarget(k.clone(), s.size)),
                    Mode::TwoWay => {
                        let s_changed = b.map(|b| !matches_base(s, b.size, b.ms, false)).unwrap_or(true);
                        let t_changed = b.map(|b| !matches_base(d, b.size, b.mt, false)).unwrap_or(true);
                        match (s_changed, t_changed) {
                            (true, false) => copies.push(Action::CopyToTarget(k.clone(), s.size)),
                            (false, true) => copies.push(Action::CopyToSource(k.clone(), d.size)),
                            _ => {
                                let source_wins = s.mtime >= d.mtime;
                                copies.push(Action::Conflict { rel: k.clone(), source_wins, size: if source_wins { s.size } else { d.size } });
                            }
                        }
                    }
                }
            }
            (Some(s), None) => {
                let deleted_on_target = mode == Mode::TwoWay && b.is_some();
                if deleted_on_target {
                    let b = b.unwrap();
                    let s_changed = !matches_base(s, b.size, b.ms, b.dir);
                    if s_changed && !s.dir {
                        copies.push(Action::CopyToTarget(k.clone(), s.size));
                        block_parents(k, &mut blocked_src_dirs);
                    } else {
                        dels.push(Action::DeleteSource(k.clone()));
                    }
                } else {
                    copies.push(if s.dir { Action::MkdirTarget(k.clone()) } else { Action::CopyToTarget(k.clone(), s.size) });
                    if !s.dir {
                        block_parents(k, &mut blocked_src_dirs);
                    }
                }
            }
            (None, Some(d)) => match mode {
                Mode::Backup => {}
                Mode::Mirror => dels.push(Action::DeleteTarget(k.clone())),
                Mode::TwoWay => {
                    if let Some(b) = b {
                        let t_changed = !matches_base(d, b.size, b.mt, b.dir);
                        if t_changed && !d.dir {
                            copies.push(Action::CopyToSource(k.clone(), d.size));
                            block_parents(k, &mut blocked_dst_dirs);
                        } else {
                            dels.push(Action::DeleteTarget(k.clone()));
                        }
                    } else {
                        copies.push(if d.dir { Action::MkdirSource(k.clone()) } else { Action::CopyToSource(k.clone(), d.size) });
                        if !d.dir {
                            block_parents(k, &mut blocked_dst_dirs);
                        }
                    }
                }
            },
            (None, None) => {}
        }
    }
    // Folder, do którego właśnie coś kopiujemy, nie może zostać usunięty – trzeba go odtworzyć.
    let mut final_dels = Vec::new();
    for a in dels {
        match &a {
            Action::DeleteSource(r) if blocked_src_dirs.contains(r) => mk.push(Action::MkdirTarget(r.clone())),
            Action::DeleteTarget(r) if blocked_dst_dirs.contains(r) => mk.push(Action::MkdirSource(r.clone())),
            _ => final_dels.push(a),
        }
    }
    // Kolejność: foldery → pliki → usuwanie plików → usuwanie folderów (od najgłębszych).
    let (mut mkdirs, files): (Vec<Action>, Vec<Action>) = copies.into_iter().partition(|a| matches!(a, Action::MkdirTarget(_) | Action::MkdirSource(_)));
    mkdirs.extend(mk);
    mkdirs.sort_by(|a, b| a.rel().cmp(&b.rel()));
    final_dels.sort_by(|a, b| {
        let da = a.rel().map(|r| r.matches('\\').count()).unwrap_or(0);
        let db = b.rel().map(|r| r.matches('\\').count()).unwrap_or(0);
        db.cmp(&da).then_with(|| b.rel().cmp(&a.rel()))
    });
    let mut actions = mkdirs;
    actions.extend(files);
    actions.extend(final_dels);
    Plan { actions, new_base }
}

fn block_parents(rel: &str, set: &mut BTreeSet<String>) {
    let mut p = rel;
    while let Some(i) = p.rfind('\\') {
        p = &p[..i];
        set.insert(p.to_string());
    }
}

pub struct ExecCtx {
    pub source: PathBuf,
    pub target: PathBuf,
    pub versioning: bool,
    pub stamp: String,
}

fn conflict_name(p: &Path, stamp: &str) -> PathBuf {
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
    p.with_file_name(format!("{stem} (konflikt {host} {stamp}){ext}"))
}

/// Stary plik w celu nie znika: trafia do .ofp-versions\<data>\… (gdy włączone wersjonowanie).
fn retire_target(ctx: &ExecCtx, rel: &str) -> std::io::Result<()> {
    let p = ctx.target.join(rel);
    let Ok(md) = std::fs::symlink_metadata(&p) else { return Ok(()) };
    if md.is_dir() {
        return match std::fs::remove_dir(&p) {
            Ok(_) => Ok(()),
            Err(e) if e.raw_os_error() == Some(145) => Ok(()),
            Err(e) => Err(e),
        };
    }
    clear_ro(&p, &md);
    if ctx.versioning {
        let v = ctx.target.join(".ofp-versions").join(&ctx.stamp).join(rel);
        if let Some(parent) = v.parent() {
            std::fs::create_dir_all(parent)?;
        }
        hide_versions(&ctx.target);
        if std::fs::rename(&p, &v).is_ok() {
            return Ok(());
        }
    }
    std::fs::remove_file(&p)
}

/// Po stronie źródła (dysk lokalny) używamy systemowego Kosza.
fn retire_source(ctx: &ExecCtx, rel: &str) -> std::io::Result<()> {
    let p = ctx.source.join(rel);
    let Ok(md) = std::fs::symlink_metadata(&p) else { return Ok(()) };
    if md.is_dir() {
        return match std::fs::remove_dir(&p) {
            Ok(_) => Ok(()),
            Err(e) if e.raw_os_error() == Some(145) => Ok(()),
            Err(e) => Err(e),
        };
    }
    if trash::delete(&p).is_ok() {
        return Ok(());
    }
    clear_ro(&p, &md);
    std::fs::remove_file(&p)
}

fn hide_versions(target: &Path) {
    let v = target.join(".ofp-versions");
    if let Ok(md) = std::fs::metadata(&v) {
        if md.file_attributes() & 0x2 == 0 {
            let _ = std::process::Command::new("attrib").arg("+h").arg(&v).status();
        }
    }
}

fn clear_ro(p: &Path, md: &std::fs::Metadata) {
    let mut perm = md.permissions();
    if perm.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = std::fs::set_permissions(p, perm);
    }
}

fn copy_atomic(from: &Path, to: &Path) -> std::io::Result<std::fs::Metadata> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = to.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = to.with_file_name(format!(".{name}.ofp-part"));
    let res = (|| {
        std::fs::copy(from, &tmp)?;
        let md = std::fs::metadata(from)?;
        if let Ok(m) = md.modified() {
            let f = std::fs::OpenOptions::new().write(true).open(&tmp)?;
            f.set_modified(m)?;
        }
        Ok::<_, std::io::Error>(md)
    })();
    match res {
        Ok(md) => {
            std::fs::rename(&tmp, to)?;
            Ok(md)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn state_after(ctx: &ExecCtx, rel: &str) -> std::io::Result<FileState> {
    let s = std::fs::metadata(ctx.source.join(rel))?;
    let t = std::fs::metadata(ctx.target.join(rel))?;
    Ok(FileState { dir: s.is_dir(), size: s.len(), ms: mtime(&s), mt: mtime(&t) })
}

pub fn execute(ctx: &ExecCtx, a: &Action) -> std::io::Result<Option<(String, FileState)>> {
    match a {
        Action::MkdirTarget(r) => {
            std::fs::create_dir_all(ctx.target.join(r))?;
            std::fs::create_dir_all(ctx.source.join(r))?;
            Ok(Some((r.clone(), state_after(ctx, r)?)))
        }
        Action::MkdirSource(r) => {
            std::fs::create_dir_all(ctx.source.join(r))?;
            std::fs::create_dir_all(ctx.target.join(r))?;
            Ok(Some((r.clone(), state_after(ctx, r)?)))
        }
        Action::CopyToTarget(r, _) => {
            let dst = ctx.target.join(r);
            if dst.exists() {
                retire_target(ctx, r)?;
            }
            copy_atomic(&ctx.source.join(r), &dst)?;
            Ok(Some((r.clone(), state_after(ctx, r)?)))
        }
        Action::CopyToSource(r, _) => {
            let dst = ctx.source.join(r);
            if dst.exists() {
                retire_source(ctx, r)?;
            }
            copy_atomic(&ctx.target.join(r), &dst)?;
            Ok(Some((r.clone(), state_after(ctx, r)?)))
        }
        Action::DeleteTarget(r) => {
            retire_target(ctx, r)?;
            Ok(None)
        }
        Action::DeleteSource(r) => {
            retire_source(ctx, r)?;
            Ok(None)
        }
        Action::Conflict { rel, source_wins, .. } => {
            // Przegrana wersja zostaje obok pod nazwą „(konflikt …)" i przy następnym przebiegu trafi na drugą stronę.
            let (loser_root, winner_root) = if *source_wins { (&ctx.target, &ctx.source) } else { (&ctx.source, &ctx.target) };
            let loser = loser_root.join(rel);
            let keep = conflict_name(&loser, &ctx.stamp);
            std::fs::rename(&loser, &keep)?;
            copy_atomic(&winner_root.join(rel), &loser)?;
            Ok(Some((rel.clone(), state_after(ctx, rel)?)))
        }
    }
}

pub fn save_base(path: &Path, base: &BTreeMap<String, FileState>) {
    if let Ok(s) = serde_json::to_string(base) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}
