//! Menu Start PowerDesk (układ Windows 10): lista aplikacji z folderu „Aplikacje” z literami,
//! szukanie od pierwszego znaku, kafelki przypiętych, konto, foldery, ustawienia i zasilanie.
//! Otwiera je przycisk Start na pasku i samo naciśnięcie Win (hak w core::startmenu).

use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use powerdesk_core::startmenu::{self, App, Power};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, VecModel};

use crate::menus::{self, it, sep, sub, Item};
use crate::model::StartGroup;
use crate::starttiles::{self as tiles, At, Head, Placed};
use crate::startsearch::{self, Row};
use crate::{StartHead, StartRow, StartTile, StartWindow, Theme};

/// Najmniejszy rozmiar Startu (lista i kolumna) – mniejszy nie ma sensu.
const MIN: (f32, f32) = (420.0, 360.0);

struct Start {
    w: StartWindow,
    /// Aplikacja każdego wiersza (None – litera).
    shown: Vec<Row>,
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
    /// Otwarty folder kafelków (grupa, kafelek).
    static FOLDER: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
    /// Ułożone kafelki i nagłówki z ostatniego rysowania – do kliknięć i upuszczania.
    static PLACED: RefCell<(Vec<Placed>, Vec<Head>)> = const { RefCell::new((Vec::new(), Vec::new())) };
    /// Otwarte jako samo wyszukiwanie (Win+S) – inaczej pełny Start.
    static SEARCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Ostatnie zamknięcie: tryb i chwila. Klik w przycisk Start najpierw zabiera fokus
    /// (Start się zamyka), a dopiero potem dociera do paska – bez tego Start otwierałby się od nowa.
    static CLOSED: std::cell::Cell<Option<(bool, std::time::Instant)>> = const { std::cell::Cell::new(None) };
}

/// Rozmiar okna samego wyszukiwania (Win+S).
const SEARCH_SIZE: (f32, f32) = (520.0, 600.0);

