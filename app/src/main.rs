#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! PowerDesk – natywny interfejs (Slint) nad backendem `powerdesk-core`.

mod archives;
mod bulk;
mod desktop;
mod dragdrop;
mod taskbar;
mod taskview;
mod start;
mod starttiles;
mod startsearch;
mod cliphistory;
mod comparing;
mod facets;
mod fmt;
mod home;
mod settings;
mod syncview;
mod spaceview;
mod termview;
mod addr;
mod menus;
mod migrate;
mod game;
mod notices;
mod overlay;
mod setup;
mod model;
mod preview;
mod props;
mod viewer;
mod wallpapers;
mod widgets;
mod zones;
mod widgetfeeds;
mod widgetmore;
mod palette;
mod player;
mod shelf;
mod syntax;
mod query;
mod thumbs;
mod transfers;
mod updates;
mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use model::{Col, GroupKey, KindFilter, Prefs, SavedSearch, Search, SortKey, Tab, View, HOME};
use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::{drives, fsx, shell};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

slint::include_modules!();

pub struct State {
    tabs: Vec<Tab>,
    active: usize,
    prefs: Prefs,
    ui: Weak<MainWindow>,
    /// Okno widoku: przesunięcie, szerokość, wysokość (piksele logiczne).
    /// Karty pokazane w panelach: [lewy] albo [lewy, prawy] po podziale (Ctrl+\).
    panes: Vec<usize>,
    /// Wskazany panel – jego karta jest tą „aktywną”, na której działa cała reszta programu.
    focused: usize,
    /// Udział lewego panelu w szerokości obszaru plików.
    split_ratio: f32,
    /// Widok każdego panelu: przewinięcie, szerokość, wysokość.
    viewports: [(f32, f32, f32); 2],
    /// Metadane zdjęć i filmów: proporcje w galerii, kolumny Wymiary / Data wykonania / Czas trwania.
    media: HashMap<String, shell::MediaInfo>,
    media_pending: HashSet<String>,
    /// Rozmiary folderów policzone w tle (klucz: ścieżka małymi literami).
    dir_sizes: HashMap<String, u64>,
    dir_pending: HashSet<String>,
    menu: Option<menus::Open>,
    /// Dyski – odświeżane po zmianie (podłączenie pendrive'a), nie przy każdym rysowaniu.
    drives: Vec<drives::Drive>,
    /// Zaznaczenie sprzed ramki (z Ctrl/Shift ramka je rozszerza); Some, dopóki trwa przeciąganie.
    marquee: Option<HashSet<String>>,
    /// Pasek transferów, powiadomienia, okna konfliktu i potwierdzenia.
    transfers: transfers::Transfers,
    /// Wiersze modelu widoku każdego panelu: (indeks elementu, epoka) w slocie;
    /// epoka rośnie przy zmianie danych.
    slots: [Vec<Option<(usize, u64)>>; 2],
    epoch: u64,
    /// Narysowany pas treści (y od–do) w każdym panelu.
    painted: [(f32, f32); 2],
    /// Ścieżki pozycji panelu bocznego (żeby wiedzieć, czy nowa ikona go dotyczy).
    side_paths: HashSet<String>,
    /// Elementy w otwartym oknie masowej zmiany nazw.
    bulk: Option<Vec<fsx::Entry>>,
    shelf: shelf::Shelf,
    /// Ścieżki ostatnio zamkniętych kart – Ctrl+Shift+T przywraca po kolei.
    closed: Vec<String>,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    /// Stan żyje w wątku UI; zdarzenia z backendu trafiają tu przez `invoke_from_event_loop`.
    static STATE: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

pub fn with_state(f: impl FnOnce(&Shared)) {
    let s = STATE.with(|s| s.borrow().clone());
    if let Some(s) = s {
        f(&s)
    }
}

/// Ścieżka z wiersza poleceń: folder albo plik (wtedy jego folder i plik do zaznaczenia).
fn arg_target(arg: &str) -> Option<(String, Option<String>)> {
    // Dysk z menu Eksploratora przychodzi jako `C:"` ("C:\" – ukośnik zjada cudzysłów).
    let mut a = arg.trim().trim_matches('"').to_string();
    if a.len() == 2 && a.ends_with(':') {
        a.push('\\');
    }
    let p = Path::new(&a);
    if p.is_dir() {
        return Some((p.to_string_lossy().to_string(), None));
    }
    let dir = p.parent().filter(|d| p.is_file() && d.is_dir())?;
    Some((dir.to_string_lossy().to_string(), Some(p.to_string_lossy().to_string())))
}

/// Argumenty od kolejnego uruchomienia: każda ścieżka w nowej karcie, bez ścieżek – karta startowa.
fn open_args(s: &Shared, args: Vec<String>) {
    // Instalator albo deinstalator prosi o zamknięcie.
    if args.iter().any(|a| a == setup::QUIT_ARG) {
        quit(s);
        return;
    }
    let targets: Vec<_> = args.iter().filter_map(|a| arg_target(a)).collect();
    // Okno schowane w zasobniku: Win+E po prostu je przywraca, z kartami jak przed schowaniem.
    let hidden = s.borrow().ui.upgrade().is_some_and(|u| !u.window().is_visible());
    if targets.is_empty() && !hidden {
        let start = settings::start_path(&s.borrow().prefs);
        new_tab_at(s, &start, true);
    }
    for (dir, file) in targets {
        new_tab_at(s, &dir, true);
        if let Some(f) = file {
            select_path(s, &f);
        }
    }
    show_window(s);
}

/// Ikona programu do zasobnika.
const TRAY_ICO: &[u8] = include_bytes!("../icon.ico");

/// Ikona w zasobniku – zawsze: PowerDesk działa w tle jako powłoka systemu.
fn start_tray() {
    powerdesk_core::tray::show(TRAY_ICO, |e| {
        let _ = slint::invoke_from_event_loop(move || with_state(|s| tray_event(s, e)));
    });
}

/// Odnawia wpisy zastępujące Eksplorator (w tle – to kilka wywołań reg.exe).
pub fn ensure_integration() {
    std::thread::spawn(|| {
        let res = powerdesk_core::integration::ensure();
        let _ = slint::invoke_from_event_loop(move || {
            with_state(|s| {
                if let Err(e) = res {
                    powerdesk_core::log::error(format!("integracja: {e}"));
                    transfers::toast(s, format!("Nie udało się zastąpić Eksploratora: {e}"), transfers::Kind::Error, None);
                }
                settings::render_system(s);
            })
        });
    });
}

fn tray_event(s: &Shared, e: powerdesk_core::tray::Event) {
    use powerdesk_core::tray::Event;
    match e {
        Event::Show => show_window(s),
        Event::NewTab => {
            let start = settings::start_path(&s.borrow().prefs);
            new_tab_at(s, &start, true);
            show_window(s);
        }
        Event::Quit => quit(s),
    }
}

/// Pokazuje okno (także schowane do zasobnika) i wysuwa je na wierzch.
pub fn show_window(s: &Shared) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let first = !ui.window().is_visible();
    let _ = ui.show();
    if ui.window().is_minimized() {
        ui.window().set_minimized(false);
    }
    if first {
        // Okno mogło dopiero powstać (start z autostartu) – cel upuszczania i tło Mica/Acrylic.
        dragdrop::register(s);
        settings::relook(s);
        // Układ i ikony liczone przy schowanym oknie miały zły rozmiar – od nowa, gdy okno już stoi.
        slint::Timer::single_shot(std::time::Duration::from_millis(120), || {
            with_state(|s| {
                render_side(s);
                load(s, false);
            })
        });
    }
    powerdesk_core::instance::bring_to_front(hwnd(s));
}

/// Krzyżyk i Alt+F4 chowają okno do zasobnika; program kończy „Zakończ” z menu ikony.
/// Gdyby ikona się nie pojawiła, krzyżyk zamyka program – inaczej nie dałoby się go zamknąć.
fn close_window(s: &Shared) {
    let to_tray = powerdesk_core::tray::visible() && s.borrow().prefs.close_to_tray;
    match s.borrow().ui.upgrade() {
        Some(ui) if to_tray => {
            let _ = ui.hide();
        }
        _ => quit(s),
    }
}

/// Koniec programu: pulpit i pasek Windows wracają (strażnik niczego już nie zastanie).
pub fn quit(_s: &Shared) {
    start::close();
    cliphistory::close();
    desktop::close();
    taskbar::close();
    game::restore();
    powerdesk_core::shellswap::restore();
    powerdesk_core::tray::hide();
    let _ = slint::quit_event_loop();
}

fn main() -> Result<(), slint::PlatformError> {
    powerdesk_core::log::install_panic_hook();
    // Strażnik: czeka na koniec PowerDesk i po awarii przywraca pulpit i pasek Windows.
    let argv: Vec<String> = std::env::args().collect();
    // Logowanie z PowerDesk jako powłoką i przytrzymany Shift (albo --safe) – ta sesja z Eksploratorem.
    let shell_start = argv.iter().any(|a| a == powerdesk_core::shellmode::SHELL_FLAG);
    if argv.iter().any(|a| a == powerdesk_core::shellmode::SAFE_FLAG) || (shell_start && powerdesk_core::shellmode::shift_held()) {
        powerdesk_core::shellmode::safe_mode();
        return Ok(());
    }
    if let Some(i) = argv.iter().position(|a| a == powerdesk_core::shellswap::GUARD_FLAG) {
        if let Some(pid) = argv.get(i + 1).and_then(|p| p.parse().ok()) {
            powerdesk_core::shellswap::guard(pid);
        }
        return Ok(());
    }
    // Przywrócenie Eksploratora (deinstalator, awaria) – bez okna.
    if std::env::args().any(|a| a == powerdesk_core::integration::REMOVE_FLAG) {
        powerdesk_core::tray::hide();
        powerdesk_core::shellswap::restore();
        if let Err(e) = powerdesk_core::integration::remove() {
            eprintln!("{e}");
            std::process::exit(1);
        }
        // Bez Eksploratora (PowerDesk był powłoką) – uruchamiamy go, żeby nie został czarny ekran.
        if powerdesk_core::shellmode::detect() {
            powerdesk_core::shellmode::fallback_to_explorer();
        }
        return Ok(());
    }
    // Instalacja i deinstalacja (wpis w „Aplikacje i funkcje”, Ustawienia → System).
    if let Some(code) = setup::handle_args(&argv) {
        std::process::exit(code);
    }
    // Działa już okno – oddajemy mu argumenty (Win+E, „Otwórz w PowerDesk”) i kończymy.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--")).collect();
    if powerdesk_core::instance::send_to_running(&args) {
        return Ok(());
    }
    // Pobrany plik uruchomiony pierwszy raz (nic nie działa, nic nie zainstalowane) – propozycja instalacji.
    if let Some(code) = setup::offer_install(&argv) {
        std::process::exit(code);
    }
    // Eksplorator zostaje w tle (Ustawienia, aplikacje UWP, powiadomienia) – PowerDesk chowa jego
    // pasek i pulpit. Bez niego (nie daje się uruchomić) PowerDesk jest powłoką. Przed `Shell_TrayWnd`.
    let shell = powerdesk_core::shellmode::ensure_explorer();
    powerdesk_core::log::info(format!(
        "start {} · {} · {}",
        powerdesk_core::update::current(),
        if shell { "powłoka" } else { "z Eksploratorem" },
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
    ));
    let ui = MainWindow::new()?;
    let mut prefs = Prefs::load();
    // Pierwszy start bez własnych ustawień – to, co było w OpenFilePower.
    if migrate::from_openfilepower(&mut prefs) {
        prefs.save();
    }
    std::thread::spawn(|| powerdesk_core::install::refresh_entry(powerdesk_core::update::current()));
    settings::apply_look(&ui, &prefs);

    let first = args.iter().find_map(|a| arg_target(a));
    let start = first.as_ref().map(|(d, _)| d.clone()).unwrap_or_else(|| settings::start_path(&prefs));
    let st: Shared = Rc::new(RefCell::new(State {
        tabs: vec![Tab::new(&start)],
        active: 0,
        prefs,
        ui: ui.as_weak(),
        panes: vec![0],
        focused: 0,
        split_ratio: 0.5,
        viewports: [(0.0, 900.0, 600.0); 2],
        media: HashMap::new(),
        media_pending: HashSet::new(),
        dir_sizes: HashMap::new(),
        dir_pending: HashSet::new(),
        menu: None,
        marquee: None,
        transfers: Default::default(),
        slots: [Vec::new(), Vec::new()],
        epoch: 0,
        painted: [(0.0, 0.0); 2],
        side_paths: HashSet::new(),
        bulk: None,
        shelf: Default::default(),
        closed: Vec::new(),
        drives: drives::list(),
    }));
    STATE.with(|s| *s.borrow_mut() = Some(st.clone()));

    powerdesk_core::events::set_sink(|name, payload| {
        let name = name.to_string();
        let _ = slint::invoke_from_event_loop(move || with_state(|s| on_event(s, &name, payload)));
    });
    fsx::init_watcher(Core);
    drives::init_watch(Core);

    wire(&ui, &st);
    shelf::restore(&st);
    termview::restore(&st);
    // Tło Mica/Acrylic ustawia DWM na gotowym oknie – po starcie pętli zdarzeń.
    let s = st.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(50), move || settings::relook(&s));
    restore_pinned(&st);
    load(&st, false);
    if let Some((_, Some(f))) = &first {
        select_path(&st, f);
    }
    powerdesk_core::instance::serve(|args| {
        let _ = slint::invoke_from_event_loop(move || with_state(|s| open_args(s, args)));
    });
    // „Wklej” pokazuje się tylko, gdy w schowku są pliki – także skopiowane w innym programie.
    let paste_timer = slint::Timer::default();
    let w = ui.as_weak();
    paste_timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(500), move || {
        if let Some(ui) = w.upgrade() {
            ui.set_can_paste(powerdesk_core::clip::has_files());
        }
    });
    // Cel upuszczania wymaga uchwytu okna – rejestrujemy go po starcie pętli.
    let s2 = st.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(50), move || dragdrop::register(&s2));
    start_tray();
    ensure_integration();
    updates::start();
    // Synchronizacja rusza w tle chwilę po starcie, żeby nie opóźniać pierwszego okna.
    let s3 = st.clone();
    slint::Timer::single_shot(std::time::Duration::from_secs(3), move || syncview::start_background(&s3));
    // Pulpit i pasek PowerDesk zawsze zastępują Windows; strażnik przywróci je po awarii.
    // Po poprzedniej awarii (plik stanu został) najpierw porządek, potem podmiana od nowa.
    powerdesk_core::shellswap::restore();
    powerdesk_core::shellswap::spawn_guard();
    desktop::open(&st);
    taskbar::open(&st);
    // Nakładka z wydajnością: skrót Alt+R, a jeśli była włączona – od razu.
    overlay::init();
    // Strefy okien: Shift przy przeciąganiu okna.
    zones::init();
    // Tryb gry (plan zasilania), jeśli włączony.
    game::init();
    // Własna historia schowka (Win+V) – nasłuch od startu, żeby nic nie umknęło.
    cliphistory::start();
    // Z autostartu: bez okna folderów – pojawia się z menu Start albo po Win+E.
    let hidden = std::env::args().any(|a| a == powerdesk_core::autostart::FLAG || a == powerdesk_core::shellmode::SHELL_FLAG) && !st.borrow().prefs.autostart_window;
    if !hidden {
        ui.show()?;
    }
    // Pętla trwa także przy schowanym oknie – kończy ją dopiero quit().
    slint::run_event_loop_until_quit()
}

thread_local! {
    /// Palce na ekranie (id → położenie w pikselach okna) i odległość dwóch z poprzedniego ruchu.
    static TOUCHES: RefCell<(HashMap<u64, (f64, f64)>, Option<f64>)> = RefCell::new((HashMap::new(), None));
}

