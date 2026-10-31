//! Hak powłoki (RegisterShellHookWindow): okna, które wołają o uwagę (FlashWindow – nowa
//! wiadomość, koniec pobierania). Pasek pokazuje je jak Windows, póki okno nie stanie się aktywne.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, RegisterShellHookWindow, RegisterWindowMessageW, MSG, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

const HSHELL_WINDOWACTIVATED: usize = 4;
const HSHELL_WINDOWDESTROYED: usize = 2;
const HSHELL_RUDEAPPACTIVATED: usize = 0x8004;
const HSHELL_FLASH: usize = 0x8006;

static FLASHING: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static MSG_ID: OnceLock<u32> = OnceLock::new();
static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Okna, które teraz migają.
pub fn flashing() -> HashSet<isize> {
    FLASHING.lock().unwrap().iter().copied().collect()
}

/// Okno zostało aktywowane z paska – przestaje migać od razu.
pub fn clear(hwnd: isize) {
    FLASHING.lock().unwrap().retain(|&h| h != hwnd);
}

fn changed() {
    if let Some(f) = ON_CHANGE.get() {
        f();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if MSG_ID.get() == Some(&msg) {
        let win = lp.0;
        let mut list = FLASHING.lock().unwrap();
        let before = list.len();
        match wp.0 & 0xFFFF_FFFF {
            HSHELL_FLASH => {
                if !list.contains(&win) {
                    list.push(win);
                }
            }
            HSHELL_WINDOWACTIVATED | HSHELL_RUDEAPPACTIVATED | HSHELL_WINDOWDESTROYED => list.retain(|&h| h != win),
            _ => return LRESULT(0),
        }
        let diff = list.len() != before;
        drop(list);
        if diff {
            changed();
        }
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// Startuje nasłuch (własny wątek z ukrytym oknem); `on_change` – lista migających się zmieniła.
pub fn start(on_change: impl Fn() + Send + Sync + 'static) {
    if ON_CHANGE.set(Box::new(on_change)).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe {
        let _ = MSG_ID.set(RegisterWindowMessageW(w!("SHELLHOOK")));
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), lpszClassName: w!("PowerDeskShellHook"), ..Default::default() };
        RegisterClassW(&class);
        // Okno najwyższego poziomu (niewidoczne) – okna tylko do komunikatów haka nie dostają.
        let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("PowerDeskShellHook"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, None, None) else { return };
        if !RegisterShellHookWindow(hwnd).as_bool() {
            crate::log::error("hak powłoki: RegisterShellHookWindow nie powiódł się");
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
}
