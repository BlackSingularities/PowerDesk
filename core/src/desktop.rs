//! Dane pulpitu: monitory z tapetą (IDesktopWallpaper – każdy monitor osobno) i elementy
//! z Pulpitu użytkownika oraz publicznego (jak pokazuje je Eksplorator).

use windows::core::PWSTR;
use windows::Win32::Foundation::RECT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper, DWPOS_CENTER, DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH, DWPOS_TILE};

use crate::fsx::{self, Entry};

/// Jak tapeta wypełnia ekran (ustawienie z Personalizacji).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    /// Jeden obraz przez wszystkie monitory.
    Span,
}

#[derive(Clone, Debug)]
pub struct Monitor {
    /// Położenie i rozmiar w pikselach ekranu (współrzędne całego pulpitu wirtualnego).
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Ścieżka obrazu tapety; pusta – sam kolor.
    pub wallpaper: String,
    pub fit: Fit,
    /// Kolor tła (RGB), gdy tapety brak albo nie wypełnia ekranu.
    pub color: (u8, u8, u8),
    pub primary: bool,
}

unsafe fn take(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

/// Monitory z tapetami; osobny wątek, bo COM chce własnego mieszkania.
pub fn monitors() -> Vec<Monitor> {
    std::thread::spawn(|| unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(w) = CoCreateInstance::<_, IDesktopWallpaper>(&DesktopWallpaper, None, CLSCTX_ALL) else { return Vec::new() };
        let fit = match w.GetPosition() {
            Ok(p) if p == DWPOS_FIT => Fit::Fit,
            Ok(p) if p == DWPOS_STRETCH => Fit::Stretch,
            Ok(p) if p == DWPOS_CENTER => Fit::Center,
            Ok(p) if p == DWPOS_TILE => Fit::Tile,
            Ok(p) if p == DWPOS_SPAN => Fit::Span,
            Ok(p) if p == DWPOS_FILL => Fit::Fill,
            _ => Fit::Fill,
        };
        let c = w.GetBackgroundColor().map(|c| c.0).unwrap_or(0);
        let color = ((c & 0xff) as u8, ((c >> 8) & 0xff) as u8, ((c >> 16) & 0xff) as u8);
        let n = w.GetMonitorDevicePathCount().unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..n {
            let Ok(id) = w.GetMonitorDevicePathAt(i) else { continue };
            let r: RECT = match w.GetMonitorRECT(PWSTR(id.0)) {
                Ok(r) => r,
                Err(_) => {
                    take(id);
                    continue;
                }
            };
            // Monitor odłączony, a wciąż pamiętany przez system – ma pusty prostokąt.
            if r.right <= r.left || r.bottom <= r.top {
                take(id);
                continue;
            }
            let wallpaper = w.GetWallpaper(PWSTR(id.0)).map(|p| take(p)).unwrap_or_default();
            take(id);
            out.push(Monitor { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top, wallpaper, fit, color, primary: r.left == 0 && r.top == 0 });
        }
        out
    })
    .join()
    .unwrap_or_default()
}

/// Okno pulpitu bez przycisku na pasku zadań i bez miejsca w Alt+Tab (styl okna narzędziowego).
pub fn hide_from_taskbar(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW};
    if hwnd == 0 {
        return;
    }
    let h = HWND(hwnd as *mut _);
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize));
    }
}

/// Foldery pulpitu: użytkownika i publiczny (wspólny dla wszystkich kont).
pub fn folders() -> Vec<String> {
    let mut v: Vec<String> = dirs::desktop_dir().into_iter().map(|p| p.to_string_lossy().to_string()).collect();
    let public = std::env::var("PUBLIC").map(|p| format!(r"{p}\Desktop")).unwrap_or_else(|_| r"C:\Users\Public\Desktop".into());
    if std::path::Path::new(&public).is_dir() {
        v.push(public);
    }
    v
}

/// Ikony pulpitu: oba foldery razem, bez ukrytych i desktop.ini; foldery przed plikami, potem nazwa.
pub fn items() -> Vec<Entry> {
    let mut all: Vec<Entry> = folders().iter().filter_map(|d| fsx::list_dir(d).ok()).flatten().filter(|e| !e.hidden && !e.name.eq_ignore_ascii_case("desktop.ini")).collect();
    all.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    all
}
