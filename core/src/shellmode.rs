//! PowerDesk działa nad Eksploratorem: explorer.exe zostaje w tle (prowadzi ImmersiveShell –
//! Ustawienia, aplikacje UWP/Store, powiadomienia, Win+A/N/V, przyciąganie okien), a PowerDesk
//! chowa jego pasek i pulpit (`shellswap`) i pokazuje własne. Eksplorator uznaje się za powłokę
//! tylko wtedy, gdy wpis `HKCU\…\Winlogon\Shell` wskazuje jego – nasz wpis ze starszych wersji
//! zdejmujemy przy starcie.
//!
//! Tryb powłoki (Eksplorator nie działa i nie daje się uruchomić) zostaje awaryjny: sami
//! ustawiamy obszar roboczy monitorów, trzymamy pulpit na dnie i obsługujemy skróty Win.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SystemParametersInfoW, SPIF_SENDCHANGE, SPI_SETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};

use crate::winkey::{exe, reg};

/// Argument, z którym Winlogon uruchamia PowerDesk jako powłokę.
pub const SHELL_FLAG: &str = "--shell";

const KEY: &str = r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon";

/// Awarie w tym oknie czasu liczą się jako „z rzędu”.
const CRASH_WINDOW_SECS: u64 = 120;
/// Tyle awarii z rzędu – wracamy do Eksploratora.
const CRASH_LIMIT: usize = 3;

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Czy PowerDesk jest teraz powłoką (Eksplorator nie prowadzi pulpitu ani paska).
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// Sprawdza, czy działa pasek Eksploratora (nasz zasobnik, też `Shell_TrayWnd`, się nie liczy).
pub fn detect() -> bool {
    let on = crate::shellswap::explorer_tray().is_none();
    ACTIVE.store(on, Ordering::SeqCst);
    on
}

/// Eksplorator w tle zamiast zastępowania go: zdejmuje nasz wpis Winlogon (starsze wersje
/// uruchamiały PowerDesk zamiast Eksploratora) i – gdy Eksplorator nie działa – uruchamia go
/// jako powłokę i czeka na jego pasek. Zwraca true, gdy mimo to go nie ma (tryb powłoki).
pub fn ensure_explorer() -> bool {
    if registered() {
        crate::log::info("zdjęcie wpisu powłoki – Eksplorator wraca w tle");
        let _ = unregister();
    }
    if crate::shellswap::explorer_tray().is_some() {
        return detect();
    }
    crate::log::info("uruchamianie Eksploratora w tle");
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let _ = std::process::Command::new(format!(r"{windir}\explorer.exe")).spawn();
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if crate::shellswap::explorer_tray().is_some() {
            break;
        }
    }
    detect()
}

/// Czy wpis Winlogon wskazuje ten plik .exe.
pub fn registered() -> bool {
    let Ok(exe) = exe() else { return false };
    reg(&["query", KEY, "/v", "Shell"]).is_ok_and(|o| o.to_lowercase().contains(&exe.to_lowercase()))
}

/// Usuwa nasz wpis – Windows wraca do powłoki z HKLM (`explorer.exe`).
pub fn unregister() -> Result<(), String> {
    if reg(&["query", KEY, "/v", "Shell"]).is_ok() {
        reg(&["delete", KEY, "/v", "Shell", "/f"])?;
    }
    Ok(())
}

// ---------- Obszar roboczy: nasz pasek i cudze AppBary ----------

/// Monitor (lewo, góra, prawo, dół) i wysokość paska PowerDesk na jego dole.
static BARS: parking_lot::Mutex<Vec<(RECT, i32)>> = parking_lot::Mutex::new(Vec::new());
/// Paski innych programów przyklejone do krawędzi (SHAppBarMessage): okno, krawędź, prostokąt.
static APPBARS: parking_lot::Mutex<Vec<AppBar>> = parking_lot::Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug)]
pub struct AppBar {
    pub hwnd: isize,
    pub callback: u32,
    /// ABE_LEFT 0, ABE_TOP 1, ABE_RIGHT 2, ABE_BOTTOM 3.
    pub edge: u32,
    pub rc: RECT,
}