/// Szczypanie w przeglądarce: dwa palce na ekranie dotykowym albo gest gładzika. Zmiana
/// odległości palców to mnożnik zoomu wokół ich środka. Zwraca true, gdy zdarzenie zużyte.
fn pinch(win: &slint::Window, ev: &slint::winit_030::winit::event::WindowEvent) -> bool {
    use slint::winit_030::winit::event::{TouchPhase, WindowEvent};
    let Some(ui) = STATE.with(|s| s.borrow().as_ref().and_then(|s| s.borrow().ui.upgrade())) else { return false };
    let g = ui.global::<ViewerState>();
    if !g.get_open() {
        TOUCHES.with(|t| *t.borrow_mut() = (HashMap::new(), None));
        return false;
    }
    let scale = win.scale_factor() as f64;
    let step = |f: f64, x: f64, y: f64| {
        g.set_pinch_factor(f as f32);
        g.set_pinch_x((x / scale) as f32);
        g.set_pinch_y((y / scale) as f32);
        g.set_pinch_serial(g.get_pinch_serial().wrapping_add(1));
    };
    match ev {
        WindowEvent::PinchGesture { delta, .. } => {
            let (cx, cy) = (win.size().width as f64 / 2.0, win.size().height as f64 / 2.0);
            step(1.0 + delta, cx, cy);
            true
        }
        WindowEvent::Touch(t) => TOUCHES.with(|tt| {
            let (pts, last) = &mut *tt.borrow_mut();
            match t.phase {
                TouchPhase::Started | TouchPhase::Moved => {
                    pts.insert(t.id, (t.location.x, t.location.y));
                }
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    pts.remove(&t.id);
                }
            }
            if pts.len() != 2 {
                *last = None;
                return false;
            }
            let mut it = pts.values();
            let (Some(a), Some(b)) = (it.next(), it.next()) else { return false };
            let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
            if let Some(prev) = *last {
                if prev > 10.0 && (d / prev - 1.0).abs() > 0.01 {
                    step(d / prev, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                    *last = Some(d);
                }
            } else {
                *last = Some(d);
            }
            // Dwa palce – to gest, nie przesuwanie ani przewijanie.
            true
        }),
        _ => false,
    }
}

fn wire(ui: &MainWindow, st: &Shared) {
    // Boczne przyciski myszy: wstecz / dalej – w całym oknie.
    ui.window().on_winit_window_event(|win, ev| {
        use slint::winit_030::winit::event::{ElementState, MouseButton, WindowEvent};
        if pinch(win, ev) {
            return slint::winit_030::EventResult::PreventDefault;
        }
        if let WindowEvent::MouseInput { state: ElementState::Released, button, .. } = ev {
            let go: Option<fn(&Shared)> = match button {
                MouseButton::Back => Some(back),
                MouseButton::Forward => Some(forward),
                _ => None,
            };
            if let Some(go) = go {
                let _ = slint::invoke_from_event_loop(move || with_state(go));
                return slint::winit_030::EventResult::PreventDefault;
            }
        }
        slint::winit_030::EventResult::Propagate
    });
    ui.set_hints(ModelRc::new(VecModel::from(query::HINTS.iter().map(|h| SharedString::from(h.0)).collect::<Vec<_>>())));
    ui.set_hint_tips(ModelRc::new(VecModel::from(query::HINTS.iter().map(|h| SharedString::from(h.1)).collect::<Vec<_>>())));
    ui.on_drag({
        let w = ui.as_weak();
        move || {
            if let Some(ui) = w.upgrade() {
                ui.window().with_winit_window(|win| {
                    let _ = win.drag_window();
                });
            }
        }
    });
    ui.on_minimize({
        let w = ui.as_weak();
        move || {
            if let Some(ui) = w.upgrade() {
                ui.window().set_minimized(true);
            }
        }
    });
    ui.on_toggle_maximize({
        let w = ui.as_weak();
        move || {
            if let Some(ui) = w.upgrade() {
                let m = !ui.window().is_maximized();
                ui.window().set_maximized(m);
                ui.set_is_maximized(m);
            }
        }
    });
    ui.on_close_window(|| with_state(close_window));
    ui.window().on_close_requested(|| {
        with_state(close_window);
        slint::CloseRequestResponse::KeepWindowShown
    });

    let s = st.clone();
    ui.on_navigate(move |p| match p.strip_prefix(SAVED) {
        Some(id) => run_saved(&s, id, false),
        None if p == ADD_NET => add_network_place(&s),
        None => navigate(&s, p.as_str(), true),
    });
    let s = st.clone();
    ui.on_back(move || back(&s));
    let s = st.clone();
    ui.on_forward(move || forward(&s));
    let s = st.clone();
    ui.on_up(move || up(&s));
    let s = st.clone();
    ui.on_refresh(move || load(&s, true));
    let s = st.clone();
    ui.on_open(move |i| open(&s, i as usize));
    let s = st.clone();
    ui.on_select(move |i, ctrl, shift| select(&s, i as usize, ctrl, shift));
    let s = st.clone();
    ui.on_marquee_begin(move |additive| {
        tab_mut(&s, |t| {
            if !additive {
                t.selected.clear();
            }
        });
        let base = tab_mut(&s, |t| t.selected.clone());
        s.borrow_mut().marquee = Some(base);
        render_items(&s);
    });
    let s = st.clone();
    ui.on_marquee_update(move |x0, y0, x1, y1| {
        {
            let mut b = s.borrow_mut();
            let b = &mut *b;
            let Some(base) = &b.marquee else { return };
            let t = &mut b.tabs[b.active];
            let mut sel = base.clone();
            for i in t.in_rect(x0, y0, x1, y1) {
                sel.insert(t.items[i].path.clone());
            }
            if sel == t.selected {
                return;
            }
            t.selected = sel;
        }
        render_items(&s);
    });
    let s = st.clone();
    ui.on_marquee_end(move || s.borrow_mut().marquee = None);
    let s = st.clone();
    ui.on_viewport_changed(move |pane, top, w, h| {
        let pane = (pane as usize).min(1);
        let relayout = {
            let mut b = s.borrow_mut();
            if pane >= b.panes.len() {
                return;
            }
            let changed = (b.viewports[pane].1 - w).abs() > 0.5;
            b.viewports[pane] = (top, w, h);
            changed
        };
        if relayout {
            layout_pane(&s, pane);
            render_chrome(&s);
            render_items(&s);
        } else {
            scroll_items(&s, pane);
        }
    });
    let s = st.clone();
    ui.on_context(move |i, x, y, shift| context(&s, i as usize, x, y, shift));
    let s = st.clone();
    ui.on_background_context(move |x, y, shift| {
        tab_mut(&s, |t| t.selected.clear());
        render_items(&s);
        let path = tab_mut(&s, |t| t.path.clone());
        if path.starts_with("ofp:") {
            return;
        }
        if shift && !archives::browsing(&path) {
            native_menu(&s, &[path], x, y);
        } else {
            menus::background_menu(&s, x, y);
        }
    });

    // Filtr na żywo i wyszukiwanie w podfolderach.
    let s = st.clone();
    ui.on_filter_changed(move |q| set_filter(&s, q.as_str()));
    // Fasety (z rodzajem plików jako pierwszą grupą), podpowiedzi zapytań, zapisane wyszukiwania.
    let s = st.clone();
    ui.on_toggle_facets(move || {
        tab_mut(&s, |t| t.facets_open = Some(!t.facets_shown(true)));
        request_media(&s);
        render_chrome(&s);
    });
    let s = st.clone();
    ui.on_toggle_facet(move |id| {
        tab_mut(&s, |t| {
            // Zmiana rodzaju zmienia też dostępne fasety, więc wybrane się zerują.
            if let Some(f) = id.strip_prefix("kind:").and_then(|i| i.parse::<usize>().ok()).and_then(|i| KindFilter::ALL.get(i).copied()) {
                t.kind_filter = if t.kind_filter == f { KindFilter::All } else { f };
                t.facets.clear();
            } else if !t.facets.remove(id.as_str()) {
                t.facets.insert(id.to_string());
            }
        });
        set_scroll(&s, 0.0);
        refresh(&s);
    });
    let s = st.clone();
    ui.on_clear_facets(move || {
        tab_mut(&s, |t| {
            t.facets.clear();
            t.kind_filter = KindFilter::All;
        });
        refresh(&s);
    });
    let s = st.clone();
    ui.on_add_hint(move |h| {
        let q = {
            let Some(ui) = s.borrow().ui.upgrade() else { return };
            let q = ui.get_query();
            let q = if q.trim().is_empty() { format!("{h} ") } else { format!("{} {h} ", q.trim()) };
            ui.set_query(q.clone().into());
            q
        };
        set_filter(&s, &q);
    });
    let s = st.clone();
    ui.on_save_search(move || save_search(&s));
    let s = st.clone();
    ui.on_side_context(move |p, x, y| {
        if let Some(id) = p.strip_prefix(SAVED) {
            menus::saved_menu(&s, id.to_string(), x, y);
        } else if !p.starts_with("ofp:") {
            menus::side_menu(&s, p.to_string(), x, y);
        }
    });
    let s = st.clone();
    ui.on_side_resized(move |w| {
        let mut b = s.borrow_mut();
        b.prefs.side_width = w.clamp(180.0, 420.0);
        b.prefs.save();
    });
    let s = st.clone();
    ui.on_side_key(move |k| side_key(&s, k.as_str()));
    let s = st.clone();
    ui.on_search(move |q| start_search(&s, q.as_str()));
    let s = st.clone();
    ui.on_clear_search(move || clear_search(&s));

    // Sortowanie, widoki, wielkość miniatur.
    let s = st.clone();
    ui.on_sort_menu(move |x, y| menus::sort_menu(&s, x, y));
    let s = st.clone();
    ui.on_header_sort(move |id| {
        let k = match Col::ALL.get(id as usize) {
            Some(c) => match c.sort() {
                Some(k) => k,
                None => return,
            },
            None => SortKey::Name,
        };
        set_sort(&s, k);
    });

    // Kolumny listy: kolejność, szerokości, wybór.
    let s = st.clone();
    ui.on_col_move(move |id, dx| {
        let Some(&c) = Col::ALL.get(id as usize) else { return };
        {
            let mut b = s.borrow_mut();
            b.prefs.move_col(c, dx);
            b.prefs.save();
        }
        render_all(&s);
    });
    let s = st.clone();
    ui.on_col_resize(move |id, w| {
        let Some(&c) = Col::ALL.get(id as usize) else { return };
        s.borrow_mut().prefs.list_col_w.insert(c, w.clamp(48.0, 600.0).round());
        render_all(&s);
    });
    let s = st.clone();
    ui.on_col_resize_end(move || s.borrow().prefs.save());
    let s = st.clone();
    ui.on_col_reset(move |id| {
        let Some(&c) = Col::ALL.get(id as usize) else { return };
        {
            let mut b = s.borrow_mut();
            b.prefs.list_col_w.remove(&c);
            b.prefs.save();
        }
        render_all(&s);
    });
    let s = st.clone();
    ui.on_col_menu(move |x, y| menus::column_menu(&s, x, y));

    // Zmiana nazwy w miejscu.
    let s = st.clone();
    ui.on_rename_commit(move |t| {
        rename_commit(&s, t.as_str());
    });
    let s = st.clone();
    ui.on_rename_cancel(move || {
        tab_mut(&s, |t| t.renaming = None);
        render_items(&s);
    });
    // Pasek transferów, powiadomienia, okna dialogowe.
    let s = st.clone();
    ui.on_drag_start(move |i| dragdrop::start(&s, i as usize));
    let s = st.clone();
    ui.on_toggle_transfers(move || transfers::toggle_hidden(&s));
    ui.on_op_pause(|id, on| powerdesk_core::ops::pause(id as u64, on));
    ui.on_op_cancel(|id| powerdesk_core::ops::cancel(id as u64));
    let s = st.clone();
    ui.on_op_dismiss(move |id| transfers::dismiss(&s, id));
    let s = st.clone();
    ui.on_toast_action(move |id| transfers::toast_action(&s, id));
    let s = st.clone();
    ui.on_toast_close(move |id| transfers::close_toast(&s, id));
    let s = st.clone();
    ui.on_conflict_pick(move |i| transfers::conflict_pick(&s, i));
    let s = st.clone();
    ui.on_confirm_answer(move |yes| transfers::confirm_answer(&s, yes));
    let g = ui.global::<Bulk>();
    let s = st.clone();
    g.on_changed(move || bulk::refresh(&s));
    let s = st.clone();
    g.on_apply(move || bulk::apply(&s));
    let s = st.clone();
    g.on_cancel(move || bulk::close(&s));
    let s = st.clone();
    ui.global::<Prompt>().on_answer(move |ok, t| transfers::prompt_answer(&s, ok.then_some(t.as_str())));
    let s = st.clone();
    ui.on_toggle_preview(move || preview::toggle(&s));
    let g = ui.global::<PreviewState>();
    let s = st.clone();
    g.on_resize(move |w| preview::resize(&s, w));
    let s = st.clone();
    g.on_resize_done(move || preview::resize_done(&s));
    let s = st.clone();
    g.on_open_item(move || preview::open_item(&s));
    let s = st.clone();
    g.on_open_with(move || preview::open_with(&s));
    let s = st.clone();
    g.on_properties(move || props::open(&s));
    let s = st.clone();
    g.on_copy_path(move || preview::copy_path(&s));
    g.on_open_map(|| preview::open_map());
    let s = st.clone();
    g.on_pdf_step(move |d| preview::pdf_step(&s, d));
    player::bind(&ui, &st);
    home::bind(&ui, &st);
    settings::bind(&ui, &st);
    syncview::bind(&ui, &st);
    spaceview::bind(&ui, &st);
    termview::bind(&ui, &st);
    addr::bind(&ui, &st);
    let g = ui.global::<ViewerState>();
    let s = st.clone();
    g.on_close(move || viewer::close(&s));
    let s = st.clone();
    g.on_escape(move || viewer::escape(&s));
    let s = st.clone();
    g.on_step(move |d| viewer::step(&s, d));
    let s = st.clone();
    g.on_go(move |i| viewer::go(&s, i as usize));
    let s = st.clone();
    g.on_first_last(move |last| viewer::first_last(&s, last));
    let s = st.clone();
    g.on_toggle_fullscreen(move || viewer::toggle_fullscreen(&s));
    let s = st.clone();
    g.on_toggle_info(move || viewer::toggle_info(&s));
    let s = st.clone();
    g.on_open_external(move || viewer::open_external(&s));
    let g = ui.global::<ShelfState>();
    let s = st.clone();
    g.on_close(move || shelf::set_open(&s, false));
    let s = st.clone();
    g.on_toggle(move || shelf::toggle(&s));
    let s = st.clone();
    g.on_pick(move |i, ctrl| shelf::pick(&s, i as usize, ctrl));
    let s = st.clone();
    g.on_remove(move |i| shelf::remove(&s, i as usize));
    let s = st.clone();
    g.on_put(move |mv| shelf::put(&s, mv));
    let s = st.clone();
    g.on_clear(move || shelf::clear(&s));
    let s = st.clone();
    g.on_drag(move |i| shelf::drag(&s, i as usize));
    let g = ui.global::<Props>();
    let s = st.clone();
    g.on_close(move || props::close(&s));
    let s = st.clone();
    g.on_compute_hash(move || props::compute_hash(&s));
    let s = st.clone();
    g.on_copy(move |t| props::copy(&s, t.as_str()));
    let s = st.clone();
    g.on_system(move || props::system(&s));
    g.on_open_map(|| props::open_map());
    let s = st.clone();
    ui.on_toggle_section(move |n| toggle_section(&s, n.as_str()));
    let s = st.clone();
    ui.on_toggle_side_mini(move || toggle_side_mini(&s));
    let s = st.clone();
    ui.on_toggle_group(move |i| toggle_group(&s, i as usize));
    let s = st.clone();
    ui.on_select_group(move |i, ctrl| select_group(&s, i as usize, ctrl));
    let s = st.clone();
    ui.on_rename_next(move |t, back| rename_next(&s, t.as_str(), back));
    let s = st.clone();
    ui.on_set_view(move |v| set_view(&s, View::from_index(v)));
    let s = st.clone();
    ui.on_zoom_changed(move |z| set_zoom(&s, z));
    let s = st.clone();
    ui.on_zoom_step(move |d| {
        let (view, g, r) = {
            let b = s.borrow();
            (b.tabs[b.active].view, b.prefs.grid_size, b.prefs.gallery_row)
        };
        match view {
            View::List => {}
            View::Grid => set_zoom(&s, g + 12.0 * d as f32),
            View::Gallery => set_zoom(&s, r + 16.0 * d as f32),
        }
    });
    let s = st.clone();
    ui.on_crumb_menu(move |p, x, y| menus::crumb_menu(&s, p.as_str(), x, y));
    let s = st.clone();
    ui.on_new_menu(move |x, y| menus::new_menu(&s, x, y));
    let s = st.clone();
    ui.on_menu_chosen(move |i| menus::chosen(&s, i as usize));
    let s = st.clone();
    ui.on_command(move |c| command(&s, c.as_str()));
    let s = st.clone();
    ui.on_key(move |k, ctrl, shift, alt| key(&s, k.as_str(), ctrl, shift, alt));

    // Karty.
    let s = st.clone();
    ui.on_new_tab(move || new_tab(&s));
    let s = st.clone();
    ui.on_activate_tab(move |i| {
        save_scroll(&s);
        {
            let mut b = s.borrow_mut();
            let i = i as usize;
            // Karta otwarta w drugim panelu – klik w nią przenosi tam wskazanie.
            if b.panes.len() == 2 && b.panes[1 - b.focused] == i {
                b.focused = 1 - b.focused;
            }
            b.active = i;
            let f = b.focused;
            b.panes[f] = i;
        }
        show_tab(&s);
    });
    let s = st.clone();
    ui.on_close_tab(move |i| close_tab(&s, i as usize));
    let g = ui.global::<Palette>();
    let s = st.clone();
    g.on_typed(move |t| palette::query(&s, t.as_str()));
    let s = st.clone();
    g.on_step(move |d| palette::step(&s, d));
    let s = st.clone();
    g.on_run(move |row| palette::run(&s, (row >= 0).then_some(row)));
    let s = st.clone();
    g.on_close(move || palette::close(&s));
    let s = st.clone();
    ui.on_focus_pane(move |p| focus_pane(&s, p as usize));
    let s = st.clone();
    ui.on_split_resize(move |r| split_resize(&s, r));
    let s = st.clone();
    ui.on_move_tab(move |i, x| move_tab(&s, i as usize, x));
    let s = st.clone();
    ui.on_tab_menu(move |i, x, y| menus::tab_menu(&s, i as usize, x, y));
}

