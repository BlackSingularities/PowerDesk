//! Schowek systemowy zgodny z Eksploratorem: lista plików + „Preferred DropEffect” (wytnij/kopiuj).

use clipboard_win::{formats, raw, Clipboard, Getter, Setter};

pub fn set_files(paths: &[String], cut: bool) -> Result<(), String> {
    let _c = Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
    raw::empty().map_err(|e| e.to_string())?;
    formats::FileList.write_clipboard(&paths).map_err(|e| e.to_string())?;
    if let Some(f) = clipboard_win::register_format("Preferred DropEffect") {
        let effect: u32 = if cut { 2 } else { 5 };
        let _ = raw::set_without_clear(f.get(), &effect.to_le_bytes());
    }
    Ok(())
}

/// Pliki w schowku i czy zostały wycięte.
pub fn get_files() -> (Vec<String>, bool) {
    let mut paths: Vec<String> = Vec::new();
    let mut cut = false;
    if let Ok(_c) = Clipboard::new_attempts(10) {
        let _ = formats::FileList.read_clipboard(&mut paths);
        if let Some(f) = clipboard_win::register_format("Preferred DropEffect") {
            let mut buf = Vec::new();
            if raw::get_vec(f.get(), &mut buf).is_ok() && buf.len() >= 4 {
                cut = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) & 2 != 0;
            }
        }
    }
    (paths, cut)
}

pub fn set_text(text: &str) -> Result<(), String> {
    clipboard_win::set_clipboard_string(text).map_err(|e| e.to_string())
}
