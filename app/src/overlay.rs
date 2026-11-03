//! Nakładka z wydajnością (Alt+R albo Ustawienia → Nakładka): FPS aktywnego programu, czas
//! klatki z wykresem, procesor, karta graficzna (obciążenie, temperatura), RAM i VRAM.
//! Okno przezroczyste, zawsze na wierzchu, nie łapie myszy; stoi w wybranym rogu monitora
//! z aktywnym programem albo tam, gdzie ją przeciągnięto (Ctrl i kursor nad nakładką włączają
//! przesuwanie). FPS – core::fpsmon (ETW), karta – core::gpumon.

use std::cell::RefCell;
use std::collections::VecDeque;
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
/// Klatek do policzenia 1% low.
const LOW_FRAMES: usize = 600;
/// Pomiarów na wykresie FPS.
const GRAPH: usize = 60;

thread_local! {
    static OV: RefCell<Option<OverlayWindow>> = const { RefCell::new(None) };
    static TIMER: slint::Timer = slint::Timer::default();
    /// Sprawdzanie Ctrl i kursora nad nakładką.
    static POLL: slint::Timer = slint::Timer::default();
    /// Włączone przesuwanie (nakładka łapie mysz).
    static MOVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Trwające przeciąganie: kursor i okno na początku (piksele ekranu).
    static DRAG: std::cell::Cell<Option<((i32, i32), (i32, i32))>> = const { std::cell::Cell::new(None) };
    /// Historia FPS aktywnego programu (nowy program – od nowa).
    static HISTORY: RefCell<(String, VecDeque<f32>)> = const { RefCell::new((String::new(), VecDeque::new())) };
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

/// Krycie tła w procentach (w ustawieniach 0 – domyślne).
pub fn opacity(p: &crate::model::Prefs) -> u8 {
    if p.overlay_opacity == 0 { 80 } else { p.overlay_opacity }
}

/// Odświeżanie w milisekundach (w ustawieniach 0 – domyślne).
pub fn refresh_ms(p: &crate::model::Prefs) -> u32 {
    if p.overlay_refresh == 0 { 500 } else { p.overlay_refresh }
}

/// Nowe odświeżanie z ustawień.
pub fn restart_timer() {
    if is_open() {
        let ms = prefs(refresh_ms).unwrap_or(500);
        TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_millis(ms as u64), tick));
    }
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
    w.on_drag_start(drag_start);
    w.on_drag_move(drag_move);
    w.on_drag_end(drag_end);
    let _ = w.show();
    // Kliknięcia przechodzą do programu pod spodem; bez przycisku na pasku i bez fokusu.
    w.window().with_winit_window(|win| {
        let _ = win.set_cursor_hittest(false);
    });
    let hwnd = crate::window_hwnd(w.window());
    powerdesk_core::desktop::hide_from_taskbar(hwnd);
    powerdesk_core::system::strip_caption(hwnd);
    powerdesk_core::system::see_through(hwnd);
    // Przeciąganie nakładki nie zabiera fokusu grze.
    taskwin::no_activate(hwnd);
    MOVING.with(|m| m.set(false));
    DRAG.with(|d| d.set(None));
    OV.with(|o| *o.borrow_mut() = Some(w));
    tick();
    restart_timer();
    POLL.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_millis(80), poll));
}

pub fn close() {
    TIMER.with(|t| t.stop());
    POLL.with(|t| t.stop());
    if let Some(w) = OV.with(|o| o.borrow_mut().take()) {
        let _ = w.hide();
    }
    fpsmon::stop();
}

/// Ctrl i kursor nad nakładką: łapie mysz i da się przeciągać; poza tym kliknięcia przechodzą dalej.
fn poll() {
    OV.with(|o| {
        let o = o.borrow();
        let Some(w) = o.as_ref() else { return };
        let (cx, cy) = taskwin::cursor();
        let (p, size) = (w.window().position(), w.window().size());
        let over = cx >= p.x && cy >= p.y && cx < p.x + size.width as i32 && cy < p.y + size.height as i32;
        let on = DRAG.with(|d| d.get()).is_some() || (over && powerdesk_core::system::ctrl_held());
        if on != MOVING.with(|m| m.replace(on)) {
            w.set_moving(on);
            w.window().with_winit_window(|win| {
                let _ = win.set_cursor_hittest(on);
            });
        }
    });
}