// ---------- Karty i nawigacja ----------

fn tab_mut<R>(s: &Shared, f: impl FnOnce(&mut Tab) -> R) -> R {
    let mut b = s.borrow_mut();
    let a = b.active;
    f(&mut b.tabs[a])
}

/// Nowa karta (Ctrl+T, „+”) – „Ten komputer”, jak nowe okno Eksploratora.
fn new_tab(s: &Shared) {
    new_tab_at(s, crate::model::HOME, true);
}

/// Nowa karta obok bieżącej; w tle (`activate == false`) jak „Otwórz w nowej karcie”.
fn new_tab_at(s: &Shared, path: &str, activate: bool) {
    save_scroll(s);
    let prev = s.borrow().active;
    let loose = first_loose(s);
    {
        let mut b = s.borrow_mut();
        let at = (b.active + 1).max(loose);
        b.tabs.insert(at, Tab::new(path));
        panes_inserted(&mut b, at);
        b.active = at;
    }
    apply_prefs(s);
    load(s, false);
    if !activate {
        s.borrow_mut().active = prev;
        show_tab(s);
    }
}

/// Karta z przeglądarką zdjęć obok bieżącej – bez wczytywania jakiegokolwiek folderu.
fn new_viewer_tab(s: &Shared, v: viewer::Viewer) {
    save_scroll(s);
    {
        let mut b = s.borrow_mut();
        let at = b.active + 1;
        // Następne zdjęcie z tej samej karty wraca do już otwartej przeglądarki, zamiast mnożyć karty.
        match b.tabs.get_mut(at).filter(|t| t.viewer.is_some()) {
            Some(t) => t.viewer = Some(v),
            None => {
                let mut t = Tab::new(viewer::TAB);
                t.viewer = Some(v);
                b.tabs.insert(at, t);
                panes_inserted(&mut b, at);
            }
        }
        b.active = at;
    }
    show_tab(s);
}

fn close_tab(s: &Shared, i: usize) {
    {
        let mut b = s.borrow_mut();
        if b.tabs.get(i).is_none_or(|t| t.pinned) {
            return;
        }
        if b.tabs.len() == 1 {
            drop(b);
            let _ = slint::quit_event_loop();
            return;
        }
        let gone = b.tabs.remove(i);
        // Zwykłe karty wracają przez Ctrl+Shift+T; przeglądarka zdjęć nie.
        if gone.viewer.is_none() && !gone.special() {
            b.closed.push(gone.path.clone());
            if b.closed.len() > 20 {
                b.closed.remove(0);
            }
        }
        if b.active >= b.tabs.len() || b.active > i {
            b.active = b.active.saturating_sub(1);
        }
        panes_removed(&mut b, i);
    }
    show_tab(s);
}

/// Przypięte karty z poprzedniego uruchomienia wracają po lewej; ich zawartość wczytuje się w tle.
fn restore_pinned(s: &Shared) {
    let paths: Vec<String> = s.borrow().prefs.pinned_tabs.clone();
    let paths: Vec<String> = paths.into_iter().filter(|p| Path::new(p).is_dir()).collect();
    if paths.is_empty() {
        return;
    }
    {
        let mut b = s.borrow_mut();
        for (i, p) in paths.iter().enumerate() {
            let mut t = Tab::new(p);
            t.pinned = true;
            b.tabs.insert(i, t);
            panes_inserted(&mut b, i);
        }
        b.active = paths.len();
    }
    // Zawartość przypiętych kart doczyta się, gdy użytkownik na nie przejdzie.
    render_chrome(s);
}


// ---------- Podział na dwa panele ----------

/// Numery kart w panelach po wstawieniu karty na miejscu `at`.
fn panes_inserted(b: &mut State, at: usize) {
    for p in b.panes.iter_mut() {
        if *p >= at {
            *p += 1;
        }
    }
}

/// Numery kart w panelach po usunięciu karty `at`; panel, który ją pokazywał,
/// bierze kartę obok, a gdy nie ma czego pokazać – podział się kończy.
fn panes_removed(b: &mut State, at: usize) {
    let last = b.tabs.len().saturating_sub(1);
    for p in b.panes.iter_mut() {
        if *p > at {
            *p -= 1;
        } else if *p == at {
            *p = at.min(last);
        }
    }
    if b.panes.len() == 2 && b.panes[0] == b.panes[1] {
        let keep = b.panes[b.focused];
        b.panes = vec![keep];
        b.focused = 0;
    }
    b.active = b.panes[b.focused];
}

/// Numery kart w panelach po przeniesieniu karty z `from` na `to`.
fn panes_moved(b: &mut State, from: usize, to: usize) {
    for p in b.panes.iter_mut() {
        if *p == from {
            *p = to;
        } else if from < *p && *p <= to {
            *p -= 1;
        } else if to <= *p && *p < from {
            *p += 1;
        }
    }
}

/// Karta wskazanego panelu jest tą „aktywną” – tak widzi ją cała reszta programu.
fn sync_panes(s: &Shared) {
    let mut b = s.borrow_mut();
    let f = b.focused;
    let active = b.active.min(b.tabs.len() - 1);
    b.active = active;
    b.panes[f] = active;
    if b.panes.len() == 2 {
        let other = b.panes[1 - f].min(b.tabs.len() - 1);
        b.panes[1 - f] = other;
    }
}

/// Ctrl+\ – podział obszaru plików na dwa panele i powrót do jednego.
fn toggle_split(s: &Shared) {
    save_scroll(s);
    let make_tab = {
        let mut b = s.borrow_mut();
        if b.panes.len() == 2 {
            let keep = b.panes[b.focused];
            b.panes = vec![keep];
            b.focused = 0;
            b.active = keep;
            None
        } else {
            let cur = b.active;
            // Obok staje inna otwarta karta, a gdy jej nie ma – nowa z tym samym folderem.
            match (0..b.tabs.len()).find(|i| *i != cur && b.tabs[*i].viewer.is_none()) {
                Some(other) => {
                    b.panes = vec![cur, other];
                    b.focused = 0;
                    None
                }
                None => Some(b.tabs[cur].path.clone()),
            }
        }
    };
    if let Some(path) = make_tab {
        let cur = s.borrow().active;
        new_tab_at(s, &path, false);
        let mut b = s.borrow_mut();
        let other = (0..b.tabs.len()).find(|i| *i != cur && b.tabs[*i].viewer.is_none()).unwrap_or(cur);
        b.panes = vec![cur, other];
        b.focused = 0;
        b.active = cur;
    }
    // Panele mają inną szerokość niż jeden obszar – układ liczymy od nowa.
    let n = s.borrow().panes.len();
    for pane in 0..n {
        layout_pane(s, pane);
    }
    show_tab(s);
}

/// Przeniesienie wskazania do panelu (klik w jego zawartość, F6).
fn focus_pane(s: &Shared, pane: usize) {
    {
        let b = s.borrow();
        if pane >= b.panes.len() || pane == b.focused {
            return;
        }
    }
    save_scroll(s);
    {
        let mut b = s.borrow_mut();
        b.focused = pane;
        b.active = b.panes[pane];
    }
    show_tab(s);
}

/// Nowy udział lewego panelu po przeciągnięciu uchwytu.
fn split_resize(s: &Shared, r: f32) {
    {
        let mut b = s.borrow_mut();
        if b.panes.len() < 2 {
            return;
        }
        b.split_ratio = r.clamp(0.2, 0.8);
    }
    render_chrome(s);
}

/// Pierwsze miejsce za przypiętymi kartami – nowe karty nie wchodzą między nie.
fn first_loose(s: &Shared) -> usize {
    s.borrow().tabs.iter().take_while(|t| t.pinned).count()
}

/// Przypięcie i odpięcie karty: przypięta idzie na koniec przypiętych, odpięta za nie.
fn pin_tab(s: &Shared, i: usize) {
    save_scroll(s);
    {
        let mut b = s.borrow_mut();
        let Some(t) = b.tabs.get_mut(i) else { return };
        if t.viewer.is_some() {
            return;
        }
        t.pinned = !t.pinned;
        // Przypięta trafia na koniec przypiętych, odpięta – zaraz za nie.
        let to = b.tabs.iter().enumerate().filter(|(j, t)| *j != i && t.pinned).count();
        let t = b.tabs.remove(i);
        b.tabs.insert(to, t);
        panes_moved(&mut b, i, to);
        b.active = to;
        let paths: Vec<String> = b.tabs.iter().filter(|t| t.pinned && !t.special()).map(|t| t.path.clone()).collect();
        b.prefs.pinned_tabs = paths;
        b.prefs.save();
    }
    show_tab(s);
}

/// Nowe miejsce karty po przeciągnięciu; `x` – wskaźnik w pikselach logicznych paska.
fn move_tab(s: &Shared, from: usize, x: f32) {
    let to = {
        let b = s.borrow();
        if from >= b.tabs.len() {
            return;
        }
        // Szerokości jak w titlebar.slint: przypięta 40 px, zwykła 210 px, odstęp 4 px, wcięcie 8 px.
        let mut edge = 8.0;
        let mut to = b.tabs.len() - 1;
        for (i, t) in b.tabs.iter().enumerate() {
            let w = if t.pinned { 40.0 } else { 210.0 };
            if x < edge + w / 2.0 {
                to = i;
                break;
            }
            edge += w + 4.0;
        }
        // Przypięte zostają przed zwykłymi.
        let pinned = b.tabs.iter().take_while(|t| t.pinned).count();
        if b.tabs[from].pinned {
            to.min(pinned.saturating_sub(1))
        } else {
            to.max(pinned)
        }
    };
    if to == from {
        return;
    }
    {
        let mut b = s.borrow_mut();
        let t = b.tabs.remove(from);
        b.tabs.insert(to, t);
        panes_moved(&mut b, from, to);
        if b.active == from {
            b.active = to;
        } else if b.active > from && b.active <= to {
            b.active -= 1;
        } else if b.active < from && b.active >= to {
            b.active += 1;
        }
        if b.tabs[to].pinned {
            let paths: Vec<String> = b.tabs.iter().filter(|t| t.pinned && !t.special()).map(|t| t.path.clone()).collect();
            b.prefs.pinned_tabs = paths;
            b.prefs.save();
        }
    }
    show_tab(s);
}

/// Ctrl+Shift+T – ostatnio zamknięta karta wraca na koniec.
fn reopen_closed(s: &Shared) {
    let Some(path) = s.borrow_mut().closed.pop() else { return };
    save_scroll(s);
    {
        let mut b = s.borrow_mut();
        b.tabs.push(Tab::new(&path));
        b.active = b.tabs.len() - 1;
        let f = b.focused;
        b.panes[f] = b.active;
    }
    apply_prefs(s);
    load(s, false);
}

/// Zamyka wszystkie karty poza `i` (przypięte zostają).
fn close_others(s: &Shared, i: usize) {
    // Od końca, żeby numery kart przed `i` się nie przesuwały.
    let n = s.borrow().tabs.len();
    for j in (0..n).rev() {
        if j != i && !s.borrow().tabs[j].pinned {
            close_tab(s, j);
        }
    }
    let at = s.borrow().tabs.iter().take(i).filter(|t| t.pinned).count();
    let last = s.borrow().tabs.len() - 1;
    s.borrow_mut().active = at.min(last);
    show_tab(s);
}

/// Zamyka karty po prawej stronie `i`.
fn close_right(s: &Shared, i: usize) {
    while s.borrow().tabs.len() > i + 1 {
        let last = s.borrow().tabs.len() - 1;
        if s.borrow().tabs[last].pinned {
            break;
        }
        close_tab(s, last);
    }
}

fn save_scroll(s: &Shared) {
    let top = { let b = s.borrow(); b.viewports[b.focused].0 };
    tab_mut(s, |t| t.scroll = top);
}

/// Przełączenie karty: zawartość zwykle jest już wczytana, tylko odświeżamy widok.
/// Przypięte karty z poprzedniego uruchomienia wczytują się przy pierwszym wejściu.
fn show_tab(s: &Shared) {
    sync_panes(s);
    if !tab_mut(s, |t| t.loaded || t.special()) {
        viewer::sync_tab(s);
        apply_prefs(s);
        load(s, false);
        return;
    }
    let (scroll, filter) = tab_mut(s, |t| (t.scroll, t.filter.clone()));
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_query(filter.into());
    }
    layout(s);
    render_all(s);
    set_scroll(s, scroll);
    watch(s);
    viewer::sync_tab(s);
}

