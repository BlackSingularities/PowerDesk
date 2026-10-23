//! Strefy okien: przeciąganie okna z wciśniętym Shiftem pokazuje strefy na monitorze pod
//! kursorem, puszczenie układa okno w strefie pod kursorem. Układy (numer jak w Ustawieniach →
//! Pulpit): 2 i 3 kolumny, 2×2, główna + 2, szeroki środek. Zdarzenia przeciągania – core::snap.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use powerdesk_core::snap;
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, VecModel};
use windows_rect::Rect;

use crate::{ZoneRect, ZonesWindow};

/// Prostokąt (lewo, góra, prawo, dół) w pikselach ekranu – bez zależności od pakietu windows w aplikacji.
mod windows_rect {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Rect {
        pub l: i32,
        pub t: i32,
        pub r: i32,
        pub b: i32,
    }
}

const GAP: i32 = 8;

thread_local! {
    static OV: RefCell<Option<ZonesWindow>> = const { RefCell::new(None) };
    static TIMER: slint::Timer = slint::Timer::default();
    /// Przeciągane okno i strefy monitora (piksele ekranu).
    static DRAG: Cell<isize> = const { Cell::new(0) };
    static ZONES: RefCell<Vec<Rect>> = const { RefCell::new(Vec::new()) };
}

/// Strefy układu `layout` w obszarze roboczym (z odstępami).
pub fn zones(layout: u8, w: Rect) -> Vec<Rect> {
    let (x, y, wd, ht) = (w.l, w.t, w.r - w.l, w.b - w.t);
    let cols = |fr: &[f32]| -> Vec<Rect> {
        let mut out = Vec::new();
        let mut left = x as f32;
        for f in fr {
            let width = wd as f32 * f;
            out.push(Rect { l: left as i32, t: y, r: (left + width) as i32, b: y + ht });
            left += width;
        }
        out
    };
    let mut v = match layout {
        1 => cols(&[1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0]),
        2 => {
            let (mx, my) = (x + wd / 2, y + ht / 2);
            vec![Rect { l: x, t: y, r: mx, b: my }, Rect { l: mx, t: y, r: x + wd, b: my }, Rect { l: x, t: my, r: mx, b: y + ht }, Rect { l: mx, t: my, r: x + wd, b: y + ht }]
        }
        3 => {
            let mx = x + wd * 6 / 10;
            let my = y + ht / 2;
            vec![Rect { l: x, t: y, r: mx, b: y + ht }, Rect { l: mx, t: y, r: x + wd, b: my }, Rect { l: mx, t: my, r: x + wd, b: y + ht }]
        }
        4 => cols(&[0.25, 0.5, 0.25]),
        _ => cols(&[0.5, 0.5]),
    };
    // Odstęp między strefami i od krawędzi.
    for r in v.iter_mut() {
        r.l += GAP / 2 + if r.l == x { GAP / 2 } else { 0 };
        r.t += GAP / 2 + if r.t == y { GAP / 2 } else { 0 };
        r.r -= GAP / 2 + if r.r == x + wd { GAP / 2 } else { 0 };
        r.b -= GAP / 2 + if r.b == y + ht { GAP / 2 } else { 0 };
    }
    v
}

fn settings() -> (bool, u8) {
    let mut out = (true, 3);
    crate::with_state(|s| {
        let b = s.borrow();
        out = (b.prefs.zones_on, b.prefs.zones_layout);
    });
    out
}

pub fn init() {
    snap::start(|start, hwnd| {
        let _ = slint::invoke_from_event_loop(move || if start { begin(hwnd) } else { end(hwnd) });
    });
}

fn begin(hwnd: isize) {
    let (on, _) = settings();
    // Własne okna PowerDesk (pulpit, pasek, nakładka) nie układamy.
    if !on || powerdesk_core::taskwin::is_own(hwnd) {
        return;
    }
    DRAG.with(|d| d.set(hwnd));
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_millis(30), follow));
}

