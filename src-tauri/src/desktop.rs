//! Pulpit PowerDesk: jedno okno na każdy monitor.
//!
//! Tryby:
//! - `Replace` – okna wpięte w warstwę tapety Eksploratora (WorkerW), systemowe ikony ukryte.
//!   Pasek zadań, Alt+Tab, Win+D i reszta powłoki działają dalej, bo explorer.exe zostaje.
//! - `Test` – zwykłe okna na obszarze roboczym każdego monitora. Systemowy pulpit nietknięty,
//!   wystarczy zamknąć okno (albo Esc), żeby wrócić.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper, DWPOS_CENTER, DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH, DWPOS_TILE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetWindowLongPtrW, GetWindowRect, IsWindow, IsWindowVisible, SendMessageTimeoutW, SetParent, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, GWL_STYLE, HWND_BOTTOM, SMTO_NORMAL, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, WS_CAPTION, WS_CHILD, WS_POPUP, WS_THICKFRAME,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Test,
    Replace,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s {
            "test" => Mode::Test,
            "replace" => Mode::Replace,
            _ => Mode::Off,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Test => "test",
            Mode::Replace => "replace",
        }
    }
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Screen {
    /// Etykieta okna Tauri (`desk-DISPLAY1`).
    pub label: String,
    pub device: String,
    pub primary: bool,
    /// Prostokąt monitora i obszaru roboczego w pikselach fizycznych.
    pub rect: [i32; 4],
    pub work: [i32; 4],
    pub scale: f64,
    pub index: usize,
    pub wallpaper: String,
    /// fill | fit | stretch | tile | center | span
    pub fit: String,
    pub mode: String,
}

static MODE: Lazy<Mutex<Mode>> = Lazy::new(|| Mutex::new(Mode::Off));
static SCREENS: Lazy<Mutex<Vec<Screen>>> = Lazy::new(|| Mutex::new(Vec::new()));
/// Okno WorkerW, do którego wpięliśmy pulpity (tryb Replace).
static PARENT: AtomicIsize = AtomicIsize::new(0);
/// Ukryty przez nas SHELLDLL_DefView – do przywrócenia.
static DEFVIEW: AtomicIsize = AtomicIsize::new(0);
static WATCHING: AtomicBool = AtomicBool::new(false);

pub fn mode() -> Mode {
    *MODE.lock()
}

pub fn screen_for(label: &str) -> Option<Screen> {
    SCREENS.lock().iter().find(|s| s.label == label).cloned()
}

/// Włącza, przełącza albo wyłącza pulpit. Bezpieczne do wielokrotnego wołania.
pub fn set(app: &AppHandle, m: Mode) {
    let prev = std::mem::replace(&mut *MODE.lock(), m);
    if prev == Mode::Replace && m != Mode::Replace {
        // Z trybu wpiętego wychodzimy przez zamknięcie okien – odpięte okna dziecięce źle się przenoszą.
        close_all(app);
        restore();
    }
    if m == Mode::Off {
        close_all(app);
        restore();
    } else {
        if prev == Mode::Test && m == Mode::Replace {
            close_all(app);
        }
        sync(app, true);
        start_watch(app.clone());
    }
    let _ = app.emit("desktop-mode", m.name());
}

fn close_all(app: &AppHandle) {
    for (label, w) in app.webview_windows() {
        if label.starts_with("desk-") {
            let _ = w.destroy();
        }
    }
    SCREENS.lock().clear();
}

/// Przywraca systemowe ikony pulpitu. Wołane też przy wyjściu i panice.
pub fn restore() {
    let dv = DEFVIEW.swap(0, Ordering::SeqCst);
    unsafe {
        if dv != 0 && IsWindow(Some(HWND(dv as _))).as_bool() {
            let _ = ShowWindow(HWND(dv as _), SW_SHOW);
        } else if let Some(h) = find_defview() {
            // Po awarii poprzedniego uruchomienia uchwyt nie jest znany – szukamy od nowa.
            if !IsWindowVisible(h).as_bool() {
                let _ = ShowWindow(h, SW_SHOW);
            }
        }
    }
    PARENT.store(0, Ordering::SeqCst);
}