fn navigate(s: &Shared, path: &str, push: bool) {
    let mut path = path.trim().trim_matches('"').to_string();
    if path.is_empty() {
        return;
    }
    if path.len() == 2 && path.ends_with(':') {
        path.push('\\');
    }
    if !path.starts_with("ofp:") && !Path::new(&path).is_dir() && !archives::browsing(&path) {
        // Ścieżka do pliku: otwieramy jego folder.
        match Path::new(&path).parent().filter(|p| p.is_dir()) {
            Some(parent) => path = parent.to_string_lossy().to_string(),
            None => return,
        }
    }
    let prev = tab_mut(s, |t| {
        if t.search.as_ref().map(|x| x.running).unwrap_or(false) {
            fsx::cancel_search();
        }
        if push {
            t.history.truncate(t.h_index + 1);
            t.history.push(path.clone());
            t.h_index = t.history.len() - 1;
        }
        t.remember_selection();
        t.renaming = None;
        let prev = std::mem::replace(&mut t.path, path.clone());
        t.search = None;
        t.filter.clear();
        t.kind_filter = KindFilter::All;
        t.facets.clear();
        t.facets_open = None;
        t.selected.clear();
        t.focus = None;
        t.anchor = None;
        prev
    });
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_query(SharedString::new());
        ui.set_query_chips(ModelRc::default());
    }
    apply_prefs(s);
    load(s, false);
    // Po wejściu „w górę” zaznaczamy folder, z którego przyszliśmy – jak w Eksploratorze.
    let came_up = Path::new(&prev).parent().map(|p| same(&p.to_string_lossy(), &path)).unwrap_or(false);
    if came_up {
        let i = tab_mut(s, |t| {
            let i = t.items.iter().position(|e| same(&e.path, &prev));
            if let Some(i) = i {
                t.selected.insert(t.items[i].path.clone());
                t.focus = Some(i);
                t.anchor = Some(i);
            }
            i
        });
        if let Some(i) = i {
            reveal(s, i);
        }
        render_items(s);
    } else if let Some(i) = tab_mut(s, |t| t.restore_selection()) {
        // Powrót do folderu przywraca poprzednie zaznaczenie.
        reveal(s, i);
        render_items(s);
    }
}

fn back(s: &Shared) {
    if tab_mut(s, |t| t.search.is_some()) {
        return clear_search(s);
    }
    let i = tab_mut(s, |t| t.h_index);
    if i > 0 {
        jump(s, i - 1);
    }
}

fn forward(s: &Shared) {
    let (i, n) = tab_mut(s, |t| (t.h_index, t.history.len()));
    if i + 1 < n {
        jump(s, i + 1);
    }
}

fn up(s: &Shared) {
    let p = tab_mut(s, |t| t.path.clone());
    if let Some(parent) = Path::new(&p).parent() {
        navigate(s, &parent.to_string_lossy(), true);
    }
}

fn jump(s: &Shared, idx: usize) {
    let p = tab_mut(s, |t| {
        t.h_index = idx;
        t.history[idx].clone()
    });
    navigate(s, &p, false);
}

fn same(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

/// Widok i sortowanie zapamiętane dla folderu.
fn apply_prefs(s: &Shared) {
    let mut b = s.borrow_mut();
    let a = b.active;
    let fallback = model::FolderPrefs { view: b.prefs.default_view, ..Default::default() };
    let p = b.prefs.folders.get(&b.tabs[a].path.to_lowercase()).copied().unwrap_or(fallback);
    let t = &mut b.tabs[a];
    t.view = p.view;
    t.sort = p.sort;
    t.desc = p.desc;
    t.group = p.group;
    t.collapsed.clear();
}

fn save_prefs(s: &Shared) {
    let mut b = s.borrow_mut();
    let a = b.active;
    let t = &b.tabs[a];
    if t.special() || t.search.is_some() {
        return;
    }
    let (key, fp) = (t.path.to_lowercase(), model::FolderPrefs { view: t.view, sort: t.sort, desc: t.desc, group: t.group });
    b.prefs.folders.insert(key, fp);
    b.prefs.save();
}

fn load(s: &Shared, keep_selection: bool) {
    let path = tab_mut(s, |t| {
        t.generation += 1;
        t.path.clone()
    });
    let res = if let Some(folders) = library_folders(&path) {
        list_library(&folders)
    } else {
        match powerdesk_core::archive::split(&path).filter(|_| !path.starts_with("ofp:")) {
            Some((arc, inner)) => powerdesk_core::archive::list(&arc, &inner),
            None => fsx::list_dir(&path),
        }
    };
    let auto = {
        let b = s.borrow();
        b.prefs.auto_gallery && !b.prefs.folders.contains_key(&path.to_lowercase())
    };
    tab_mut(s, |t| {
        t.loaded = true;
        match res {
            Ok(list) => {
                t.entries = list;
                t.error.clear();
            }
            Err(e) => {
                t.entries.clear();
                t.error = e;
            }
        }
        if !keep_selection {
            t.selected.clear();
            t.focus = None;
            t.anchor = None;
            // Folder pełen zdjęć/filmów od razu otwiera się w galerii (o ile nie ustawiono inaczej).
            if auto && t.entries.len() >= 6 {
                let files: Vec<_> = t.entries.iter().filter(|e| !e.dir).collect();
                let media = files.iter().filter(|e| fmt::is_media(e)).count();
                if !files.is_empty() && media as f32 / files.len() as f32 >= 0.7 {
                    t.view = View::Gallery;
                }
            }
        }
    });
    refresh(s);
    if !keep_selection {
        set_scroll(s, 0.0);
    }
    watch(s);
}

/// Obserwujemy foldery wszystkich otwartych kart.
fn watch(s: &Shared) {
    let b = s.borrow();
    let mut dirs: Vec<String> = b.tabs.iter().filter(|t| !t.special()).flat_map(|t| library_folders(&t.path).unwrap_or_else(|| vec![t.path.clone()])).collect();
    dirs.sort();
    dirs.dedup();
    fsx::watch_dirs(dirs);
}

/// Filtr/sortowanie → układ → widok.
fn refresh(s: &Shared) {
    {
        let mut b = s.borrow_mut();
        let b = &mut *b;
        let (hidden, ff) = (b.prefs.show_hidden, b.prefs.folders_first);
        let (sizes, media) = (&b.dir_sizes, &b.media);
        b.tabs[b.active].rebuild(hidden, ff, |e| size_of(sizes, e), |e| media.get(&e.path).copied());
    }
    request_media(s);
    request_dir_sizes(s);
    layout(s);
    render_all(s);
}

/// Układ elementów we wskazanym panelu (szerokość bierze z jego widoku).
fn layout_pane(s: &Shared, pane: usize) {
    let mut b = s.borrow_mut();
    let b = &mut *b;
    let Some(&tab) = b.panes.get(pane) else { return };
    let (w, g, r) = (b.viewports[pane].1, b.prefs.grid_size, b.prefs.gallery_row);
    let media = &b.media;
    let t = &mut b.tabs[tab];
    t.layout(w, g, r, |e| ratio_of(media, e));
}

/// Układ panelu wskazanego – tak działa cała reszta programu, która nie wie o podziale.
fn layout(s: &Shared) {
    let pane = s.borrow().focused;
    layout_pane(s, pane);
}

/// Jak `ratioOf` w FileView.svelte.
fn ratio_of(media: &HashMap<String, shell::MediaInfo>, e: &fsx::Entry) -> f32 {
    if e.dir || !fmt::is_media(e) {
        return 1.0;
    }
    match media.get(&e.path) {
        None => 1.5,
        Some(m) if m.w == 0 || m.h == 0 => 1.0,
        Some(m) => (m.w as f32 / m.h as f32).clamp(0.33, 3.2),
    }
}

/// Rozmiar do sortowania – dla folderu policzony w tle albo -1 (jak `sizeOf` w state.svelte.ts).
fn size_of(sizes: &HashMap<String, u64>, e: &fsx::Entry) -> i64 {
    if e.dir {
        sizes.get(&e.path.to_lowercase()).map(|n| *n as i64).unwrap_or(-1)
    } else {
        e.size as i64
    }
}

thread_local! {
    static MEDIA_REDRAW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Po partii metadanych – jedno przerysowanie na 150 ms, nawet gdy partie przychodzą szybciej.
fn media_redraw(s: &Shared) {
    if MEDIA_REDRAW.with(|r| r.replace(true)) {
        return;
    }
    let s = s.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(150), move || {
        MEDIA_REDRAW.with(|r| r.set(false));
        // Sortowanie po metadanych – kolejność zmienia się wraz z kolejnymi partiami.
        if tab_mut(&s, |t| t.sort.media() || t.group == GroupKey::Taken || !t.facets.is_empty()) {
            refresh(&s);
        } else {
            layout(&s);
            render_all(&s);
        }
    });
}

/// Metadane zdjęć i filmów – partiami w tle (galeria albo kolumny z metadanymi), potem ponowny układ.
fn request_media(s: &Shared) {
    let todo: Vec<(String, i64)> = {
        let mut b = s.borrow_mut();
        let b = &mut *b;
        let t = &b.tabs[b.active];
        let cols = t.view == View::List && b.prefs.list_cols.iter().any(|c| c.media());
        let facets = t.facets_shown(true);
        if t.view != View::Gallery && !cols && !t.sort.media() && t.group != GroupKey::Taken && !facets {
            return;
        }
        let v: Vec<(String, i64)> = t
            .items
            .iter()
            .filter(|e| fmt::is_media(e) && !b.media.contains_key(&e.path) && !b.media_pending.contains(&e.path))
            .map(|e| (e.path.clone(), e.modified))
            .collect();
        for (p, _) in &v {
            b.media_pending.insert(p.clone());
        }
        v
    };
    if todo.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for chunk in todo.chunks(200) {
            let info = shell::media_info(chunk.to_vec());
            let res: Vec<(String, shell::MediaInfo)> = chunk.iter().zip(info).map(|((p, _), m)| (p.clone(), m)).collect();
            let _ = slint::invoke_from_event_loop(move || {
                with_state(|s| {
                    {
                        let mut b = s.borrow_mut();
                        for (p, m) in res {
                            b.media_pending.remove(&p);
                            b.media.insert(p, m);
                        }
                    }
                    media_redraw(s);
                })
            });
        }
    });
}

// ---------- Renderowanie ----------

/// Kolejka rozmiarów folderów: nowsze żądania (bieżący folder) wyprzedzają stare, liczą dwa wątki naraz.
static DS_QUEUE: LazyLock<Mutex<std::collections::VecDeque<String>>> = LazyLock::new(Default::default);
static DS_RUNNING: AtomicUsize = AtomicUsize::new(0);

