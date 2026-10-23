//! Ustawienia z OpenFilePower (wersja Tauri, `%APPDATA%\OpenFilePowerWin\state.json`) przenoszone
//! do PowerDesk – raz, przy instalacji albo przy pierwszym starcie bez własnych ustawień.

use serde_json::Value;

use crate::model::{Prefs, View};

fn state() -> Option<Value> {
    let p = dirs::data_dir()?.join("OpenFilePowerWin").join("state.json");
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// Przenosi to, co obie wersje mają wspólne. Zwraca true, gdy coś przeniesiono.
pub fn from_openfilepower(p: &mut Prefs) -> bool {
    if p.migrated_ofp {
        return false;
    }
    p.migrated_ofp = true;
    let Some(v) = state() else { return false };
    let s = &v["settings"];
    let b = |k: &str, out: &mut bool| {
        if let Some(x) = s[k].as_bool() {
            *out = x;
        }
    };
    let f = |k: &str, out: &mut f32| {
        if let Some(x) = s[k].as_f64() {
            *out = x as f32;
        }
    };
    b("showHidden", &mut p.show_hidden);
    b("foldersFirst", &mut p.folders_first);
    b("confirmTrash", &mut p.confirm_trash);
    b("closeToTray", &mut p.close_to_tray);
    b("previewPane", &mut p.preview_open);
    b("autoGallery", &mut p.auto_gallery);
    b("singleClickOpen", &mut p.single_click);
    b("folderSizes", &mut p.folder_sizes);
    b("shelfOpen", &mut p.shelf_open);
    f("sidebarWidth", &mut p.side_width);
    f("previewWidth", &mut p.preview_width);
    f("gridSize", &mut p.grid_size);
    f("galleryRow", &mut p.gallery_row);
    f("popupOpacity", &mut p.popup_opacity);
    if let Some(t) = s["theme"].as_str() {
        p.theme = t.to_string();
    }
    if let Some(a) = s["accent"].as_str() {
        p.accent = a.to_string();
    }
    if let Some(e) = s["windowEffect"].as_str() {
        p.window_effect = e.to_string();
    }
    p.default_view = match s["defaultView"].as_str() {
        Some("grid") => View::Grid,
        Some("gallery") => View::Gallery,
        _ => p.default_view,
    };
    // Sekcje panelu bocznego: wyłączone w starej wersji – ukryte tutaj.
    let side = &s["sidebar"];
    for (key, name) in [("showSaved", "Zapisane wyszukiwania"), ("showFav", "Przypięte"), ("showLibs", "Biblioteki"), ("showDrives", "Dyski"), ("showNet", "Sieć"), ("showSync", "Synchronizacja")] {
        if side[key].as_bool() == Some(false) && !p.side_hidden.iter().any(|h| h == name) {
            p.side_hidden.push(name.to_string());
        }
    }
    if s["showQuickAccess"].as_bool() == Some(false) && !p.side_hidden.iter().any(|h| h == "Szybki dostęp") {
        p.side_hidden.push("Szybki dostęp".into());
    }
    for fav in v["favorites"].as_array().into_iter().flatten() {
        if let Some(path) = fav["path"].as_str() {
            if !p.favorites.iter().any(|x| x.eq_ignore_ascii_case(path)) {
                p.favorites.push(path.to_string());
            }
        }
    }
    for tab in v["tabs"].as_array().into_iter().flatten() {
        if tab["pinned"].as_bool() == Some(true) {
            if let Some(path) = tab["path"].as_str() {
                if !p.pinned_tabs.iter().any(|x| x.eq_ignore_ascii_case(path)) {
                    p.pinned_tabs.push(path.to_string());
                }
            }
        }
    }
    for e in v["shelf"].as_array().into_iter().flatten() {
        if let Some(path) = e["path"].as_str() {
            if !p.shelf.iter().any(|x| x.eq_ignore_ascii_case(path)) {
                p.shelf.push(path.to_string());
            }
        }
    }
    true
}
