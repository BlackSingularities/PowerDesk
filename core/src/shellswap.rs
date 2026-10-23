//! Podmiana pulpitu i paska zadań Windows na PowerDesk oraz przywracanie.
//!
//! Pasek: systemowe paski (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd`) są ukryte i przestawione
//! na autoukrywanie (zwalniają miejsce), a paski PowerDesk rejestrują się jako AppBar – okna
//! zmaksymalizowane kończą się nad nimi. Pulpit: okna pulpitu PowerDesk są dziećmi `Progman`
//! (Win+D ich nie chowa), widok ikon Windows (`SHELLDLL_DefView`) jest ukryty.
//!
//! Stan sprzed podmiany leży w pliku – przywraca go zamknięcie programu, strażnik po awarii
//! (`--shell-guard <pid>`) i deinstalator (`--remove-integration`).

use std::path::PathBuf;

use windows::core::{w, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABE_BOTTOM, ABM_GETSTATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_SETSTATE, ABS_ALWAYSONTOP, ABS_AUTOHIDE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetWindowRect, IsWindow, IsWindowVisible, SetParent, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNA,
};

/// Flaga strażnika: czeka na koniec procesu PowerDesk i przywraca Windows, jeśli trzeba.
pub const GUARD_FLAG: &str = "--shell-guard";

fn h(v: isize) -> HWND {
    HWND(v as *mut _)
}

fn state_file() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join("PowerDesk").join("shell-swap.txt")
}

/// Ukryte przez nas okna WorkerW (uchwyty, po jednym w wierszu) – przywracamy dokładnie te.
fn workers_file() -> PathBuf {
    state_file().with_file_name("shell-swap-workerw.txt")
}

/// Okna WorkerW najwyższego poziomu: z widokiem ikon i warstwa tapety – leżą nad Progman.
fn workers() -> Vec<HWND> {
    unsafe extern "system" fn each(w: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<HWND>);
        if class_of(w) == "WorkerW" {
            out.push(w);
        }
        BOOL(1)
    }
    let mut out: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

unsafe fn class_of(w: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = GetClassNameW(w, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Paski Windows: główny i paski na pozostałych monitorach.
fn system_bars() -> Vec<HWND> {
    unsafe extern "system" fn each(w: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<HWND>);
        let c = class_of(w);
        if c == "Shell_TrayWnd" || c == "Shell_SecondaryTrayWnd" {
            out.push(w);
        }
        BOOL(1)
    }
    let mut out: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Widok ikon pulpitu Windows – w `Progman` albo w `WorkerW` (po zmianie tapety z animacją).
fn desktop_views() -> Vec<HWND> {
    unsafe extern "system" fn each(w: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<HWND>);
        let c = class_of(w);
        if c == "Progman" || c == "WorkerW" {
            if let Ok(v) = FindWindowExW(Some(w), None, w!("SHELLDLL_DefView"), None) {
                out.push(v);
            }
        }
        BOOL(1)
    }
    let mut out: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

fn taskbar_state() -> u32 {
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    if let Ok(t) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
        d.hWnd = t;
    }
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) as u32 }
}

fn set_taskbar_state(state: u32) {
    let Ok(t) = (unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }) else { return };
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: t, lParam: LPARAM(state as isize), ..Default::default() };
    unsafe {
        SHAppBarMessage(ABM_SETSTATE, &mut d);
    }
}

/// Ukrywa pasek Windows. Stan zapisujemy tylko za pierwszym razem – ponowne wywołanie
/// (co sekundę, po restarcie Eksploratora) nie nadpisze go stanem już zmienionym.
pub fn hide_taskbar() {
    let file = state_file();
    if !file.exists() {
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
        let _ = std::fs::write(&file, taskbar_state().to_string());
    }
    if taskbar_state() & ABS_AUTOHIDE == 0 {
        set_taskbar_state(ABS_AUTOHIDE | ABS_ALWAYSONTOP);
    }
    for b in system_bars() {
        unsafe {
            if IsWindowVisible(b).as_bool() {
                let _ = ShowWindow(b, SW_HIDE);
            }
        }
    }
}

