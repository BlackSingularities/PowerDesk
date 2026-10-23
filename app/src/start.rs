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

/// Najmniejszy rozmiar Startu (lista i kolumna) – mniejszy nie ma sensu.
const MIN: (f32, f32) = (420.0, 360.0);

struct Start {
    w: StartWindow,
    /// Aplikacja każdego wiersza (None – litera).
    shown: Vec<Option<usize>>,
    menu: Vec<Item>,
}

thread_local! {
    static START: RefCell<Option<Start>> = const { RefCell::new(None) };
    /// „Pokaż ukryte” – ukryte aplikacje na liście (przygaszone); po zamknięciu Startu wraca do false.
    static SHOW_HIDDEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Przeciąganie krawędzi: która, kursor, rozmiar i dół okna (piksele ekranu) na początku.
    static DRAG: std::cell::Cell<Option<(i32, (i32, i32), (i32, i32), i32)>> = const { std::cell::Cell::new(None) };
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
    let (lw, lh) = size();
    let (pw, ph) = ((lw * scale) as i32, (lh * scale) as i32);
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
    w.on_resize_start(|edge| {
        START.with(|s| {
            if let Some(s) = s.borrow().as_ref() {
                let (sz, pos) = (s.w.window().size(), s.w.window().position());
                let bottom = pos.y + sz.height as i32;
                DRAG.with(|d| d.set(Some((edge, powerdesk_core::taskwin::cursor(), (sz.width as i32, sz.height as i32), bottom))));
            }
        })
    });
    w.on_resize_move(resize_move);
    w.on_resize_end(|| {
        DRAG.with(|d| d.set(None));
        // Rozmiar zapamiętany w pikselach logicznych – ten sam na monitorach o innej skali.
        START.with(|s| {
            if let Some(s) = s.borrow().as_ref() {
                let l = s.w.window().size().to_logical(s.w.window().scale_factor());
                crate::with_state(|st| {
                    let mut b = st.borrow_mut();
                    b.prefs.start_size = (l.width, l.height);
                    b.prefs.save();
                });
            }
        })
    });
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

fn size() -> (f32, f32) {
    let mut sz = (660.0, 640.0);
    crate::with_state(|s| sz = s.borrow().prefs.start_size);
    (sz.0.max(MIN.0), sz.1.max(MIN.1))
}

/// Nowy rozmiar z ruchu kursora; dół i lewa krawędź zostają na miejscu (nad paskiem).
fn resize_move() {
    let Some((edge, (cx, cy), (w0, h0), bottom)) = DRAG.with(|d| d.get()) else { return };
    let Some(w) = START.with(|s| s.borrow().as_ref().map(|s| s.w.clone_strong())) else { return };
    let win = w.window();
    let scale = win.scale_factor();
    let (x, y) = powerdesk_core::taskwin::cursor();
    let (minw, minh) = ((MIN.0 * scale) as i32, (MIN.1 * scale) as i32);
    // Krawędź górna zmienia wysokość, prawa – szerokość, róg – obie.
    let width = if edge == 0 { w0 } else { (w0 + x - cx).max(minw) };
    let height = if edge == 1 { h0 } else { (h0 + cy - y).max(minh) };
    let left = win.position().x;
    win.set_size(PhysicalSize::new(width as u32, height as u32));
    win.set_position(PhysicalPosition::new(left, bottom - height));
}

fn has_focus() -> bool {
    START.with(|s| s.borrow().as_ref().is_some_and(|s| powerdesk_core::taskwin::foreground() == crate::window_hwnd(s.w.window())))
}

pub fn close() {
    SHOW_HIDDEN.with(|h| h.set(false));
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

fn hidden() -> Vec<String> {
    let mut h = Vec::new();
    crate::with_state(|s| h = s.borrow().prefs.start_hidden.clone());
    h
}

fn toggle_hidden(path: &str) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        let list = &mut b.prefs.start_hidden;
        if let Some(i) = list.iter().position(|p| p == path) {
            list.remove(i);
        } else {
            list.push(path.to_string());
        }
        b.prefs.save();
    });
    render();
}

fn grid() -> bool {
    let mut g = false;
    crate::with_state(|s| g = s.borrow().prefs.start_grid);
    g
}

/// Przełącznik widoku listy: nazwy z małymi ikonami albo same duże ikony.
fn view_switch() -> Item {
    if grid() {
        it("Widok: lista z nazwami", "list", |_| set_grid(false))
    } else {
        it("Widok: duże ikony", "grid", |_| set_grid(true))
    }
}