/// Dopasowuje okna do monitorów. `force` – także gdy układ się nie zmienił.
fn sync(app: &AppHandle, force: bool) {
    let m = mode();
    if m == Mode::Off {
        return;
    }
    let mut now = monitors();
    let walls = wallpapers();
    for s in &mut now {
        if let Some((p, fit)) = walls.iter().find(|(r, _, _)| *r == s.rect).map(|(_, p, f)| (p.clone(), f.clone())) {
            s.wallpaper = p;
            s.fit = fit;
        }
        s.mode = m.name().into();
    }
    let parent = if m == Mode::Replace { worker_w() } else { None };
    let parent_changed = parent.map(|h| h.0 as isize).unwrap_or(0) != PARENT.load(Ordering::SeqCst);
    let changed = *SCREENS.lock() != now;
    if !force && !changed && !parent_changed {
        return;
    }
    if parent_changed && m == Mode::Replace {
        // Eksplorator wystartował od nowa – stare okna są osierocone, tworzymy je ponownie.
        close_all(app);
    }
    let labels: Vec<String> = now.iter().map(|s| s.label.clone()).collect();
    for (label, w) in app.webview_windows() {
        if label.starts_with("desk-") && !labels.contains(&label) {
            let _ = w.destroy();
        }
    }
    *SCREENS.lock() = now.clone();
    if let Some(p) = parent {
        PARENT.store(p.0 as isize, Ordering::SeqCst);
        hide_defview();
    }
    for s in &now {
        let w = match app.get_webview_window(&s.label) {
            Some(w) => w,
            None => {
                let title = if m == Mode::Test { format!("PowerDesk – pulpit testowy ({})", s.device) } else { "PowerDesk – pulpit".into() };
                match WebviewWindowBuilder::new(app, &s.label, WebviewUrl::App("index.html#desktop".into()))
                    .title(title)
                    .decorations(false)
                    .resizable(false)
                    .shadow(false)
                    .skip_taskbar(m == Mode::Replace)
                    .visible(false)
                    .focused(false)
                    .background_color(tauri::window::Color(0, 0, 0, 255))
                    .disable_drag_drop_handler()
                    .build()
                {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("pulpit {}: {e}", s.label);
                        continue;
                    }
                }
            }
        };
        let r = if m == Mode::Test { s.work } else { s.rect };
        match (m, parent, w.hwnd()) {
            (Mode::Replace, Some(p), Ok(h)) => unsafe { attach(HWND(h.0), p, r) },
            _ => {
                let _ = w.set_position(PhysicalPosition::new(r[0], r[1]));
                let _ = w.set_size(PhysicalSize::new((r[2] - r[0]) as u32, (r[3] - r[1]) as u32));
            }
        }
    }
    let _ = app.emit("desktop-screens", ());
}

unsafe fn attach(h: HWND, parent: HWND, r: [i32; 4]) {
    let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
    let style = (style & !(WS_POPUP.0 | WS_CAPTION.0 | WS_THICKFRAME.0)) | WS_CHILD.0;
    SetWindowLongPtrW(h, GWL_STYLE, style as isize);
    let _ = SetParent(h, Some(parent));
    // Współrzędne dziecka są względne wobec WorkerW (rozpiętego na cały wirtualny ekran).
    let mut pr = RECT::default();
    let _ = GetWindowRect(parent, &mut pr);
    let _ = SetWindowPos(h, Some(HWND_BOTTOM), r[0] - pr.left, r[1] - pr.top, r[2] - r[0], r[3] - r[1], SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

fn start_watch(app: AppHandle) {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    // Podłączenie monitora, zmiana skali, nowa tapeta, restart Eksploratora – sprawdzamy co 2 s.
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        if mode() == Mode::Off {
            continue;
        }
        let a = app.clone();
        let _ = app.run_on_main_thread(move || sync(&a, false));
    });
}

// ---------- Monitory ----------

fn monitors() -> Vec<Screen> {
    unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let v = &mut *(data.0 as *mut Vec<HMONITOR>);
        v.push(h);
        BOOL(1)
    }
    let mut hs: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut hs as *mut _ as isize));
    }
    let mut out = Vec::new();
    for (i, h) in hs.into_iter().enumerate() {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !unsafe { GetMonitorInfoW(h, &mut mi as *mut _ as *mut MONITORINFO) }.as_bool() {
            continue;
        }
        let dev = String::from_utf16_lossy(&mi.szDevice).trim_end_matches('\0').to_string();
        let short: String = dev.trim_start_matches(r"\\.\").chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
        let r = mi.monitorInfo.rcMonitor;
        let wk = mi.monitorInfo.rcWork;
        out.push(Screen {
            label: format!("desk-{}", if short.is_empty() { i.to_string() } else { short }),
            device: dev,
            primary: mi.monitorInfo.dwFlags & 1 != 0,
            rect: [r.left, r.top, r.right, r.bottom],
            work: [wk.left, wk.top, wk.right, wk.bottom],
            scale: dx as f64 / 96.0,
            index: i,
            wallpaper: String::new(),
            fit: "fill".into(),
            mode: String::new(),
        });
    }
    out.sort_by_key(|s| (!s.primary, s.rect[0], s.rect[1]));
    for (i, s) in out.iter_mut().enumerate() {
        s.index = i;
    }
    out
}