fn drag_start() {
    OV.with(|o| {
        if let Some(w) = o.borrow().as_ref() {
            let p = w.window().position();
            DRAG.with(|d| d.set(Some((taskwin::cursor(), (p.x, p.y)))));
        }
    });
}

/// Liczone z kursora na ekranie – okno jedzie razem z myszą, więc współrzędne w oknie by się znosiły.
fn drag_move() {
    let Some(((cx, cy), (wx, wy))) = DRAG.with(|d| d.get()) else { return };
    let (x, y) = taskwin::cursor();
    OV.with(|o| {
        if let Some(w) = o.borrow().as_ref() {
            w.window().set_position(PhysicalPosition::new(wx + x - cx, wy + y - cy));
        }
    });
}

fn drag_end() {
    if DRAG.with(|d| d.take()).is_none() {
        return;
    }
    let at = OV.with(|o| o.borrow().as_ref().map(|w| w.window().position()));
    let Some(at) = at else { return };
    crate::with_state(|s| {
        {
            let mut b = s.borrow_mut();
            b.prefs.overlay_pos = Some((at.x, at.y));
            b.prefs.save();
        }
        crate::settings::render_overlay(s);
    });
}

/// Polecenia ścieżki wykresu FPS (pole 100×40): linia i wypełnienie pod nią. Nowe pomiary z prawej.
fn graph(values: &VecDeque<f32>) -> (String, String) {
    if values.len() < 2 {
        return (String::new(), String::new());
    }
    let top = values.iter().copied().fold(60.0f32, f32::max) * 1.1;
    let step = 100.0 / (GRAPH - 1) as f32;
    let first = 100.0 - (values.len() - 1) as f32 * step;
    let mut line = String::new();
    for (i, v) in values.iter().enumerate() {
        let cmd = if i == 0 { 'M' } else { 'L' };
        line.push_str(&format!("{cmd} {:.2} {:.2} ", first + i as f32 * step, 40.0 - v / top * 38.0));
    }
    let area = format!("{line}L 100 40 L {first:.2} 40 Z");
    (line, area)
}