/// Ukrywa ikony pulpitu Windows i okna WorkerW nad Progman (inaczej przykryłyby pulpit PowerDesk).
pub fn hide_desktop_icons() {
    for v in desktop_views() {
        unsafe {
            if IsWindowVisible(v).as_bool() {
                let _ = ShowWindow(v, SW_HIDE);
            }
        }
    }
    let mut hidden = Vec::new();
    for w in workers() {
        unsafe {
            if IsWindowVisible(w).as_bool() {
                let _ = ShowWindow(w, SW_HIDE);
                hidden.push(w.0 as isize);
            }
        }
    }
    if !hidden.is_empty() {
        let file = workers_file();
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
        let mut text = std::fs::read_to_string(&file).unwrap_or_default();
        for h in hidden {
            text.push_str(&format!("{h}\n"));
        }
        let _ = std::fs::write(file, text);
    }
}

/// Czy coś jest podmienione (plik stanu istnieje).
pub fn swapped() -> bool {
    state_file().exists() || workers_file().exists()
}

/// Przywraca pasek i ikony Windows do stanu sprzed podmiany. Bezpieczne do wielokrotnego
/// wywołania i bez pliku stanu (wtedy tylko pokazuje ukryte elementy).
pub fn restore() {
    let file = state_file();
    let prev = std::fs::read_to_string(&file).ok().and_then(|s| s.trim().parse::<u32>().ok());
    if let Some(state) = prev {
        set_taskbar_state(state);
    }
    for b in system_bars() {
        unsafe {
            let _ = ShowWindow(b, SW_SHOWNA);
        }
    }
    let wf = workers_file();
    for line in std::fs::read_to_string(&wf).unwrap_or_default().lines() {
        if let Ok(v) = line.trim().parse::<isize>() {
            unsafe {
                if IsWindow(Some(h(v))).as_bool() {
                    let _ = ShowWindow(h(v), SW_SHOWNA);
                }
            }
        }
    }
    for v in desktop_views() {
        unsafe {
            let _ = ShowWindow(v, SW_SHOWNA);
        }
    }
    let _ = std::fs::remove_file(wf);
    let _ = std::fs::remove_file(file);
}

/// Okno pulpitu PowerDesk jako dziecko `Progman`, na monitorze (x, y, w, h – piksele ekranu).
/// Zwraca false, gdy nie ma `Progman` (Eksplorator nie działa).
pub fn attach_desktop(hwnd: isize, x: i32, y: i32, width: i32, height: i32) -> bool {
    unsafe {
        let Ok(progman) = FindWindowW(w!("Progman"), None) else { return false };
        if hwnd == 0 || SetParent(h(hwnd), Some(progman)).is_err() {
            return false;
        }
        // Współrzędne dziecka liczą się od lewego górnego rogu Progman (cały pulpit wirtualny).
        let mut r = RECT::default();
        let _ = GetWindowRect(progman, &mut r);
        let _ = SetWindowPos(h(hwnd), Some(HWND_TOP), x - r.left, y - r.top, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        true
    }
}

/// Czy okno nadal istnieje (po restarcie Eksploratora dzieci Progman giną razem z nim).
pub fn alive(hwnd: isize) -> bool {
    unsafe { IsWindow(Some(h(hwnd))).as_bool() }
}

/// Pasek PowerDesk jako AppBar na dole monitora – system odejmuje go od obszaru roboczego.
pub fn appbar_add(hwnd: isize, x: i32, y: i32, width: i32, height: i32) {
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: h(hwnd), uCallbackMessage: 0x8000 + 0x50, ..Default::default() };
    unsafe {
        SHAppBarMessage(ABM_NEW, &mut d);
        d.uEdge = ABE_BOTTOM;
        d.rc = RECT { left: x, top: y, right: x + width, bottom: y + height };
        SHAppBarMessage(ABM_QUERYPOS, &mut d);
        // System mógł przesunąć dół (np. inny AppBar) – wysokość zostaje nasza.
        d.rc.top = d.rc.bottom - height;
        SHAppBarMessage(ABM_SETPOS, &mut d);
    }
}

pub fn appbar_remove(hwnd: isize) {
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: h(hwnd), ..Default::default() };
    unsafe {
        SHAppBarMessage(ABM_REMOVE, &mut d);
    }
}

/// Uruchamia strażnika: osobny proces, który po końcu tego (także po awarii) przywraca Windows.
pub fn spawn_guard() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg(GUARD_FLAG).arg(std::process::id().to_string()).spawn();
    }
}

/// Strażnik: czeka na koniec procesu `pid`; jeśli zostawił podmianę (plik stanu) – przywraca.
pub fn guard(pid: u32) {
    unsafe {
        if let Ok(p) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(p, INFINITE);
            let _ = windows::Win32::Foundation::CloseHandle(p);
        }
    }
    if swapped() {
        restore();
    }
}