/// Lista aplikacji w tle (folder „Aplikacje” bywa wolny przy pierwszym czytaniu).
pub fn preload() {
    powerdesk_core::quickindex::start();
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
/// `search` – samo wyszukiwanie (Win+S). Otwarty Start w tym samym trybie się zamyka, w innym –
/// przełącza (Start → Win+S zamyka Start i otwiera wyszukiwanie, i odwrotnie).
pub fn toggle(x: i32, bottom: i32, scale: f32, light: bool, accent: slint::Color, alpha: f32, search: bool) {
    if is_open() {
        let same = SEARCH.with(|m| m.get()) == search;
        close();
        if same {
            return;
        }
    } else if CLOSED.with(|c| c.get()).is_some_and(|(m, at)| m == search && at.elapsed() < Duration::from_millis(300)) {
        // Klik w ten sam przycisk, który właśnie zamknął Start utratą fokusu.
        return;
    }
    SEARCH.with(|m| m.set(search));
    let Ok(w) = StartWindow::new() else { return };
    let _ = (light, accent, alpha);
    crate::settings::popup_theme(&w.global::<Theme>());
    w.set_user(std::env::var("USERNAME").unwrap_or_default().into());
    w.set_search_only(search);
    let (lw, lh) = if search { SEARCH_SIZE } else { size() };
    let (pw, ph) = ((lw * scale) as i32, (lh * scale) as i32);
    w.window().set_position(PhysicalPosition::new(x, bottom - ph));
    w.window().set_size(PhysicalSize::new(pw as u32, ph as u32));
    w.on_search(|_| render());
    w.on_launch_first(|| {
        let first = START.with(|s| s.borrow().as_ref().and_then(|s| s.shown.iter().position(|r| !matches!(r, Row::Header))));
        if let Some(r) = first {
            launch_row(r);
        }
    });
    w.on_launch(|row| launch_row(row as usize));
    w.on_launch_tile(|t| launch_tile(t as usize));
    w.on_row_context(|row, x, y| {
        let r = START.with(|s| s.borrow().as_ref().and_then(|s| s.shown.get(row as usize).cloned()));
        match r {
            Some(Row::App(i)) => {
                let app = APPS.with(|a| a.borrow().get(i).cloned());
                if let Some(app) = app {
                    app_menu(&app.shell_path(), &app.id, None, x, y);
                }
            }
            Some(Row::File(p)) => {
                let (p1, p2) = (p.clone(), p);
                show_menu(
                    vec![
                        it("Otwórz", "open", move |_| {
                            let _ = open::that_detached(&p1);
                            slint::Timer::single_shot(Duration::ZERO, close);
                        }),
                        it("Pokaż w folderze", "folder", move |s| {
                            crate::open_args(s, vec![p2.clone()]);
                            crate::show_window(s);
                            slint::Timer::single_shot(Duration::ZERO, close);
                        }),
                    ],
                    x,
                    y,
                );
            }
            _ => {}
        }
    });
    w.on_tile_context(|t, x, y| tile_menu(t as usize, x, y));
    w.on_tile_drop(|t, x, y| tile_drop(t as usize, x, y));
    w.on_group_rename(|g, name| {
        edit_groups(|gs| {
            if let Some(gr) = gs.get_mut(g as usize) {
                gr.name = name.trim().to_string();
            }
        })
    });
    w.on_folder_rename(|name| {
        if let Some((g, t)) = FOLDER.with(|f| f.get()) {
            edit_groups(|gs| {
                if let Some(f) = gs.get_mut(g).and_then(|gr| gr.tiles.get_mut(t)) {
                    f.name = name.trim().to_string();
                }
            });
        }
    });
    w.on_tiles_context(|x, y| {
        // Płaska lista: podmenu wyszłoby poza okno Startu (menu rysuje się w jego obrębie).
        let add: Vec<Item> = crate::widgets::KINDS
            .iter()
            .map(|&(kind, name, icon, size, _)| it(format!("Dodaj widżet: {name}"), icon, move |_| edit_groups(|gs| tiles::add_widget(gs, kind, size))))
            .collect();
        show_menu(add, x, y);
    });
    w.on_tile_note(|i, text| {
        let Some(at) = placed(i as usize) else { return };
        // Bez przerysowania – pole pisania zostaje z kursorem.
        crate::with_state(|s| {
            let mut b = s.borrow_mut();
            if let Some(p) = tiles::get_mut(&mut b.prefs.start_groups, at) {
                p.note = text.to_string();
            }
            b.prefs.save();
        });
    });
    w.on_folder_back(|| {
        FOLDER.with(|f| f.set(None));
        render();
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
                // Menu podręczne Startu (osobne okienko) nie zamyka Startu.
                let still = START.with(|s| s.borrow().as_ref().is_some_and(|s| s.w.window().is_visible()));
                if still && !has_focus() && !menus::pop_open() {
                    close();
                }
            });
        }
        slint::winit_030::EventResult::Propagate
    });
    let _ = w.show();
    let hwnd = crate::window_hwnd(w.window());
    powerdesk_core::desktop::hide_from_taskbar(hwnd);
    crate::taskbar::popup_look(hwnd);
    powerdesk_core::taskwin::activate(hwnd);
    w.invoke_focus_search();
    crate::widgets::apply(&w.global::<crate::WidgetData>());
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
    // Jednym ruchem (położenie i rozmiar razem) – dół zostaje przyklejony do paska, a okno rośnie
    // w górę bez przeskoku. Slint dostaje nowy rozmiar z WM_SIZE i rysuje całość od razu.
    powerdesk_core::taskwin::set_rect(crate::window_hwnd(win), left, bottom - height, width, height);
    win.set_size(PhysicalSize::new(width as u32, height as u32));
    win.request_redraw();
}

fn has_focus() -> bool {
    START.with(|s| s.borrow().as_ref().is_some_and(|s| powerdesk_core::taskwin::foreground() == crate::window_hwnd(s.w.window())))
}

pub fn close() {
    SHOW_HIDDEN.with(|h| h.set(false));
    FOLDER.with(|f| f.set(None));
    if let Some(s) = START.with(|s| s.borrow_mut().take()) {
        CLOSED.with(|c| c.set(Some((SEARCH.with(|m| m.get()), std::time::Instant::now()))));
        let _ = s.w.hide();
        crate::taskbar::start_state();
    }
}

/// Okno Start, jeśli otwarte (widżety odświeżają jego dane).
pub fn each_window(f: impl Fn(&StartWindow)) {
    START.with(|s| {
        if let Some(s) = s.borrow().as_ref() {
            f(&s.w);
        }
    });
}

