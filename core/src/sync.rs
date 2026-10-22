//! Synchronizacja folderów z dyskiem zewnętrznym lub sieciowym.
//!
//! * Dysk zewnętrzny rozpoznajemy po numerze seryjnym woluminu, więc zmiana litery (E: → F:) nic nie psuje.
//! * Gdy cel jest podłączony – synchronizacja ciągła (obserwacja zmian + okresowe skanowanie).
//! * Gdy nie jest – zapamiętujemy, kiedy był widziany i ile zmian czeka.
//! * Tryby: kopia lustrzana, dwukierunkowa (z wykrywaniem konfliktów), kopia zapasowa (bez usuwania).
//! * Bezpieczeństwo: wersjonowanie nadpisanych/usuniętych plików, ochrona przed masowym usunięciem,
//!   atomowe zapisy, stan bazowy do odróżniania „usunięto" od „nowy plik".

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use crate::events::{AppHandle, Emitter};

use crate::drives;
use crate::sync_core::{self, Action, FileState, Mode, Plan};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Target {
    /// Pełna ścieżka w chwili konfiguracji (np. E:\Kopie\Zdjęcia albo \\nas\dane\Zdjęcia).
    pub path: String,
    /// Ścieżka względem katalogu głównego woluminu – do odnalezienia dysku pod inną literą.
    #[serde(default)]
    pub rel: String,
    #[serde(default)]
    pub serial: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub network: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub source: String,
    pub target: Target,
    pub mode: Mode,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub continuous: bool,
    /// Pełne skanowanie co N minut (wychwytuje zmiany po stronie celu, np. na NAS).
    #[serde(default = "default_interval")]
    pub interval_min: u32,
    #[serde(default = "yes")]
    pub versioning: bool,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub include_hidden: bool,
}