fn overlaps(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// Pasek PowerDesk na dole monitora – zapamiętany i od razu w obszarze roboczym.
pub fn set_bar(left: i32, top: i32, right: i32, bottom: i32, height: i32) {
    let mon = RECT { left, top, right, bottom };
    {
        let mut bars = BARS.lock();
        bars.retain(|(m, _)| !(m.left == left && m.top == top));
        bars.push((mon, height));
    }
    recompute();
}

/// Pasek zamknięty – monitor wraca do pełnego obszaru (bez cudzych AppBarów).
pub fn clear_bar(left: i32, top: i32, right: i32, bottom: i32) {
    BARS.lock().retain(|(m, _)| !(m.left == left && m.top == top));
    reset_work_area(left, top, right, bottom);
}

/// Monitor, na którym leży prostokąt, i wolne miejsce na nim po odjęciu naszego paska
/// i AppBarów innych niż `skip`.
fn free_area(rc: &RECT, skip: isize) -> Option<(RECT, RECT)> {
    let bars = BARS.lock().clone();
    let (mon, h) = bars.iter().find(|(m, _)| overlaps(m, rc)).copied().or_else(|| bars.first().copied())?;
    let mut area = RECT { bottom: mon.bottom - h, ..mon };
    for a in APPBARS.lock().iter().filter(|a| a.hwnd != skip && overlaps(&a.rc, &mon)) {
        match a.edge {
            0 => area.left = area.left.max(a.rc.right),
            1 => area.top = area.top.max(a.rc.bottom),
            2 => area.right = area.right.min(a.rc.left),
            _ => area.bottom = area.bottom.min(a.rc.top),
        }
    }
    Some((mon, area))
}

/// Obszar roboczy każdego monitora: bez naszego paska i bez cudzych AppBarów.
fn recompute() {
    let bars = BARS.lock().clone();
    for (mon, _) in bars {
        if let Some((_, area)) = free_area(&mon, 0) {
            set_work_area(area.left, area.top, area.right, area.bottom);
        }
    }
}

/// AppBar pyta o miejsce (QUERYPOS/SETPOS): prostokąt przycięty do wolnego obszaru przy jego krawędzi.
pub fn appbar_fit(hwnd: isize, edge: u32, mut rc: RECT) -> RECT {
    let Some((_, area)) = free_area(&rc, hwnd) else { return rc };
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    match edge {
        0 => {
            rc.left = area.left;
            rc.right = area.left + w;
        }
        1 => {
            rc.top = area.top;
            rc.bottom = area.top + h;
        }
        2 => {
            rc.right = area.right;
            rc.left = area.right - w;
        }
        _ => {
            rc.bottom = area.bottom;
            rc.top = area.bottom - h;
        }
    }
    if edge == 0 || edge == 2 {
        rc.top = rc.top.max(area.top);
        rc.bottom = rc.bottom.min(area.bottom);
    } else {
        rc.left = rc.left.max(area.left);
        rc.right = rc.right.min(area.right);
    }
    rc
}

pub fn appbar_new(hwnd: isize, callback: u32) -> bool {
    let mut bars = APPBARS.lock();
    if bars.iter().any(|a| a.hwnd == hwnd) {
        return false;
    }
    bars.push(AppBar { hwnd, callback, edge: 3, rc: RECT::default() });
    true
}

pub fn appbar_remove(hwnd: isize) {
    APPBARS.lock().retain(|a| a.hwnd != hwnd);
    recompute();
    notify_appbars(hwnd);
}

/// Zapis położenia AppBaru i nowy obszar roboczy; pozostałe AppBary dostają ABN_POSCHANGED.
pub fn appbar_set(hwnd: isize, edge: u32, rc: RECT) {
    {
        let mut bars = APPBARS.lock();
        if let Some(a) = bars.iter_mut().find(|a| a.hwnd == hwnd) {
            a.edge = edge;
            a.rc = rc;
        }
    }
    recompute();
    notify_appbars(hwnd);
}

/// Zamknięte okna AppBarów (bez ABM_REMOVE) – zwolnienie ich miejsca.
pub fn appbar_prune() {
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    let before = APPBARS.lock().len();
    APPBARS.lock().retain(|a| unsafe { IsWindow(Some(HWND(a.hwnd as *mut _))).as_bool() });
    if APPBARS.lock().len() != before {
        recompute();
    }
}

fn notify_appbars(except: isize) {
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    for a in APPBARS.lock().iter().filter(|a| a.hwnd != except && a.callback != 0) {
        // ABN_POSCHANGED = 1.
        unsafe {
            let _ = PostMessageW(Some(HWND(a.hwnd as *mut _)), a.callback, WPARAM(1), LPARAM(0));
        }
    }
}

/// Prostokąt naszego paska na monitorze głównym (ABM_GETTASKBARPOS).
pub fn taskbar_rect() -> Option<RECT> {
    let bars = BARS.lock();
    let (m, h) = bars.iter().find(|(m, _)| m.left == 0 && m.top == 0).or(bars.first())?;
    Some(RECT { top: m.bottom - h, ..*m })
}

/// Obszar roboczy monitora (bez paska). Bez Eksploratora nikt inny go nie ustawi.
pub fn set_work_area(left: i32, top: i32, right: i32, bottom: i32) {
    let mut r = RECT { left, top, right, bottom };
    unsafe {
        let _ = SystemParametersInfoW(SPI_SETWORKAREA, 0, Some(&mut r as *mut _ as *mut _), SPIF_SENDCHANGE);
    }
}

/// Obszar roboczy na cały monitor (przy zamykaniu paska).
pub fn reset_work_area(left: i32, top: i32, right: i32, bottom: i32) {
    let mut r = RECT { left, top, right, bottom };
    unsafe {
        let _ = SystemParametersInfoW(SPI_SETWORKAREA, 0, Some(&mut r as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
}

// ---------- Pulpit na dnie ----------

/// Okno pulpitu zostaje pod wszystkimi oknami, także po kliknięciu (podklasa okna: każda
/// zmiana kolejności kończy się na dnie). Wołać z wątku okna.
pub fn keep_at_bottom(hwnd: isize) {
    use windows::Win32::UI::Shell::SetWindowSubclass;
    unsafe extern "system" fn proc(w: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
        use windows::Win32::UI::Shell::DefSubclassProc;
        use windows::Win32::UI::WindowsAndMessaging::{SWP_NOZORDER, WINDOWPOS, WM_WINDOWPOSCHANGING};
        if msg == WM_WINDOWPOSCHANGING {
            let p = &mut *(lp.0 as *mut WINDOWPOS);
            match bottom_slot(w) {
                Some(after) => {
                    p.hwndInsertAfter = after;
                    p.flags &= !SWP_NOZORDER;
                }
                // Już na swoim miejscu – kolejność bez zmian.
                None => p.flags |= SWP_NOZORDER,
            }
        }
        DefSubclassProc(w, msg, wp, lp)
    }
    unsafe {
        let _ = SetWindowSubclass(HWND(hwnd as *mut _), Some(proc), 0x5044, 0);
    }
}

/// Okno pulpitu jako okno powłoki (GetShellWindow) – programy i system traktują je jak pulpit
/// (np. przy szukaniu okna do aktywacji). SetShellWindow z user32 – nieudokumentowane, bez nagłówka.
pub fn register_shell_window(hwnd: isize) {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    type SetShell = unsafe extern "system" fn(HWND) -> i32;
    unsafe {
        let Ok(user) = GetModuleHandleW(w!("user32.dll")) else { return };
        if let Some(f) = GetProcAddress(user, windows::core::s!("SetShellWindow")) {
            let f: SetShell = std::mem::transmute(f);
            f(HWND(hwnd as *mut _));
        }
    }
}

/// Miejsce okna pulpitu w kolejności okien: tuż nad pulpitem Windows (`Progman` rysuje tapetę na
/// całym ekranie i zasłoniłby wszystko pod sobą), a bez Eksploratora – na samym dnie.
/// None – okno już stoi na swoim miejscu.
fn bottom_slot(me: HWND) -> Option<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_HWNDPREV, HWND_BOTTOM};
    unsafe {
        let Ok(progman) = FindWindowW(w!("Progman"), None) else { return Some(HWND_BOTTOM) };
        // Okno wstawiamy „za” to, które jest tuż nad Progman – czyli między nie a Progman.
        match GetWindow(progman, GW_HWNDPREV) {
            Ok(prev) if prev != me && !prev.is_invalid() => Some(prev),
            _ => None,
        }
    }
}

/// Ustawia okno pulpitu na jego miejscu w kolejności (po zmianach, które ominęły podklasę).
pub fn lower(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};
    let h = HWND(hwnd as *mut _);
    if let Some(after) = bottom_slot(h) {
        unsafe {
            let _ = SetWindowPos(h, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

/// Okno pulpitu na monitorze (x, y, w, h – piksele ekranu), na dnie.
pub fn place_desktop(hwnd: isize, x: i32, y: i32, width: i32, height: i32) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_SHOWWINDOW};
    unsafe {
        let h = HWND(hwnd as *mut _);
        match bottom_slot(h) {
            Some(after) => {
                let _ = SetWindowPos(h, Some(after), x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
            None => {
                let _ = SetWindowPos(h, None, x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER);
            }
        }
    }
}

// ---------- Skróty ----------

/// Win+R – systemowe okno „Uruchom” (shell32, RunFileDlg, porządkowo 61).
pub fn run_dialog() {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type RunFileDlg = unsafe extern "system" fn(HWND, isize, *const u16, *const u16, *const u16, u32) -> i32;
    std::thread::spawn(|| unsafe {
        let Ok(lib) = LoadLibraryW(w!("shell32.dll")) else { return };
        let Some(f) = GetProcAddress(lib, windows::core::PCSTR(61 as *const u8)) else { return };
        let f: RunFileDlg = std::mem::transmute(f);
        f(HWND::default(), 0, std::ptr::null(), std::ptr::null(), std::ptr::null(), 0);
    });
}

/// Okna schowane przez „Pokaż pulpit” – drugie wywołanie je przywraca.
static SHOWN_DESKTOP: parking_lot::Mutex<Vec<isize>> = parking_lot::Mutex::new(Vec::new());

/// Win+D bez Eksploratora: minimalizuje okna programów; ponownie – przywraca te same.
pub fn toggle_desktop() {
    use crate::taskwin;
    let mut saved = SHOWN_DESKTOP.lock();
    let open: Vec<isize> = taskwin::list().into_iter().filter(|w| !w.minimized).map(|w| w.hwnd).collect();
    if open.is_empty() && !saved.is_empty() {
        for &h in saved.iter().rev() {
            taskwin::activate(h);
        }
        saved.clear();
        return;
    }
    for &h in &open {
        taskwin::minimize(h);
    }
    *saved = open;
}

// ---------- Strażnik ----------

fn crash_file() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join("PowerDesk").join("shell-crashes.txt")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Zapisuje awarię PowerDesk. False – za dużo awarii z rzędu (nie restartować). Przy true
/// czeka chwilę, aż Windows posprząta po poprzednim procesie (okna, ikony zasobnika).
pub fn count_crash() -> bool {
    let file = crash_file();
    let now = now_secs();
    let mut times: Vec<u64> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .filter(|&t| now.saturating_sub(t) < CRASH_WINDOW_SECS)
        .collect();
    times.push(now);
    crate::log::error(format!("PowerDesk zakończył się nieoczekiwanie ({}. raz w ciągu {CRASH_WINDOW_SECS} s)", times.len()));
    if times.len() >= CRASH_LIMIT {
        crate::log::error("za dużo awarii z rzędu – bez ponownego uruchamiania");
        let _ = std::fs::remove_file(&file);
        return false;
    }
    let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
    let _ = std::fs::write(&file, times.iter().map(|t| format!("{t}
")).collect::<String>());
    std::thread::sleep(std::time::Duration::from_millis(500));
    true
}

/// Argument awaryjny: jak przytrzymany Shift – ta sesja z Eksploratorem.
pub const SAFE_FLAG: &str = "--safe";

/// Shift przytrzymany w chwili startu (logowanie z PowerDesk jako powłoką).
pub fn shift_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
    unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) < 0 }
}

/// Tryb awaryjny: sam Eksplorator, bez PowerDesk, i bez wpisu powłoki.
pub fn safe_mode() {
    crate::log::info("tryb awaryjny – sam Eksplorator");
    fallback_to_explorer();
}

/// Awaryjnie: wpis zdjęty (Eksplorator uzna się za powłokę) i start Eksploratora.
pub fn fallback_to_explorer() {
    let _ = unregister();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let _ = std::process::Command::new(format!(r"{windir}\explorer.exe")).spawn();
}