/// Okno Start (do pominięcia na liście okien paska).
pub fn hwnd() -> Option<isize> {
    START.with(|s| s.borrow().as_ref().map(|s| crate::window_hwnd(s.w.window())))
}

/// Wiersz listy: aplikacja, plik, ustawienie albo wynik obliczenia.
fn launch_row(row: usize) {
    let Some(r) = START.with(|s| s.borrow().as_ref().and_then(|s| s.shown.get(row).cloned())) else { return };
    match r {
        Row::App(i) => launch(i),
        Row::Header => {}
        other => {
            if startsearch::activate(&other) {
                slint::Timer::single_shot(Duration::ZERO, close);
            }
        }
    }
}

/// Grupy kafelków; stary zapis (sama lista) przenoszony za pierwszym razem.
fn groups() -> Vec<StartGroup> {
    let mut g = Vec::new();
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        let prefs = &mut b.prefs;
        if tiles::migrate(&mut prefs.start_pins, &mut prefs.start_groups) {
            prefs.save();
        }
        g = prefs.start_groups.clone();
    });
    g
}

/// Zmiana grup: zapis i przerysowanie. Otwarty folder, który zniknął, się zamyka.
fn edit_groups(f: impl FnOnce(&mut Vec<StartGroup>)) {
    let mut gs = groups();
    f(&mut gs);
    if let Some((g, t)) = FOLDER.with(|x| x.get()) {
        if !gs.get(g).and_then(|gr| gr.tiles.get(t)).is_some_and(|p| p.is_folder()) {
            FOLDER.with(|x| x.set(None));
        }
    }
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        b.prefs.start_groups = gs;
        b.prefs.save();
    });
    render();
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

fn placed(i: usize) -> Option<At> {
    PLACED.with(|p| p.borrow().0.get(i).map(|p| p.at))
}

/// Klik w kafelek: aplikacja się uruchamia, folder otwiera.
fn launch_tile(i: usize) {
    let Some(at) = placed(i) else { return };
    let Some(pin) = tiles::get(&groups(), at).cloned() else { return };
    if pin.is_folder() {
        FOLDER.with(|f| f.set(Some((at.group, at.tile))));
        render();
        return;
    }
    // Widżet: notatka – pisanie, kalendarz – kalendarz Windows, reszta nic nie uruchamia.
    if !pin.widget.is_empty() {
        match pin.widget.as_str() {
            "note" => START.with(|s| {
                if let Some(s) = s.borrow().as_ref() {
                    s.w.set_editing(i as i32);
                }
            }),
            "calendar" => {
                let _ = open::that_detached("outlookcal:");
            }
            "system" => {
                let _ = std::process::Command::new("taskmgr.exe").spawn();
            }
            "media" => crate::widgetfeeds::media(1),
            "photos" => crate::widgetfeeds::next_photo(),
            _ => {}
        }
        return;
    }
    startmenu::launch(&pin.path);
    slint::Timer::single_shot(Duration::ZERO, close);
}

fn tile_drop(i: usize, x: f32, y: f32) {
    let Some(from) = placed(i) else { return };
    let target = PLACED.with(|p| {
        let p = p.borrow();
        tiles::target(&p.0, &p.1, FOLDER.with(|f| f.get()).is_some(), x, y)
    });
    edit_groups(|gs| tiles::drop(gs, from, target));
}

/// Podmenu rozmiaru kafelka (zaznaczony bieżący).
fn size_menu(at: At, now: u8) -> Item {
    let opt = |label: &str, size: u8| {
        it(label, "grid", move |_| {
            edit_groups(|gs| {
                if let Some(p) = tiles::get_mut(gs, at) {
                    p.size = size;
                }
            })
        })
        .checked(now == size)
    };
    sub("Zmień rozmiar", "grid", vec![opt("Mały", 1), opt("Średni", 0), opt("Szeroki", 2), opt("Duży", 3)])
}

