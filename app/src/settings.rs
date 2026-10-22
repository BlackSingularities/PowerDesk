//! Ekran ustawień (SettingsView.svelte) na karcie `ofp:settings`: motyw i akcent, start,
//! domyślny widok, przeglądanie, podgląd, potwierdzenia, skróty i informacje o programie.
//! Każda zmiana od razu działa i trafia do prefs.json.

use slint::{Color, ComponentHandle};

use crate::model::{Prefs, View};
use crate::{SettingsState, Shared, Theme};

pub const TAB: &str = "ofp:settings";

fn with_ui(s: &Shared, f: impl FnOnce(&SettingsState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<SettingsState>());
    }
}

/// Otwiera ustawienia: przechodzi do ich karty albo dodaje nową.
pub fn open(s: &Shared) {
    let at = s.borrow().tabs.iter().position(|t| t.path == TAB);
    match at {
        Some(i) => {
            crate::save_scroll(s);
            {
                let mut b = s.borrow_mut();
                b.active = i;
                let f = b.focused;
                b.panes[f] = i;
            }
            crate::show_tab(s);
        }
        None => crate::new_tab_at(s, TAB, true),
    }
}

/// Wołane przy każdym odświeżeniu okna: ekran widać tylko na jego karcie.
pub fn sync(s: &Shared) {
    let on = crate::tab_mut(s, |t| t.path == TAB);
    let was = s.borrow().ui.upgrade().is_some_and(|u| u.global::<SettingsState>().get_open());
    with_ui(s, |g| g.set_open(on));
    if on && !was {
        render(s);
    }
}

