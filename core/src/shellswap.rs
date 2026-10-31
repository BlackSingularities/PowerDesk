//! Podmiana pulpitu i paska zadań Windows na PowerDesk oraz przywracanie.
//!
//! Pasek: systemowe paski (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd`) są tylko ukryte – bez
//! autoukrywania Windows dalej rezerwuje dla nich dół każdego monitora, a tam stoi pasek
//! PowerDesk (ta sama wysokość). Nic nie przeliczamy, więc nic nie miga; hak zdarzeń chowa
//! pasek Windows od razu, gdy ten się pokaże (np. po restarcie Eksploratora).
//! Pulpit: okna pulpitu PowerDesk leżą w warstwie tapety (`WorkerW` – w Windows 11 24H2
//! dziecko `Progman`, wcześniej okno za ikonami), jak żywe tapety; Win+D ich nie chowa.
//! Widok ikon Windows (`SHELLDLL_DefView`) jest ukryty.
//!
//! Stan sprzed podmiany leży w pliku – przywraca go zamknięcie programu, strażnik po awarii
//! (`--shell-guard <pid>`) i deinstalator (`--remove-integration`).

use std::path::PathBuf;

use windows::core::{w, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetWindowRect, IsWindow, IsWindowVisible, SetParent, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNA,
};

/// Podmiana trwa – hak zdarzeń chowa wtedy wysuwający się pasek Windows.
static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static HOOKED: std::sync::Once = std::sync::Once::new();

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

/// Okno pulpitu PowerDesk na wierzch w swojej warstwie (nad obrazem tapety Windows).
pub fn raise(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE};
    unsafe {
        let _ = SetWindowPos(h(hwnd), Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

unsafe fn class_of(w: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = GetClassNameW(w, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Okno z innego procesu niż PowerDesk (nasz zasobnik też ma klasę `Shell_TrayWnd`).
unsafe fn foreign(w: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    GetWindowThreadProcessId(w, Some(&mut pid));
    pid != std::process::id()
}

/// Paski Windows: główny i paski na pozostałych monitorach (bez naszego okna zasobnika).
fn system_bars() -> Vec<HWND> {
    unsafe extern "system" fn each(w: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<HWND>);
        let c = class_of(w);
        if (c == "Shell_TrayWnd" || c == "Shell_SecondaryTrayWnd") && foreign(w) {
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

/// Główny pasek Eksploratora (`Shell_TrayWnd` spoza PowerDesk) – None, gdy Eksplorator nie działa.
pub fn explorer_tray() -> Option<isize> {
    system_bars().into_iter().find(|&w| unsafe { class_of(w) } == "Shell_TrayWnd").map(|w| w.0 as isize)
}

fn taskbar_state() -> u32 {
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    if let Some(t) = explorer_tray() {
        d.hWnd = h(t);
    }
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) as u32 }
}

fn set_taskbar_state(state: u32) {
    let Some(t) = explorer_tray().map(h) else { return };
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: t, lParam: LPARAM(state as isize), ..Default::default() };
    unsafe {
        SHAppBarMessage(ABM_SETSTATE, &mut d);
    }
}

/// Ukrywa pasek Windows. Stan zapisujemy tylko za pierwszym razem – ponowne wywołanie
/// (co sekundę, po restarcie Eksploratora) nie nadpisze go stanem już zmienionym.
pub fn hide_taskbar() {
    ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
    HOOKED.call_once(hook_taskbar);
    let file = state_file();
    if !file.exists() {
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
        // Znacznik podmiany; stan autoukrywania zostaje użytkownika (nie zmieniamy go).
        let _ = std::fs::write(&file, "keep");
    }
    for b in system_bars() {
        unsafe {
            if IsWindowVisible(b).as_bool() {
                let _ = ShowWindow(b, SW_HIDE);
            }
        }
    }
}

/// Ukrywa ikony pulpitu Windows. W starszym układzie widok ikon siedzi w osobnym `WorkerW`
/// na całym ekranie – chowamy też je, inaczej przechwytywałoby kliknięcia (zapis do przywrócenia).
pub fn hide_desktop_icons() {
    use windows::Win32::UI::WindowsAndMessaging::GetParent;
    let mut hidden = Vec::new();
    for v in desktop_views() {
        unsafe {
            if IsWindowVisible(v).as_bool() {
                let _ = ShowWindow(v, SW_HIDE);
            }
            if let Ok(p) = GetParent(v) {
                if class_of(p) == "WorkerW" && IsWindowVisible(p).as_bool() {
                    let _ = ShowWindow(p, SW_HIDE);
                    hidden.push(p.0 as isize);
                }
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
    ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    let file = state_file();
    // Starsze wersje przestawiały pasek na autoukrywanie i zapisywały stan sprzed – oddajemy go.
    let prev = std::fs::read_to_string(&file).ok().and_then(|s| s.trim().parse::<u32>().ok());
    if let Some(state) = prev {
        if state != taskbar_state() {
            set_taskbar_state(state);
        }
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
        let Some(layer) = wallpaper_layer() else { return false };
        if hwnd == 0 || SetParent(h(hwnd), Some(layer)).is_err() {
            return false;
        }
        // Współrzędne dziecka liczą się od lewego górnego rogu warstwy (cały pulpit wirtualny).
        let mut r = RECT::default();
        let _ = GetWindowRect(layer, &mut r);
        let _ = SetWindowPos(h(hwnd), Some(HWND_TOP), x - r.left, y - r.top, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        true
    }
}

/// Warstwa tapety, w której żywe tapety rysują swoje okna. Komunikat 0x052C prosi Progman
/// o jej utworzenie. Windows 11 24H2: `WorkerW` jest dzieckiem Progman; wcześniej to okno
/// najwyższego poziomu tuż za oknem z widokiem ikon.
unsafe fn wallpaper_layer() -> Option<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_NORMAL};
    let progman = FindWindowW(w!("Progman"), None).ok()?;
    let mut res = 0usize;
    let _ = SendMessageTimeoutW(progman, 0x052C, windows::Win32::Foundation::WPARAM(0xD), LPARAM(1), SMTO_NORMAL, 1000, Some(&mut res));
    if let Ok(w) = FindWindowExW(Some(progman), None, w!("WorkerW"), None) {
        return Some(w);
    }
    unsafe extern "system" fn each(w: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Option<HWND>);
        if FindWindowExW(Some(w), None, w!("SHELLDLL_DefView"), None).is_ok() {
            if let Ok(next) = FindWindowExW(None, Some(w), w!("WorkerW"), None) {
                *out = Some(next);
                return BOOL(0);
            }
        }
        BOOL(1)
    }
    let mut out: Option<HWND> = None;
    let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    out
}

/// Czy okno nadal istnieje (po restarcie Eksploratora dzieci Progman giną razem z nim).
pub fn alive(hwnd: isize) -> bool {
    unsafe { IsWindow(Some(h(hwnd))).as_bool() }
}

/// Hak zdarzeń okien: pasek Windows pokazany albo przesunięty (autoukrywanie wysuwa go przy
/// dolnej krawędzi) – chowamy go od razu, póki trwa podmiana. Własny wątek z pętlą komunikatów.
fn hook_taskbar() {
    use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, EVENT_OBJECT_SHOW, MSG, WINEVENT_OUTOFCONTEXT};
    unsafe extern "system" fn on_event(_: HWINEVENTHOOK, _: u32, w: HWND, obj: i32, _: i32, _: u32, _: u32) {
        if obj != 0 || !ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let c = class_of(w);
        if (c == "Shell_TrayWnd" || c == "Shell_SecondaryTrayWnd") && IsWindowVisible(w).as_bool() && foreign(w) {
            let _ = ShowWindow(w, SW_HIDE);
        }
    }
    std::thread::spawn(|| unsafe {
        let _ = SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, None, Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    });
}

/// Czy któreś widoczne okno procesu `pid` nie odpowiada (Windows uznaje je za zawieszone po 5 s).
fn hung(pid: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsHungAppWindow};
    unsafe extern "system" fn each(w: HWND, lp: LPARAM) -> BOOL {
        let (pid, found) = &mut *(lp.0 as *mut (u32, bool));
        let mut owner = 0u32;
        GetWindowThreadProcessId(w, Some(&mut owner));
        if owner == *pid && IsWindowVisible(w).as_bool() && IsHungAppWindow(w).as_bool() {
            *found = true;
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut data = (pid, false);
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut data as *mut _ as isize));
    }
    data.1
}

/// Uruchamia strażnika: osobny proces, który po końcu tego (także po awarii) przywraca Windows.
pub fn spawn_guard() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg(GUARD_FLAG).arg(std::process::id().to_string()).spawn();
    }
}