fn tile_menu(i: usize, x: f32, y: f32) {
    let Some(at) = placed(i) else { return };
    let Some(pin) = tiles::get(&groups(), at).cloned() else { return };
    if !pin.widget.is_empty() {
        let mut items = Vec::new();
        if pin.widget == "note" {
            items.push(it("Edytuj notatkę", "edit", move |_| {
                START.with(|s| {
                    if let Some(s) = s.borrow().as_ref() {
                        s.w.set_editing(i as i32);
                    }
                })
            }));
        }
        items.extend(crate::widgetfeeds::config_items(&pin.widget));
        items.push(size_menu(at, pin.size));
        items.push(sep());
        items.push(it("Usuń widżet", "trash", move |_| edit_groups(|gs| tiles::remove(gs, at))).danger());
        show_menu(items, x, y);
        return;
    }
    if !pin.is_folder() {
        let id = pin.path.trim_start_matches("shell:AppsFolder\\").to_string();
        app_menu(&pin.path, &id, Some((at, pin.size)), x, y);
        return;
    }
    let items = vec![
        it("Otwórz folder", "folder", move |_| {
            FOLDER.with(|f| f.set(Some((at.group, at.tile))));
            render();
        }),
        size_menu(at, pin.size),
        sep(),
        it("Rozgrupuj folder", "close", move |_| {
            edit_groups(|gs| {
                if let Some(gr) = gs.get_mut(at.group) {
                    if at.tile < gr.tiles.len() {
                        let f = gr.tiles.remove(at.tile);
                        for (k, c) in f.items.into_iter().enumerate() {
                            gr.tiles.insert(at.tile + k, c);
                        }
                    }
                }
            })
        }),
    ];
    show_menu(items, x, y);
}

fn launch(i: usize) {
    if let Some(app) = APPS.with(|a| a.borrow().get(i).cloned()) {
        startmenu::launch(&app.shell_path());
        // Licznik do „Polecanych”.
        crate::with_state(|s| {
            let mut b = s.borrow_mut();
            *b.prefs.app_launches.entry(app.shell_path()).or_default() += 1;
            b.prefs.save();
        });
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
        // Wyniki szukania (pliki, ustawienia) – zawsze jako lista.
        let grid = grid() && parts.is_empty();
        st.w.set_grid(grid);
        let px = if grid { (36.0 * st.w.window().scale_factor()) as u32 } else { px };
        let mut rows = Vec::new();
        let mut shown = Vec::new();
        let raw_query = st.w.get_query().to_string();
        let lpx = (24.0 * st.w.window().scale_factor()) as u32;
        let push = |rows: &mut Vec<StartRow>, shown: &mut Vec<Row>, extra: Vec<(StartRow, Row)>| {
            for (r, k) in extra {
                rows.push(r);
                shown.push(k);
            }
        };
        if parts.is_empty() && !grid {
            let mut launches = std::collections::HashMap::new();
            crate::with_state(|s| launches = s.borrow().prefs.app_launches.clone());
            let top = APPS.with(|a| startsearch::top(&a.borrow(), &launches, lpx));
            push(&mut rows, &mut shown, top);
        } else if !parts.is_empty() {
            push(&mut rows, &mut shown, startsearch::before_apps(&raw_query));
        }
        let apps_from = rows.len();
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
                        rows.push(StartRow { header: true, name: l.clone().into(), ..Default::default() });
                        shown.push(Row::Header);
                        last = l;
                    }
                }
                let img = crate::thumbs::get(&app.shell_path(), px, true);
                // Przy szukaniu nagłówek „Aplikacje”, gdy coś pasuje.
                if !parts.is_empty() && rows.len() == apps_from {
                    rows.push(StartRow { header: true, name: "Aplikacje".into(), ..Default::default() });
                    shown.push(Row::Header);
                }
                rows.push(StartRow { header: false, name: app.name.clone().into(), has_img: img.is_some(), img: img.unwrap_or_default(), hidden: is_hidden, ..Default::default() });
                shown.push(Row::App(i));
            }
        });
        if !parts.is_empty() {
            push(&mut rows, &mut shown, startsearch::after_apps(&raw_query, lpx));
        }
        st.w.set_rows(ModelRc::new(VecModel::from(rows)));
        st.shown = shown;
        let scale = st.w.window().scale_factor();
        let gs = groups();
        let folder = FOLDER.with(|f| f.get());
        let (placed, heads, height) = tiles::layout(&gs, folder);
        let name_of = |path: &str| APPS.with(|a| a.borrow().iter().find(|x| x.shell_path() == path).map(|x| x.name.clone())).unwrap_or_else(|| path.rsplit('\\').next().unwrap_or(path).to_string());
        let model: Vec<StartTile> = placed
            .iter()
            .filter_map(|p| {
                let pin = tiles::get(&gs, p.at)?;
                let px = ((if pin.size == 3 { 48.0 } else { 32.0 }) * scale) as u32;
                let img = if pin.is_folder() { None } else { crate::thumbs::get(&pin.path, px, true) };
                let minis: Vec<slint::Image> = pin.items.iter().take(4).filter_map(|c| crate::thumbs::get(&c.path, (18.0 * scale) as u32, true)).collect();
                let name = if !pin.widget.is_empty() {
                    crate::widgets::name(&pin.widget).to_string()
                } else if pin.is_folder() { if pin.name.is_empty() { format!("Folder ({})", pin.items.len()) } else { pin.name.clone() } } else { name_of(&pin.path) };
                Some(StartTile {
                    name: name.into(),
                    has_img: img.is_some(),
                    img: img.unwrap_or_default(),
                    x: p.x,
                    y: p.y,
                    w: p.w,
                    h: p.h,
                    size: pin.size as i32,
                    folder: pin.is_folder(),
                    minis: ModelRc::new(VecModel::from(minis)),
                    widget: pin.widget.clone().into(),
                    note: pin.note.clone().into(),
                })
            })
            .collect();
        let head_model: Vec<StartHead> = heads.iter().map(|h| StartHead { name: gs.get(h.group).map(|g| g.name.clone()).unwrap_or_default().into(), y: h.y }).collect();
        st.w.set_tiles(ModelRc::new(VecModel::from(model)));
        st.w.set_heads(ModelRc::new(VecModel::from(head_model)));
        st.w.set_tiles_h(height);
        st.w.set_in_folder(folder.is_some());
        let fname = folder.and_then(|(g, t)| gs.get(g).and_then(|gr| gr.tiles.get(t))).map(|f| f.name.clone()).unwrap_or_default();
        st.w.set_folder_name(fname.into());
        PLACED.with(|x| *x.borrow_mut() = (placed, heads));
    });
}

