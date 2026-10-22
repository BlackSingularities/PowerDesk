//! Pulpit PowerDesk – na razie tryb testowy: na każdym monitorze okno bez ramki z tapetą
//! z ustawień Windows, a na głównym także ikony z Pulpitu użytkownika i publicznego.
//! Pulpitu Windows nie ruszamy; Esc (albo menu ikony w zasobniku) zamyka wszystkie okna.

use std::cell::RefCell;
use std::collections::HashSet;

use powerdesk_core::desktop::{self as data, Fit, Monitor};
use powerdesk_core::fsx::Entry;
use slint::{Color, ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::{fmt, transfers, DeskIcon, DesktopWindow, Shared};

/// Flaga wiersza poleceń: start od razu z pulpitem testowym.
pub const TEST_FLAG: &str = "--desktop-test";

thread_local! {
    /// Okna pulpitu (po jednym na monitor); pierwsze – monitor główny z ikonami.
    static WINDOWS: RefCell<Vec<DesktopWindow>> = const { RefCell::new(Vec::new()) };
    static ITEMS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

pub fn is_open() -> bool {
    WINDOWS.with(|w| !w.borrow().is_empty())
}

pub fn toggle(s: &Shared) {
    if is_open() {
        close();
    } else {
        open(s);
    }
}

/// Tapety dekodujemy w tle – duży obraz na wątku UI zamroziłby okno.
fn decode(path: &str) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::open(path).ok()?.to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

pub fn open(_s: &Shared) {
    if is_open() {
        return;
    }
    std::thread::spawn(|| {
        let mons = data::monitors();
        let walls: Vec<_> = mons.iter().map(|m| if m.wallpaper.is_empty() { None } else { decode(&m.wallpaper) }).collect();
        let items = data::items();
        let _ = slint::invoke_from_event_loop(move || crate::with_state(|s| build(s, mons, walls, items)));
    });
}

fn build(s: &Shared, mons: Vec<Monitor>, walls: Vec<Option<SharedPixelBuffer<Rgba8Pixel>>>, items: Vec<Entry>) {
    if mons.is_empty() {
        transfers::toast(s, "Nie udało się odczytać monitorów i tapet", transfers::Kind::Error, None);
        return;
    }
    ITEMS.with(|i| *i.borrow_mut() = items);
    // Główny monitor pierwszy – to on dostaje ikony.
    let mut order: Vec<usize> = (0..mons.len()).collect();
    order.sort_by_key(|&i| !mons[i].primary);
    let mut walls = walls;
    let mut out = Vec::new();
    for (n, i) in order.into_iter().enumerate() {
        let m = &mons[i];
        let Ok(w) = DesktopWindow::new() else { continue };
        w.set_bg(Color::from_rgb_u8(m.color.0, m.color.1, m.color.2));
        if let Some(buf) = walls[i].take() {
            w.set_wallpaper(slint::Image::from_rgba8(buf));
            w.set_has_wallpaper(true);
        }
        w.set_fit(match m.fit {
            Fit::Fill | Fit::Span => 0,
            Fit::Fit => 1,
            Fit::Stretch => 2,
            Fit::Center => 3,
            Fit::Tile => 4,
        });
        w.window().set_position(PhysicalPosition::new(m.x, m.y));
        w.window().set_size(PhysicalSize::new(m.width as u32, m.height as u32));
        if n == 0 {
            let s2 = s.clone();
            w.on_open(move |i| open_item(&s2, i as usize));
            let ww = w.as_weak();
            w.on_context(move |i, x, y| {
                if let Some(w) = ww.upgrade() {
                    context(&w, i as usize, x, y);
                }
            });
        }
        // Okno nie może zniknąć w trakcie własnego zdarzenia – zamykamy w następnym obiegu pętli.
        w.on_close(|| slint::Timer::single_shot(std::time::Duration::ZERO, close));
        let s2 = s.clone();
        w.on_refresh(move || refresh(&s2));
        let _ = w.show();
        data::hide_from_taskbar(crate::window_hwnd(w.window()));
        out.push(w);
    }
    WINDOWS.with(|ws| *ws.borrow_mut() = out);
    render_icons();
}

pub fn close() {
    let ws = WINDOWS.with(|w| std::mem::take(&mut *w.borrow_mut()));
    for w in ws {
        let _ = w.hide();
    }
}

/// Nazwa jak na pulpicie Windows: skróty bez .lnk / .url.
fn label(e: &Entry) -> String {
    let low = e.name.to_lowercase();
    if !e.dir && (low.ends_with(".lnk") || low.ends_with(".url")) {
        e.name[..e.name.len() - 4].to_string()
    } else {
        e.name.clone()
    }
}

fn render_icons() {
    WINDOWS.with(|ws| {
        let ws = ws.borrow();
        let Some(w) = ws.first() else { return };
        let px = (48.0 * w.window().scale_factor()) as u32;
        let icons: Vec<DeskIcon> = ITEMS.with(|i| {
            i.borrow()
                .iter()
                .map(|e| {
                    // Zdjęcia i filmy z miniaturą, reszta (skróty, dokumenty) – ikona systemowa.
                    let img = crate::thumbs::get(&e.path, px, !fmt::is_media(e));
                    DeskIcon { name: label(e).into(), path: e.path.clone().into(), has_img: img.is_some(), img: img.unwrap_or_default(), dir: e.dir }
                })
                .collect()
        });
        w.set_icons(ModelRc::new(VecModel::from(icons)));
    });
}

/// Miniatury i ikony dochodzą w tle – przerysowujemy, gdy trafiły ikony z pulpitu.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    if is_open() && ITEMS.with(|i| i.borrow().iter().any(|e| paths.contains(&e.path))) {
        render_icons();
    }
}

fn refresh(_s: &Shared) {
    std::thread::spawn(|| {
        let items = data::items();
        let _ = slint::invoke_from_event_loop(move || {
            ITEMS.with(|i| *i.borrow_mut() = items);
            render_icons();
        });
    });
}

/// Dwuklik: folder w oknie PowerDesk, plik i skrót – w swoim programie.
fn open_item(s: &Shared, i: usize) {
    let Some(e) = ITEMS.with(|it| it.borrow().get(i).cloned()) else { return };
    if e.dir {
        crate::open_args(s, vec![e.path]);
    } else {
        let _ = open::that_detached(&e.path);
    }
}

/// Prawy przycisk na ikonie – pełne menu Eksploratora (jak na pulpicie Windows).
fn context(w: &DesktopWindow, i: usize, x: f32, y: f32) {
    let Some(e) = ITEMS.with(|it| it.borrow().get(i).cloned()) else { return };
    let scale = w.window().scale_factor();
    let h = crate::window_hwnd(w.window());
    if let Ok(true) = powerdesk_core::shell::native_menu(h, &[e.path], (x * scale) as i32, (y * scale) as i32, false) {
        // Polecenie mogło zmienić pulpit (usunięcie, zmiana nazwy).
        crate::with_state(refresh);
    }
}
