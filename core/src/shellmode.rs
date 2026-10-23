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
        use windows::Win32::UI::WindowsAndMessaging::{HWND_BOTTOM, SWP_NOZORDER, WINDOWPOS, WM_WINDOWPOSCHANGING};
        if msg == WM_WINDOWPOSCHANGING {
            let p = &mut *(lp.0 as *mut WINDOWPOS);
            p.hwndInsertAfter = HWND_BOTTOM;
            p.flags &= !SWP_NOZORDER;
        }
        DefSubclassProc(w, msg, wp, lp)
    }
    unsafe {
        let _ = SetWindowSubclass(HWND(hwnd as *mut _), Some(proc), 0x5044, 0);
    }
}

/// Okno pulpitu na monitorze (x, y, w, h – piksele ekranu), na dnie.
pub fn place_desktop(hwnd: isize, x: i32, y: i32, width: i32, height: i32) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_BOTTOM, SWP_NOACTIVATE, SWP_SHOWWINDOW};
    unsafe {
        let _ = SetWindowPos(HWND(hwnd as *mut _), Some(HWND_BOTTOM), x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
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
    if times.len() >= CRASH_LIMIT {
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

/// Awaryjnie: wpis zdjęty (Eksplorator uzna się za powłokę) i start Eksploratora.
pub fn fallback_to_explorer() {
    let _ = unregister();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let _ = std::process::Command::new(format!(r"{windir}\explorer.exe")).spawn();
}