fn end(hwnd: isize) {
    TIMER.with(|t| t.stop());
    let shown = OV.with(|o| o.borrow().is_some());
    if shown && DRAG.with(|d| d.get()) == hwnd && powerdesk_core::shellmode::shift_held() {
        if let Some(z) = zone_under_cursor() {
            // Chwila po puszczeniu – Windows kończy własne przesuwanie, potem nasze miejsce.
            slint::Timer::single_shot(Duration::from_millis(30), move || {
                snap::place(hwnd, windows::to(z));
            });
        }
    }
    hide();
    DRAG.with(|d| d.set(0));
}

mod windows {
    pub fn to(r: super::Rect) -> powerdesk_core::snap::Rect {
        powerdesk_core::snap::Rect { left: r.l, top: r.t, right: r.r, bottom: r.b }
    }
}

fn zone_under_cursor() -> Option<Rect> {
    let (cx, cy) = snap::cursor();
    ZONES.with(|z| z.borrow().iter().copied().find(|r| cx >= r.l && cx < r.r && cy >= r.t && cy < r.b))
}

/// Co 30 ms w trakcie przeciągania: Shift wciśnięty – strefy widoczne i podświetlona ta pod kursorem.
fn follow() {
    if !powerdesk_core::shellmode::shift_held() {
        hide();
        return;
    }
    let (mon, work) = snap::monitor_at_cursor();
    let work = Rect { l: work.left, t: work.top, r: work.right, b: work.bottom };
    let (_, layout) = settings();
    let zs = zones(layout, work);
    let active = zone_under_cursor_in(&zs);
    ZONES.with(|z| *z.borrow_mut() = zs.clone());
    OV.with(|o| {
        let mut o = o.borrow_mut();
        if o.is_none() {
            let Ok(w) = ZonesWindow::new() else { return };
            w.window().set_position(PhysicalPosition::new(mon.left, mon.top));
            w.window().set_size(PhysicalSize::new((mon.right - mon.left) as u32, (mon.bottom - mon.top) as u32));
            let _ = w.show();
            w.window().with_winit_window(|win| {
                let _ = win.set_cursor_hittest(false);
            });
            let hwnd = crate::window_hwnd(w.window());
            powerdesk_core::desktop::hide_from_taskbar(hwnd);
            powerdesk_core::system::see_through(hwnd);
            *o = Some(w);
        }
        let Some(w) = o.as_ref() else { return };
        // Okno stref mogło zostać na innym monitorze – przenosimy je pod kursor.
        let pos = w.window().position();
        if pos.x != mon.left || pos.y != mon.top {
            w.window().set_position(PhysicalPosition::new(mon.left, mon.top));
            w.window().set_size(PhysicalSize::new((mon.right - mon.left) as u32, (mon.bottom - mon.top) as u32));
        }
        let scale = w.window().scale_factor();
        let rows: Vec<ZoneRect> = zs
            .iter()
            .enumerate()
            .map(|(i, r)| ZoneRect {
                x: (r.l - mon.left) as f32 / scale,
                y: (r.t - mon.top) as f32 / scale,
                w: (r.r - r.l) as f32 / scale,
                h: (r.b - r.t) as f32 / scale,
                active: active == Some(i),
                num: i as i32 + 1,
            })
            .collect();
        w.set_zones(ModelRc::new(VecModel::from(rows)));
        powerdesk_core::taskwin::keep_topmost(crate::window_hwnd(w.window()));
    });
}

fn zone_under_cursor_in(zs: &[Rect]) -> Option<usize> {
    let (cx, cy) = snap::cursor();
    zs.iter().position(|r| cx >= r.l && cx < r.r && cy >= r.t && cy < r.b)
}

fn hide() {
    if let Some(w) = OV.with(|o| o.borrow_mut().take()) {
        let _ = w.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_columns_fill_work_area() {
        let z = zones(0, Rect { l: 0, t: 0, r: 1000, b: 800 });
        assert_eq!(z.len(), 2);
        assert_eq!(z[0].l, 8);
        assert_eq!(z[1].r, 992);
        assert_eq!(z[0].r + 8, z[1].l);
    }

    #[test]
    fn grid_has_four() {
        assert_eq!(zones(2, Rect { l: 0, t: 0, r: 1000, b: 800 }).len(), 4);
    }
}