fn yes() -> bool {
    true
}
fn default_interval() -> u32 {
    15
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Status {
    pub id: String,
    pub connected: bool,
    pub resolved_target: String,
    pub last_seen: i64,
    pub last_sync: i64,
    pub last_result: String,
    pub last_counts: Counts,
    pub state: String, // idle | scanning | syncing | waiting | offline | paused | confirm | error
    pub pending: u64,
    pub progress: f32,
    pub current: String,
    pub needs_confirm: Option<Counts>,
    #[serde(default)]
    pub history: Vec<HistoryItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Counts {
    pub to_target: u64,
    pub to_source: u64,
    pub del_target: u64,
    pub del_source: u64,
    pub conflicts: u64,
    pub bytes: u64,
    pub errors: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistoryItem {
    pub at: i64,
    pub summary: String,
    pub ok: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    profiles: Vec<Profile>,
    status: HashMap<String, Status>,
}

struct Runtime {
    store: Store,
    dirty: HashSet<String>,
    running: HashSet<String>,
    confirmed: HashSet<String>,
    force: HashSet<String>,
    last_full: HashMap<String, Instant>,
    watchers: HashMap<String, (String, RecommendedWatcher)>,
    cancel: HashMap<String, Arc<AtomicBool>>,
}

static RT: Lazy<Mutex<Runtime>> = Lazy::new(|| {
    Mutex::new(Runtime {
        store: Store::default(),
        dirty: HashSet::new(),
        running: HashSet::new(),
        confirmed: HashSet::new(),
        force: HashSet::new(),
        last_full: HashMap::new(),
        watchers: HashMap::new(),
        cancel: HashMap::new(),
    })
});
static APP: Lazy<Mutex<Option<AppHandle>>> = Lazy::new(|| Mutex::new(None));
static LAST_EVENT: Lazy<Mutex<HashMap<String, Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn data_dir() -> PathBuf {
    let d = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("OpenFilePowerWin");
    let _ = std::fs::create_dir_all(d.join("sync"));
    d
}

fn store_path() -> PathBuf {
    data_dir().join("sync.json")
}

fn base_path(id: &str) -> PathBuf {
    data_dir().join("sync").join(format!("{id}.base.json"))
}

fn save(rt: &Runtime) {
    if let Ok(s) = serde_json::to_string_pretty(&rt.store) {
        let p = store_path();
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }
}

fn emit_status(st: &Status) {
    if let Some(app) = APP.lock().as_ref() {
        let _ = app.emit("sync-status", st.clone());
    }
}

fn update<F: FnOnce(&mut Status)>(id: &str, f: F) {
    let st = {
        let mut rt = RT.lock();
        let st = rt.store.status.entry(id.to_string()).or_insert_with(|| Status { id: id.to_string(), ..Default::default() });
        f(st);
        st.clone()
    };
    emit_status(&st);
}

/// Tylko wczytanie profili i stanu z dysku – do pokazania, bez uruchamiania synchronizacji.
pub fn load() {
    let mut rt = RT.lock();
    if let Ok(s) = std::fs::read_to_string(store_path()) {
        if let Ok(st) = serde_json::from_str::<Store>(&s) {
            rt.store = st;
        }
    }
}

pub fn init(app: AppHandle) {
    *APP.lock() = Some(app);
    load();
    {
        let mut rt = RT.lock();
        for st in rt.store.status.values_mut() {
            if matches!(st.state.as_str(), "scanning" | "syncing") {
                st.state = "idle".into();
            }
            st.progress = 0.0;
            st.current.clear();
        }
        // Po starcie każdy profil przechodzi pełne porównanie.
        let ids: Vec<String> = rt.store.profiles.iter().map(|p| p.id.clone()).collect();
        rt.dirty.extend(ids);
    }
    std::thread::Builder::new().name("sync-supervisor".into()).spawn(supervisor).ok();
}

pub fn profiles() -> (Vec<Profile>, Vec<Status>) {
    let rt = RT.lock();
    let st = rt
        .store
        .profiles
        .iter()
        .map(|p| rt.store.status.get(&p.id).cloned().unwrap_or(Status { id: p.id.clone(), ..Default::default() }))
        .collect();
    (rt.store.profiles.clone(), st)
}

/// Uzupełnia identyfikację celu (numer seryjny woluminu, ścieżka względna).
pub fn describe_target(path: &str) -> Target {
    let p = path.trim_end_matches('\\').to_string();
    let network = p.starts_with(r"\\");
    let mut t = Target { path: p.clone(), rel: String::new(), serial: 0, label: String::new(), network };
    if !network && p.len() >= 2 && p.as_bytes()[1] == b':' {
        let root = format!("{}\\", &p[..2]);
        t.rel = p[2..].trim_start_matches('\\').to_string();
        let is_remote = drives::list().iter().any(|d| d.path.eq_ignore_ascii_case(&root) && d.kind == "network");
        if is_remote {
            t.network = true;
            // Dysk zmapowany – zapamiętujemy ścieżkę UNC, działa też, gdy litera zniknie.
            if let Some(remote) = drives::list().into_iter().find(|d| d.path.eq_ignore_ascii_case(&root)).and_then(|d| d.remote) {
                t.label = remote.clone();
            }
        } else if let Some(v) = drives::volume_info(&root, Duration::from_secs(3)) {
            t.serial = v.serial;
            t.label = v.label;
        }
    }
    t
}

pub fn save_profile(mut p: Profile) -> Result<Profile, String> {
    if p.source.trim().is_empty() || p.target.path.trim().is_empty() {
        return Err("Wybierz folder źródłowy i docelowy".into());
    }
    let s = sync_core::norm(&p.source);
    let t = sync_core::norm(&p.target.path);
    if s == t || t.starts_with(&(s.clone() + "\\")) || s.starts_with(&(t.clone() + "\\")) {
        return Err("Folder docelowy nie może być tym samym folderem ani leżeć wewnątrz źródła".into());
    }
    if p.id.is_empty() {
        p.id = format!("s{}", now_ms());
    }
    let fresh = describe_target(&p.target.path);
    if fresh.serial != 0 || fresh.network {
        p.target = fresh;
    }
    let mut rt = RT.lock();
    let changed_paths = rt
        .store
        .profiles
        .iter()
        .find(|x| x.id == p.id)
        .map(|old| old.source != p.source || old.target.path != p.target.path || old.mode != p.mode)
        .unwrap_or(true);
    if changed_paths {
        let _ = std::fs::remove_file(base_path(&p.id));
    }
    if let Some(x) = rt.store.profiles.iter_mut().find(|x| x.id == p.id) {
        *x = p.clone();
    } else {
        rt.store.profiles.push(p.clone());
    }
    rt.store.status.entry(p.id.clone()).or_insert_with(|| Status { id: p.id.clone(), state: "idle".into(), ..Default::default() });
    rt.dirty.insert(p.id.clone());
    rt.watchers.remove(&p.id);
    save(&rt);
    Ok(p)
}

pub fn delete_profile(id: &str) {
    let mut rt = RT.lock();
    rt.store.profiles.retain(|p| p.id != id);
    rt.store.status.remove(id);
    rt.watchers.remove(id);
    if let Some(c) = rt.cancel.get(id) {
        c.store(true, Ordering::SeqCst);
    }
    let _ = std::fs::remove_file(base_path(id));
    save(&rt);
}

pub fn sync_now(id: Option<String>, confirm: bool) {
    let mut rt = RT.lock();
    let ids: Vec<String> = match id {
        Some(i) => vec![i],
        None => rt.store.profiles.iter().filter(|p| p.enabled).map(|p| p.id.clone()).collect(),
    };
    for i in ids {
        if confirm {
            rt.confirmed.insert(i.clone());
        }
        rt.force.insert(i.clone());
        rt.dirty.insert(i);
    }
}

pub fn set_enabled(id: &str, enabled: bool) {
    let mut rt = RT.lock();
    if let Some(p) = rt.store.profiles.iter_mut().find(|p| p.id == id) {
        p.enabled = enabled;
    }
    if !enabled {
        if let Some(c) = rt.cancel.get(id) {
            c.store(true, Ordering::SeqCst);
        }
        rt.watchers.remove(id);
    } else {
        rt.dirty.insert(id.to_string());
    }
    save(&rt);
}

pub fn cancel(id: &str) {
    if let Some(c) = RT.lock().cancel.get(id) {
        c.store(true, Ordering::SeqCst);
    }
}

/// Gdzie dziś jest cel? Litera dysku mogła się zmienić – szukamy woluminu po numerze seryjnym.
pub fn resolve(t: &Target) -> Option<String> {
    let timeout = Duration::from_millis(2500);
    if t.network {
        if drives::path_reachable(&t.path, timeout) {
            return Some(t.path.clone());
        }
        // Zmapowana litera zniknęła – próbujemy bezpośrednio po UNC.
        if t.label.starts_with(r"\\") {
            let unc = if t.rel.is_empty() { t.label.clone() } else { format!("{}\\{}", t.label.trim_end_matches('\\'), t.rel) };
            if drives::path_reachable(&unc, timeout) {
                return Some(unc);
            }
        }
        return None;
    }
    if t.serial == 0 {
        return drives::path_reachable(&t.path, timeout).then(|| t.path.clone());
    }
    for d in drives::list() {
        if d.online && d.serial == t.serial && d.kind != "network" {
            let p = if t.rel.is_empty() { d.path.clone() } else { format!("{}{}", d.path, t.rel) };
            if !Path::new(&p).exists() {
                let _ = std::fs::create_dir_all(&p);
            }
            if Path::new(&p).is_dir() {
                return Some(p);
            }
        }
    }
    None
}

fn ensure_watch(rt: &mut Runtime, p: &Profile) {
    if !p.continuous {
        rt.watchers.remove(&p.id);
        return;
    }
    if rt.watchers.get(&p.id).map(|(src, _)| src == &p.source).unwrap_or(false) {
        return;
    }
    let id = p.id.clone();
    let source = p.source.clone();
    let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            let relevant = ev.paths.iter().any(|p| {
                let s = p.to_string_lossy();
                !s.ends_with(".ofp-part") && !s.contains(".ofp-versions")
            });
            if relevant && matches!(ev.kind, notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)) {
                LAST_EVENT.lock().insert(id.clone(), Instant::now());
                let mut rt = RT.lock();
                if !rt.running.contains(&id) {
                    rt.dirty.insert(id.clone());
                }
                let st = rt.store.status.entry(id.clone()).or_default();
                if !st.connected {
                    st.pending += ev.paths.len() as u64;
                }
            }
        }
    });
    if let Ok(mut w) = w {
        if w.watch(Path::new(&source), RecursiveMode::Recursive).is_ok() {
            rt.watchers.insert(p.id.clone(), (source, w));
        }
    }
}

