//! Globalne skróty klawiszowe (RegisterHotKey) – działają w każdym programie, także w grach
//! w oknie. Własny wątek z kolejką komunikatów; wywołanie z tego wątku.

use std::sync::OnceLock;

use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_NOREPEAT};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

static HANDLER: OnceLock<Box<dyn Fn(i32) + Send + Sync>> = OnceLock::new();

/// Nakładka: Alt+R.
pub const OVERLAY: i32 = 1;

/// Rejestruje skróty programu; `on_key(id)` przy naciśnięciu. Drugie wywołanie – nic.
/// Zwraca od razu; jeśli inny program zajął skrót, po prostu nie zadziała.
pub fn start(on_key: impl Fn(i32) + Send + Sync + 'static) {
    if HANDLER.set(Box::new(on_key)).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe {
        let _ = RegisterHotKey(None, OVERLAY, HOT_KEY_MODIFIERS(MOD_ALT.0 | MOD_NOREPEAT.0), b'R' as u32);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY {
                if let Some(f) = HANDLER.get() {
                    f(msg.wParam.0 as i32);
                }
            }
        }
    });
}