fn hex(c: &str) -> Option<Color> {
    let c = c.strip_prefix('#')?;
    let v = u32::from_str_radix(c, 16).ok().filter(|_| c.len() == 6)?;
    Some(Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Motyw z ustawień; przy pierwszym uruchomieniu – z ustawień wersji webowej (state.json).
fn theme_of(p: &Prefs) -> String {
    if !p.theme.is_empty() {
        return p.theme.clone();
    }
    dirs::data_dir()
        .map(|d| d.join("OpenFilePowerWin").join("state.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["settings"]["theme"].as_str().map(String::from))
        .unwrap_or_else(|| "system".into())
}

/// Motyw i akcent na całe okno.
pub fn apply_look(ui: &crate::MainWindow, p: &Prefs) {
    let light = match theme_of(p).as_str() {
        "light" => true,
        "dark" => false,
        _ => crate::system_light(),
    };
    let t = ui.global::<Theme>();
    t.set_light(light);
    let accent = match p.accent.as_str() {
        "system" => powerdesk_core::system::accent_color().and_then(|c| hex(&c)),
        c => hex(c),
    };
    t.set_accent_override(accent.unwrap_or(Color::from_argb_u8(0, 0, 0, 0)));
    let effect = effect_of(p);
    t.set_effect(effect as i32);
    t.set_popup_alpha(p.popup_opacity.clamp(60.0, 100.0) / 100.0);
    powerdesk_core::system::set_backdrop(window_hwnd(ui), effect);
}

fn effect_of(p: &Prefs) -> u32 {
    match p.window_effect.as_str() {
        "mica" => 1,
        "acrylic" => 2,
        _ => 0,
    }
}

fn window_hwnd(ui: &crate::MainWindow) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    ui.window()
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => 0,
        })
        .unwrap_or(0)
}

fn prefs_path() -> String {
    dirs::data_dir().map(|d| d.join("PowerDesk").join("prefs.json").to_string_lossy().to_string()).unwrap_or_default()
}

fn render(s: &Shared) {
    let b = s.borrow();
    let p = &b.prefs;
    let theme = match theme_of(p).as_str() {
        "dark" => 1,
        "light" => 2,
        _ => 0,
    };
    let sys = powerdesk_core::system::accent_color().and_then(|c| hex(&c)).unwrap_or(Color::from_rgb_u8(0x4f, 0x8c, 0xff));
    let Some(ui) = b.ui.upgrade() else { return };
    let g = ui.global::<SettingsState>();
    g.set_theme(theme);
    g.set_accent(p.accent.clone().into());
    g.set_effect(effect_of(p) as i32);
    g.set_popup_opacity(p.popup_opacity.clamp(60.0, 100.0));
    g.set_system_accent(sys);
    g.set_start(match p.start.as_str() {
        "computer" => 1,
        "folder" => 2,
        _ => 0,
    });
    g.set_start_folder(p.start_folder.clone().into());
    g.set_default_view(p.default_view.index());
    g.set_show_hidden(p.show_hidden);
    g.set_folders_first(p.folders_first);
    g.set_auto_gallery(p.auto_gallery);
    g.set_folder_sizes(p.folder_sizes);
    g.set_preview_open(p.preview_open);
    g.set_viewer_info(p.viewer_info);
    g.set_confirm_trash(p.confirm_trash);
    g.set_win_e(powerdesk_core::winkey::enabled());
    g.set_remembered(p.folders.len() as i32);
    g.set_version(env!("CARGO_PKG_VERSION").into());
    g.set_prefs_path(prefs_path().into());
}

/// Zmiana ustawień, zapis i odświeżenie ekranu.
fn change(s: &Shared, f: impl FnOnce(&mut Prefs)) {
    {
        let mut b = s.borrow_mut();
        f(&mut b.prefs);
        b.prefs.save();
    }
    render(s);
}

/// Wygląd całego okna od nowa (także tło DWM – wymaga już utworzonego okna).
pub fn relook(s: &Shared) {
    let b = s.borrow();
    if let Some(ui) = b.ui.upgrade() {
        apply_look(&ui, &b.prefs);
    }
}

pub fn set_theme(s: &Shared, i: i32) {
    change(s, |p| p.theme = ["system", "dark", "light"][i.clamp(0, 2) as usize].into());
    relook(s);
}

pub fn set_effect(s: &Shared, i: i32) {
    change(s, |p| p.window_effect = ["none", "mica", "acrylic"][i.clamp(0, 2) as usize].into());
    relook(s);
}

pub fn set_popup_opacity(s: &Shared, v: f32) {
    change(s, |p| p.popup_opacity = v.clamp(60.0, 100.0));
    relook(s);
}

pub fn set_accent(s: &Shared, a: &str) {
    change(s, |p| p.accent = a.to_string());
    relook(s);
}

pub fn set_start(s: &Shared, i: i32) {
    change(s, |p| p.start = ["home", "computer", "folder"][i.clamp(0, 2) as usize].into());
}

pub fn pick_start_folder(s: &Shared) {
    let start = s.borrow().prefs.start_folder.clone();
    let picked = powerdesk_core::shell::pick_folder(crate::hwnd(s), "Folder po uruchomieniu", (!start.is_empty()).then_some(start.as_str()));
    if let Some(p) = picked {
        change(s, |x| {
            x.start_folder = p;
            x.start = "folder".into();
        });
    }
}

pub fn set_default_view(s: &Shared, i: i32) {
    let v = match i {
        1 => View::Grid,
        2 => View::Gallery,
        _ => View::List,
    };
    change(s, |p| p.default_view = v);
}

pub fn toggle(s: &Shared, key: &str) {
    match key {
        "preview" => {
            crate::preview::toggle(s);
            render(s);
            return;
        }
        "show-hidden" => change(s, |p| p.show_hidden = !p.show_hidden),
        "folders-first" => change(s, |p| p.folders_first = !p.folders_first),
        "auto-gallery" => change(s, |p| p.auto_gallery = !p.auto_gallery),
        "folder-sizes" => change(s, |p| p.folder_sizes = !p.folder_sizes),
        "viewer-info" => change(s, |p| p.viewer_info = !p.viewer_info),
        "confirm-trash" => change(s, |p| p.confirm_trash = !p.confirm_trash),
        // Stan żyje w rejestrze, nie w prefs.json.
        "win-e" => {
            let on = !powerdesk_core::winkey::enabled();
            match powerdesk_core::winkey::set(on) {
                Ok(()) => crate::transfers::toast(s, if on { "Win+E otwiera teraz PowerDesk" } else { "Win+E znowu otwiera Eksplorator" }, crate::transfers::Kind::Ok, None),
                Err(e) => crate::transfers::toast(s, format!("Nie udało się zmienić Win+E: {e}"), crate::transfers::Kind::Error, None),
            }
            with_ui(s, |g| g.set_win_e(powerdesk_core::winkey::enabled()));
            return;
        }
        _ => return,
    }
    // Karty z folderami wczytają się od nowa przy następnym wejściu.
    if matches!(key, "show-hidden" | "folders-first" | "folder-sizes") {
        for t in s.borrow_mut().tabs.iter_mut().filter(|t| !t.special()) {
            t.loaded = false;
        }
    }
}

pub fn forget_views(s: &Shared) {
    change(s, |p| p.folders.clear());
}

/// Ścieżka startowa według ustawień (przypięte karty dochodzą osobno).
pub fn start_path(p: &Prefs) -> String {
    let home = || dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "C:\\".into());
    match p.start.as_str() {
        "computer" => crate::model::HOME.to_string(),
        "folder" if std::path::Path::new(&p.start_folder).is_dir() => p.start_folder.clone(),
        _ => home(),
    }
}

pub fn bind(ui: &crate::MainWindow, st: &Shared) {
    let g = ui.global::<SettingsState>();
    let s = st.clone();
    g.on_set_theme(move |i| set_theme(&s, i));
    let s = st.clone();
    g.on_set_accent(move |a| {
        let a = a.trim();
        // Własny kolor tylko jako #rrggbb – literówka nie psuje wyglądu.
        if a == "mono" || a == "system" || hex(a).is_some() {
            set_accent(&s, a);
        }
    });
    let s = st.clone();
    g.on_set_effect(move |i| set_effect(&s, i));
    let s = st.clone();
    g.on_set_popup_opacity(move |v| set_popup_opacity(&s, v));
    let s = st.clone();
    g.on_set_start(move |i| set_start(&s, i));
    let s = st.clone();
    g.on_pick_start_folder(move || pick_start_folder(&s));
    let s = st.clone();
    g.on_set_default_view(move |i| set_default_view(&s, i));
    let s = st.clone();
    g.on_toggle(move |k| toggle(&s, &k));
    let s = st.clone();
    g.on_forget_views(move || forget_views(&s));
    g.on_open_prefs_folder(|| {
        if let Some(d) = dirs::data_dir() {
            let _ = open::that_detached(d.join("PowerDesk"));
        }
    });
}