/// Rozmiary folderów – tylko gdy widać kolumnę Rozmiar albo sortujemy po rozmiarze; bez dysków sieciowych.
fn request_dir_sizes(s: &Shared) {
    let todo: Vec<String> = {
        let mut b = s.borrow_mut();
        let b = &mut *b;
        let t = &b.tabs[b.active];
        let shown = (t.view == View::List && b.prefs.list_cols.contains(&Col::Size)) || t.sort == SortKey::Size;
        if !b.prefs.folder_sizes || t.search.is_some() || !shown {
            return;
        }
        let net: Vec<String> = b.drives.iter().filter(|d| d.kind == "network").map(|d| d.path[..2.min(d.path.len())].to_lowercase()).collect();
        let v: Vec<String> = t
            .items
            .iter()
            .filter(|e| e.dir && !e.path.starts_with("\\\\"))
            .map(|e| e.path.to_lowercase())
            .filter(|p| !b.dir_sizes.contains_key(p) && !b.dir_pending.contains(p) && !net.iter().any(|n| p.starts_with(n.as_str())))
            .collect();
        b.dir_pending.extend(v.iter().cloned());
        v
    };
    if todo.is_empty() {
        return;
    }
    {
        let mut q = DS_QUEUE.lock().unwrap();
        for p in todo.into_iter().rev() {
            q.push_front(p);
        }
    }
    while DS_RUNNING.load(Ordering::SeqCst) < 2 {
        DS_RUNNING.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(|| {
            loop {
                let next = DS_QUEUE.lock().unwrap().pop_front();
                let Some(p) = next else { break };
                let size = fsx::dir_size(&p).size;
                let _ = slint::invoke_from_event_loop(move || {
                    with_state(|s| {
                        let by_size = {
                            let mut b = s.borrow_mut();
                            b.dir_pending.remove(&p);
                            b.dir_sizes.insert(p, size);
                            b.tabs[b.active].sort == SortKey::Size
                        };
                        if by_size {
                            refresh(s);
                        } else {
                            render_items(s);
                        }
                    })
                });
            }
            DS_RUNNING.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// Ścieżka pozycji panelu bocznego dla zapisanego wyszukiwania: „ofp:saved|<id>”.
pub const SAVED: &str = "ofp:saved|";

fn set_filter(s: &Shared, q: &str) {
    tab_mut(s, |t| t.filter = q.to_string());
    if let Some(ui) = s.borrow().ui.upgrade() {
        let chips: Vec<SharedString> = query::parse(q).chips.into_iter().map(Into::into).collect();
        ui.set_query_chips(ModelRc::new(VecModel::from(chips)));
    }
    refresh(s);
}

fn save_search(s: &Shared) {
    let name = {
        let mut b = s.borrow_mut();
        let t = &b.tabs[b.active];
        let Some(x) = &t.search else { return };
        let base = Path::new(&t.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| t.path.clone());
        let v = SavedSearch {
            id: format!("{:x}", chrono::Local::now().timestamp_millis()),
            name: format!("{} w {base}", x.query),
            root: t.path.clone(),
            query: x.query.clone(),
        };
        let name = v.name.clone();
        b.prefs.saved_searches.push(v);
        b.prefs.save();
        name
    };
    transfers::toast(s, format!("Zapisano „{name}” w panelu bocznym"), transfers::Kind::Ok, None);
    render_chrome(s);
}

/// Uruchamia zapisane wyszukiwanie (w nowej karcie albo w bieżącej).
pub fn run_saved(s: &Shared, id: &str, new: bool) {
    let Some(v) = s.borrow().prefs.saved_searches.iter().find(|v| v.id == id).cloned() else { return };
    if new {
        new_tab_at(s, &v.root, true);
    } else if !tab_mut(s, |t| same(&t.path, &v.root)) {
        navigate(s, &v.root, true);
    }
    start_search(s, &v.query);
}

/// Nowa nazwa zapisanego wyszukiwania (okno z polem tekstowym).
pub fn rename_saved(s: &Shared, id: &str) {
    let Some(name) = s.borrow().prefs.saved_searches.iter().find(|v| v.id == id).map(|v| v.name.clone()) else { return };
    let id = id.to_string();
    transfers::prompt(
        s,
        "Zmień nazwę wyszukiwania",
        &name,
        "Zmień",
        Rc::new(move |s: &Shared, n: &str| {
            {
                let mut b = s.borrow_mut();
                if let Some(v) = b.prefs.saved_searches.iter_mut().find(|v| v.id == id) {
                    v.name = n.to_string();
                }
                b.prefs.save();
            }
            render_side(s);
        }),
    );
}

pub fn remove_saved(s: &Shared, id: &str) {
    {
        let mut b = s.borrow_mut();
        b.prefs.saved_searches.retain(|v| v.id != id);
        b.prefs.save();
    }
    render_side(s);
}

fn render_all(s: &Shared) {
    render_chrome(s);
    render_items(s);
    // Półka pokazuje folder docelowy – zmienia się razem z kartą.
    shelf::render(s);
    // Ekran „Ten komputer” zamiast listy, gdy to jego karta.
    home::sync(s);
    settings::sync(s);
    syncview::sync_tab(s);
    spaceview::sync_tab(s);
    termview::follow(s);
}

/// Wszystko poza samymi elementami: karty, pasek narzędzi, panel boczny, stan.
/// Napis na pustym widoku: błąd, wynik filtra albo pusty folder.
fn empty_text(t: &Tab) -> String {
    if !t.error.is_empty() {
        return t.error.clone();
    }
    if !t.items.is_empty() {
        return String::new();
    }
    match &t.search {
        Some(x) if x.running => "Szukam…".into(),
        Some(_) => "Nic nie znaleziono".into(),
        None if !t.filter.is_empty() || t.kind_filter != KindFilter::All => "Brak elementów pasujących do filtra".into(),
        None => "Ten folder jest pusty".into(),
    }
}

/// Kolumny listy: kolejność i szerokości są wspólne, podświetlone sortowanie bierze się z karty.
fn columns_of(prefs: &Prefs, t: &Tab) -> Vec<Column> {
    let has_type = prefs.list_cols.contains(&Col::Type);
    prefs
        .list_cols
        .iter()
        .map(|&c| Column {
            id: Col::ALL.iter().position(|x| *x == c).unwrap_or(0) as i32,
            label: c.label().into(),
            w: prefs.col_w(c),
            right: c.right(),
            sorted: c.sort() == Some(t.sort) && (c != Col::Ext || !has_type),
        })
        .collect()
}

fn render_chrome(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.tabs[b.active];
    ui.set_path(t.path.clone().into());
    ui.set_special(t.special());
    ui.set_can_back(t.h_index > 0 || t.search.is_some());
    ui.set_can_forward(t.h_index + 1 < t.history.len());
    ui.set_crumbs(ModelRc::new(VecModel::from(crumbs(&t.path, &b.drives))));
    ui.set_view(t.view.index());
    ui.set_gallery_ok(t.view == View::Gallery || mostly_media(t));
    ui.set_sort_name(t.sort == SortKey::Name);
    ui.set_folder(if t.path.starts_with("ofp:") || t.search.is_some() { SharedString::new() } else { t.path.clone().into() });
    ui.set_desc(t.desc);
    let flist = if t.special() { Vec::new() } else { t.facet_list(b.prefs.show_hidden, |e| b.media.get(&e.path).copied()) };
    // Kategorie są w panelu filtrów jako grupa „Rodzaj” – tylko gdy w folderze są co najmniej dwie.
    let kinds: Vec<(KindFilter, usize)> = if t.special() { Vec::new() } else { t.shown_filters(b.prefs.show_hidden).into_iter().filter(|(f, _)| *f != KindFilter::All).collect() };
    let kinds = if kinds.len() > 1 || t.kind_filter != KindFilter::All { kinds } else { Vec::new() };
    let any = !flist.is_empty() || !kinds.is_empty();
    let shown = t.facets_shown(any);
    ui.set_has_facets(any);
    ui.set_facets_open(shown);
    ui.set_facets_chosen(t.facets.len() as i32 + (t.kind_filter != KindFilter::All) as i32);
    ui.set_filter_summary(filter_summary(t, &flist).into());
    let filtered = !t.facets.is_empty() || t.kind_filter != KindFilter::All;
    ui.set_facet_results(if filtered { format!("{} wyników", t.items.len()).into() } else { SharedString::new() });
    // Panel faset: do 620 px szerokości, minus wewnętrzne marginesy.
    let width = (ui.window().size().to_logical(ui.window().scale_factor()).width - 48.0).min(620.0) - 24.0;
    ui.set_facet_rows(ModelRc::new(VecModel::from(if shown { facet_rows(&kinds, t.kind_filter, &flist, &t.facets, width) } else { Vec::new() })));
    ui.set_can_save_search(match &t.search {
        Some(x) => !x.running && !b.prefs.saved_searches.iter().any(|v| v.query == x.query && same(&v.root, &t.path)),
        None => false,
    });
    ui.set_columns(ModelRc::new(VecModel::from(columns_of(&b.prefs, t))));
    ui.set_sort_label(
        if t.group == GroupKey::None { t.sort.label().to_string() } else { format!("{} · {}", t.sort.label(), t.group.label()) }.into(),
    );
    let (zmin, zmax, z) = match t.view {
        View::Gallery => (120.0, 420.0, b.prefs.gallery_row),
        _ => (84.0, 280.0, b.prefs.grid_size),
    };
    ui.set_zoom_min(zmin);
    ui.set_zoom_max(zmax);
    ui.set_zoom(z);
    ui.set_searching(t.search.as_ref().map(|x| x.running).unwrap_or(false));
    ui.set_search_active(t.search.is_some());
    ui.set_total_height(t.total_h);
    ui.set_empty_text(empty_text(t).into());
    // Tytuł okna = bieżąca karta – tak widać ją na pasku zadań i w Alt+Tab (jak w Eksploratorze).
    ui.set_window_title(t.title().into());
    ui.set_tabs(ModelRc::new(VecModel::from(
        b.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabData {
                tip: if t.special() { t.title().into() } else { t.path.clone().into() },
                path: if t.special() { SharedString::new() } else { t.path.clone().into() },
                title: t.title().into(),
                icon: tab_icon(t),
                active: i == b.active,
                pinned: t.pinned,
                loading: t.search.as_ref().map(|x| x.running).unwrap_or(false),
            })
            .collect::<Vec<_>>(),
    )));
    drop(b);
    render_pane2(s);
    render_side(s);
    render_status(s);
}

/// Drugi panel: te same dane co pasek narzędzi daje wskazanemu, ale dla karty obok.
/// Pasek adresu i polecenia zostają jedne – pracują na wskazanym panelu.
fn render_pane2(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let split = b.panes.len() == 2;
    ui.set_split(split);
    ui.set_split_ratio(b.split_ratio);
    ui.set_focused_pane(b.focused as i32);
    if !split {
        return;
    }
    let t = &b.tabs[b.panes[1]];
    ui.set_view2(t.view.index());
    ui.set_sort_name2(t.sort == SortKey::Name);
    ui.set_desc2(t.desc);
    ui.set_folder2(if t.path.starts_with("ofp:") || t.search.is_some() { SharedString::new() } else { t.path.clone().into() });
    ui.set_total_height2(t.total_h);
    ui.set_empty_text2(empty_text(t).into());
    ui.set_scroll_y2(-b.viewports[1].0);
    ui.set_columns2(ModelRc::new(VecModel::from(columns_of(&b.prefs, t))));
}

fn render_status(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.tabs[b.active];
    let mut text = fmt::items(t.items.len());
    if let Some(x) = &t.search {
        text = format!("{text} · przeszukano {}", x.scanned);
        let chips = query::parse(&x.query).chips;
        if !chips.is_empty() {
            text = format!("{text} · {}", chips.join(" · "));
        }
    }
    let sel: Vec<_> = t.items.iter().filter(|e| t.selected.contains(&e.path)).collect();
    if !sel.is_empty() {
        let bytes: u64 = sel.iter().filter(|e| !e.dir).map(|e| e.size).sum();
        text = format!("{text} · zaznaczono {}", sel.len());
        if bytes > 0 {
            text = format!("{text} ({})", fmt::size(bytes));
        }
    }
    ui.set_status(text.into());
    ui.set_has_selection(!sel.is_empty());
}

/// Elementy w oknie widoku – reszta nie istnieje w drzewie Slint (wirtualne przewijanie).
/// Pełne przerysowanie: zmieniły się dane (zaznaczenie, miniatury, układ, folder).
pub fn render_items(s: &Shared) {
    s.borrow_mut().epoch += 1;
    paint_items(s);
    render_status(s);
    // Podgląd nadąża za zaznaczeniem (sam pomija, gdy nic się nie zmieniło).
    preview::sync(s);
}

/// Przewinięcie: rysujemy z zapasem (ekran wyżej i dwa niżej), a ponownie dopiero,
/// gdy widok zbliży się do krawędzi narysowanego obszaru – Slint sam przesuwa treść.
fn scroll_items(s: &Shared, pane: usize) {
    let skip = {
        let b = s.borrow();
        let (top, _, h) = b.viewports[pane];
        let (lo, hi) = b.painted[pane];
        top - h * 0.3 >= lo && top + h * 1.3 <= hi
    };
    if !skip {
        paint_pane(s, pane);
    }
}

/// Stałe sloty: element i zawsze trafia do wiersza i % pojemność, więc przy przewijaniu
/// zmieniają się tylko wiersze, które weszły w zakres – reszta zostaje nietknięta.
fn paint_pane(s: &Shared, pane: usize) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let Some((lo, hi, range)) = ({
        let b = s.borrow();
        b.panes.get(pane).map(|&tab| {
            let (top, _, h) = b.viewports[pane];
            let (lo, hi) = (top - h, top + h * 2.0);
            (lo, hi, b.tabs[tab].visible_in(lo, hi))
        })
    }) else {
        return;
    };
    let model = if pane == 0 { ui.get_items() } else { ui.get_items2() };
    let fits = s.borrow().slots[pane].len();
    let vm = match model.as_any().downcast_ref::<VecModel<VisItem>>() {
        Some(vm) if fits >= range.len() && vm.row_count() == fits && fits > 0 => vm,
        _ => {
            let cap = (range.len() + 64).next_multiple_of(32);
            s.borrow_mut().slots[pane] = vec![None; cap];
            let fresh = ModelRc::new(VecModel::from(vec![VisItem::default(); cap]));
            if pane == 0 {
                ui.set_items(fresh);
            } else {
                ui.set_items2(fresh);
            }
            return paint_pane(s, pane);
        }
    };
    let scale = ui.window().scale_factor();
    let mut b = s.borrow_mut();
    let cap = b.slots[pane].len();
    let epoch = b.epoch;
    let mut slots = std::mem::take(&mut b.slots[pane]);
    let st: &State = &*b;
    let t = &st.tabs[st.panes[pane]];
    for (slot, cur) in slots.iter_mut().enumerate() {
        // Indeks z zakresu, który wypada na ten slot (najwyżej jeden, bo zakres ≤ pojemność).
        let want = {
            let first = range.start - range.start % cap + slot;
            let i = if first < range.start { first + cap } else { first };
            (i < range.end).then_some(i)
        };
        match (want, *cur) {
            (Some(i), Some((j, e))) if j == i && e == epoch => continue,
            (None, None) => continue,
            _ => {}
        }
        let item = want.and_then(|i| {
            let (e, bx) = (t.items.get(i)?, t.boxes.get(i)?);
            (bx.w > 0.0).then(|| vis_item(st, t, i, e, *bx, scale))
        });
        match item {
            Some(it) => {
                vm.set_row_data(slot, it);
                *cur = want.map(|i| (i, epoch));
            }
            None => {
                if cur.is_some() {
                    vm.set_row_data(slot, VisItem::default());
                }
                *cur = None;
            }
        }
    }
    let pad = if t.view == View::List { 6.0 } else { 12.0 };
    let heads: Vec<GroupHead> = match &t.groups {
        Some(groups) => t
            .heads
            .iter()
            .filter(|hd| hd.y + 38.0 >= lo && hd.y <= hi)
            .map(|hd| GroupHead {
                index: hd.group as i32,
                x: pad,
                y: hd.y,
                w: (st.viewports[pane].1 - pad * 2.0).max(100.0),
                label: groups[hd.group].label.clone().into(),
                count: (groups[hd.group].end - groups[hd.group].start) as i32,
                shut: hd.shut,
            })
            .collect(),
        None => Vec::new(),
    };
    let old = if pane == 0 { ui.get_heads() } else { ui.get_heads2() };
    if old.row_count() != heads.len() || old.iter().zip(&heads).any(|(a, b)| a != *b) {
        let fresh = ModelRc::new(VecModel::from(heads));
        if pane == 0 {
            ui.set_heads(fresh);
        } else {
            ui.set_heads2(fresh);
        }
    }
    b.slots[pane] = slots;
    b.painted[pane] = (lo, hi);
}

/// Rysuje oba panele (drugi tylko przy podziale).
fn paint_items(s: &Shared) {
    let n = s.borrow().panes.len();
    for pane in 0..n {
        paint_pane(s, pane);
    }
}

/// Dotarły miniatury lub ikony: odświeżamy tylko wiersze tych plików, a panel boczny – gdy dotyczy.
pub fn thumbs_arrived(s: &Shared, paths: &HashSet<String>) {
    desktop::thumbs_arrived(paths);
    taskbar::thumbs_arrived(paths);
    start::thumbs_arrived(paths);
    cliphistory::thumbs_arrived(paths);
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let side = s.borrow().side_paths.iter().any(|p| paths.contains(p));
    {
        let b = s.borrow();
        let scale = ui.window().scale_factor();
        for pane in 0..b.panes.len() {
            let model = if pane == 0 { ui.get_items() } else { ui.get_items2() };
            let Some(vm) = model.as_any().downcast_ref::<VecModel<VisItem>>() else { continue };
            let t = &b.tabs[b.panes[pane]];
            for (slot, cur) in b.slots[pane].iter().enumerate() {
                let Some((i, _)) = cur else { continue };
                let (Some(e), Some(bx)) = (t.items.get(*i), t.boxes.get(*i)) else { continue };
                if paths.contains(&e.path) && slot < vm.row_count() {
                    vm.set_row_data(slot, vis_item(&b, t, *i, e, *bx, scale));
                }
            }
        }
    }
    if side {
        render_side(s);
    }
    preview::thumbs_arrived(s, paths);
    viewer::thumbs_arrived(s, paths);
    home::thumbs_arrived(s, paths);
    if shelf::shows_any(s, paths) {
        shelf::render(s);
    }
}

fn vis_item(b: &State, t: &Tab, i: usize, e: &fsx::Entry, bx: model::BoxR, scale: f32) -> VisItem {
    let media = fmt::is_media(e);
    let (thumb, icon) = match t.view {
        View::List => (None, thumbs::get(&e.path, (18.0 * scale) as u32, true)),
        View::Grid => (thumbs::get(&e.path, ((bx.w - 12.0) * scale) as u32, e.dir), None),
        View::Gallery => (thumbs::get(&e.path, (bx.w.max(bx.h) * scale) as u32, e.dir || !media), None),
    };
    VisItem {
        tip: tip(b, t, e).into(),
        index: i as i32,
        x: bx.x,
        y: bx.y,
        w: bx.w,
        h: bx.h,
        name: e.name.clone().into(),
        path: e.path.clone().into(),
        cells: if t.view == View::List { ModelRc::new(VecModel::from(cells(b, e))) } else { ModelRc::default() },
        dir: e.dir,
        hidden: e.hidden,
        media,
        selected: t.selected.contains(&e.path),
        focused: t.focus == Some(i),
        renaming: t.renaming.as_deref() == Some(e.path.as_str()),
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        has_thumb: thumb.is_some(),
        thumb: thumb.unwrap_or_default(),
    }
}

/// Zdjęcia i filmy to ponad połowa plików na liście (foldery się nie liczą).
fn mostly_media(t: &Tab) -> bool {
    let (files, media) = t.items.iter().filter(|e| !e.dir).fold((0, 0), |(f, m), e| (f + 1, m + fmt::is_media(e) as usize));
    files > 0 && media * 2 > files
}

/// Opis aktywnych filtrów do paska adresu: rodzaj, potem wybrane fasety.
fn filter_summary(t: &Tab, flist: &[facets::Facet]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if t.kind_filter != KindFilter::All {
        parts.push(t.kind_filter.short().into());
    }
    parts.extend(t.facets.iter().map(|id| flist.iter().find(|f| &f.id == id).map(|f| f.label.clone()).unwrap_or_else(|| id.clone())));
    parts.join(" · ")
}

/// Fasety po grupach: każda grupa od nowego wiersza, z nazwą w kolumnie po lewej; filtry grupy
/// zawijają się w obrębie dostępnej szerokości. Pierwsza grupa to rodzaj plików.
fn facet_rows(kinds: &[(KindFilter, usize)], kind: KindFilter, list: &[facets::Facet], chosen: &std::collections::BTreeSet<String>, width: f32) -> Vec<FacetRow> {
    // Szacunek szerokości żetonu (Segoe UI 12 px, licznik 11 px, marginesy) – wystarczy do zawijania.
    let est = |label: &str, count: &str| (label.chars().count() as f32 * 12.0 + count.chars().count() as f32 * 11.0) * 0.56 + 26.0;
    let chip = |id: String, label: &str, count: usize, on: bool, tip: String| FacetChip { kind: 1, tip: tip.into(), id: id.into(), label: label.into(), count: count.to_string().into(), on };
    let mut groups: Vec<(String, Vec<FacetChip>)> = Vec::new();
    if !kinds.is_empty() {
        let v = kinds
            .iter()
            .map(|(f, n)| {
                let i = KindFilter::ALL.iter().position(|x| x == f).unwrap_or(0);
                chip(format!("kind:{i}"), f.short(), *n, *f == kind, format!("{} ({n})", f.label()))
            })
            .collect();
        groups.push(("Rodzaj".into(), v));
    }
    for f in list {
        let c = chip(f.id.clone(), &f.label, f.count, chosen.contains(&f.id), format!("{}: {} ({})", f.group, f.label, f.count));
        match groups.last_mut().filter(|(g, _)| g == f.group) {
            Some((_, v)) => v.push(c),
            None => groups.push((f.group.to_string(), vec![c])),
        }
    }
    // Kolumna nazw grup: 76 px plus odstęp.
    let room = (width - 84.0).max(120.0);
    let mut rows = Vec::new();
    for (group, chips) in groups {
        let mut line: Vec<FacetChip> = Vec::new();
        let mut x = 0.0;
        let mut first = true;
        for c in chips {
            let w = est(&c.label, &c.count);
            if !line.is_empty() && x + w > room {
                rows.push(FacetRow { group: if std::mem::take(&mut first) { group.clone().into() } else { SharedString::new() }, items: ModelRc::new(VecModel::from(std::mem::take(&mut line))) });
                x = 0.0;
            }
            x += w + 5.0;
            line.push(c);
        }
        rows.push(FacetRow { group: if first { group.into() } else { SharedString::new() }, items: ModelRc::new(VecModel::from(line)) });
    }
    rows
}

/// Podpowiedź elementu jak w Eksploratorze (tip w FileView.svelte).
fn tip(b: &State, t: &Tab, e: &fsx::Entry) -> String {
    let mut lines = vec![e.name.clone(), format!("Typ: {}", fmt::type_label(e))];
    if e.dir {
        if let Some(n) = b.dir_sizes.get(&e.path.to_lowercase()) {
            lines.push(format!("Rozmiar: {}", fmt::size(*n)));
        }
        lines.push(format!("Data utworzenia: {}", fmt::date(e.created)));
    } else {
        if let Some(m) = b.media.get(&e.path) {
            if m.w > 0 {
                lines.push(format!("Wymiary: {} × {}", m.w, m.h));
            }
            if m.dur > 0 {
                lines.push(format!("Długość: {}", fmt::duration(m.dur)));
            }
            if m.taken > 0 && m.taken != e.modified {
                lines.push(format!("Data wykonania: {}", fmt::date(m.taken)));
            }
        }
        lines.push(format!("Rozmiar: {}", fmt::size(e.size)));
        lines.push(format!("Data modyfikacji: {}", fmt::date(e.modified)));
    }
    let attrs: Vec<&str> = [(e.readonly, "tylko do odczytu"), (e.hidden, "ukryty")].iter().filter(|x| x.0).map(|x| x.1).collect();
    if !attrs.is_empty() {
        lines.push(format!("Atrybuty: {}", attrs.join(", ")));
    }
    if e.cloud {
        lines.push("Dostępny tylko w chmurze".into());
    }
    if t.search.is_some() {
        if let Some(p) = Path::new(&e.path).parent() {
            lines.push(format!("Lokalizacja: {}", p.to_string_lossy()));
        }
    }
    lines.join("\n")
}

/// Teksty kolumn listy (`cell` w FileView.svelte).
fn cells(b: &State, e: &fsx::Entry) -> Vec<SharedString> {
    let m = b.media.get(&e.path);
    b.prefs
        .list_cols
        .iter()
        .map(|c| match c {
            Col::Modified => fmt::date(e.modified),
            Col::Created => fmt::date(e.created),
            Col::Type => fmt::type_label(e),
            Col::Ext => if e.dir { String::new() } else { e.ext.to_uppercase() },
            Col::Size if e.dir => match b.dir_sizes.get(&e.path.to_lowercase()) {
                Some(n) => fmt::size(*n),
                None if b.prefs.folder_sizes => "…".into(),
                None => String::new(),
            },
            Col::Size => fmt::size(e.size),
            Col::Dims => m.filter(|m| m.w > 0).map(|m| format!("{} × {}", m.w, m.h)).unwrap_or_default(),
            Col::Taken => m.filter(|m| m.taken > 0).map(|m| fmt::date(m.taken)).unwrap_or_default(),
            Col::Duration => m.filter(|m| m.dur > 0).map(|m| fmt::duration(m.dur)).unwrap_or_default(),
            Col::Attrs => [(e.readonly, "R"), (e.hidden, "H"), (e.system, "S"), (e.link, "L"), (e.cloud, "C")]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, l)| *l)
                .collect::<Vec<_>>()
                .join(" "),
        }
        .into())
        .collect()
}

