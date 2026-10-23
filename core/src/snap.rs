//! Strefy okien (jak FancyZones): początek i koniec przeciągania okna dowolnego programu
//! (zdarzenia systemowe), obszar roboczy monitora pod kursorem i ustawienie okna w strefie
//! z poprawką na niewidoczne ramki Windows 10/11.

use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetMessageW, GetWindowRect, IsZoomed, SetWindowPos, ShowWindow, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE, WINEVENT_OUTOFCONTEXT,
};

/// Prostokąt w pikselach ekranu (lewo, góra, prawo, dół).
pub type Rect = RECT;

static HANDLER: OnceLock<Box<dyn Fn(bool, isize) + Send + Sync>> = OnceLock::new();

unsafe extern "system" fn on_event(_: HWINEVENTHOOK, ev: u32, w: HWND, obj: i32, _: i32, _: u32, _: u32) {
    if obj != 0 {
        return;
    }
    if let Some(f) = HANDLER.get() {
        f(ev == EVENT_SYSTEM_MOVESIZESTART, w.0 as isize);
    }
}

/// `on_move(true, okno)` – początek przeciągania, `(false, okno)` – koniec. Własny wątek.
pub fn start(on_move: impl Fn(bool, isize) + Send + Sync + 'static) {
    if HANDLER.set(Box::new(on_move)).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe {
        let _ = SetWinEventHook(EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND, None, Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    });
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Monitor pod kursorem: (cały monitor, obszar roboczy bez paska) – lewo, góra, prawo, dół.
pub fn monitor_at_cursor() -> (RECT, RECT) {
    let (x, y) = cursor();
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(m, &mut mi);
        (mi.rcMonitor, mi.rcWork)
    }
}

/// Okno dokładnie w prostokącie (piksele ekranu). Zmaksymalizowane – najpierw przywrócone;
/// niewidoczne ramki (cień) doliczone, żeby widoczna krawędź trafiła w strefę.
pub fn place(hwnd: isize, r: RECT) {
    let h = HWND(hwnd as *mut _);
    unsafe {
        if IsZoomed(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let mut win = RECT::default();
        let mut vis = RECT::default();
        let _ = GetWindowRect(h, &mut win);
        let ok = DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut vis as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_ok();
        let (l, t, rr, b) = if ok { (vis.left - win.left, vis.top - win.top, win.right - vis.right, win.bottom - vis.bottom) } else { (0, 0, 0, 0) };
        let _ = SetWindowPos(h, None, r.left - l, r.top - t, r.right - r.left + l + rr, r.bottom - r.top + t + b, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}
