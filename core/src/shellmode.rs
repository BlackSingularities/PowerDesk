//! Tryb powłoki: PowerDesk zamiast Eksploratora po zalogowaniu. Wpis
//! `HKCU\…\Winlogon\Shell` (tylko bieżący użytkownik, bez administratora) wskazuje
//! `powerdesk.exe --shell`; Eksplorator nie startuje, pasek i pulpit Windows nie istnieją.
//!
//! Bez Eksploratora sami ustawiamy obszar roboczy monitorów (miejsce na pasek), trzymamy
//! pulpit na dnie, prowadzimy zasobnik (`trayhost`) i skróty Win+E/R/D.
//! Strażnik restartuje PowerDesk po awarii; po kilku awariach z rzędu zdejmuje wpis
//! i uruchamia `explorer.exe` – użytkownik nie zostaje z czarnym ekranem.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetSystemMetrics, SystemParametersInfoW, SM_SHUTTINGDOWN, SPIF_SENDCHANGE, SPI_SETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};

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

/// Sprawdza przy starcie, czy działa pasek Eksploratora. Wołać przed `trayhost::start`,
/// które tworzy własne okno `Shell_TrayWnd`.
pub fn detect() -> bool {
    let explorer = unsafe { FindWindowW(w!("Shell_TrayWnd"), None).is_ok() };
    let on = !explorer;
    ACTIVE.store(on, Ordering::SeqCst);
    on
}

/// Zainstalowany PowerDesk zastępuje Eksplorator od razu, bez wylogowania: prosi pasek Eksploratora
/// o zamknięcie (to samo co „Zakończ Eksploratora” z Ctrl+Shift+prawy przycisk na pasku), a gdy
/// nie zdąży – kończy go. Potem PowerDesk działa jako powłoka (`detect` zwraca true).
pub fn take_over() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    unsafe {
        let Ok(tray) = FindWindowW(w!("Shell_TrayWnd"), None) else { return detect() };
        crate::log::info("przejęcie: zamykanie Eksploratora");
        // WM_USER + 436 – Eksplorator zamyka się porządnie (bez automatycznego ponownego startu).
        let _ = PostMessageW(Some(tray), 0x0400 + 436, WPARAM(0), LPARAM(0));
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            if FindWindowW(w!("Shell_TrayWnd"), None).is_err() {
                break;
            }
        }
    }
    if unsafe { FindWindowW(w!("Shell_TrayWnd"), None).is_ok() } {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill").args(["/f", "/im", "explorer.exe"]).creation_flags(0x0800_0000).status();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    detect()
}

/// Czy wpis Winlogon wskazuje ten plik .exe.
pub fn registered() -> bool {
    let Ok(exe) = exe() else { return false };
    reg(&["query", KEY, "/v", "Shell"]).is_ok_and(|o| o.to_lowercase().contains(&exe.to_lowercase()))
}

/// PowerDesk jako powłoka od następnego logowania.
pub fn register() -> Result<(), String> {
    reg(&["add", KEY, "/v", "Shell", "/d", &format!("\"{}\" {SHELL_FLAG}", exe()?), "/f"])?;
    Ok(())
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

/// Strażnik w trybie powłoki, po końcu procesu PowerDesk: przy wylogowaniu nic; inaczej
/// restart PowerDesk, a po `CRASH_LIMIT` końcach w krótkim czasie – powrót do Eksploratora.
/// Zwraca true, gdy czymś się zajął (wtedy zwykłe przywracanie nie jest potrzebne).
pub fn after_exit() -> bool {
    if unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) } != 0 {
        return true;
    }
    // Wpis zdjęty (deinstalacja, `--remove-integration`) – bez restartu, od razu Eksplorator.
    if !registered() {
        fallback_to_explorer();
        return true;
    }
    // Podmiana .exe przy aktualizacji: czekamy na skrypt i restartujemy bez liczenia awarii.
    if crate::update::marker().exists() {
        crate::update::wait_for_apply();
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).arg(SHELL_FLAG).spawn();
        }
        return true;
    }
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
        crate::log::error("za dużo awarii z rzędu – powrót do Eksploratora");
        let _ = std::fs::remove_file(&file);
        fallback_to_explorer();
        return true;
    }
    let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
    let _ = std::fs::write(&file, times.iter().map(|t| format!("{t}\n")).collect::<String>());
    // Chwila przerwy – Windows sprząta po poprzednim procesie (okna, ikony zasobnika).
    std::thread::sleep(std::time::Duration::from_millis(500));
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg(SHELL_FLAG).spawn();
    }
    true
}

/// Argument awaryjny: jak przytrzymany Shift – ta sesja z Eksploratorem.
pub const SAFE_FLAG: &str = "--safe";

/// Shift przytrzymany w chwili startu (logowanie z PowerDesk jako powłoką).
pub fn shift_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
    unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) < 0 }
}

/// Tryb awaryjny: ta sesja z Eksploratorem, wpis zostaje na następne logowanie. Eksplorator
/// uznaje się za powłokę, gdy wpis `Shell` wskazuje jego – zdejmujemy nasz na czas jego startu.
pub fn safe_mode() {
    crate::log::info("tryb awaryjny – ta sesja z Eksploratorem");
    let _ = unregister();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let _ = std::process::Command::new(format!(r"{windir}\explorer.exe")).spawn();
    // Eksplorator czyta wpis na samym początku; pasek pojawia się po kilku sekundach.
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if unsafe { FindWindowW(w!("Shell_TrayWnd"), None).is_ok() } {
            break;
        }
    }
    let _ = register();
}

/// Awaryjnie: wpis zdjęty (Eksplorator uzna się za powłokę) i start Eksploratora.
pub fn fallback_to_explorer() {
    let _ = unregister();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let _ = std::process::Command::new(format!(r"{windir}\explorer.exe")).spawn();
}
