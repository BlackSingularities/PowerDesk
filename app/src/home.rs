//! Ekran „Ten komputer” (Home.svelte): powitanie z wyszukiwaniem, dyski, stan komputera,
//! rozmiary folderów użytkownika i ostatnio zmienione pliki. Pokazuje się na karcie `ofp:home`
//! zamiast listy; dane o systemie odświeża co 3 s, tylko gdy ekran jest widoczny.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use chrono::{Local, Timelike};
use powerdesk_core::fsx::{self, Entry};
use powerdesk_core::{shell, sysmon};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::fmt::{self, Kind};
use crate::menus::{it, sep};
use crate::model::HOME;
use crate::{HomeDrive, HomeFile, HomeFolder, HomeState, Shared};

/// Ile ostatnich plików wczytujemy i pokazujemy.
const RECENT_LIMIT: usize = 120;
const SHOWN: usize = 30;

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    /// Ostatnie pliki (wszystkie rodzaje) – filtr wybiera z nich bez ponownego czytania dysku.
    static FILES: RefCell<Option<Vec<Entry>>> = const { RefCell::new(None) };
    /// Rozmiary folderów użytkownika – liczone raz na uruchomienie.
    static SIZES: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
    static CPU: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

fn with_ui(s: &Shared, f: impl FnOnce(&HomeState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<HomeState>());
    }
}

fn known() -> Vec<(&'static str, PathBuf, &'static str)> {
    [
        ("Pulpit", dirs::desktop_dir(), "desktop"),
        ("Pobrane", dirs::download_dir(), "download"),
        ("Dokumenty", dirs::document_dir(), "doc"),
        ("Obrazy", dirs::picture_dir(), "image"),
        ("Wideo", dirs::video_dir(), "video"),
        ("Muzyka", dirs::audio_dir(), "music"),
    ]
    .into_iter()
    .filter_map(|(n, p, i)| p.filter(|p| p.is_dir()).map(|p| (n, p, i)))
    .collect()
}

fn ic(name: &str) -> slint::SharedString {
    crate::icons::icon(name).into()
}

/// Wołane po każdym przełączeniu karty: włącza albo wyłącza ekran.
pub fn sync(s: &Shared) {
    let on = crate::tab_mut(s, |t| t.path == HOME && t.viewer.is_none() && t.search.is_none());
    let was = s.borrow().ui.upgrade().is_some_and(|u| u.global::<HomeState>().get_open());
    with_ui(s, |g| g.set_open(on));
    if !on {
        TIMER.with(|t| t.stop());
        return;
    }
    if was {
        return;
    }
    render_static(s);
    tick(s);
    let s2 = s.clone();
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(3), move || tick(&s2)));
    if FILES.with(|f| f.borrow().is_none()) {
        load_files(s);
    } else {
        render_files(s);
    }
    count_folders();
}

/// Powitanie, dyski i foldery – to, co zmienia się rzadko.
fn render_static(s: &Shared) {
    let now = Local::now();
    let greeting = match now.hour() {
        5..=11 => "Dzień dobry",
        12..=17 => "Miłego popołudnia",
        18..=22 => "Dobry wieczór",
        _ => "Dobranoc",
    };
    let drives: Vec<HomeDrive> = s
        .borrow()
        .drives
        .iter()
        .filter(|d| d.online && d.total > 0)
        .map(|d| HomeDrive {
            name: if d.label.is_empty() { format!("Dysk ({})", d.letter) } else { format!("{} ({})", d.label, d.letter) }.into(),
            path: d.path.clone().into(),
            detail: format!("{} wolne z {}", fmt::size(d.free), fmt::size(d.total)).into(),
            usage: 1.0 - d.free as f32 / d.total as f32,
            icon: ic(if d.kind == "network" { "network" } else { "drive" }),
        })
        .collect();
    let (total, free) = s.borrow().drives.iter().filter(|d| d.online && d.kind != "network").fold((0u64, 0u64), |(t, f), d| (t + d.total, f + d.free));
    with_ui(s, |g| {
        g.set_greeting(greeting.into());
        g.set_today(today(&now).into());
        g.set_drives(ModelRc::new(VecModel::from(drives)));
        g.set_storage(if total > 0 { format!("{} wolne z {}", fmt::size(free), fmt::size(total)) } else { String::new() }.into());
        g.set_storage_usage(if total > 0 { 1.0 - free as f32 / total as f32 } else { 0.0 });
    });
    render_folders(s);
}