/// Strażnik: czeka na koniec procesu `pid`. Aktualizacja – nic (skrypt uruchamia nową wersję).
/// Bez Eksploratora (tryb powłoki) – uruchamia go, żeby nie został czarny ekran. Plik stanu
/// został – to awaria (zamknięcie go sprząta): PowerDesk wraca, a po kilku awariach z rzędu
/// zostają pasek i pulpit Windows.
pub fn guard(pid: u32) {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};
    unsafe {
        if let Ok(p) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            // Co 3 s sprawdzamy, czy PowerDesk nie zawisł: okna nie odpowiadają od 15 s – pokazujemy
            // pasek Windows, żeby dało się dalej pracować (PowerDesk po odwieszeniu schowa go sam).
            let mut hung_since: Option<std::time::Instant> = None;
            let mut shown = false;
            while WaitForSingleObject(p, 3000) == windows::Win32::Foundation::WAIT_TIMEOUT {
                if hung(pid) {
                    let since = *hung_since.get_or_insert_with(std::time::Instant::now);
                    if !shown && since.elapsed() >= std::time::Duration::from_secs(15) {
                        crate::log::error("PowerDesk nie odpowiada – pokazuję pasek Windows");
                        for b in system_bars() {
                            let _ = ShowWindow(b, SW_SHOWNA);
                        }
                        shown = true;
                    }
                } else {
                    hung_since = None;
                    shown = false;
                }
            }
            let _ = windows::Win32::Foundation::CloseHandle(p);
        }
        if GetSystemMetrics(SM_SHUTTINGDOWN) != 0 {
            return;
        }
    }
    if crate::update::marker().exists() {
        crate::update::wait_for_apply();
        return;
    }
    let crashed = swapped();
    if crate::shellmode::detect() {
        crate::shellmode::fallback_to_explorer();
    }
    if !crashed {
        return;
    }
    // Najpierw pasek i pulpit Windows – bez czekania na start nowego PowerDesk ekran nie zostaje pusty.
    restore();
    if crate::shellmode::count_crash() {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).arg(crate::autostart::FLAG).spawn();
            return;
        }
    }
    restore();
}
