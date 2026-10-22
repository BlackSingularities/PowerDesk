//! Pasek zadań PowerDesk – na razie tryb testowy: na dole każdego monitora okno na wierzchu
//! (nad paskiem Windows, który działa dalej) z przyciskiem Start, oknami z tego monitora
//! i zegarem. Lista okien odświeża się co sekundę.

use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use chrono::Local;
use powerdesk_core::desktop as data;
use powerdesk_core::taskwin::{self, Win};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{Shared, TaskButton, TaskbarWindow, Theme};

/// Flaga wiersza poleceń: start od razu z paskiem testowym.
pub const TEST_FLAG: &str = "--taskbar-test";
/// Wysokość paska w pikselach logicznych.
const HEIGHT: f32 = 48.0;

struct Bar {
    w: TaskbarWindow,
    /// Lewy górny róg monitora – po nim przypisujemy okna.
    origin: (i32, i32),
    /// Położenie okna paska na ekranie (do menu podręcznego).
    pos: (i32, i32),
    /// Okna w kolejności przycisków.
    wins: Vec<Win>,
    fg: isize,
}

thread_local! {
    static BARS: RefCell<Vec<Bar>> = const { RefCell::new(Vec::new()) };
    static TIMER: slint::Timer = slint::Timer::default();
}

pub fn is_open() -> bool {
    BARS.with(|b| !b.borrow().is_empty())
}

pub fn toggle(s: &Shared) {
    if is_open() {
        close();
    } else {
        open(s);
    }
}

pub fn open(_s: &Shared) {
    if is_open() {
        return;
    }
    std::thread::spawn(|| {
        let mons = data::monitors();
        let _ = slint::invoke_from_event_loop(move || crate::with_state(|s| build(s, mons)));
    });
}

fn build(s: &Shared, mons: Vec<data::Monitor>) {
    if mons.is_empty() {
        crate::transfers::toast(s, "Nie udało się odczytać monitorów", crate::transfers::Kind::Error, None);
        return;
    }
    let look = s.borrow().ui.upgrade().map(|u| crate::settings::look(&u));
    let mut bars = Vec::new();
    for (n, m) in mons.iter().enumerate() {
        let Ok(w) = TaskbarWindow::new() else { continue };
        if let Some(l) = &look {
            let t = w.global::<Theme>();
            t.set_light(l.light);
            t.set_accent_override(l.accent);
        }
        w.on_start(taskwin::open_start);
        w.on_activate(move |i| {
            if let Some(h) = hwnd_at(n, i as usize) {
                taskwin::click(h);
                // Stan (aktywne / zminimalizowane) od razu, bez czekania na zegar.
                slint::Timer::single_shot(Duration::from_millis(150), refresh);
            }
        });
        w.on_context(move |i, x, y| context(n, i as usize, x, y));
        w.on_close_test(|| slint::Timer::single_shot(Duration::ZERO, close));
        let _ = w.show();
        let scale = w.window().scale_factor();
        let h = (HEIGHT * scale).round() as i32;
        let pos = (m.x, m.y + m.height - h);
        let hwnd = crate::window_hwnd(w.window());
        data::hide_from_taskbar(hwnd);
        taskwin::place_topmost(hwnd, pos.0, pos.1, m.width, h);
        bars.push(Bar { w, origin: (m.x, m.y), pos, wins: Vec::new(), fg: 0 });
    }
    BARS.with(|b| *b.borrow_mut() = bars);
    refresh();
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(1), refresh));
}

pub fn close() {
    TIMER.with(|t| t.stop());
    let bars = BARS.with(|b| std::mem::take(&mut *b.borrow_mut()));
    for b in bars {
        let _ = b.w.hide();
    }
}

fn hwnd_at(bar: usize, i: usize) -> Option<isize> {
    BARS.with(|b| b.borrow().get(bar).and_then(|b| b.wins.get(i)).map(|w| w.hwnd))
}

/// Prawy przycisk na oknie – systemowe menu z „Zamknij okno”.
fn context(bar: usize, i: usize, x: f32, y: f32) {
    let Some(target) = hwnd_at(bar, i) else { return };
    let Some((owner, pos, scale)) = BARS.with(|b| b.borrow().get(bar).map(|b| (crate::window_hwnd(b.w.window()), b.pos, b.w.window().scale_factor()))) else { return };
    let (sx, sy) = (pos.0 + (x * scale) as i32, pos.1 + (y * scale) as i32);
    match taskwin::pick(owner, &["Przełącz na to okno", "Zamknij okno"], sx, sy) {
        Some(0) => taskwin::click(target),
        Some(1) => taskwin::close(target),
        _ => {}
    }
    slint::Timer::single_shot(Duration::from_millis(300), refresh);
}

/// Stała kolejność: okna już na pasku zostają na swoich miejscach, nowe dochodzą na koniec.
fn keep_order(prev: &[Win], now: Vec<Win>) -> Vec<Win> {
    let mut out: Vec<Win> = prev.iter().filter_map(|p| now.iter().find(|w| w.hwnd == p.hwnd).cloned()).collect();
    for w in now {
        if !out.iter().any(|o| o.hwnd == w.hwnd) {
            out.push(w);
        }
    }
    out
}

fn buttons(wins: &[Win], fg: isize, px: u32) -> Vec<TaskButton> {
    wins.iter()
        .map(|w| {
            let img = if w.exe.is_empty() { None } else { crate::thumbs::get(&w.exe, px, true) };
            TaskButton { title: w.title.clone().into(), has_img: img.is_some(), img: img.unwrap_or_default(), active: w.hwnd == fg, minimized: w.minimized }
        })
        .collect()
}

fn refresh() {
    let all = taskwin::list();
    let fg = taskwin::foreground();
    let now = Local::now();
    let (time, date) = (now.format("%H:%M").to_string(), now.format("%d.%m.%Y").to_string());
    BARS.with(|b| {
        for bar in b.borrow_mut().iter_mut() {
            let mine: Vec<Win> = all.iter().filter(|w| w.monitor == bar.origin).cloned().collect();
            let wins = keep_order(&bar.wins, mine);
            // Model tylko przy zmianie – inaczej co sekundę migałyby podpowiedzi.
            if wins != bar.wins || fg != bar.fg {
                let px = (20.0 * bar.w.window().scale_factor()) as u32;
                bar.w.set_buttons(ModelRc::new(VecModel::from(buttons(&wins, fg, px))));
                bar.wins = wins;
                bar.fg = fg;
            }
            bar.w.set_time(time.clone().into());
            bar.w.set_date(date.clone().into());
        }
    });
}

/// Ikony programów dochodzą w tle – przerysowujemy paski, na których są.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            if bar.wins.iter().any(|w| paths.contains(&w.exe)) {
                let px = (20.0 * bar.w.window().scale_factor()) as u32;
                bar.w.set_buttons(ModelRc::new(VecModel::from(buttons(&bar.wins, bar.fg, px))));
            }
        }
    });
}