/// „Środa, 22 października 2026”.
fn today(d: &chrono::DateTime<Local>) -> String {
    use chrono::Datelike;
    const DAYS: [&str; 7] = ["Poniedziałek", "Wtorek", "Środa", "Czwartek", "Piątek", "Sobota", "Niedziela"];
    const MONTHS: [&str; 12] = ["stycznia", "lutego", "marca", "kwietnia", "maja", "czerwca", "lipca", "sierpnia", "września", "października", "listopada", "grudnia"];
    format!("{}, {} {} {}", DAYS[d.weekday().num_days_from_monday() as usize], d.day(), MONTHS[d.month0() as usize], d.year())
}

/// Zegar, procesor, pamięć, czas pracy i aktywność.
fn tick(s: &Shared) {
    let i = sysmon::info();
    let hist = CPU.with(|h| {
        let mut h = h.borrow_mut();
        h.push(i.cpu_usage);
        let n = h.len();
        if n > 20 {
            h.drain(..n - 20);
        }
        h.clone()
    });
    let up = i.uptime;
    let uptime = match (up / 86_400, up / 3600 % 24, up / 60 % 60) {
        (0, 0, m) => format!("{m} min"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, h, _) => format!("{d} d {h} h"),
    };
    let shelf = s.borrow().shelf.items.len();
    with_ui(s, |g| {
        g.set_clock(Local::now().format("%H:%M").to_string().into());
        g.set_host(i.host.clone().into());
        g.set_os(i.os.clone().into());
        g.set_cpu(format!("{} · {} rdz. / {} wątków", i.cpu, i.cores, i.threads).into());
        g.set_cpu_usage(i.cpu_usage);
        g.set_cpu_hist(ModelRc::new(VecModel::from(hist)));
        g.set_mem(format!("{} / {}", fmt::size(i.mem_used), fmt::size(i.mem_total)).into());
        g.set_mem_usage(if i.mem_total > 0 { i.mem_used as f32 / i.mem_total as f32 } else { 0.0 });
        g.set_uptime(uptime.into());
        g.set_activity(if shelf > 0 { format!("Na półce: {}", fmt::items(shelf)) } else { String::new() }.into());
    });
}

fn render_folders(s: &Shared) {
    let sizes = SIZES.with(|x| x.borrow().clone());
    let max = sizes.values().copied().max().unwrap_or(0).max(1);
    let rows: Vec<HomeFolder> = known()
        .into_iter()
        .map(|(n, p, icon)| {
            let p = p.to_string_lossy().to_string();
            let sz = sizes.get(&p).copied();
            HomeFolder {
                name: n.into(),
                icon: ic(icon),
                size: sz.map(fmt::size).unwrap_or_default().into(),
                ratio: sz.map(|v| v as f32 / max as f32).unwrap_or(-1.0),
                path: p.into(),
            }
        })
        .collect();
    with_ui(s, |g| g.set_folders(ModelRc::new(VecModel::from(rows))));
}

/// Rozmiary folderów w tle, po kolei; każdy wynik od razu na ekran.
fn count_folders() {
    let todo: Vec<String> = known().into_iter().map(|(_, p, _)| p.to_string_lossy().to_string()).filter(|p| SIZES.with(|x| !x.borrow().contains_key(p))).collect();
    if todo.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for p in todo {
            let size = fsx::dir_size(&p).size;
            let _ = slint::invoke_from_event_loop(move || {
                SIZES.with(|x| x.borrow_mut().insert(p, size));
                crate::with_state(render_folders);
            });
        }
    });
}

fn load_files(s: &Shared) {
    with_ui(s, |g| g.set_files_loading(true));
    std::thread::spawn(|| {
        let dirs: Vec<String> = known().into_iter().map(|(_, p, _)| p.to_string_lossy().to_string()).collect();
        let list = fsx::recent_files(&dirs, 14, RECENT_LIMIT);
        let _ = slint::invoke_from_event_loop(move || {
            FILES.with(|f| *f.borrow_mut() = Some(list));
            crate::with_state(|s| {
                with_ui(s, |g| g.set_files_loading(false));
                render_files(s);
            });
        });
    });
}

