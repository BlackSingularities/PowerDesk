//! Odtwarzanie wideo i audio w panelu podglądu i w przeglądarce (Media Foundation, `core::media`).
//! Gra najwyżej jeden plik naraz; klatki i pozycję odbiera timer w wątku interfejsu.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use powerdesk_core::media::Player;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::{PlayerState, Shared};

/// Kto gra: panel podglądu albo przeglądarka.
pub const PREVIEW: i32 = 1;
pub const VIEWER: i32 = 2;

thread_local! {
    static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
    static TIMER: slint::Timer = slint::Timer::default();
    /// Głośność i wyciszenie przechodzą na kolejne pliki.
    static VOLUME: Cell<f64> = const { Cell::new(1.0) };
    static MUTED: Cell<bool> = const { Cell::new(false) };
}

fn with_ui(s: &Shared, f: impl FnOnce(&PlayerState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<PlayerState>());
    }
}

/// 75 s → „1:15”, godziny tylko, gdy są.
fn clock(t: f64) -> String {
    let t = t.max(0.0) as u64;
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Zaczyna grać `path` w miejscu `target`; poprzednie odtwarzanie znika.
pub fn start(s: &Shared, target: i32, path: &str) {
    stop(s);
    let p = match Player::open(path, true) {
        Ok(p) => p,
        Err(_) => {
            with_ui(s, |g| {
                g.set_target(target);
                g.set_path(path.into());
                g.set_error(true);
            });
            return;
        }
    };
    p.set_volume(VOLUME.with(|v| v.get()));
    p.set_muted(MUTED.with(|m| m.get()));
    PLAYER.with(|x| *x.borrow_mut() = Some(p));
    with_ui(s, |g| {
        g.set_target(target);
        g.set_path(path.into());
        g.set_playing(true);
        g.set_error(false);
        g.set_volume(VOLUME.with(|v| v.get()) as f32);
        g.set_muted(MUTED.with(|m| m.get()));
        g.set_time("0:00".into());
    });
    let s2 = s.clone();
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_millis(15), move || tick(&s2)));
}

/// Klatka i stan odtwarzania do interfejsu.
fn tick(s: &Shared) {
    let Some((st, frame)) = PLAYER.with(|x| x.borrow_mut().as_mut().map(|p| (p.status(), p.frame()))) else { return };
    with_ui(s, |g| {
        if let Some(f) = frame {
            let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&f.rgba, f.width, f.height);
            g.set_frame(Image::from_rgba8(buf));
            g.set_has_frame(true);
        }
        g.set_has_video(st.has_video);
        g.set_playing(!st.paused && !st.ended);
        g.set_position(st.position as f32);
        g.set_duration(st.duration as f32);
        g.set_error(st.error);
        let total = if st.duration > 0.0 { format!(" / {}", clock(st.duration)) } else { String::new() };
        g.set_time(format!("{}{total}", clock(st.position)).into());
    });
}

pub fn stop(s: &Shared) {
    TIMER.with(|t| t.stop());
    PLAYER.with(|x| x.borrow_mut().take());
    with_ui(s, |g| {
        g.set_target(0);
        g.set_path("".into());
        g.set_playing(false);
        g.set_has_frame(false);
        g.set_frame(Default::default());
        g.set_has_video(false);
        g.set_position(0.0);
        g.set_duration(0.0);
    });
}

/// Zatrzymuje tylko odtwarzanie danego miejsca (np. panel zmienił plik, a przeglądarka gra dalej).
pub fn stop_target(s: &Shared, target: i32) {
    let mut cur = 0;
    with_ui(s, |g| cur = g.get_target());
    if cur == target {
        stop(s);
    }
}

pub fn toggle(s: &Shared) {
    PLAYER.with(|x| {
        if let Some(p) = x.borrow().as_ref() {
            p.toggle();
        }
    });
    tick(s);
}

pub fn seek(s: &Shared, t: f64) {
    PLAYER.with(|x| {
        if let Some(p) = x.borrow().as_ref() {
            p.seek(t);
        }
    });
    tick(s);
}

pub fn set_volume(s: &Shared, v: f64) {
    VOLUME.with(|x| x.set(v));
    MUTED.with(|m| m.set(false));
    PLAYER.with(|x| {
        if let Some(p) = x.borrow().as_ref() {
            p.set_volume(v);
            p.set_muted(false);
        }
    });
    with_ui(s, |g| {
        g.set_volume(v as f32);
        g.set_muted(false);
    });
}

pub fn toggle_mute(s: &Shared) {
    let m = !MUTED.with(|m| m.get());
    MUTED.with(|x| x.set(m));
    PLAYER.with(|x| {
        if let Some(p) = x.borrow().as_ref() {
            p.set_muted(m);
        }
    });
    with_ui(s, |g| g.set_muted(m));
}

pub fn bind(ui: &crate::MainWindow, st: &Shared) {
    let g = ui.global::<PlayerState>();
    let s = st.clone();
    g.on_start(move |t, p| start(&s, t, &p));
    let s = st.clone();
    g.on_toggle(move || toggle(&s));
    let s = st.clone();
    g.on_seek(move |t| seek(&s, t as f64));
    let s = st.clone();
    g.on_set_volume(move |v| set_volume(&s, v as f64));
    let s = st.clone();
    g.on_toggle_mute(move || toggle_mute(&s));
    let s = st.clone();
    g.on_stop(move || stop(&s));
}