fn supervisor() {
    let mut tick = 0u64;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        tick += 1;
        let profiles: Vec<Profile> = RT.lock().store.profiles.clone();
        for p in profiles {
            if !p.enabled {
                update(&p.id, |s| {
                    if s.state != "paused" {
                        s.state = "paused".into();
                    }
                });
                continue;
            }
            if RT.lock().running.contains(&p.id) {
                continue;
            }
            // Dostępność celu sprawdzamy co ~6 s (sieć bywa wolna).
            if tick % 3 != 0 && !RT.lock().dirty.contains(&p.id) {
                continue;
            }
            let target = resolve(&p.target);
            let now = now_ms();
            let was_connected;
            {
                let mut rt = RT.lock();
                ensure_watch(&mut rt, &p);
                let st = rt.store.status.entry(p.id.clone()).or_insert_with(|| Status { id: p.id.clone(), ..Default::default() });
                was_connected = st.connected;
                st.connected = target.is_some();
                if let Some(t) = &target {
                    st.last_seen = now;
                    st.resolved_target = t.clone();
                }
            }
            let Some(target) = target else {
                update(&p.id, |s| {
                    if s.state != "offline" {
                        s.state = "offline".into();
                    }
                });
                if was_connected {
                    save(&RT.lock());
                }
                continue;
            };
            let due = {
                let rt = RT.lock();
                let interval = Duration::from_secs(p.interval_min.max(1) as u64 * 60);
                let periodic = rt.last_full.get(&p.id).map(|t| t.elapsed() > interval).unwrap_or(true);
                let quiet = LAST_EVENT.lock().get(&p.id).map(|t| t.elapsed() > Duration::from_secs(3)).unwrap_or(true);
                (rt.dirty.contains(&p.id) && quiet) || periodic || !was_connected || rt.force.contains(&p.id)
            };
            if !due {
                update(&p.id, |s| {
                    if s.state == "offline" || s.state == "paused" || s.state.is_empty() {
                        s.state = "idle".into();
                    }
                });
                continue;
            }
            let flag = Arc::new(AtomicBool::new(false));
            {
                let mut rt = RT.lock();
                rt.dirty.remove(&p.id);
                rt.force.remove(&p.id);
                rt.running.insert(p.id.clone());
                rt.cancel.insert(p.id.clone(), flag.clone());
            }
            let p2 = p.clone();
            std::thread::spawn(move || {
                let confirmed = RT.lock().confirmed.remove(&p2.id);
                run(&p2, &target, confirmed, &flag);
                let mut rt = RT.lock();
                rt.running.remove(&p2.id);
                rt.cancel.remove(&p2.id);
                rt.last_full.insert(p2.id.clone(), Instant::now());
                save(&rt);
            });
        }
    }
}