/// Tapety Windows dla każdego monitora: (prostokąt, ścieżka, dopasowanie).
fn wallpapers() -> Vec<([i32; 4], String, String)> {
    // COM w osobnym wątku STA – wątek główny Tauri ma już własny model.
    std::thread::spawn(|| unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(dw) = CoCreateInstance::<_, IDesktopWallpaper>(&DesktopWallpaper, None, CLSCTX_ALL) else { return Vec::new() };
        let fit = match dw.GetPosition() {
            Ok(p) if p == DWPOS_FIT => "fit",
            Ok(p) if p == DWPOS_STRETCH => "stretch",
            Ok(p) if p == DWPOS_TILE => "tile",
            Ok(p) if p == DWPOS_CENTER => "center",
            Ok(p) if p == DWPOS_SPAN => "span",
            Ok(p) if p == DWPOS_FILL => "fill",
            _ => "fill",
        };
        let n = dw.GetMonitorDevicePathCount().unwrap_or(0);
        let mut v = Vec::new();
        for i in 0..n {
            let Ok(id) = dw.GetMonitorDevicePathAt(i) else { continue };
            let pid = PCWSTR(id.0);
            if let (Ok(r), Ok(p)) = (dw.GetMonitorRECT(pid), dw.GetWallpaper(pid)) {
                v.push(([r.left, r.top, r.right, r.bottom], p.to_string().unwrap_or_default(), fit.to_string()));
                CoTaskMemFree(Some(p.0 as _));
            }
            CoTaskMemFree(Some(id.0 as _));
        }
        v
    })
    .join()
    .unwrap_or_default()
}

// ---------- Warstwa pulpitu Eksploratora ----------

/// WorkerW za ikonami pulpitu – ta sama technika co w Wallpaper Engine / Lively.
fn worker_w() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
        // 0x052C każe Progmanowi utworzyć WorkerW pod ikonami.
        let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0xD), LPARAM(1), SMTO_NORMAL, 1000, None);
        let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0), LPARAM(0), SMTO_NORMAL, 1000, None);
        // Windows 11 24H2+: WorkerW jest dzieckiem Progmana.
        if let Ok(w) = FindWindowExW(Some(progman), None, w!("WorkerW"), PCWSTR::null()) {
            return Some(w);
        }
        // Starsze: WorkerW to rodzeństwo okna z SHELLDLL_DefView.
        unsafe extern "system" fn cb(h: HWND, data: LPARAM) -> BOOL {
            if FindWindowExW(Some(h), None, w!("SHELLDLL_DefView"), PCWSTR::null()).is_ok() {
                if let Ok(w) = FindWindowExW(None, Some(h), w!("WorkerW"), PCWSTR::null()) {
                    *(data.0 as *mut isize) = w.0 as isize;
                    return BOOL(0);
                }
            }
            BOOL(1)
        }
        let mut found: isize = 0;
        let _ = EnumWindows(Some(cb), LPARAM(&mut found as *mut _ as isize));
        if found != 0 {
            return Some(HWND(found as _));
        }
        Some(progman)
    }
}

fn find_defview() -> Option<HWND> {
    unsafe {
        if let Ok(p) = FindWindowW(w!("Progman"), PCWSTR::null()) {
            if let Ok(d) = FindWindowExW(Some(p), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
                return Some(d);
            }
        }
        unsafe extern "system" fn cb(h: HWND, data: LPARAM) -> BOOL {
            if let Ok(d) = FindWindowExW(Some(h), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
                *(data.0 as *mut isize) = d.0 as isize;
                return BOOL(0);
            }
            BOOL(1)
        }
        let mut found: isize = 0;
        let _ = EnumWindows(Some(cb), LPARAM(&mut found as *mut _ as isize));
        (found != 0).then(|| HWND(found as _))
    }
}

/// Systemowe ikony chowamy, żeby nie zasłaniały naszego pulpitu i nie przejmowały myszy.
fn hide_defview() {
    if let Some(d) = find_defview() {
        unsafe {
            if IsWindowVisible(d).as_bool() {
                let _ = ShowWindow(d, SW_HIDE);
            }
        }
        DEFVIEW.store(d.0 as isize, Ordering::SeqCst);
    }
}
