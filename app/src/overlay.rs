//! Nakładka z wydajnością (Alt+R albo Ustawienia → Nakładka): FPS aktywnego programu, czas
//! klatki z wykresem, procesor, karta graficzna (obciążenie, temperatura), RAM i VRAM.
//! Okno przezroczyste, zawsze na wierzchu, nie łapie myszy; stoi w wybranym rogu monitora
//! z aktywnym programem. FPS – core::fpsmon (ETW), karta – core::gpumon.

use std::cell::RefCell;
use std::time::Duration;

use powerdesk_core::fpsmon::{self, State};
use powerdesk_core::taskwin;
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, VecModel};

use crate::{fmt, OverlayWindow};

/// Odstęp od krawędzi monitora (piksele logiczne).
const MARGIN: f32 = 16.0;
/// Słupków na wykresie czasu klatek.
const HIST: usize = 90;

thread_local! {
    static OV: RefCell<Option<OverlayWindow>> = const { RefCell::new(None) };
    static TIMER: slint::Timer = slint::Timer::default();
}

fn prefs<R>(f: impl FnOnce(&crate::model::Prefs) -> R) -> Option<R> {
    let mut out = None;
    crate::with_state(|s| out = Some(f(&s.borrow().prefs)));
    out
}

/// Okno nakładki – pasek, widok zadań i lista okien go pomijają (to nie jest okno programu).
pub fn hwnd() -> Option<isize> {
    OV.with(|o| o.borrow().as_ref().map(|w| crate::window_hwnd(w.window())))
}

pub fn is_open() -> bool {
    OV.with(|o| o.borrow().is_some())
}

/// Alt+R i przełącznik w Ustawieniach – zapamiętane, wraca po ponownym uruchomieniu.
pub fn toggle() {
    let on = !is_open();
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        b.prefs.overlay_on = on;
        b.prefs.save();
    });
    if on {
        open();
    } else {
        close();
    }
    crate::with_state(crate::settings::render_overlay);
}

/// Przy starcie programu: nakładka, jeśli była włączona; skrót Alt+R zawsze.
pub fn init() {
    powerdesk_core::hotkey::start(|id| {
        if id == powerdesk_core::hotkey::OVERLAY {
            let _ = slint::invoke_from_event_loop(toggle);
        }
    });
    if prefs(|p| p.overlay_on).unwrap_or(false) {
        open();
    }
}

pub fn open() {
    if is_open() {
        return;
    }
    let Ok(w) = OverlayWindow::new() else { return };
    fpsmon::start();
    powerdesk_core::gpumon::start();
    let _ = w.show();
    // Kliknięcia przechodzą do programu pod spodem; bez przycisku na pasku i bez fokusu.
    w.window().with_winit_window(|win| {
        let _ = win.set_cursor_hittest(false);
    });
    let hwnd = crate::window_hwnd(w.window());
    powerdesk_core::desktop::hide_from_taskbar(hwnd);
    powerdesk_core::system::see_through(hwnd);
    OV.with(|o| *o.borrow_mut() = Some(w));
    tick();
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_millis(500), tick));
}

pub fn close() {
    TIMER.with(|t| t.stop());
    if let Some(w) = OV.with(|o| o.borrow_mut().take()) {
        let _ = w.hide();
    }
    fpsmon::stop();
}

/// Odświeżenie danych i miejsca (aktywny program mógł przejść na inny monitor).
fn tick() {
    OV.with(|o| {
        let o = o.borrow();
        let Some(w) = o.as_ref() else { return };
        let (corner, full) = prefs(|p| (p.overlay_corner, p.overlay_full)).unwrap_or((1, true));
        w.set_full(full);
        let fg = taskwin::foreground_app();
        // Nakładka nad własnymi oknami PowerDesk (pulpit, pasek) nie ma czego mierzyć.
        let own = std::process::id();
        let target = fg.as_ref().filter(|(pid, _, _)| *pid != own);
        let st = fpsmon::state();
        w.set_note(match &st {
            State::NoAccess => crate::tr("FPS: brak uprawnień – Ustawienia → Nakładka → „Zezwól na pomiar FPS”").into(),
            State::Error(e) => powerdesk_core::trf!("FPS niedostępne ({e})").into(),
            _ => "".into(),
        });
        match target.and_then(|(pid, _, _)| fpsmon::stats(*pid, HIST)) {
            Some(s) if s.fps > 0.0 => {
                w.set_fps(format!("{:.0}", s.fps).into());
                w.set_fps_level(if s.fps >= 55.0 { 1 } else if s.fps >= 30.0 { 2 } else { 3 });
                w.set_frame(powerdesk_core::trf!("{:.1} ms · maks. {:.0}", s.avg_ms, s.max_ms).into());
                let top = s.times_ms.iter().copied().fold(1.0f32, f32::max).max(s.avg_ms as f32 * 2.0);
                let mut hist: Vec<f32> = vec![0.0; HIST.saturating_sub(s.times_ms.len())];
                hist.extend(s.times_ms.iter().map(|&t| (t / top).min(1.0)));
                w.set_hist(ModelRc::new(VecModel::from(hist)));
            }
            _ => {
                w.set_fps("".into());
                w.set_fps_level(0);
                w.set_frame("".into());
                w.set_hist(ModelRc::new(VecModel::from(Vec::<f32>::new())));
            }
        }
        w.set_app(target.map(|(_, n, _)| n.clone()).unwrap_or_default().into());
        let info = powerdesk_core::sysmon::info();
        w.set_cpu(format!("{:.0}%", info.cpu_usage).into());
        w.set_ram(format!("{} / {}", fmt::size(info.mem_used), fmt::size(info.mem_total)).into());
        let g = powerdesk_core::gpumon::snapshot();
        w.set_gpu(format!("{:.0}%", g.usage).into());
        w.set_gpu_temp(g.temp.map(|t| format!("{t}°C")).unwrap_or_default().into());
        w.set_vram(if g.vram_total > 0 { format!("{} / {}", fmt::size(g.vram_used), fmt::size(g.vram_total)) } else { fmt::size(g.vram_used) }.into());
        // Róg monitora aktywnego programu: 0 – lewy górny, 1 – prawy górny, 2 – lewy dolny, 3 – prawy dolny.
        let (mx, my, mw, mh) = fg.map(|f| f.2).unwrap_or((0, 0, 1920, 1080));
        let scale = w.window().scale_factor();
        let size = w.window().size();
        let m = (MARGIN * scale) as i32;
        let x = if corner % 2 == 0 { mx + m } else { mx + mw - size.width as i32 - m };
        let y = if corner < 2 { my + m } else { my + mh - size.height as i32 - m };
        w.window().set_position(PhysicalPosition::new(x, y));
        taskwin::keep_topmost(crate::window_hwnd(w.window()));
    });
}