fn run(p: &Profile, target: &str, confirmed: bool, cancel: &AtomicBool) {
    update(&p.id, |s| {
        s.state = "scanning".into();
        s.progress = 0.0;
        s.current = "Porównywanie folderów…".into();
    });
    let excl = sync_core::Excludes::new(&p.exclude);
    let src = match sync_core::scan(Path::new(&p.source), &excl, p.include_hidden, cancel) {
        Ok(s) => s,
        Err(e) => {
            finish_err(p, format!("Źródło niedostępne: {e}"));
            return;
        }
    };
    let dst = match sync_core::scan(Path::new(target), &excl, p.include_hidden, cancel) {
        Ok(s) => s,
        Err(e) => {
            finish_err(p, format!("Cel niedostępny: {e}"));
            return;
        }
    };
    if cancel.load(Ordering::SeqCst) {
        update(&p.id, |s| s.state = "idle".into());
        return;
    }
    let base: BTreeMap<String, FileState> = std::fs::read_to_string(base_path(&p.id))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let plan = sync_core::plan(p.mode, &src, &dst, &base);
    let counts = counts_of(&plan);
    // Ochrona przed katastrofą: pusty/odłączony folder nie może skasować całej kopii.
    let deletions = counts.del_target + counts.del_source;
    let total = src.len().max(dst.len()).max(1) as u64;
    if !confirmed && deletions > 25 && deletions * 100 / total > 30 {
        update(&p.id, |s| {
            s.state = "confirm".into();
            s.needs_confirm = Some(counts.clone());
            s.current = format!("Wstrzymano: plan usuwa {deletions} elementów – potwierdź");
        });
        return;
    }
    let total_actions = plan.actions.len().max(1);
    if plan.actions.is_empty() {
        sync_core::save_base(&base_path(&p.id), &plan.new_base);
        update(&p.id, |s| {
            s.state = "idle".into();
            s.last_sync = now_ms();
            s.pending = 0;
            s.progress = 1.0;
            s.current.clear();
            s.needs_confirm = None;
            s.last_result = "Aktualne".into();
        });
        return;
    }
    update(&p.id, |s| {
        s.state = "syncing".into();
        s.needs_confirm = None;
    });
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M%S").to_string();
    let ctx = sync_core::ExecCtx {
        source: PathBuf::from(&p.source),
        target: PathBuf::from(target),
        versioning: p.versioning,
        stamp,
    };
    let mut errors = Vec::new();
    let mut new_base = plan.new_base.clone();
    let mut last = Instant::now();
    for (i, a) in plan.actions.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        if last.elapsed() > Duration::from_millis(150) {
            let label = a.label();
            update(&p.id, |s| {
                s.progress = i as f32 / total_actions as f32;
                s.current = label;
            });
            last = Instant::now();
        }
        match sync_core::execute(&ctx, a) {
            Ok(Some((rel, st))) => {
                new_base.insert(rel, st);
            }
            Ok(None) => {
                if let Action::DeleteTarget(rel) | Action::DeleteSource(rel) = a {
                    new_base.remove(rel);
                }
            }
            Err(e) => {
                // Nieudana akcja nie trafia do bazy – zostanie ponowiona następnym razem.
                if let Some(rel) = a.rel() {
                    if let Some(old) = base.get(rel) {
                        new_base.insert(rel.to_string(), old.clone());
                    } else {
                        new_base.remove(rel);
                    }
                }
                errors.push(format!("{}: {e}", a.rel().unwrap_or("")));
            }
        }
    }
    let cancelled = cancel.load(Ordering::SeqCst);
    if !cancelled {
        sync_core::save_base(&base_path(&p.id), &new_base);
    }
    let mut c = counts;
    c.errors = errors.len() as u64;
    let summary = summarize(&c, cancelled);
    let ok = errors.is_empty() && !cancelled;
    update(&p.id, move |s| {
        s.state = if ok { "idle".into() } else if cancelled { "idle".into() } else { "error".into() };
        s.last_sync = if cancelled { s.last_sync } else { now_ms() };
        s.pending = 0;
        s.progress = 1.0;
        s.current = errors.first().cloned().unwrap_or_default();
        s.last_result = summary.clone();
        s.last_counts = c;
        s.history.insert(0, HistoryItem { at: now_ms(), summary, ok });
        s.history.truncate(40);
    });
}

