//! Historia schowka PowerDesk (zamiast systemowej Win+V): nasłuch zmian schowka, zapamiętane
//! teksty, obrazy i listy plików (ostatnie 50 + przypięte), zapis na dysku, przywracanie
//! wpisu do schowka i wklejenie w okno, w którym użytkownik pracował.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use clipboard_win::{formats, Clipboard, Getter, Setter};
use serde::{Deserialize, Serialize};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::AddClipboardFormatListener;
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DefWindowProcW, GetMessageW, RegisterClassW, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSW};

/// Ile wpisów bez przypięcia trzymamy (jak Windows – 25; u nas więcej).
const LIMIT: usize = 50;
/// Większych tekstów nie zapamiętujemy (wklejony log nie zapcha historii).
const MAX_TEXT: usize = 1 << 20;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Kind {
    Text(String),
    /// Obraz zapisany jako PNG w folderze historii (nazwa pliku).
    Image(String),
    Files(Vec<String>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub kind: Kind,
    /// Sekundy od 1970 – do „przed chwilą / 5 min temu”.
    pub time: u64,
    pub pinned: bool,
}

static HISTORY: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);
/// Zmiana schowka, którą zrobiliśmy sami (przywrócenie wpisu) – nie dopisujemy jej drugi raz.
static OURS: AtomicBool = AtomicBool::new(false);
static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

fn dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join("PowerDesk").join("clipboard")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Pełna ścieżka obrazu wpisu (do miniatury w okienku).
pub fn image_path(name: &str) -> String {
    dir().join(name).to_string_lossy().into_owned()
}

fn save(list: &[Entry]) {
    let _ = std::fs::create_dir_all(dir());
    if let Ok(json) = serde_json::to_string(list) {
        let _ = std::fs::write(dir().join("history.json"), json);
    }
}

fn load() {
    let list: Vec<Entry> = std::fs::read_to_string(dir().join("history.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let next = list.iter().map(|e| e.id).max().unwrap_or(0) + 1;
    NEXT.store(next, Ordering::SeqCst);
    *HISTORY.lock().unwrap() = list;
}

/// Wpisy od najnowszego; przypięte na górze.
pub fn list() -> Vec<Entry> {
    let mut v = HISTORY.lock().unwrap().clone();
    v.sort_by_key(|e| (!e.pinned, std::cmp::Reverse(e.id)));
    v
}

/// Odczyt bieżącej zawartości schowka: pliki, obraz albo tekst (w tej kolejności, jak Windows).
fn read_clipboard() -> Option<Kind> {
    let _clip = Clipboard::new_attempts(10).ok()?;
    let mut files: Vec<String> = Vec::new();
    if formats::FileList.read_clipboard(&mut files).is_ok() && !files.is_empty() {
        return Some(Kind::Files(files));
    }
    let mut bmp: Vec<u8> = Vec::new();
    if formats::Bitmap.read_clipboard(&mut bmp).is_ok() && !bmp.is_empty() {
        let img = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok()?;
        let name = format!("{}.png", now() * 1000 + NEXT.load(Ordering::SeqCst) % 1000);
        let _ = std::fs::create_dir_all(dir());
        img.save(dir().join(&name)).ok()?;
        return Some(Kind::Image(name));
    }
    let mut text = String::new();
    if formats::Unicode.read_clipboard(&mut text).is_ok() && !text.trim().is_empty() && text.len() <= MAX_TEXT {
        return Some(Kind::Text(text));
    }
    None
}

fn on_update() {
    if OURS.swap(false, Ordering::SeqCst) {
        return;
    }
    let Some(kind) = read_clipboard() else { return };
    {
        let mut h = HISTORY.lock().unwrap();
        // To samo co ostatnio (np. ponowne Ctrl+C) – tylko przesuwamy na górę.
        if let Some(i) = h.iter().position(|e| e.kind == kind) {
            let mut e = h.remove(i);
            e.id = NEXT.fetch_add(1, Ordering::SeqCst);
            e.time = now();
            h.push(e);
        } else {
            h.push(Entry { id: NEXT.fetch_add(1, Ordering::SeqCst), kind, time: now(), pinned: false });
        }
        trim(&mut h);
        save(&h);
    }
    if let Some(f) = ON_CHANGE.get() {
        f();
    }
}

/// Najstarsze nieprzypięte ponad limit – usuwane (z plikami obrazów).
fn trim(h: &mut Vec<Entry>) {
    while h.iter().filter(|e| !e.pinned).count() > LIMIT {
        let Some(i) = h.iter().position(|e| !e.pinned) else { break };
        let e = h.remove(i);
        if let Kind::Image(name) = e.kind {
            let _ = std::fs::remove_file(dir().join(name));
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        on_update();
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// Startuje nasłuch schowka (własny wątek z oknem komunikatów); `on_change` – po nowym wpisie.
pub fn start(on_change: impl Fn() + Send + Sync + 'static) {
    if ON_CHANGE.set(Box::new(on_change)).is_err() {
        return;
    }
    load();
    std::thread::spawn(|| unsafe {
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), lpszClassName: w!("PowerDeskClipboard"), ..Default::default() };
        RegisterClassW(&class);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PowerDeskClipboard"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None) else { return };
        let _ = AddClipboardFormatListener(hwnd);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
        }
    });
}

/// Wpis z powrotem do schowka (bez dopisywania go jako nowy).
pub fn restore(id: u64) -> bool {
    let Some(e) = HISTORY.lock().unwrap().iter().find(|e| e.id == id).cloned() else { return false };
    OURS.store(true, Ordering::SeqCst);
    let ok = match &e.kind {
        Kind::Text(t) => crate::clip::set_text(t).is_ok(),
        Kind::Files(f) => crate::clip::set_files(f, false).is_ok(),
        Kind::Image(name) => (|| {
            let img = image::open(dir().join(name)).ok()?;
            let mut bmp = std::io::Cursor::new(Vec::new());
            img.write_to(&mut bmp, image::ImageFormat::Bmp).ok()?;
            let _clip = Clipboard::new_attempts(10).ok()?;
            formats::Bitmap.write_clipboard(bmp.get_ref()).ok()
        })()
        .is_some(),
    };
    if !ok {
        OURS.store(false, Ordering::SeqCst);
    }
    ok
}

/// Ctrl+V w bieżące okno – po przywróceniu wpisu i powrocie fokusu.
pub fn paste() {
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } },
    };
    let v = VIRTUAL_KEY(b'V' as u16);
    unsafe {
        SendInput(&[key(VK_CONTROL, false), key(v, false), key(v, true), key(VK_CONTROL, true)], std::mem::size_of::<INPUT>() as i32);
    }
}

fn change(f: impl FnOnce(&mut Vec<Entry>)) {
    let mut h = HISTORY.lock().unwrap();
    f(&mut h);
    save(&h);
}

pub fn toggle_pin(id: u64) {
    change(|h| {
        if let Some(e) = h.iter_mut().find(|e| e.id == id) {
            e.pinned = !e.pinned;
        }
    });
}

pub fn remove(id: u64) {
    change(|h| {
        if let Some(i) = h.iter().position(|e| e.id == id) {
            if let Kind::Image(name) = h.remove(i).kind {
                let _ = std::fs::remove_file(dir().join(name));
            }
        }
    });
}

/// Wyczyść wszystko poza przypiętymi (jak w Windows).
pub fn clear() {
    change(|h| {
        for e in h.iter().filter(|e| !e.pinned) {
            if let Kind::Image(name) = &e.kind {
                let _ = std::fs::remove_file(dir().join(name));
            }
        }
        h.retain(|e| e.pinned);
    });
}