fn set_scroll(s: &Shared, top: f32) {
    let ui = s.borrow().ui.upgrade();
    if let Some(ui) = ui {
        let pane = {
            let mut b = s.borrow_mut();
            let pane = b.focused;
            b.viewports[pane].0 = top;
            pane
        };
        if pane == 0 {
            ui.set_scroll_y(-top);
        } else {
            ui.set_scroll_y2(-top);
        }
        render_items(s);
    }
}

/// Przewija tak, żeby element był widoczny.
fn reveal(s: &Shared, i: usize) {
    let (top, h, bx) = {
        let b = s.borrow();
        (b.viewports[b.focused].0, b.viewports[b.focused].2, b.tabs[b.active].boxes.get(i).copied())
    };
    let Some(bx) = bx else { return };
    if bx.y < top {
        set_scroll(s, (bx.y - 6.0).max(0.0));
    } else if bx.y + bx.h > top + h {
        set_scroll(s, bx.y + bx.h - h + 6.0);
    }
}

// ---------- Zaznaczanie i otwieranie ----------

fn select(s: &Shared, i: usize, ctrl: bool, shift: bool) {
    tab_mut(s, |t| {
        let Some(e) = t.items.get(i) else { return };
        let p = e.path.clone();
        if shift {
            let from = t.anchor.unwrap_or(i);
            let (lo, hi) = (from.min(i), from.max(i));
            if !ctrl {
                t.selected.clear();
            }
            for e in &t.items[lo..=hi] {
                t.selected.insert(e.path.clone());
            }
        } else if ctrl {
            if !t.selected.remove(&p) {
                t.selected.insert(p);
            }
            t.anchor = Some(i);
        } else {
            t.selected.clear();
            t.selected.insert(p);
            t.anchor = Some(i);
        }
        t.focus = Some(i);
    });
    render_items(s);
}

/// Otwiera element listy po ścieżce (menu, panel podglądu).
pub fn open_path(s: &Shared, path: &str) {
    if let Some(i) = tab_mut(s, |t| t.items.iter().position(|e| e.path == path)) {
        open(s, i);
    }
}

fn open(s: &Shared, i: usize) {
    let e = tab_mut(s, |t| t.items.get(i).cloned());
    let Some(e) = e else { return };
    if e.dir || archives::browsable(&e) {
        navigate(s, &e.path, true);
    } else if fmt::is_media(&e) && !archives::inside(&e.path) && viewer::open(s, i) {
        // Zdjęcia i filmy – w przeglądarce w oknie aplikacji.
    } else if archives::inside(&e.path) {
        archives::open_file(&e.path);
    } else {
        let _ = open::that_detached(&e.path);
    }
}

fn hwnd(s: &Shared) -> isize {
    let Some(ui) = s.borrow().ui.upgrade() else { return 0 };
    window_hwnd(ui.window())
}

/// Uchwyt Win32 dowolnego okna Slint (główne, pulpit); 0 – okno jeszcze nie powstało.
pub fn window_hwnd(win: &slint::Window) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    win
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => 0,
        })
        .unwrap_or(0)
}

/// Prawy przycisk na elemencie: własne menu, z Shift – od razu menu Eksploratora.
fn context(s: &Shared, i: usize, x: f32, y: f32, shift: bool) {
    let paths = tab_mut(s, |t| {
        let Some(e) = t.items.get(i) else { return Vec::new() };
        if !t.selected.contains(&e.path) {
            t.selected.clear();
            t.selected.insert(e.path.clone());
            t.focus = Some(i);
            t.anchor = Some(i);
        }
        t.items.iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.path.clone()).collect::<Vec<_>>()
    });
    render_items(s);
    if paths.is_empty() {
        return;
    }
    // Elementy z wnętrza archiwum nie istnieją dla Eksploratora – tam zostaje własne menu.
    if shift && !paths.iter().any(|p| archives::inside(p)) {
        native_menu(s, &paths, x, y);
    } else {
        menus::item_menu(s, x, y);
    }
}

/// Pełne menu Eksploratora (IContextMenu) w miejscu kliknięcia.
fn native_menu(s: &Shared, paths: &[String], x: f32, y: f32) {
    let scale = s.borrow().ui.upgrade().map(|u| u.window().scale_factor()).unwrap_or(1.0);
    let h = hwnd(s);
    if let Ok(true) = shell::native_menu(h, paths, (x * scale) as i32, (y * scale) as i32, false) {
        load(s, true);
    }
}

pub fn selected_paths(s: &Shared) -> Vec<String> {
    tab_mut(s, |t| t.items.iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.path.clone()).collect())
}

fn command(s: &Shared, c: &str) {
    let sel = selected_paths(s);
    match c {
        "cut" => menus::copy(&sel, true),
        "copy" => menus::copy(&sel, false),
        "paste" => menus::paste(s, None),
        "trash" => menus::trash(s, &sel),
        "props" => props::open(s),
        "rename" if archives::read_only(s, &sel) => {}
        "rename" if sel.len() == 1 => start_rename(s),
        "rename" if sel.len() > 1 => bulk::open(s),
        _ => {}
    }
}

fn select_all(s: &Shared) {
    tab_mut(s, |t| t.selected = t.items.iter().map(|e| e.path.clone()).collect());
    render_items(s);
}

/// Zaznacza element o podanej ścieżce (np. cel skrótu) i przewija do niego.
fn select_path(s: &Shared, path: &str) {
    let i = tab_mut(s, |t| {
        let i = t.items.iter().position(|e| same(&e.path, path));
        if let Some(i) = i {
            t.selected.clear();
            t.selected.insert(t.items[i].path.clone());
            t.focus = Some(i);
            t.anchor = Some(i);
        }
        i
    });
    if let Some(i) = i {
        reveal(s, i);
    }
    render_items(s);
}

pub fn create(s: &Shared, folder: bool) {
    create_named(s, if folder { "Nowy folder" } else { "Nowy dokument tekstowy.txt" }, folder);
}

/// Tworzy element w bieżącym folderze, zaznacza go i otwiera pole nazwy (jak w Eksploratorze).
pub fn create_named(s: &Shared, name: &str, folder: bool) {
    let Some(p) = make(s, name, folder) else { return };
    load(s, true);
    select_path(s, &p);
    start_rename(s);
}

/// Sam zapis na dysku; błąd trafia do powiadomienia.
fn make(s: &Shared, name: &str, folder: bool) -> Option<String> {
    let dir = tab_mut(s, |t| t.path.clone());
    if dir.starts_with("ofp:") {
        transfers::toast(s, "Tu nie można niczego utworzyć – otwórz zwykły folder", transfers::Kind::Info, None);
        return None;
    }
    if archives::read_only_dest(s, &dir) {
        return None;
    }
    powerdesk_core::ops::create(&dir, name, folder).map_err(|e| transfers::toast(s, e, transfers::Kind::Error, None)).ok()
}

/// Nowy folder i przeniesienie do niego zaznaczonych elementów (newFolderWith).
pub fn create_with_selection(s: &Shared) {
    let sel = selected_paths(s);
    if sel.is_empty() {
        return;
    }
    let Some(p) = make(s, &format!("Nowy folder ({} elem.)", sel.len()), true) else { return };
    transfers::transfer(s, sel, &p, "move");
    load(s, true);
    select_path(s, &p);
    start_rename(s);
}

// ---------- Zmiana nazwy w miejscu ----------

/// Pole nazwy nad zaznaczonym elementem; zaznaczona jest nazwa bez rozszerzenia.
pub fn start_rename(s: &Shared) {
    let sel = tab_mut(s, |t| {
        let e = t.items.iter().find(|e| t.selected.contains(&e.path))?.clone();
        if archives::inside(&e.path) {
            return None;
        }
        t.renaming = Some(e.path.clone());
        let i = t.items.iter().position(|x| x.path == e.path);
        t.focus = i;
        Some((e, i))
    });
    let Some((e, i)) = sel else { return };
    let dot = if e.dir { None } else { e.name.rfind('.').filter(|d| *d > 0) };
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_rename_sel(dot.unwrap_or(e.name.len()) as i32);
    }
    if let Some(i) = i {
        reveal(s, i);
    }
    render_items(s);
}

/// Zatwierdza nazwę; zwraca nową ścieżkę (albo starą, gdy nic się nie zmieniło).
fn rename_commit(s: &Shared, name: &str) -> Option<String> {
    let path = tab_mut(s, |t| t.renaming.take())?;
    let old = Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if name.trim().is_empty() || name == old {
        render_items(s);
        return Some(path);
    }
    match powerdesk_core::ops::rename_one(&path, name) {
        Ok(p) => {
            load(s, true);
            select_path(s, &p);
            Some(p)
        }
        Err(e) => {
            transfers::toast(s, e, transfers::Kind::Error, None);
            render_items(s);
            None
        }
    }
}

/// Tab / Shift+Tab: zatwierdza i przechodzi do zmiany nazwy sąsiada.
fn rename_next(s: &Shared, name: &str, back: bool) {
    let Some(p) = rename_commit(s, name) else { return };
    let next = tab_mut(s, |t| {
        let i = t.items.iter().position(|e| e.path == p)?;
        let j = if back { i.checked_sub(1)? } else { i + 1 };
        t.items.get(j).map(|e| e.path.clone())
    });
    if let Some(n) = next {
        select_path(s, &n);
        start_rename(s);
    }
}


// ---------- Sortowanie, widoki, wyszukiwanie ----------

pub fn set_group(s: &Shared, g: GroupKey) {
    tab_mut(s, |t| {
        t.group = g;
        t.collapsed.clear();
    });
    save_prefs(s);
    refresh(s);
}

fn toggle_group(s: &Shared, gi: usize) {
    tab_mut(s, |t| {
        let Some(label) = t.groups.as_ref().and_then(|g| g.get(gi)).map(|g| g.label.clone()) else { return };
        if !t.collapsed.remove(&label) {
            t.collapsed.insert(label);
        }
    });
    layout(s);
    render_all(s);
}

/// Klik w etykietę grupy zaznacza jej elementy (Ctrl – dokłada).
fn select_group(s: &Shared, gi: usize, ctrl: bool) {
    tab_mut(s, |t| {
        let Some(g) = t.groups.as_ref().and_then(|g| g.get(gi)).cloned() else { return };
        if !ctrl {
            t.selected.clear();
        }
        for e in &t.items[g.start..g.end] {
            t.selected.insert(e.path.clone());
        }
        t.focus = Some(g.start);
        t.anchor = Some(g.start);
    });
    render_items(s);
}

fn set_sort(s: &Shared, k: SortKey) {
    tab_mut(s, |t| {
        if t.sort == k {
            t.desc = !t.desc;
        } else {
            t.sort = k;
            t.desc = matches!(k, SortKey::Modified | SortKey::Created | SortKey::Size);
        }
    });
    save_prefs(s);
    refresh(s);
}

fn set_view(s: &Shared, v: View) {
    tab_mut(s, |t| t.view = v);
    save_prefs(s);
    refresh(s);
    if let Some(f) = tab_mut(s, |t| t.focus) {
        reveal(s, f);
    }
}

