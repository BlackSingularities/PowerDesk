//! Pulpity wirtualne Windows na pasku PowerDesk: lista, nazwy i bieżący pulpit z rejestru
//! (to, co zapisuje Eksplorator), przełączanie systemowymi skrótami Win+Ctrl+strzałki.
//! Pulpity wirtualne prowadzi Eksplorator – w trybie powłoki ich nie ma.

use windows::core::HSTRING;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_SZ};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_F4, VK_LEFT, VK_LWIN, VK_RIGHT};

const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops";

fn binary(key: &str, value: &str) -> Option<Vec<u8>> {
    unsafe {
        let mut len = 0u32;
        let (k, v) = (HSTRING::from(key), HSTRING::from(value));
        if RegGetValueW(HKEY_CURRENT_USER, &k, &v, RRF_RT_REG_BINARY, None, None, Some(&mut len)).is_err() || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        RegGetValueW(HKEY_CURRENT_USER, &k, &v, RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)).ok().ok()?;
        buf.truncate(len as usize);
        Some(buf)
    }
}

fn string(key: &str, value: &str) -> Option<String> {
    unsafe {
        let mut buf = [0u16; 260];
        let mut len = (buf.len() * 2) as u32;
        RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(value), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)).ok().ok()?;
        let s = String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]);
        (!s.is_empty()).then_some(s)
    }
}

/// GUID z 16 bajtów rejestru w zapisie {…} (jak nazwy podkluczy Desktops).
fn guid(b: &[u8]) -> String {
    let d1 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let d2 = u16::from_le_bytes([b[4], b[5]]);
    let d3 = u16::from_le_bytes([b[6], b[7]]);
    format!("{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}", b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15])
}

/// Pulpity (nazwy – własne albo „Pulpit N”) i numer bieżącego. Jeden pulpit Windows nie zapisuje
/// listy w rejestrze – wtedy „Pulpit 1”. Wołający ukrywa przełącznik w trybie powłoki.
pub fn list() -> Option<(Vec<String>, usize)> {
    let Some(ids) = binary(KEY, "VirtualDesktopIDs") else { return Some((vec!["Pulpit 1".into()], 0)) };
    let ids: Vec<&[u8]> = ids.chunks_exact(16).collect();
    if ids.is_empty() {
        return Some((vec!["Pulpit 1".into()], 0));
    }
    let cur = binary(KEY, "CurrentVirtualDesktop").filter(|c| c.len() == 16);
    let current = cur.as_ref().and_then(|c| ids.iter().position(|i| *i == c.as_slice())).unwrap_or(0);
    let names = ids
        .iter()
        .enumerate()
        .map(|(n, id)| string(&format!(r"{KEY}\Desktops\{}", guid(id)), "Name").unwrap_or_else(|| format!("Pulpit {}", n + 1)))
        .collect();
    Some((names, current))
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { Default::default() };
    if vk == VK_LEFT || vk == VK_RIGHT {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: flags, ..Default::default() } } }
}

fn win_ctrl(vk: VIRTUAL_KEY) {
    let seq = [key(VK_LWIN, false), key(VK_CONTROL, false), key(vk, false), key(vk, true), key(VK_CONTROL, true), key(VK_LWIN, true)];
    unsafe {
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Przejście o `steps` pulpitów (ujemne – w lewo).
pub fn step(steps: i32) {
    let vk = if steps < 0 { VK_LEFT } else { VK_RIGHT };
    for _ in 0..steps.unsigned_abs() {
        win_ctrl(vk);
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
}

/// Przejście na pulpit numer `to`.
pub fn go(to: usize) {
    if let Some((_, cur)) = list() {
        step(to as i32 - cur as i32);
    }
}

/// Nowy pulpit (Win+Ctrl+D) – Windows od razu na niego przechodzi.
pub fn add() {
    win_ctrl(VIRTUAL_KEY(b'D' as u16));
}

/// Zamyka bieżący pulpit (Win+Ctrl+F4) – okna przechodzą na sąsiedni.
pub fn close_current() {
    win_ctrl(VK_F4);
}
