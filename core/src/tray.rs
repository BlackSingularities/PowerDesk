//! Ikona w zasobniku (obok zegara): klik pokazuje okno, prawy przycisk – menu.
//! Działa we własnym wątku z ukrytym oknem, które odbiera komunikaty ikony;
//! po restarcie Eksploratora (TaskbarCreated) ikona wraca sama.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    LookupIconIdFromDirectoryEx, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, HICON, LR_DEFAULTCOLOR, MF_SEPARATOR,
    MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

/// Co użytkownik zrobił z ikoną.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Klik albo „Otwórz PowerDesk”.
    Show,
    /// „Nowa karta” z menu.
    NewTab,
    /// „Pulpit testowy” – włącza albo wyłącza pulpit PowerDesk.
    Desktop,
    /// „Pasek testowy” – włącza albo wyłącza pasek zadań PowerDesk.
    Taskbar,
    /// „Zakończ” – zamknięcie programu, także z wyłączonym zamykaniem do zasobnika.
    Quit,
}

const MSG_ICON: u32 = WM_APP + 1;
const ID_SHOW: usize = 1;
const ID_TAB: usize = 2;
const ID_QUIT: usize = 3;
const ID_DESKTOP: usize = 4;
const ID_TASKBAR: usize = 5;

static HWND_TRAY: AtomicIsize = AtomicIsize::new(0);
static HANDLER: OnceLock<Box<dyn Fn(Event) + Send + Sync>> = OnceLock::new();
static ICON: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

/// Ikona 16 px z pliku .ico (wybór rozmiaru jak w powłoce).
fn icon_from_ico(data: &[u8]) -> Option<HICON> {
    unsafe {
        let off = LookupIconIdFromDirectoryEx(data.as_ptr(), true, 16, 16, LR_DEFAULTCOLOR);
        let part = data.get(off as usize..)?;
        CreateIconFromResourceEx(part, true, 0x0003_0000, 16, 16, LR_DEFAULTCOLOR).ok()
    }
}

fn notify_data(hwnd: HWND, tip: &str) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = MSG_ICON;
    nid.hIcon = HICON(ICON.load(Ordering::SeqCst) as *mut _);
    for (d, s) in nid.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
        *d = s;
    }
    nid
}

fn add_icon(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &notify_data(hwnd, "PowerDesk"));
    }
}

fn fire(e: Event) {
    if let Some(h) = HANDLER.get() {
        h(e);
    }
}

/// Menu pod prawym przyciskiem – systemowe, jak u innych ikon w zasobniku.
unsafe fn menu(hwnd: HWND) {
    let Ok(m) = CreatePopupMenu() else { return };
    let _ = AppendMenuW(m, MF_STRING, ID_SHOW, w!("Otwórz PowerDesk"));
    let _ = AppendMenuW(m, MF_STRING, ID_TAB, w!("Nowa karta"));
    let _ = AppendMenuW(m, MF_STRING, ID_DESKTOP, w!("Pulpit testowy (włącz / wyłącz)"));
    let _ = AppendMenuW(m, MF_STRING, ID_TASKBAR, w!("Pasek testowy (włącz / wyłącz)"));
    let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(m, MF_STRING, ID_QUIT, w!("Zakończ"));
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // Bez tego menu nie znika po kliknięciu obok.
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), hwnd, None);
    let _ = DestroyMenu(m);
    match cmd.0 as usize {
        ID_SHOW => fire(Event::Show),
        ID_TAB => fire(Event::NewTab),
        ID_DESKTOP => fire(Event::Desktop),
        ID_TASKBAR => fire(Event::Taskbar),
        ID_QUIT => fire(Event::Quit),
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        MSG_ICON => {
            match lp.0 as u32 {
                WM_LBUTTONUP => fire(Event::Show),
                WM_RBUTTONUP => menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = Shell_NotifyIconW(NIM_DELETE, &notify_data(hwnd, ""));
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        // Eksplorator wystartował od nowa – stare ikony zniknęły razem z paskiem.
        m if Some(&m) == TASKBAR_CREATED.get() => {
            add_icon(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Pokazuje ikonę; `on_event` woła się z wątku zasobnika. Drugie wywołanie przy widocznej ikonie nic nie robi.
pub fn show(ico: &'static [u8], on_event: impl Fn(Event) + Send + Sync + 'static) {
    let _ = HANDLER.set(Box::new(on_event));
    if HWND_TRAY.load(Ordering::SeqCst) != 0 {
        return;
    }
    // Zaznaczamy od razu, żeby szybkie ponowne włączenie nie utworzyło drugiej ikony.
    HWND_TRAY.store(-1, Ordering::SeqCst);
    std::thread::spawn(move || unsafe {
        if ICON.load(Ordering::SeqCst) == 0 {
            if let Some(i) = icon_from_ico(ico) {
                ICON.store(i.0 as isize, Ordering::SeqCst);
            }
        }
        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let class = HSTRING::from("PowerDeskTray");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: PCWSTR(class.as_ptr()), ..Default::default() };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), &class, w!("PowerDesk"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, Some(inst.into()), None) else {
            HWND_TRAY.store(0, Ordering::SeqCst);
            return;
        };
        HWND_TRAY.store(hwnd.0 as isize, Ordering::SeqCst);
        add_icon(hwnd);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        HWND_TRAY.store(0, Ordering::SeqCst);
    });
}

/// Usuwa ikonę (wyłączenie w ustawieniach, wyjście z programu).
pub fn hide() {
    let h = HWND_TRAY.load(Ordering::SeqCst);
    if h > 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Ikona jest widoczna (albo właśnie się pojawia).
pub fn visible() -> bool {
    HWND_TRAY.load(Ordering::SeqCst) != 0
}