fn set_zoom(s: &Shared, z: f32) {
    {
        let mut b = s.borrow_mut();
        let a = b.active;
        match b.tabs[a].view {
            View::Gallery => b.prefs.gallery_row = z.clamp(120.0, 420.0),
            _ => b.prefs.grid_size = z.clamp(84.0, 280.0),
        }
        b.prefs.save();
    }
    layout(s);
    render_all(s);
}

fn start_search(s: &Shared, q: &str) {
    let (root, hidden) = {
        let b = s.borrow();
        (b.tabs[b.active].path.clone(), b.prefs.show_hidden)
    };
    if root.starts_with("ofp:") {
        return;
    }
    // Słowa idą do wyszukiwarki, filtry (typ:, rozmiar:, data:…) sprawdzamy przy odbiorze wyników.
    let id = fsx::search(Core, root, query::parse(q).text, hidden);
    tab_mut(s, |t| {
        t.filter.clear();
        t.selected.clear();
        t.focus = None;
        t.search = Some(Search { query: q.to_string(), id, running: true, scanned: 0, results: Vec::new() });
    });
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_query(SharedString::new());
        ui.set_query_chips(ModelRc::default());
    }
    set_scroll(s, 0.0);
    refresh(s);
}

fn clear_search(s: &Shared) {
    let had = tab_mut(s, |t| {
        t.filter.clear();
        if t.search.as_ref().map(|x| x.running).unwrap_or(false) {
            fsx::cancel_search();
        }
        t.search.take().is_some()
    });
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_query(SharedString::new());
        ui.set_query_chips(ModelRc::default());
    }
    refresh(s);
    if had {
        set_scroll(s, 0.0);
    }
}

#[derive(serde::Deserialize)]
struct SearchBatch {
    id: u64,
    items: Vec<fsx::Entry>,
    done: bool,
    scanned: u64,
}

fn on_event(s: &Shared, name: &str, payload: serde_json::Value) {
    match name {
        "sync-status" => {
            syncview::on_status(s);
            render_side(s);
        }
        "space-progress" => spaceview::on_progress(s, payload),
        "term-data" => termview::on_data(s, payload),
        "term-exit" => termview::on_exit(s, payload),
        "search" => {
            let Ok(batch) = serde_json::from_value::<SearchBatch>(payload) else { return };
            let hit = {
                let mut b = s.borrow_mut();
                let mut hit = false;
                for t in b.tabs.iter_mut() {
                    if let Some(x) = t.search.as_mut().filter(|x| x.id == batch.id) {
                        x.results.extend(batch.items.iter().cloned());
                        x.running = !batch.done;
                        x.scanned = batch.scanned;
                        hit = true;
                    }
                }
                hit
            };
            if hit {
                refresh(s);
            }
        }
        "op-progress" => {
            if let Ok(p) = serde_json::from_value::<powerdesk_core::ops::Progress>(payload) {
                transfers::on_progress(s, p);
            }
        }
        "drives-changed" => {
            s.borrow_mut().drives = drives::list();
            render_side(s);
        }
        "fs-changed" => {
            let Ok(dirs) = serde_json::from_value::<Vec<String>>(payload) else { return };
            {
                let mut b = s.borrow_mut();
                let low: Vec<String> = dirs.iter().map(|d| d.to_lowercase()).collect();
                b.dir_sizes.retain(|k, _| !low.iter().any(|d| d.starts_with(k.as_str())));
            }
            let (path, searching) = tab_mut(s, |t| (t.path.clone(), t.search.is_some()));
            if !searching && dirs.iter().any(|d| same(d, &path)) {
                load(s, true);
            }
        }
        _ => {}
    }
}

// ---------- Klawiatura ----------

fn key(s: &Shared, k: &str, ctrl: bool, shift: bool, alt: bool) {
    use slint::platform::Key;
    let is = |key: Key| k == SharedString::from(key).as_str();
    // Przy otwartej palecie klawisze obsługuje ona sama; skrót zamyka ją z powrotem.
    if palette::is_open(s) {
        // Skrót tej samej palety zamyka, skrót drugiej przełącza na nią.
        if ctrl && shift && k.eq_ignore_ascii_case("p") {
            palette::toggle(s, false);
        } else if ctrl && k.eq_ignore_ascii_case("k") {
            palette::toggle(s, true);
        }
        return;
    }
    if ctrl && k == "," {
        return settings::open(s);
    }
    if ctrl && k == "`" {
        return termview::toggle(s);
    }
    if alt && is(Key::LeftArrow) {
        return back(s);
    }
    if alt && is(Key::RightArrow) {
        return forward(s);
    }
    if alt && is(Key::UpArrow) {
        return up(s);
    }
    if alt && is(Key::Return) {
        return command(s, "props");
    }
    if alt && k.eq_ignore_ascii_case("p") {
        return preview::toggle(s);
    }
    if ctrl && shift && k.eq_ignore_ascii_case("s") {
        return shelf::shortcut(s);
    }
    if ctrl && shift && k.eq_ignore_ascii_case("n") {
        return create(s, true);
    }
    if ctrl && shift && k.eq_ignore_ascii_case("t") {
        return reopen_closed(s);
    }
    if ctrl && k == "\\" {
        return toggle_split(s);
    }
    if ctrl && shift && k.eq_ignore_ascii_case("p") {
        return palette::open(s);
    }
    if ctrl && k.eq_ignore_ascii_case("k") {
        return palette::open_places(s);
    }
    if is(Key::F6) {
        let (panes, focused) = { let b = s.borrow(); (b.panes.len(), b.focused) };
        if panes == 2 {
            return focus_pane(s, 1 - focused);
        }
        return;
    }
    if ctrl && k.eq_ignore_ascii_case("b") {
        return toggle_side_mini(s);
    }
    if ctrl && k == "t" {
        return new_tab(s);
    }
    if ctrl && k == "w" {
        let a = s.borrow().active;
        return close_tab(s, a);
    }
    if ctrl && is(Key::Tab) {
        save_scroll(s);
        {
            let mut b = s.borrow_mut();
            let n = b.tabs.len();
            b.active = (b.active + if shift { n - 1 } else { 1 }) % n;
            let f = b.focused;
            b.panes[f] = b.active;
        }
        return show_tab(s);
    }
    if ctrl && (k == "1" || k == "2" || k == "3") {
        return set_view(s, View::from_index(k.parse::<i32>().unwrap_or(1) - 1));
    }
    if ctrl && k == "h" {
        {
            let mut b = s.borrow_mut();
            b.prefs.show_hidden = !b.prefs.show_hidden;
            b.prefs.save();
        }
        return refresh(s);
    }
    if ctrl && k == "a" {
        return select_all(s);
    }
    if ctrl && shift && k.eq_ignore_ascii_case("c") {
        let sel = selected_paths(s);
        let _ = powerdesk_core::clip::set_text(&sel.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\n"));
        return;
    }
    if ctrl && k == "c" {
        return command(s, "copy");
    }
    if ctrl && k == "x" {
        return command(s, "cut");
    }
    if ctrl && k == "v" {
        return command(s, "paste");
    }
    if is(Key::Delete) && !shift {
        return command(s, "trash");
    }
    if is(Key::Delete) && shift {
        return transfers::delete_permanently(s, selected_paths(s));
    }
    if ctrl && (k == "z" || k == "Z") && !shift {
        return transfers::undo(s);
    }
    if ctrl && (k == "y" || k == "Y" || ((k == "z" || k == "Z") && shift)) {
        return transfers::redo(s);
    }
    if is(Key::F5) {
        return load(s, true);
    }
    // Spacja – zdjęcie albo film pod kursorem w przeglądarce (Podgląd).
    if k == " " && !ctrl && !alt {
        if let Some(i) = tab_mut(s, |t| t.focus.filter(|i| t.items.get(*i).is_some_and(|e| fmt::is_media(e)))) {
            viewer::open(s, i);
            return;
        }
    }
    if is(Key::F2) {
        return command(s, "rename");
    }
    if is(Key::Backspace) {
        return if tab_mut(s, |t| t.search.is_some()) { back(s) } else { up(s) };
    }
    if is(Key::Escape) {
        tab_mut(s, |t| t.selected.clear());
        return render_items(s);
    }
    if is(Key::Return) {
        if let Some(f) = tab_mut(s, |t| t.focus) {
            open(s, f);
        }
        return;
    }
    // Pisanie przeskakuje do pasującej nazwy (kolejne litery w ciągu 0,8 s wydłużają początek).
    // Klawisze specjalne Slinta leżą w obszarze prywatnym U+F700–U+F8FF.
    if !ctrl && !alt && k.chars().count() == 1 && !k.chars().any(|c| c.is_control() || (0xF700..=0xF8FF).contains(&(c as u32))) {
        return type_ahead(s, k);
    }
    let dir = if is(Key::LeftArrow) {
        Some((-1, 0))
    } else if is(Key::RightArrow) {
        Some((1, 0))
    } else if is(Key::UpArrow) {
        Some((0, -1))
    } else if is(Key::DownArrow) {
        Some((0, 1))
    } else if is(Key::Home) {
        Some((-9, 0))
    } else if is(Key::End) {
        Some((9, 0))
    } else {
        None
    };
    if let Some(d) = dir {
        move_focus(s, d, shift, ctrl);
    }
}

thread_local! {
    static TYPED: RefCell<(String, Option<std::time::Instant>)> = const { RefCell::new((String::new(), None)) };
}

fn type_ahead(s: &Shared, k: &str) {
    let typed = TYPED.with(|t| {
        let mut t = t.borrow_mut();
        if t.1.is_none_or(|at| at.elapsed() > std::time::Duration::from_millis(800)) {
            t.0.clear();
        }
        t.0.push_str(&k.to_lowercase());
        t.1 = Some(std::time::Instant::now());
        t.0.clone()
    });
    let i = tab_mut(s, |t| {
        let n = t.items.len();
        if n == 0 {
            return None;
        }
        let f = t.focus.unwrap_or(n - 1);
        // Pierwsza litera szuka od następnego elementu – ta sama litera chodzi po kolejnych.
        let start = if typed.chars().count() == 1 { f + 1 } else { f };
        let i = (0..n).map(|k| (start + k) % n).find(|&i| t.items[i].name.to_lowercase().starts_with(&typed))?;
        t.selected.clear();
        t.selected.insert(t.items[i].path.clone());
        t.focus = Some(i);
        t.anchor = Some(i);
        Some(i)
    });
    if let Some(i) = i {
        reveal(s, i);
        render_items(s);
    }
}

/// Strzałki po geometrii układu: w lewo/prawo – sąsiad, w górę/dół – najbliższy w sąsiednim wierszu.
fn move_focus(s: &Shared, (dx, dy): (i32, i32), shift: bool, ctrl: bool) {
    let to = tab_mut(s, |t| {
        let n = t.items.len();
        if n == 0 {
            return None;
        }
        let Some(f) = t.focus else { return Some(0) };
        let horizontal = t.view != View::List;
        let center = |i: usize| t.boxes[i].x + t.boxes[i].w / 2.0;
        Some(match (dx, dy) {
            (-9, _) => 0,
            (9, _) => n - 1,
            (dx, 0) if horizontal => (f as i32 + dx).clamp(0, n as i32 - 1) as usize,
            (_, 0) => f,
            (_, dy) => {
                let (cy, cx) = (t.boxes[f].y, center(f));
                let mut j = f as i32;
                // Pierwszy element w sąsiednim wierszu…
                loop {
                    j += dy;
                    if j < 0 || j >= n as i32 {
                        return Some(f);
                    }
                    if t.boxes[j as usize].w > 0.0 && (t.boxes[j as usize].y - cy).abs() > 1.0 {
                        break;
                    }
                }
                // …i w nim ten najbliższy w poziomie.
                let row_y = t.boxes[j as usize].y;
                let mut best = j as usize;
                while j >= 0 && j < n as i32 && t.boxes[j as usize].w > 0.0 && (t.boxes[j as usize].y - row_y).abs() < 1.0 {
                    if (center(j as usize) - cx).abs() < (center(best) - cx).abs() {
                        best = j as usize;
                    }
                    j += dy;
                }
                best
            }
        })
        // Elementy zwiniętych grup są pomijane – idziemy dalej w tym samym kierunku.
        .map(|to| {
            let back = (dx < 0 && dx != -9) || dy < 0 || dx == 9;
            let mut k = to as i32;
            while k >= 0 && k < n as i32 && t.boxes[k as usize].w <= 0.0 {
                k += if back { -1 } else { 1 };
            }
            if k >= 0 && k < n as i32 { k as usize } else { f }
        })
    });
    let Some(to) = to else { return };
    if ctrl && !shift {
        tab_mut(s, |t| t.focus = Some(to));
        render_items(s);
    } else {
        select(s, to, false, shift);
    }
    reveal(s, to);
}

// ---------- Pomocnicze ----------

fn ic(name: &str) -> SharedString {
    icons::icon(name).into()
}

fn tab_icon(t: &Tab) -> SharedString {
    let path = &t.path;
    if t.viewer.is_some() {
        ic("gallery")
    } else if t.search.is_some() {
        ic("search")
    } else if path == HOME {
        ic("pc")
    } else if path == settings::TAB {
        ic("settings")
    } else if path == syncview::TAB {
        ic("sync")
    } else if path.starts_with(spaceview::PREFIX) {
        ic("grid")
    } else if path.len() <= 3 && path.contains(':') {
        ic("drive")
    } else if path.starts_with("\\\\") {
        ic("network")
    } else {
        ic("folder")
    }
}

/// Okruszki: Dysk (C:) → Users → redzi.
fn crumbs(path: &str, drives: &[drives::Drive]) -> Vec<Crumb> {
    if path.starts_with("ofp:") {
        return Vec::new();
    }
    let mut v = Vec::new();
    let mut acc = PathBuf::new();
    for c in Path::new(path).components() {
        acc.push(c.as_os_str());
        let name = match c {
            std::path::Component::Prefix(_) => continue,
            std::path::Component::RootDir => {
                let root = acc.to_string_lossy().to_string();
                let letter = root.trim_end_matches('\\').to_string();
                let label = drives.iter().find(|d| same(&d.path, &root)).map(|d| d.label.clone()).unwrap_or_default();
                if label.is_empty() {
                    format!("Dysk ({letter})")
                } else {
                    format!("{label} ({letter})")
                }
            }
            other => other.as_os_str().to_string_lossy().to_string(),
        };
        v.push(Crumb { name: name.into(), path: acc.to_string_lossy().to_string().into() });
    }
    v
}

/// Panel boczny z ikonami systemowymi (te same co w Eksploratorze).
pub fn render_side(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.tabs[b.active];
    let scale = ui.window().scale_factor();
    let px = (18.0 * scale) as u32;
    // Zapisane wyszukiwania – nad szybkim dostępem, jak w Sidebar.svelte.
    let saved = b.prefs.saved_searches.iter().enumerate().map(|(i, v)| SideItem {
        tip: format!("{}\nw {}", v.query, v.root).into(),
        section: if i == 0 { "Zapisane wyszukiwania".into() } else { SharedString::new() },
        name: v.name.clone().into(),
        icon: ic("search"),
        path: format!("{SAVED}{}", v.id).into(),
        detail: SharedString::new(),
        usage: 0.0,
        active: t.search.as_ref().is_some_and(|x| x.query == v.query) && same(&t.path, &v.root),
        img: Default::default(),
        has_img: false,
        shut: false,
        header_only: false,
    });
    let (extra, syncs) = side_extra(&t.path, &b.prefs.favorites, px);
    // Kolejność: zapisane, szybki dostęp, przypięte, biblioteki, dyski, sieć, synchronizacja.
    let base = side_items(&t.path, &b.drives, &b.prefs.net_places);
    let quick = base.iter().take_while(|i| !i.section.starts_with("Dyski") && !i.section.starts_with("Sieć")).count();
    let mut base = base.into_iter();
    let quick: Vec<SideItem> = base.by_ref().take(quick).collect();
    let items: Vec<SideItem> = saved
        .chain(quick)
        .chain(extra)
        .chain(base)
        .chain(syncs)
        .map(|mut it| {
            if it.path.starts_with("ofp:") {
                return it;
            }
            if it.tip.is_empty() {
                it.tip = it.path.clone();
            }
            if it.has_img {
                return it;
            }
            if let Some(img) = thumbs::get(it.path.as_str(), px, true) {
                it.img = img;
                it.has_img = true;
            }
            it
        })
        .collect();
    let mut paths: HashSet<String> = items.iter().map(|i| i.path.to_string()).collect();
    // Ikony bibliotek pochodzą z plików .library-ms – po ich dotarciu panel też trzeba przerysować.
    LIBS.with(|l| paths.extend(l.borrow().iter().flatten().map(|x| x.path.clone())));
    // Zwinięta sekcja zostawia sam nagłówek; w pasku ikon nagłówków nie ma, więc pokazuje wszystko.
    let mut section = SharedString::new();
    let mut items_out = Vec::with_capacity(items.len());
    for mut it in items {
        if !it.section.is_empty() {
            section = it.section.clone();
        }
        // Sekcja wyłączona w Ustawieniach – bez nagłówka i pozycji.
        if b.prefs.side_hidden.iter().any(|x| x == section.as_str()) {
            continue;
        }
        if !it.section.is_empty() {
            it.shut = b.prefs.side_shut.iter().any(|x| x == section.as_str());
            if it.shut && !b.prefs.side_mini {
                it.header_only = true;
                items_out.push(it);
                continue;
            }
        } else if b.prefs.side_shut.iter().any(|x| x == section.as_str()) && !b.prefs.side_mini {
            continue;
        }
        items_out.push(it);
    }
    ui.set_side_mini(b.prefs.side_mini);
    ui.set_side_width(b.prefs.side_width.clamp(180.0, 420.0));
    ui.set_side(ModelRc::new(VecModel::from(items_out)));
    ui.set_folder_icon(thumbs::get(&t.path, (16.0 * scale) as u32, true).unwrap_or_default());
    drop(b);
    s.borrow_mut().side_paths = paths;
}

pub fn is_favorite(s: &Shared, path: &str) -> bool {
    s.borrow().prefs.favorites.iter().any(|f| same(f, path))
}

/// Przypina folder w panelu bocznym albo go odpina.
pub fn toggle_favorite(s: &Shared, path: &str) {
    {
        let mut b = s.borrow_mut();
        let fav = &mut b.prefs.favorites;
        match fav.iter().position(|f| same(f, path)) {
            Some(i) => {
                fav.remove(i);
            }
            None => fav.push(path.to_string()),
        }
        b.prefs.save();
    }
    render_side(s);
}

/// Klawiatura w panelu bocznym (po F6): strzałki, Home/End, Enter lub spacja otwiera.
/// Pusty klawisz – panel właśnie dostał fokus: kursor na bieżącym folderze.
fn side_key(s: &Shared, key: &str) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let side = ui.get_side();
    // Wiersze, na które da się wejść – bez samych nagłówków zwiniętych sekcji.
    let rows: Vec<usize> = (0..side.row_count()).filter(|&i| side.row_data(i).is_some_and(|it| !it.header_only)).collect();
    if rows.is_empty() {
        return;
    }
    let cur = rows.iter().position(|&i| i as i32 == ui.get_side_cursor());
    let is = |k: slint::platform::Key| SharedString::from(k) == key;
    let pos = match key {
        "" => rows.iter().position(|&i| side.row_data(i).is_some_and(|it| it.active)).unwrap_or(0),
        _ if is(slint::platform::Key::UpArrow) => cur.map_or(0, |c| (c + rows.len() - 1) % rows.len()),
        _ if is(slint::platform::Key::DownArrow) => cur.map_or(0, |c| (c + 1) % rows.len()),
        _ if is(slint::platform::Key::Home) => 0,
        _ if is(slint::platform::Key::End) => rows.len() - 1,
        "\n" | " " => {
            let Some(it) = cur.and_then(|c| side.row_data(rows[c])) else { return };
            let p = it.path.to_string();
            match p.strip_prefix(SAVED) {
                Some(id) => run_saved(s, id, false),
                None if p == ADD_NET => add_network_place(s),
                None => navigate(s, &p, true),
            }
            return;
        }
        _ => return,
    };
    ui.set_side_cursor(rows[pos] as i32);
}

