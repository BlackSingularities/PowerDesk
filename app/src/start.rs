//! Menu Start PowerDesk (układ Windows 10): lista aplikacji z folderu „Aplikacje” z literami,
//! szukanie od pierwszego znaku, kafelki przypiętych, konto, foldery, ustawienia i zasilanie.
//! Otwiera je przycisk Start na pasku i samo naciśnięcie Win (hak w core::startmenu).

use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use powerdesk_core::startmenu::{self, App, Power};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, VecModel};

use crate::menus::{self, it, sep, Item};
use crate::{StartRow, StartTile, StartWindow, Theme};

const WIDTH: f32 = 660.0;
const HEIGHT: f32 = 640.0;

struct Start {
    w: StartWindow,
    /// Aplikacja każdego wiersza (None – litera).
    shown: Vec<Option<usize>>,
    menu: Vec<Item>,
}

thread_local! {
    static START: RefCell<Option<Start>> = const { RefCell::new(None) };
    /// Lista aplikacji – czytana w tle przy starcie i przy każdym otwarciu.
    static APPS: RefCell<Vec<App>> = const { RefCell::new(Vec::new()) };
}

/// Lista aplikacji w tle (folder „Aplikacje” bywa wolny przy pierwszym czytaniu).
pub fn preload() {
    std::thread::spawn(|| {
        let apps = startmenu::apps();
        let _ = slint::invoke_from_event_loop(move || {
            APPS.with(|a| *a.borrow_mut() = apps);
            render();
        });
    });
}

pub fn is_open() -> bool {
    START.with(|s| s.borrow().is_some())
}

/// Otwiera nad paskiem: lewy dolny róg w (x, bottom) – piksele ekranu, `light`/`accent` z paska.
pub fn toggle(x: i32, bottom: i32, scale: f32, light: bool, accent: slint::Color) {
    if is_open() {
        close();
        return;
    }
    let Ok(w) = StartWindow::new() else { return };
    let t = w.global::<Theme>();
    t.set_light(light);
    t.set_accent_override(accent);
    w.set_user(std::env::var("USERNAME").unwrap_or_default().into());
    let (pw, ph) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32);
    w.window().set_position(PhysicalPosition::new(x, bottom - ph));
    w.window().set_size(PhysicalSize::new(pw as u32, ph as u32));
    w.on_search(|_| render());
    w.on_launch_first(|| {
        let first = START.with(|s| s.borrow().as_ref().and_then(|s| s.shown.iter().flatten().next().copied()));
        if let Some(i) = first {
            launch(i);
        }
    });
    w.on_launch(|row| {
        if let Some(i) = app_at(row as usize) {
            launch(i);
        }
    });
    w.on_launch_tile(|t| {
        if let Some(p) = tile_path(t as usize) {
            startmenu::launch(&p);
            slint::Timer::single_shot(Duration::ZERO, close);
        }
    });
    w.on_row_context(|row, x, y| {
        if let Some(i) = app_at(row as usize) {
            let app = APPS.with(|a| a.borrow().get(i).cloned());
            if let Some(app) = app {
                app_menu(&app.shell_path(), &app.id, x, y);
            }
        }
    });
    w.on_tile_context(|t, x, y| {
        if let Some(p) = tile_path(t as usize) {
            let id = p.trim_start_matches("shell:AppsFolder\\").to_string();
            app_menu(&p, &id, x, y);
        }
    });
    w.on_rail(rail);
    w.on_menu_chosen(|i| {
        let item = START.with(|s| s.borrow().as_ref().and_then(|s| s.menu.get(i as usize).cloned()));
        if let Some(item) = item {
            crate::with_state(|s| menus::run(s, &item));
        }
    });
    w.on_dismiss(|| slint::Timer::single_shot(Duration::ZERO, close));
    // Klik obok (utrata fokusu) zamyka – jak Start w Windows.
    w.window().on_winit_window_event(|_, ev| {
        if let slint::winit_030::winit::event::WindowEvent::Focused(false) = ev {
            slint::Timer::single_shot(Duration::from_millis(80), || {
                // Menu podręczne w oknie Start nie zamyka Startu.
                let still = START.with(|s| s.borrow().as_ref().is_some_and(|s| s.w.window().is_visible()));
                if still && !has_focus() {
                    close();
                }
            });
        }
        slint::winit_030::EventResult::Propagate
    });
    let _ = w.show();
    let hwnd = crate::window_hwnd(w.window());
    powerdesk_core::desktop::hide_from_taskbar(hwnd);
    powerdesk_core::taskwin::activate(hwnd);
    w.invoke_focus_search();
    START.with(|s| *s.borrow_mut() = Some(Start { w, shown: Vec::new(), menu: Vec::new() }));
    render();
    preload();
}

