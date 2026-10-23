//! Widok zadań (Win+Tab) w trybie powłoki – bez Eksploratora systemowego nie ma. Okno na cały
//! monitor z kursorem, karty wszystkich okien w siatce i ich podglądy DWM.

use std::cell::RefCell;
use std::time::Duration;

use powerdesk_core::taskwin::{self, Win};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, VecModel};

use crate::{TaskCard, TaskViewWindow, Theme};

/// Pasek tytułu karty i marginesy podglądu (te same co w TaskViewWindow).
const TITLE: f32 = 34.0;
const PAD: f32 = 8.0;
const GAP: f32 = 24.0;

struct View {
    w: TaskViewWindow,
    wins: Vec<Win>,
    thumbs: Vec<isize>,
    /// Okno aktywne przed otwarciem – wraca po Esc.
    prev: isize,
}

thread_local! {
    static VIEW: RefCell<Option<View>> = const { RefCell::new(None) };
}

pub fn toggle() {
    if VIEW.with(|v| v.borrow().is_some()) {
        close(true);
    } else {
        open();
    }
}

/// Siatka kart: kolumny tak, żeby karty były jak największe (proporcje 16:10 + tytuł).
fn grid(n: usize, w: f32, h: f32) -> (usize, f32, f32) {
    let mut best = (1, 0.0, 0.0);
    for cols in 1..=n.max(1) {
        let rows = n.div_ceil(cols).max(1);
        let cw = ((w - GAP * (cols as f32 + 1.0)) / cols as f32).min(420.0);
        let ch = cw * 0.625 + TITLE;
        let ch = ch.min((h - GAP * (rows as f32 + 1.0)) / rows as f32);
        let cw = cw.min((ch - TITLE) / 0.625);
        if cw > best.1 {
            best = (cols, cw, cw * 0.625 + TITLE);
        }
    }
    best
}

fn open() {
    let Some((origin, mon, scale)) = crate::taskbar::monitor_under_cursor() else { return };
    let Ok(w) = TaskViewWindow::new() else { return };
    let mut look = None;
    crate::with_state(|s| look = s.borrow().ui.upgrade().map(|u| crate::settings::look(&u)));
    if let Some(l) = look {
        let t = w.global::<Theme>();
        t.set_light(l.light);
        t.set_accent_override(l.accent);
    }
    let own: Vec<isize> = [crate::start::hwnd(), crate::cliphistory::hwnd()].into_iter().flatten().collect();
    let wins: Vec<Win> = taskwin::list().into_iter().filter(|x| !own.contains(&x.hwnd)).collect();
    let prev = taskwin::foreground();
    let (lw, lh) = (mon.0 as f32 / scale, mon.1 as f32 / scale);
    let (cols, cw, ch) = grid(wins.len(), lw, lh);
    let rows = wins.len().div_ceil(cols.max(1));
    let total_w = cols.min(wins.len().max(1)) as f32 * (cw + GAP) - GAP;
    let total_h = rows as f32 * (ch + GAP) - GAP;
    let (x0, y0) = ((lw - total_w) / 2.0, (lh - total_h) / 2.0);
    let rects: Vec<(f32, f32)> = (0..wins.len()).map(|i| (x0 + (i % cols) as f32 * (cw + GAP), y0 + (i / cols) as f32 * (ch + GAP))).collect();
    let px = (16.0 * scale) as u32;
    let cards: Vec<TaskCard> = wins
        .iter()
        .zip(&rects)
        .map(|(win, &(x, y))| {
            let img = if win.exe.is_empty() { None } else { crate::thumbs::get(&win.exe, px, true) };
            TaskCard { title: win.title.clone().into(), has_img: img.is_some(), img: img.unwrap_or_default(), active: win.hwnd == prev, x, y, w: cw, h: ch }
        })
        .collect();
    w.set_cards(ModelRc::new(VecModel::from(cards)));
    w.on_pick(|i| {
        let h = VIEW.with(|v| v.borrow().as_ref().and_then(|v| v.wins.get(i as usize).map(|w| w.hwnd)));
        slint::Timer::single_shot(Duration::ZERO, move || {
            close(false);
            if let Some(h) = h {
                taskwin::activate(h);
            }
        });
    });
    w.on_close_win(|i| {
        let h = VIEW.with(|v| v.borrow().as_ref().and_then(|v| v.wins.get(i as usize).map(|w| w.hwnd)));
        if let Some(h) = h {
            taskwin::close(h);
            // Okno potrzebuje chwili na zamknięcie (może też zapytać o zapis) – widok od nowa.
            slint::Timer::single_shot(Duration::from_millis(400), || {
                if VIEW.with(|v| v.borrow().is_some()) {
                    close(false);
                    open();
                }
            });
        }
    });
    w.on_dismiss(|| slint::Timer::single_shot(Duration::ZERO, || close(true)));
    w.window().on_winit_window_event(|_, ev| {
        if let slint::winit_030::winit::event::WindowEvent::Focused(false) = ev {
            slint::Timer::single_shot(Duration::from_millis(80), || close(false));
        }
        slint::winit_030::EventResult::Propagate
    });
    w.window().set_position(PhysicalPosition::new(origin.0, origin.1));
    w.window().set_size(PhysicalSize::new(mon.0 as u32, mon.1 as u32));
    let _ = w.show();
    let dest = crate::window_hwnd(w.window());
    powerdesk_core::desktop::hide_from_taskbar(dest);
    powerdesk_core::system::see_through(dest);
    taskwin::activate(dest);
    let mut thumbs = Vec::new();
    for (win, &(x, y)) in wins.iter().zip(&rects) {
        let Some((id, (sw, sh))) = taskwin::thumb_register(dest, win.hwnd) else { continue };
        let (bx, by, bw, bh) = (x + PAD, y + TITLE, cw - 2.0 * PAD, ch - TITLE - PAD);
        let ratio = if sw > 0 && sh > 0 { sw as f32 / sh as f32 } else { bw / bh };
        let (tw, th) = if ratio > bw / bh { (bw, bw / ratio) } else { (bh * ratio, bh) };
        let (tx, ty) = (bx + (bw - tw) / 2.0, by + (bh - th) / 2.0);
        if !win.minimized {
            taskwin::thumb_place(id, (tx * scale) as i32, (ty * scale) as i32, (tw * scale) as i32, (th * scale) as i32);
        }
        thumbs.push(id);
    }
    VIEW.with(|v| *v.borrow_mut() = Some(View { w, wins, thumbs, prev }));
}

/// Zamyka widok; `back` – wraca do okna, które było aktywne.
fn close(back: bool) {
    let Some(v) = VIEW.with(|v| v.borrow_mut().take()) else { return };
    for id in v.thumbs {
        taskwin::thumb_unregister(id);
    }
    let _ = v.w.hide();
    if back && v.prev != 0 {
        taskwin::activate(v.prev);
    }
}
