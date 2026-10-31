//! Język klawiatury na pasku: układ aktywnego okna (każdy wątek ma własny) i przełączanie
//! na następny zainstalowany – jak przycisk „POL/ENG” paska Windows.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Globalization::{GetLocaleInfoW, LOCALE_SISO639LANGNAME2, LOCALE_SLOCALIZEDDISPLAYNAME};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, GetKeyboardLayoutList, HKL};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, PostMessageW, WM_INPUTLANGCHANGEREQUEST};

/// Układ klawiatury: skrót (POL, ENG) i pełna nazwa języka.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub short: String,
    pub name: String,
}

fn locale(lang: u16, kind: u32) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetLocaleInfoW(lang as u32, kind, Some(&mut buf)) };
    if n <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize - 1])
}

fn layout_of(hkl: HKL) -> Layout {
    let lang = (hkl.0 as usize & 0xFFFF) as u16;
    let short = locale(lang, LOCALE_SISO639LANGNAME2).to_uppercase();
    let name = locale(lang, LOCALE_SLOCALIZEDDISPLAYNAME);
    Layout { short: if short.is_empty() { format!("{lang:04X}") } else { short }, name }
}

/// Zainstalowane układy.
fn installed() -> Vec<HKL> {
    unsafe {
        let n = GetKeyboardLayoutList(None);
        if n <= 0 {
            return Vec::new();
        }
        let mut list = vec![HKL::default(); n as usize];
        let got = GetKeyboardLayoutList(Some(&mut list));
        list.truncate(got.max(0) as usize);
        list
    }
}

fn current_hkl(win: isize) -> HKL {
    unsafe {
        let thread = if win == 0 { 0 } else { GetWindowThreadProcessId(HWND(win as *mut _), None) };
        GetKeyboardLayout(thread)
    }
}

/// Układ okna `win` (0 – bieżący wątek). None, gdy zainstalowany jest jeden – Windows wtedy też nic nie pokazuje.
pub fn current(win: isize) -> Option<Layout> {
    if installed().len() < 2 {
        return None;
    }
    Some(layout_of(current_hkl(win)))
}

/// Okno `win` dostaje następny układ z listy (jak Win+Spacja).
pub fn next(win: isize) {
    let list = installed();
    if list.len() < 2 || win == 0 {
        return;
    }
    let cur = current_hkl(win);
    let i = list.iter().position(|h| *h == cur).map_or(0, |i| (i + 1) % list.len());
    unsafe {
        let _ = PostMessageW(Some(HWND(win as *mut _)), WM_INPUTLANGCHANGEREQUEST, WPARAM(0), LPARAM(list[i].0 as isize));
    }
}