fn set_grid(on: bool) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        b.prefs.start_grid = on;
        b.prefs.save();
    });
    render();
}

/// „Pokaż ukryte (N)” / „Schowaj ukryte” – tylko gdy coś jest ukryte.
fn hidden_switch() -> Option<Item> {
    let n = hidden().len();
    if n == 0 {
        return None;
    }
    Some(if SHOW_HIDDEN.with(|h| h.get()) {
        it("Schowaj ukryte", "eye", |_| {
            SHOW_HIDDEN.with(|h| h.set(false));
            render();
        })
    } else {
        it(format!("Pokaż ukryte ({n})"), "eye", |_| {
            SHOW_HIDDEN.with(|h| h.set(true));
            render();
        })
    })
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
        let hidden = hidden();
        let show_hidden = SHOW_HIDDEN.with(|h| h.get());
        let grid = grid();
        st.w.set_grid(grid);
        let px = if grid { (36.0 * st.w.window().scale_factor()) as u32 } else { px };
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
                let is_hidden = hidden.contains(&app.shell_path());
                if is_hidden && !show_hidden {
                    continue;
                }
                if parts.is_empty() && !grid {
                    let l = letter(&app.name);
                    if l != last {
                        rows.push(StartRow { header: true, name: l.clone().into(), img: Default::default(), has_img: false, hidden: false });
                        shown.push(None);
                        last = l;
                    }
                }
                let img = crate::thumbs::get(&app.shell_path(), px, true);
                rows.push(StartRow { header: false, name: app.name.clone().into(), has_img: img.is_some(), img: img.unwrap_or_default(), hidden: is_hidden });
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
        let (entries, width, mx, my) = menus::layout(&items, x, y, s.w.window().size().to_logical(s.w.window().scale_factor()));
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
    let (p1, p2, p3, p4) = (path.to_string(), path.to_string(), path.to_string(), path.to_string());
    let pinned = pins().iter().any(|p| p == path);
    let is_hidden = hidden().iter().any(|p| p == path);
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
    items.push(sep());
    items.push(it(if is_hidden { "Przywróć w menu Start" } else { "Ukryj w menu Start" }, "eye", move |_| toggle_hidden(&p4)));
    items.extend(hidden_switch());
    items.push(view_switch());
    show_menu(items, x, y);
}

/// Zasilanie i sesja – w Starcie (przycisk zasilania) i na pasku (prawy przycisk na pustym miejscu).
pub fn power_items() -> Vec<Item> {
    // Start zamykamy od razu – inaczej zostałby na ekranie blokady i przy wylogowaniu.
    let go = |p: Power| {
        move |_: &crate::Shared| {
            close();
            startmenu::power(p);
        }
    };
    let mut v = vec![
        it("Zablokuj", "shield", go(Power::Lock)).kbd("Win+L"),
        it("Przełącz użytkownika", "refresh", go(Power::SwitchUser)),
        it("Wyloguj", "close", go(Power::SignOut)),
        sep(),
        it("Uśpij", "history", go(Power::Sleep)),
    ];
    if startmenu::can_hibernate() {
        v.push(it("Hibernuj", "history", go(Power::Hibernate)));
    }
    v.extend([it("Zamknij system", "power", go(Power::Shutdown)).danger(), it("Uruchom ponownie", "refresh", go(Power::Restart))]);
    v
}

fn known(sub: &str) -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    format!("{home}\\{sub}")
}

/// Lewa kolumna: konto, Dokumenty, Obrazy, Ustawienia, zasilanie.
fn rail(i: i32, x: f32, y: f32) {
    match i {
        0 => show_menu(
            Some(view_switch()).into_iter().chain(hidden_switch()).chain(Some(sep())).chain([
                it("Zmień ustawienia konta", "settings", |_| {
                    let _ = open::that_detached("ms-settings:yourinfo");
                    slint::Timer::single_shot(Duration::ZERO, close);
                }),
                sep(),
                it("Zablokuj", "shield", |_| startmenu::power(Power::Lock)),
                it("Przełącz użytkownika", "refresh", |_| startmenu::power(Power::SwitchUser)),
                it("Wyloguj", "close", |_| startmenu::power(Power::SignOut)),
            ])
            .skip_while(|i| i.is_sep())
            .collect(),
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
        _ => show_menu(power_items(), x, y),
    }
}