/// Odświeżenie danych i miejsca (aktywny program mógł przejść na inny monitor).
fn tick() {
    OV.with(|o| {
        let o = o.borrow();
        let Some(w) = o.as_ref() else { return };
        let (corner, full, pos, alpha, no_app, no_graph) = prefs(|p| (p.overlay_corner, p.overlay_full, p.overlay_pos, opacity(p), p.overlay_no_app, p.overlay_no_graph)).unwrap_or((1, true, None, 80, false, false));
        w.set_full(full);
        w.set_bg_alpha(alpha as f32 / 100.0);
        w.set_show_app(!no_app);
        w.set_show_graph(!no_graph);
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
        let name = target.map(|(_, n, _)| n.clone()).unwrap_or_default();
        match target.and_then(|(pid, _, _)| fpsmon::stats(*pid, LOW_FRAMES)) {
            Some(s) if s.fps > 0.0 => {
                w.set_fps(format!("{:.0}", s.fps).into());
                w.set_fps_level(if s.fps >= 55.0 { 1 } else if s.fps >= 30.0 { 2 } else { 3 });
                w.set_frame(powerdesk_core::trf!("{:.1} ms · maks. {:.0}", s.avg_ms, s.max_ms).into());
                // 1% low: średnia z najdłuższego 1% klatek, przeliczona na FPS.
                let mut sorted = s.times_ms.clone();
                sorted.sort_by(|a, b| b.total_cmp(a));
                let n = (sorted.len() / 100).max(1);
                let worst = sorted.iter().take(n).sum::<f32>() / n as f32;
                w.set_fps_low(if worst > 0.0 { format!("{:.0}", 1000.0 / worst) } else { String::new() }.into());
                HISTORY.with(|h| {
                    let mut h = h.borrow_mut();
                    if h.0 != name {
                        h.0 = name.clone();
                        h.1.clear();
                    }
                    h.1.push_back(s.fps as f32);
                    while h.1.len() > GRAPH {
                        h.1.pop_front();
                    }
                    let avg = h.1.iter().sum::<f32>() / h.1.len() as f32;
                    let min = h.1.iter().copied().fold(f32::MAX, f32::min);
                    w.set_fps_avg(format!("{avg:.0}").into());
                    w.set_fps_min(format!("{min:.0}").into());
                    let (line, area) = graph(&h.1);
                    w.set_graph_line(line.into());
                    w.set_graph_area(area.into());
                });
                let tail = &s.times_ms[s.times_ms.len().saturating_sub(HIST)..];
                let top = tail.iter().copied().fold(1.0f32, f32::max).max(s.avg_ms as f32 * 2.0);
                let mut hist: Vec<f32> = vec![0.0; HIST.saturating_sub(tail.len())];
                hist.extend(tail.iter().map(|&t| (t / top).min(1.0)));
                w.set_hist(ModelRc::new(VecModel::from(hist)));
            }
            _ => {
                w.set_fps("".into());
                w.set_fps_level(0);
                w.set_frame("".into());
                w.set_fps_avg("".into());
                w.set_fps_low("".into());
                w.set_fps_min("".into());
                w.set_graph_line("".into());
                w.set_graph_area("".into());
                HISTORY.with(|h| h.borrow_mut().1.clear());
                w.set_hist(ModelRc::new(VecModel::from(Vec::<f32>::new())));
            }
        }
        w.set_app(name.into());
        let info = powerdesk_core::sysmon::info();
        w.set_cpu(format!("{:.0}%", info.cpu_usage).into());
        w.set_cpu_level(info.cpu_usage / 100.0);
        w.set_ram_level(if info.mem_total > 0 { info.mem_used as f32 / info.mem_total as f32 } else { 0.0 });
        w.set_ram(format!("{} / {}", fmt::size(info.mem_used), fmt::size(info.mem_total)).into());
        let g = powerdesk_core::gpumon::snapshot();
        w.set_gpu(format!("{:.0}%", g.usage).into());
        w.set_gpu_level(g.usage / 100.0);
        w.set_vram_level(if g.vram_total > 0 { g.vram_used as f32 / g.vram_total as f32 } else { 0.0 });
        w.set_gpu_temp(g.temp.map(|t| format!("{t}°C")).unwrap_or_default().into());
        w.set_vram(if g.vram_total > 0 { format!("{} / {}", fmt::size(g.vram_used), fmt::size(g.vram_total)) } else { fmt::size(g.vram_used) }.into());
        // Róg monitora aktywnego programu: 0 – lewy górny, 1 – prawy górny, 2 – lewy dolny, 3 – prawy dolny.
        let (mx, my, mw, mh) = fg.map(|f| f.2).unwrap_or((0, 0, 1920, 1080));
        let scale = w.window().scale_factor();
        let size = w.window().size();
        let m = (MARGIN * scale) as i32;
        let x = if corner % 2 == 0 { mx + m } else { mx + mw - size.width as i32 - m };
        let y = if corner < 2 { my + m } else { my + mh - size.height as i32 - m };
        // Przesunięta myszą zostaje na swoim miejscu (o ile ten monitor nadal jest); w trakcie przesuwania nie ruszamy jej.
        let (x, y) = pos.filter(|&(px, py)| taskwin::on_screen(px, py)).unwrap_or((x, y));
        if !MOVING.with(|m| m.get()) {
            w.window().set_position(PhysicalPosition::new(x, y));
        }
        taskwin::keep_topmost(crate::window_hwnd(w.window()));
    });
}