/// „5 min temu”, „wczoraj”, „3 dni temu”.
fn ago(ms: i64) -> String {
    let mins = (Local::now().timestamp_millis() - ms).max(0) / 60_000;
    match mins {
        0 => "przed chwilą".into(),
        1..=59 => format!("{mins} min temu"),
        60..=1439 => format!("{} godz. temu", mins / 60),
        1440..=2879 => "wczoraj".into(),
        _ => format!("{} dni temu", mins / 1440),
    }
}

fn shown(s: &Shared) -> Vec<Entry> {
    let mut kind = 0;
    with_ui(s, |g| kind = g.get_kind());
    FILES.with(|f| {
        f.borrow()
            .iter()
            .flatten()
            .filter(|e| match kind {
                1 => matches!(fmt::kind_of(e), Kind::Image | Kind::Raw),
                2 => matches!(fmt::kind_of(e), Kind::Doc | Kind::Sheet | Kind::Pdf | Kind::Text),
                3 => fmt::kind_of(e) == Kind::Video,
                _ => true,
            })
            .take(SHOWN)
            .cloned()
            .collect()
    })
}

fn render_files(s: &Shared) {
    let px = s.borrow().ui.upgrade().map(|u| (110.0 * u.window().scale_factor()) as u32).unwrap_or(110);
    let rows: Vec<HomeFile> = shown(s)
        .into_iter()
        .map(|e| {
            let media = fmt::is_media(&e);
            let img = crate::thumbs::get(&e.path, px, false);
            HomeFile { name: e.name.clone().into(), path: e.path.clone().into(), when: ago(e.modified).into(), has_img: img.is_some(), img: img.unwrap_or_default(), media }
        })
        .collect();
    with_ui(s, |g| g.set_files(ModelRc::new(VecModel::from(rows))));
}

pub fn thumbs_arrived(s: &Shared, paths: &HashSet<String>) {
    let open = s.borrow().ui.upgrade().is_some_and(|u| u.global::<HomeState>().get_open());
    if open && shown(s).iter().any(|e| paths.contains(&e.path)) {
        render_files(s);
    }
}

pub fn set_kind(s: &Shared, k: i32) {
    with_ui(s, |g| g.set_kind(k));
    render_files(s);
}

pub fn refresh_files(s: &Shared) {
    FILES.with(|f| *f.borrow_mut() = None);
    with_ui(s, |g| g.set_files(ModelRc::default()));
    load_files(s);
}

/// Wyszukiwanie z ekranu – w folderze domowym, w tej samej karcie.
pub fn search(s: &Shared, q: &str) {
    let Some(home) = dirs::home_dir() else { return };
    crate::navigate(s, &home.to_string_lossy(), true);
    crate::start_search(s, q);
}

fn reveal(s: &Shared, path: &str) {
    if let Some(dir) = std::path::Path::new(path).parent() {
        crate::navigate(s, &dir.to_string_lossy(), true);
        crate::select_path(s, path);
    }
}

pub fn file_menu(s: &Shared, path: String, x: f32, y: f32) {
    let (p1, p2, p3, p4) = (path.clone(), path.clone(), path.clone(), path);
    let items = vec![
        it("Otwórz", "open", move |_| {
            let _ = open::that_detached(&p1);
        }),
        it("Pokaż w folderze", "folder", move |s| reveal(s, &p2)),
        sep(),
        it("Kopiuj ścieżkę", "copy", move |s| crate::props::copy(s, &p3)),
        it("Właściwości systemowe", "info", move |s| {
            shell::show_properties(crate::hwnd(s), &p4);
        }),
    ];
    crate::menus::show(s, items, x, y);
}

pub fn bind(ui: &crate::MainWindow, st: &Shared) {
    let g = ui.global::<HomeState>();
    let s = st.clone();
    g.on_go(move |p| crate::navigate(&s, &p, true));
    let s = st.clone();
    g.on_go_new_tab(move |p| crate::new_tab_at(&s, &p, false));
    g.on_open_file(|p| {
        let _ = open::that_detached(p.as_str());
    });
    let s = st.clone();
    g.on_file_menu(move |p, x, y| file_menu(&s, p.into(), x, y));
    let s = st.clone();
    g.on_search(move |q| search(&s, &q));
    let s = st.clone();
    g.on_set_kind(move |k| set_kind(&s, k));
    let s = st.clone();
    g.on_refresh_files(move || refresh_files(&s));
}