fn has_focus() -> bool {
    START.with(|s| s.borrow().as_ref().is_some_and(|s| powerdesk_core::taskwin::foreground() == crate::window_hwnd(s.w.window())))
}

pub fn close() {
    if let Some(s) = START.with(|s| s.borrow_mut().take()) {
        let _ = s.w.hide();
    }
}

/// Okno Start (do pominięcia na liście okien paska).
pub fn hwnd() -> Option<isize> {
    START.with(|s| s.borrow().as_ref().map(|s| crate::window_hwnd(s.w.window())))
}

fn app_at(row: usize) -> Option<usize> {
    START.with(|s| s.borrow().as_ref().and_then(|s| s.shown.get(row).copied().flatten()))
}

fn pins() -> Vec<String> {
    let mut p = Vec::new();
    crate::with_state(|s| p = s.borrow().prefs.start_pins.clone());
    p
}

fn tile_path(i: usize) -> Option<String> {
    pins().get(i).cloned()
}

fn launch(i: usize) {
    if let Some(app) = APPS.with(|a| a.borrow().get(i).cloned()) {
        startmenu::launch(&app.shell_path());
    }
    slint::Timer::single_shot(Duration::ZERO, close);
}

/// Litera grupy jak w Windows: cyfry i znaki pod „#”, polskie litery pod łacińskimi.
fn letter(name: &str) -> String {
    let c = crate::palette::fold(name).chars().next().unwrap_or('#');
    if c.is_alphabetic() {
        c.to_uppercase().to_string()
    } else {
        "#".into()
    }
}

/// Wiersze listy: bez zapytania – wszystko z literami; z zapytaniem – pasujące (każde słowo).
fn render() {
    START.with(|st| {
        let mut st = st.borrow_mut();
        let Some(st) = st.as_mut() else { return };
        let q = crate::palette::fold(&st.w.get_query());
        let parts: Vec<&str> = q.split_whitespace().collect();
        let px = (24.0 * st.w.window().scale_factor()) as u32;
        let mut rows = Vec::new();
        let mut shown = Vec::new();
        APPS.with(|a| {
            let apps = a.borrow();
            let mut last = String::new();
            for (i, app) in apps.iter().enumerate() {
                let name = crate::palette::fold(&app.name);
                if !parts.iter().all(|p| name.contains(p)) {
                    continue;
                }
                if parts.is_empty() {
                    let l = letter(&app.name);
                    if l != last {
                        rows.push(StartRow { header: true, name: l.clone().into(), img: Default::default(), has_img: false });
                        shown.push(None);
                        last = l;
                    }
                }
                let img = crate::thumbs::get(&app.shell_path(), px, true);
                rows.push(StartRow { header: false, name: app.name.clone().into(), has_img: img.is_some(), img: img.unwrap_or_default() });
                shown.push(Some(i));
            }
        });
        st.w.set_rows(ModelRc::new(VecModel::from(rows)));
        st.shown = shown;
        let tpx = (32.0 * st.w.window().scale_factor()) as u32;
        let tiles: Vec<StartTile> = pins()
            .iter()
            .map(|p| {
                let name = APPS.with(|a| a.borrow().iter().find(|x| &x.shell_path() == p).map(|x| x.name.clone())).unwrap_or_else(|| p.rsplit('\\').next().unwrap_or(p).to_string());
                let img = crate::thumbs::get(p, tpx, true);
                StartTile { name: name.into(), has_img: img.is_some(), img: img.unwrap_or_default() }
            })
            .collect();
        st.w.set_tiles(ModelRc::new(VecModel::from(tiles)));
    });
}