/// Ikony aplikacji dochodzą w tle.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    if is_open() && paths.iter().any(|p| p.starts_with("shell:AppsFolder\\")) {
        render();
    }
}

fn show_menu(items: Vec<Item>, x: f32, y: f32) {
    let Some(w) = START.with(|s| s.borrow().as_ref().map(|s| s.w.clone_strong())) else { return };
    menus::show_in(w.window(), items, x, y);
}

/// Menu podręczne Startu zamknięte: po wyborze Start wraca na wierzch (jeśli dalej otwarty),
/// po kliknięciu gdzie indziej – zamyka się jak po utracie fokusu.
pub fn menu_closed(chosen: bool) {
    let Some(hwnd) = hwnd() else { return };
    if chosen {
        powerdesk_core::taskwin::activate(hwnd);
    } else if powerdesk_core::taskwin::foreground() != hwnd {
        close();
    }
}

fn toggle_pin(path: &str) {
    let path = path.to_string();
    edit_groups(move |gs| {
        if tiles::contains(gs, &path) {
            tiles::unpin(gs, &path);
        } else {
            tiles::pin(gs, &path);
        }
    });
}

/// Menu aplikacji: uruchom, jako administrator, kafelek, pasek (tylko zwykłe programy .exe).
/// `tile` – menu kafelka: miejsce i rozmiar (dochodzi wybór rozmiaru).
fn app_menu(path: &str, id: &str, tile: Option<(At, u8)>, x: f32, y: f32) {
    let (p1, p2, p3, p4) = (path.to_string(), path.to_string(), path.to_string(), path.to_string());
    let pinned = tiles::contains(&groups(), path);
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
    if let Some((at, size)) = tile {
        items.push(size_menu(at, size));
    }
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

/// Dokumenty albo Obrazy – z ustawień Windows (także przeniesione do OneDrive).
fn known(documents: bool) -> String {
    let dir = if documents { dirs::document_dir() } else { dirs::picture_dir() };
    dir.map(|d| d.to_string_lossy().to_string()).unwrap_or_else(|| std::env::var("USERPROFILE").unwrap_or_default())
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
            let dir = known(i == 1);
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
        5 => {
            crate::with_state(|s| {
                crate::open_computer(s);
                crate::show_window(s);
            });
            slint::Timer::single_shot(Duration::ZERO, close);
        }
        _ => show_menu(power_items(), x, y),
    }
}