/// Przypina upuszczone foldery (pliki pomija); już przypiętych nie dubluje.
pub fn pin_folders(s: &Shared, paths: &[String]) {
    let added = {
        let mut b = s.borrow_mut();
        let mut added = 0;
        for p in paths.iter().filter(|p| Path::new(p).is_dir()) {
            if !b.prefs.favorites.iter().any(|f| same(f, p)) {
                b.prefs.favorites.push(p.clone());
                added += 1;
            }
        }
        b.prefs.save();
        added
    };
    if added > 0 {
        // Zwinięta sekcja zasłoniłaby nowo przypięte.
        s.borrow_mut().prefs.side_shut.retain(|x| x != "Przypięte");
        render_side(s);
    }
}

fn toggle_section(s: &Shared, name: &str) {
    {
        let mut b = s.borrow_mut();
        let shut = &mut b.prefs.side_shut;
        if let Some(i) = shut.iter().position(|x| x == name) {
            shut.remove(i);
        } else {
            shut.push(name.to_string());
        }
        b.prefs.save();
    }
    render_side(s);
}

/// Panel boczny jako pasek samych ikon (Ctrl+B).
fn toggle_side_mini(s: &Shared) {
    {
        let mut b = s.borrow_mut();
        b.prefs.side_mini = !b.prefs.side_mini;
        b.prefs.save();
    }
    render_side(s);
}

fn side_item(section: &mut bool, title: &str, name: String, icon: SharedString, path: String, current: &str) -> SideItem {
    SideItem {
        section: if std::mem::take(section) { title.into() } else { SharedString::new() },
        name: name.into(),
        icon,
        active: same(&path, current),
        path: path.into(),
        detail: SharedString::new(),
        usage: 0.0,
        img: Default::default(),
        has_img: false,
        tip: SharedString::new(),
        shut: false,
        header_only: false,
    }
}

thread_local! {
    /// Biblioteki Windows – wczytywane raz w tle (COM bywa wolny).
    static LIBS: RefCell<Option<Vec<powerdesk_core::library::Library>>> = const { RefCell::new(None) };
    static LIBS_ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SYNC_LOADED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Ścieżka karty z całą biblioteką: „ofp:lib|<plik .library-ms>”.
pub const LIB: &str = "ofp:lib|";

fn with_library<R>(path: &str, f: impl FnOnce(&powerdesk_core::library::Library) -> R) -> Option<R> {
    let file = path.strip_prefix(LIB)?;
    // Karta przywrócona po starcie może wyprzedzić wczytanie bibliotek w tle.
    if LIBS.with(|l| l.borrow().is_none()) {
        let libs = powerdesk_core::library::list();
        LIBS.with(|l| *l.borrow_mut() = Some(libs));
    }
    LIBS.with(|l| l.borrow().iter().flatten().find(|x| same(&x.path, file)).map(f))
}

/// Foldery biblioteki, gdy `path` to karta biblioteki.
pub fn library_folders(path: &str) -> Option<Vec<String>> {
    with_library(path, |l| l.folders.clone())
}

pub fn library_name(path: &str) -> Option<String> {
    with_library(path, |l| l.name.clone())
}

/// Zawartość wszystkich folderów biblioteki naraz; błąd tylko, gdy żaden się nie otworzył.
fn list_library(folders: &[String]) -> Result<Vec<fsx::Entry>, String> {
    let mut all = Vec::new();
    let mut err = None;
    for f in folders {
        match fsx::list_dir(f) {
            Ok(v) => all.extend(v),
            Err(e) => err = Some(e),
        }
    }
    match err {
        Some(e) if all.is_empty() => Err(e),
        _ => Ok(all),
    }
}

/// Przypięte, biblioteki i synchronizacje – sekcje panelu poza szybkim dostępem i dyskami.
fn side_extra(current: &str, favorites: &[String], px: u32) -> (Vec<SideItem>, Vec<SideItem>) {
    let mut v = Vec::new();
    let mut first = true;
    for f in favorites {
        let name = Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
        v.push(side_item(&mut first, "Przypięte", name, ic("pin"), f.clone(), current));
    }
    if !LIBS_ASKED.with(|x| x.replace(true)) {
        std::thread::spawn(|| {
            let libs = powerdesk_core::library::list();
            let _ = slint::invoke_from_event_loop(move || {
                LIBS.with(|l| *l.borrow_mut() = Some(libs));
                with_state(render_side);
            });
        });
    }
    let mut first = true;
    LIBS.with(|l| {
        for lib in l.borrow().iter().flatten().filter(|l| !l.folders.is_empty()) {
            // Jeden folder – zwykły folder; kilka – karta z całą biblioteką.
            let path = if lib.folders.len() > 1 { format!("{LIB}{}", lib.path) } else { lib.folders[0].clone() };
            let mut it = side_item(&mut first, "Biblioteki", lib.name.clone(), ic("archive"), path, current);
            it.tip = format!("{}
{}", lib.name, lib.folders.join("
")).into();
            // Ikona biblioteki z Windows (z pliku .library-ms), nie folderu.
            if let Some(img) = thumbs::get(&lib.path, px, true) {
                it.img = img;
                it.has_img = true;
            }
            v.push(it);
        }
    });
    // Synchronizacje – sam odczyt profili z dysku, silnik startuje dopiero na ekranie synchronizacji.
    if !SYNC_LOADED.with(|x| x.replace(true)) {
        powerdesk_core::sync::load();
    }
    let mut tail = Vec::new();
    let mut first = true;
    let (profiles, status) = powerdesk_core::sync::profiles();
    let on_sync = current == syncview::TAB;
    for (p, st) in profiles.iter().zip(status.iter()) {
        let name = if p.name.is_empty() { p.source.clone() } else { p.name.clone() };
        let mut it = side_item(&mut first, "Synchronizacja", name, ic("sync"), syncview::TAB.to_string(), "");
        it.active = on_sync;
        it.detail = match st.state.as_str() {
            "syncing" => format!("synchronizuję {:.0}%", st.progress * 100.0),
            "scanning" => "porównuję…".into(),
            "offline" => "cel odłączony".into(),
            "error" => "błąd".into(),
            _ if !p.enabled => "wstrzymana".into(),
            _ => String::new(),
        }
        .into();
        it.tip = format!("{} → {}", p.source, p.target.path).into();
        tail.push(it);
    }
    (v, tail)
}

fn side_items(current: &str, drives: &[drives::Drive], places: &[String]) -> Vec<SideItem> {
    let mut v = Vec::new();
    let known: [(&str, Option<PathBuf>, &str); 6] = [
        ("Pulpit", dirs::desktop_dir(), "desktop"),
        ("Pobrane", dirs::download_dir(), "download"),
        ("Dokumenty", dirs::document_dir(), "doc"),
        ("Obrazy", dirs::picture_dir(), "image"),
        ("Wideo", dirs::video_dir(), "video"),
        ("Muzyka", dirs::audio_dir(), "music"),
    ];
    let mut first = true;
    for (name, p, icon) in known {
        let Some(p) = p.filter(|p| p.is_dir()) else { continue };
        let p = p.to_string_lossy().to_string();
        v.push(SideItem {
            section: if std::mem::take(&mut first) { "Szybki dostęp".into() } else { SharedString::new() },
            name: name.into(),
            icon: ic(icon),
            active: same(&p, current),
            path: p.into(),
            detail: SharedString::new(),
            usage: 0.0,
            img: Default::default(),
            has_img: false,
            tip: SharedString::new(),
            shut: false,
            header_only: false,
        });
    }
    // Dyski lokalne, potem sieciowe we własnej sekcji (jak w Sidebar.svelte).
    let mut first_local = true;
    let mut first_net = true;
    let mut order: Vec<&drives::Drive> = drives.iter().filter(|d| d.online).collect();
    order.sort_by_key(|d| d.kind == "network");
    for d in order {
        let name = if d.label.is_empty() { format!("Dysk ({})", d.letter) } else { format!("{} ({})", d.label, d.letter) };
        let used = d.total.saturating_sub(d.free);
        let first = if d.kind == "network" { &mut first_net } else { &mut first_local };
        let title = if d.kind == "network" { "Sieć" } else { "Dyski" };
        v.push(SideItem {
            section: if std::mem::take(first) { title.into() } else { SharedString::new() },
            name: name.into(),
            icon: match d.kind.as_str() {
                "removable" => ic("usb"),
                "network" => ic("network"),
                _ => ic("drive"),
            },
            active: same(&d.path, current),
            path: d.path.clone().into(),
            detail: if d.total > 0 { format!("{} wolne z {}", fmt::size(d.free), fmt::size(d.total)).into() } else { SharedString::new() },
            usage: if d.total > 0 { used as f32 / d.total as f32 } else { 0.0 },
            img: Default::default(),
            has_img: false,
            tip: if d.total > 0 { format!("{}
{} wolne z {} · zajęte {:.0}%", d.path, fmt::size(d.free), fmt::size(d.total), used as f64 * 100.0 / d.total as f64).into() } else { d.path.clone().into() },
            shut: false,
            header_only: false,
        });
    }
    // Dodane lokalizacje (\\serwer\udział) i na końcu pozycja do dodawania kolejnych.
    for p in places {
        let name = p.trim_start_matches('\\').replace('\\', " › ");
        v.push(side_item(&mut first_net, "Sieć", name, ic("network"), p.clone(), current));
    }
    let mut add = side_item(&mut first_net, "Sieć", "Dodaj lokalizację…".into(), ic("plus"), ADD_NET.into(), current);
    add.tip = "Dodaj udział sieciowy (\\\\serwer\\udział)".into();
    v.push(add);
    v
}

/// Pozycja „Dodaj lokalizację…” w sekcji Sieć.
pub const ADD_NET: &str = "ofp:addnet";

/// Okno z polem na ścieżkę UNC; sprawdzamy ją w tle (udział może długo nie odpowiadać).
pub fn add_network_place(s: &Shared) {
    transfers::prompt(
        s,
        "Dodaj lokalizację sieciową",
        r"\\",
        "Dodaj",
        Rc::new(|s: &Shared, p: &str| {
            let p = p.trim().trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_string();
            if !p.starts_with(r"\\") || p.len() < 3 {
                transfers::toast(s, "Podaj ścieżkę w postaci \\\\serwer\\udział", transfers::Kind::Error, None);
                return;
            }
            transfers::toast(s, format!("Łączę z {p}…"), transfers::Kind::Info, None);
            std::thread::spawn(move || {
                let ok = Path::new(&p).is_dir();
                let _ = slint::invoke_from_event_loop(move || {
                    with_state(|s| {
                        if !ok {
                            transfers::toast(s, format!("Nie udało się otworzyć {p}"), transfers::Kind::Error, None);
                            return;
                        }
                        {
                            let mut b = s.borrow_mut();
                            if !b.prefs.net_places.iter().any(|x| same(x, &p)) {
                                b.prefs.net_places.push(p.clone());
                                b.prefs.save();
                            }
                        }
                        navigate(s, &p, true);
                        render_side(s);
                    })
                });
            });
        }),
    );
}

pub fn is_net_place(s: &Shared, path: &str) -> bool {
    s.borrow().prefs.net_places.iter().any(|p| same(p, path))
}

pub fn remove_net_place(s: &Shared, path: &str) {
    {
        let mut b = s.borrow_mut();
        b.prefs.net_places.retain(|p| !same(p, path));
        b.prefs.save();
    }
    render_side(s);
}

fn system_light() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "AppsUseLightTheme"])
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("0x1"))
        .unwrap_or(false)
}
