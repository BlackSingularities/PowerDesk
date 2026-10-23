//! Okienko historii schowka PowerDesk (Win+V): lista z core::cliphist, klik wkleja wpis
//! w okno, które było aktywne przed otwarciem; przypinanie, usuwanie, czyszczenie.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::time::Duration;

use powerdesk_core::cliphist::{self, Kind};
use powerdesk_core::taskwin;
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, VecModel};

use crate::{ClipItem, ClipWindow, Theme};

const WIDTH: f32 = 380.0;
const HEIGHT: f32 = 500.0;

thread_local! {
    static WIN: RefCell<Option<ClipWindow>> = const { RefCell::new(None) };
    /// Identyfikatory wpisów w kolejności wierszy.
    static IDS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    /// Okno, do którego wklejamy (aktywne przed Win+V).
    static TARGET: Cell<isize> = const { Cell::new(0) };
}

/// Nasłuch schowka i Win+V – wywoływane raz przy starcie programu.
pub fn start() {
    cliphist::start(|| {
        let _ = slint::invoke_from_event_loop(render);
    });
    powerdesk_core::startmenu::hook_win_v(|| {
        let _ = slint::invoke_from_event_loop(toggle);
    });
}

pub fn hwnd() -> Option<isize> {
    WIN.with(|w| w.borrow().as_ref().map(|w| crate::window_hwnd(w.window())))
}

pub fn toggle() {
    if WIN.with(|w| w.borrow().is_some()) {
        close();
        return;
    }
    TARGET.with(|t| t.set(taskwin::foreground()));
    let Ok(w) = ClipWindow::new() else { return };
    // Motyw i krycie jak okienka paska; miejsce – nad paskiem monitora z kursorem, na środku.
    if let Some((light, accent, alpha, (x, bottom, mw), scale)) = crate::taskbar::popup_place() {
        let _ = (light, accent, alpha);
        crate::settings::popup_theme(&w.global::<Theme>());
        let (pw, ph) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32);
        w.window().set_position(PhysicalPosition::new(x + (mw - pw) / 2, bottom - ph - (8.0 * scale) as i32));
        w.window().set_size(PhysicalSize::new(pw as u32, ph as u32));
    }
    w.on_pick(|i| pick(i as usize));
    w.on_pin(|i| {
        if let Some(id) = id_at(i as usize) {
            cliphist::toggle_pin(id);
            render();
        }
    });
    w.on_remove(|i| {
        if let Some(id) = id_at(i as usize) {
            cliphist::remove(id);
            render();
        }
    });
    w.on_clear(|| {
        cliphist::clear();
        render();
    });
    w.on_dismiss(|| slint::Timer::single_shot(Duration::ZERO, close));
    w.window().on_winit_window_event(|_, ev| {
        if let slint::winit_030::winit::event::WindowEvent::Focused(false) = ev {
            slint::Timer::single_shot(Duration::ZERO, close);
        }
        slint::winit_030::EventResult::Propagate
    });
    let _ = w.show();
    let hwnd = crate::window_hwnd(w.window());
    powerdesk_core::desktop::hide_from_taskbar(hwnd);
    crate::taskbar::popup_look(hwnd);
    taskwin::activate(hwnd);
    w.invoke_focus_keys();
    WIN.with(|x| *x.borrow_mut() = Some(w));
    render();
}

pub fn close() {
    if let Some(w) = WIN.with(|w| w.borrow_mut().take()) {
        let _ = w.hide();
    }
}

fn id_at(i: usize) -> Option<u64> {
    IDS.with(|ids| ids.borrow().get(i).copied())
}

/// Wpis do schowka, okienko znika, fokus wraca do poprzedniego okna i Ctrl+V.
fn pick(i: usize) {
    let Some(id) = id_at(i) else { return };
    if !cliphist::restore(id) {
        return;
    }
    close();
    let target = TARGET.with(|t| t.get());
    if target != 0 {
        taskwin::activate(target);
    }
    slint::Timer::single_shot(Duration::from_millis(120), cliphist::paste);
}

fn ago(t: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(t);
    let s = now.saturating_sub(t);
    match s {
        0..=59 => "przed chwilą".into(),
        60..=3599 => format!("{} min temu", s / 60),
        3600..=86399 => format!("{} godz. temu", s / 3600),
        _ => format!("{} dni temu", s / 86400),
    }
}

/// Tekst wpisu: do 400 znaków (dłuższe skraca okno), pliki – nazwy po przecinku.
fn preview(k: &Kind) -> String {
    match k {
        Kind::Text(t) => {
            let t = t.trim();
            if t.chars().count() > 400 {
                format!("{}…", t.chars().take(400).collect::<String>())
            } else {
                t.to_string()
            }
        }
        Kind::Files(f) => {
            let names: Vec<String> = f.iter().take(6).map(|p| p.rsplit('\\').next().unwrap_or(p).to_string()).collect();
            let more = if f.len() > 6 { format!(" i {} więcej", f.len() - 6) } else { String::new() };
            format!("{}{more}", names.join(", "))
        }
        Kind::Image(_) => String::new(),
    }
}

fn render() {
    WIN.with(|w| {
        let w = w.borrow();
        let Some(w) = w.as_ref() else { return };
        let px = (300.0 * w.window().scale_factor()) as u32;
        let list = cliphist::list();
        let items: Vec<ClipItem> = list
            .iter()
            .map(|e| {
                let (kind, img) = match &e.kind {
                    Kind::Text(_) => (0, None),
                    Kind::Image(name) => (1, crate::thumbs::get(&cliphist::image_path(name), px, false)),
                    Kind::Files(_) => (2, None),
                };
                ClipItem { kind, text: preview(&e.kind).into(), has_img: img.is_some(), img: img.unwrap_or_default(), pinned: e.pinned, when: ago(e.time).into() }
            })
            .collect();
        IDS.with(|ids| *ids.borrow_mut() = list.iter().map(|e| e.id).collect());
        w.set_items(ModelRc::new(VecModel::from(items)));
    });
}

/// Miniatury obrazów dochodzą w tle.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    if WIN.with(|w| w.borrow().is_some()) && paths.iter().any(|p| p.contains("\\PowerDesk\\clipboard\\")) {
        render();
    }
}