fn finish_err(p: &Profile, msg: String) {
    update(&p.id, |s| {
        s.state = "error".into();
        s.current = msg.clone();
        s.last_result = msg.clone();
        s.history.insert(0, HistoryItem { at: now_ms(), summary: msg, ok: false });
        s.history.truncate(40);
    });
}

fn counts_of(plan: &Plan) -> Counts {
    let mut c = Counts::default();
    for a in &plan.actions {
        match a {
            Action::CopyToTarget(_, size) => {
                c.to_target += 1;
                c.bytes += size;
            }
            Action::CopyToSource(_, size) => {
                c.to_source += 1;
                c.bytes += size;
            }
            Action::DeleteTarget(_) => c.del_target += 1,
            Action::DeleteSource(_) => c.del_source += 1,
            Action::Conflict { .. } => c.conflicts += 1,
            _ => {}
        }
    }
    c
}

fn summarize(c: &Counts, cancelled: bool) -> String {
    let mut parts = Vec::new();
    if c.to_target > 0 {
        parts.push(format!("→ {}", c.to_target));
    }
    if c.to_source > 0 {
        parts.push(format!("← {}", c.to_source));
    }
    if c.del_target + c.del_source > 0 {
        parts.push(format!("usunięto {}", c.del_target + c.del_source));
    }
    if c.conflicts > 0 {
        parts.push(format!("konflikty {}", c.conflicts));
    }
    if c.errors > 0 {
        parts.push(format!("błędy {}", c.errors));
    }
    let mut s = if parts.is_empty() { "Aktualne".to_string() } else { parts.join(" · ") };
    if cancelled {
        s = format!("Przerwano ({s})");
    }
    s
}

/// Podgląd planu bez wykonywania – „co się stanie, jeśli zsynchronizuję teraz".
#[derive(Serialize)]
pub struct Preview {
    pub counts: Counts,
    pub items: Vec<String>,
}

pub fn preview(id: &str) -> Result<Preview, String> {
    let p = RT.lock().store.profiles.iter().find(|p| p.id == id).cloned().ok_or("Brak profilu")?;
    let target = resolve(&p.target).ok_or("Cel jest niedostępny")?;
    let excl = sync_core::Excludes::new(&p.exclude);
    let no = AtomicBool::new(false);
    let src = sync_core::scan(Path::new(&p.source), &excl, p.include_hidden, &no).map_err(|e| e.to_string())?;
    let dst = sync_core::scan(Path::new(&target), &excl, p.include_hidden, &no).map_err(|e| e.to_string())?;
    let base: BTreeMap<String, FileState> = std::fs::read_to_string(base_path(&p.id)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let plan = sync_core::plan(p.mode, &src, &dst, &base);
    Ok(Preview { counts: counts_of(&plan), items: plan.actions.iter().take(500).map(|a| a.label()).collect() })
}