/// Ikony aplikacji dochodzą w tle.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    if is_open() && paths.iter().any(|p| p.starts_with("shell:AppsFolder\\")) {
        render();
    }
}

fn show_menu(items: Vec<Item>, x: f32, y: f32) {
    START.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return };
        let (entries, width, mx, my) = menus::layout(&items, x, y, slint::LogicalSize::new(WIDTH, HEIGHT));
        s.w.set_menu_x(mx);
        s.w.set_menu_y(my);
        s.w.set_menu_width(width);
        s.w.set_menu_entries(ModelRc::new(VecModel::from(entries)));
        s.menu = items;
        s.w.invoke_show_menu();
    });
}

fn toggle_pin(path: &str) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        let pins = &mut b.prefs.start_pins;
        if let Some(i) = pins.iter().position(|p| p == path) {
            pins.remove(i);
        } else {
            pins.push(path.to_string());
        }
        b.prefs.save();
    });
    render();
}

/// Menu aplikacji: uruchom, jako administrator, kafelek, pasek (tylko zwykłe programy .exe).
fn app_menu(path: &str, id: &str, x: f32, y: f32) {
    let (p1, p2, p3) = (path.to_string(), path.to_string(), path.to_string());
    let pinned = pins().iter().any(|p| p == path);
    let mut items = vec![
        it("Otwórz", "open", move |_| {
            startmenu::launch(&p1);
            slint::Timer::single_shot(Duration::ZERO, close);
        }),
        it("Uruchom jako administrator", "shield", move |_| {
            startmenu::launch_admin(&p2);
            slint::Timer::single_shot(Duration::ZERO, close);
        }),
        sep(),
        it(if pinned { "Odepnij od Startu" } else { "Przypnij do Startu" }, "pin", move |_| toggle_pin(&p3)),
    ];
    // Pasek przypina pliki programów – tylko gdy aplikacja jest zwykłym .exe ze ścieżką.
    if id.contains(":\\") && id.to_lowercase().ends_with(".exe") {
        let exe = id.to_string();
        items.push(it("Przypnij do paska zadań", "pin", move |_| crate::taskbar::pin_program(&exe)));
    }
    show_menu(items, x, y);
}

fn known(sub: &str) -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    format!("{home}\\{sub}")
}

/// Lewa kolumna: konto, Dokumenty, Obrazy, Ustawienia, zasilanie.
fn rail(i: i32, x: f32, y: f32) {
    match i {
        0 => show_menu(
            vec![
                it("Zmień ustawienia konta", "settings", |_| {
                    let _ = open::that_detached("ms-settings:yourinfo");
                    slint::Timer::single_shot(Duration::ZERO, close);
                }),
                it("Zablokuj", "shield", |_| startmenu::power(Power::Lock)),
                it("Wyloguj", "close", |_| startmenu::power(Power::SignOut)),
            ],
            x,
            y,
        ),
        1 | 2 => {
            let dir = known(if i == 1 { "Documents" } else { "Pictures" });
            crate::with_state(|s| {
                crate::open_args(s, vec![dir]);
                crate::show_window(s);
            });
            slint::Timer::single_shot(Duration::ZERO, close);
        }
        3 => {
            let _ = open::that_detached("ms-settings:");
            slint::Timer::single_shot(Duration::ZERO, close);
        }
        _ => show_menu(
            vec![
                it("Uśpij", "history", |_| startmenu::power(Power::Sleep)),
                it("Zamknij system", "power", |_| startmenu::power(Power::Shutdown)),
                it("Uruchom ponownie", "refresh", |_| startmenu::power(Power::Restart)),
            ],
            x,
            y,
        ),
    }
}
