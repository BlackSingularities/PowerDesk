//! Ekran ustawień (SettingsView.svelte) na karcie `ofp:settings`: motyw i akcent, start,
//! domyślny widok, przeglądanie, podgląd, potwierdzenia, skróty i informacje o programie.
//! Każda zmiana od razu działa i trafia do prefs.json.

use slint::{Color, ComponentHandle, Model, ModelRc, VecModel};

use crate::model::{Prefs, View};
use crate::{EdgeRow, SettingsState, Shared, SideSection, SysItem, Theme};

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

/// Numer sekcji „Pulpit” na liście po lewej (settings.slint).
pub const SECTION_DESKTOP: i32 = 5;
pub const SECTION_TASKBAR: i32 = 6;

/// Ustawienia otwarte od razu na podanej sekcji.
pub fn open_section(s: &Shared, section: i32) {
    with_ui(s, |g| g.set_section(section));
    open(s);
}

/// Wołane przy każdym odświeżeniu okna: ekran widać tylko na jego karcie.
pub fn sync(s: &Shared) {
    let on = crate::tab_mut(s, |t| t.path == TAB);
    let was = s.borrow().ui.upgrade().is_some_and(|u| u.global::<SettingsState>().get_open());
    with_ui(s, |g| g.set_open(on));
    if on && !was {
        render(s);
        render_system(s);
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

/// Jasny motyw i akcent z ustawień – dla okien bez głównego okna (wybór pliku dla innych programów).
pub fn light_accent(p: &Prefs) -> (bool, Color) {
    let light = match theme_of(p).as_str() {
        "light" => true,
        "dark" => false,
        _ => crate::system_light(),
    };
    let accent = match p.accent.as_str() {
        "system" => powerdesk_core::system::accent_color().and_then(|c| hex(&c)),
        c => hex(c),
    };
    (light, accent.unwrap_or(Color::from_argb_u8(0, 0, 0, 0)))
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
    t.set_popup_alpha(p.popup_opacity.clamp(30.0, 100.0) / 100.0);
    t.set_window_alpha(if p.window_opacity > 0.0 { p.window_opacity.clamp(20.0, 100.0) / 100.0 } else { 0.0 });
    // Acrylic jak w okienkach – rozmycie przez SetWindowCompositionAttribute (tło systemowe
    // Windows 11 na oknie bez ramki znikało); Mica zostaje systemowa.
    let hwnd = window_hwnd(ui);
    if effect == 2 {
        powerdesk_core::system::set_backdrop(hwnd, 0);
        powerdesk_core::system::see_through(hwnd);
        powerdesk_core::system::blur_behind(hwnd, true);
    } else {
        powerdesk_core::system::blur_behind(hwnd, false);
        powerdesk_core::system::set_backdrop(hwnd, effect);
    }
}

/// Wygląd głównego okna – globalne Slint są osobne dla każdego okna, więc pasek i pulpit
/// przepisują go od głównego.
pub struct Look {
    pub light: bool,
    pub accent: Color,
    /// Krycie okienek (menu, głośność, Start…) – to samo co w oknie.
    pub popup_alpha: f32,
}

pub fn look(ui: &crate::MainWindow) -> Look {
    let t = ui.global::<Theme>();
    Look { light: t.get_light(), accent: t.get_accent_override(), popup_alpha: t.get_popup_alpha() }
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
    g.set_popup_opacity(p.popup_opacity.clamp(30.0, 100.0));
    g.set_popup_blur(p.popup_blur);
    g.set_taskbar_opacity(p.taskbar_opacity.clamp(0.0, 100.0));
    g.set_taskbar_clear_desktop(p.taskbar_clear_desktop);
    g.set_taskbar_labels(p.taskbar_labels);
    g.set_taskbar_center(p.taskbar_center);
    g.set_taskbar_seconds(p.taskbar_seconds);
    g.set_taskbar_date(p.taskbar_date);
    g.set_taskbar_blur(p.taskbar_blur);
    g.set_tray_all(p.tray_all);
    g.set_taskbar_height(p.taskbar_height.round() as i32);
    g.set_taskbar_icon(p.taskbar_icon.round() as i32);
    g.set_start_image(p.start_image.clone().into());
    g.set_start_frames(p.start_frames as i32);
    g.set_desktop_icon_size(p.desktop_icon_size as i32);
    g.set_desktop_free(p.desktop_free);
    g.set_desktop_computer(p.desktop_computer);
    g.set_shell_mode(powerdesk_core::shellmode::active());
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
    g.set_single_click(p.single_click);
    let secs: Vec<SideSection> = SIDE_SECTIONS.iter().map(|n| SideSection { name: (*n).into(), on: !p.side_hidden.iter().any(|h| h == n) }).collect();
    g.set_side_sections(ModelRc::new(VecModel::from(secs)));
    let home: Vec<SideSection> = crate::home::SECTIONS.iter().map(|(k, t)| SideSection { name: (*t).into(), on: !p.home_hidden.iter().any(|h| h == k) }).collect();
    g.set_home_sections(ModelRc::new(VecModel::from(home)));
    g.set_window_opacity(p.window_opacity);
    g.set_close_to_tray(p.close_to_tray);
    g.set_autostart_window(p.autostart_window);
    g.set_autostart(p.autostart);
    g.set_wall_mode(match p.wallpaper_mode.as_str() {
        "slideshow" => 1,
        "bing" => 2,
        _ => 0,
    });
    g.set_wall_folder(p.wallpaper_folder.clone().into());
    g.set_zones_on(p.zones_on);
    g.set_vd_button(p.vd_button);
    g.set_taskbar_all(p.taskbar_all_monitors);
    g.set_zones_layout(p.zones_layout as i32);
    g.set_wall_minutes(p.wallpaper_minutes as i32);
    g.set_remembered(p.folders.len() as i32);
    g.set_version(env!("CARGO_PKG_VERSION").into());
    g.set_prefs_path(prefs_path().into());
    drop(b);
    render_overlay(s);
    refresh_edges(s);
}

/// Lista paneli przy krawędziach; przy tej samej liczbie paneli tylko podmienia wiersze
/// (przesuwany suwak szerokości nie traci myszy).
pub fn refresh_edges(s: &Shared) {
    let rows: Vec<EdgeRow> = s
        .borrow()
        .prefs
        .edge_panels
        .iter()
        .map(|e| EdgeRow {
            name: std::path::Path::new(&e.folder).file_name().map_or_else(|| e.folder.clone(), |n| n.to_string_lossy().to_string()).into(),
            path: e.folder.clone().into(),
            monitor: e.monitor as i32,
            edge: e.edge as i32,
            width: e.width,
        })
        .collect();
    let monitors = crate::desktop::windows().len().max(1) as i32;
    with_ui(s, |g| {
        g.set_edge_monitors(monitors);
        let model = g.get_edge_rows();
        if model.row_count() == rows.len() && rows.len() > 0 {
            for (i, r) in rows.into_iter().enumerate() {
                if model.row_data(i).as_ref() != Some(&r) {
                    model.set_row_data(i, r);
                }
            }
        } else {
            g.set_edge_rows(ModelRc::new(VecModel::from(rows)));
        }
    });
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
    {
        let b = s.borrow();
        if let Some(ui) = b.ui.upgrade() {
            apply_look(&ui, &b.prefs);
        }
    }
    // Paski i ich okienka mają własne kopie motywu – od razu nowy wygląd.
    crate::taskbar::relook();
}

/// Motyw okienka (jasny/ciemny, akcent, krycie) jak w głównym oknie – jedno źródło dla
/// wszystkich wyskakujących okienek.
pub fn popup_theme(t: &Theme) {
    crate::with_state(|s| {
        if let Some(ui) = s.borrow().ui.upgrade() {
            let m = ui.global::<Theme>();
            t.set_light(m.get_light());
            t.set_accent_override(m.get_accent_override());
            t.set_popup_alpha(m.get_popup_alpha());
        }
    });
}

/// Zakończenie PowerDesk – tylko z Ustawień → System. 0 – uruchom ponownie, 1 – zakończ,
/// 2 – przełącz na Eksplorator (tryb powłoki: wpis zdjęty, strażnik uruchomi explorer.exe).
fn app_quit(s: &Shared, what: i32) {
    use std::os::windows::process::CommandExt;
    let shell = powerdesk_core::shellmode::active();
    match what {
        // Jako powłoka strażnik i tak uruchamia PowerDesk od nowa; inaczej osobny proces czeka,
        // aż ten się zakończy (pasek i pulpit Windows wracają na chwilę), i uruchamia nową kopię.
        0 if !shell => {
            if let Ok(me) = std::env::current_exe() {
                let pid = std::process::id();
                let script = format!(
                    "/c (for /l %i in (1,1,30) do (tasklist /fi \"PID eq {pid}\" | find \"{pid}\" >nul && timeout /t 1 /nobreak >nul)) & start \"\" \"{}\" {}",
                    me.display(),
                    powerdesk_core::autostart::FLAG
                );
                let _ = std::process::Command::new("cmd").raw_arg(script).creation_flags(0x0800_0000).spawn();
            }
        }
        2 if shell => {
            if let Err(e) = powerdesk_core::shellmode::unregister() {
                crate::transfers::toast(s, format!("Nie udało się zdjąć wpisu powłoki: {e}"), crate::transfers::Kind::Error, None);
                return;
            }
        }
        _ => {}
    }
    crate::quit(s);
}

/// Rozmycie tła pod okienkami (ustawienie).
pub fn popup_blur() -> bool {
    let mut on = true;
    crate::with_state(|s| on = s.borrow().prefs.popup_blur);
    on
}

pub fn set_theme(s: &Shared, i: i32) {
    change(s, |p| p.theme = ["system", "dark", "light"][i.clamp(0, 2) as usize].into());
    relook(s);
}

pub fn set_effect(s: &Shared, i: i32) {
    change(s, |p| p.window_effect = ["none", "mica", "acrylic"][i.clamp(0, 2) as usize].into());
    relook(s);
}

pub fn set_taskbar_opacity(s: &Shared, v: f32) {
    change(s, |p| p.taskbar_opacity = v.clamp(0.0, 100.0));
}

pub fn set_popup_opacity(s: &Shared, v: f32) {
    change(s, |p| p.popup_opacity = v.clamp(30.0, 100.0));
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
        "popup-blur" => {
            change(s, |p| p.popup_blur = !p.popup_blur);
            crate::taskbar::relook();
        }
        "folder-sizes" => change(s, |p| p.folder_sizes = !p.folder_sizes),
        "viewer-info" => change(s, |p| p.viewer_info = !p.viewer_info),
        "confirm-trash" => change(s, |p| p.confirm_trash = !p.confirm_trash),
        // Pasek czyta ustawienie przy każdym odświeżeniu (co sekundę).
        "taskbar-clear-desktop" => change(s, |p| p.taskbar_clear_desktop = !p.taskbar_clear_desktop),
        "taskbar-labels" | "taskbar-center" | "taskbar-seconds" | "taskbar-date" | "taskbar-blur" | "tray-all" => {
            change(s, |p| match key {
                "taskbar-labels" => p.taskbar_labels = !p.taskbar_labels,
                "taskbar-center" => p.taskbar_center = !p.taskbar_center,
                "taskbar-seconds" => p.taskbar_seconds = !p.taskbar_seconds,
                "taskbar-date" => p.taskbar_date = !p.taskbar_date,
                "taskbar-blur" => p.taskbar_blur = !p.taskbar_blur,
                _ => p.tray_all = !p.tray_all,
            });
            crate::taskbar::restyle();
        }
        "desktop-free" | "desktop-computer" => {
            change(s, |p| {
                if key == "desktop-free" {
                    p.desktop_free = !p.desktop_free;
                } else {
                    p.desktop_computer = !p.desktop_computer;
                }
            });
            crate::desktop::relayout();
        }
        "single-click" => change(s, |p| p.single_click = !p.single_click),
        "close-to-tray" => change(s, |p| p.close_to_tray = !p.close_to_tray),
        "autostart-window" => change(s, |p| p.autostart_window = !p.autostart_window),
        "autostart" => {
            change(s, |p| p.autostart = !p.autostart);
            crate::ensure_integration();
        }
        "overlay" => {
            crate::overlay::toggle();
            return;
        }
        "overlay-full" => change(s, |p| p.overlay_full = !p.overlay_full),
        "zones" => change(s, |p| p.zones_on = !p.zones_on),
        // Pasek czyta ustawienie przy każdym odświeżeniu (co sekundę).
        "vd-button" => change(s, |p| p.vd_button = !p.vd_button),
        // Paski od nowa – na wszystkich monitorach albo tylko na głównym.
        "taskbar-all" => {
            change(s, |p| p.taskbar_all_monitors = !p.taskbar_all_monitors);
            crate::taskbar::close();
            crate::taskbar::open(s);
        }
        "game-power" => {
            change(s, |p| p.game_power = !p.game_power);
            crate::game::init();
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

/// Sekcja System: czy PowerDesk zastępuje Eksplorator (stan z rejestru, czytany w tle).
pub fn render_system(_s: &Shared) {
    std::thread::spawn(|| {
        let rows = powerdesk_core::integration::status();
        let inst = powerdesk_core::install::installed();
        let here = powerdesk_core::install::running_installed();
        let _ = slint::invoke_from_event_loop(move || {
            crate::with_state(|s| {
                let items: Vec<SysItem> = rows.into_iter().map(|(n, d, ok)| SysItem { name: n.into(), detail: d.into(), ok }).collect();
                with_ui(s, |g| {
                    g.set_system(ModelRc::new(VecModel::from(items)));
                    g.set_installed(inst.is_some());
                    if let Some((exe, all)) = &inst {
                        let who = if *all { "dla wszystkich użytkowników" } else { "dla bieżącego użytkownika" };
                        let note = if here { "" } else { " · działa inna kopia (bez instalacji)" };
                        g.set_install_place(format!("Zainstalowany {who}: {}{note}", exe.display()).into());
                    }
                });
            })
        });
    });
}

/// Sekcje panelu bocznego, które da się wyłączyć (nazwy nagłówków z main.rs).
pub const SIDE_SECTIONS: [&str; 7] = ["Zapisane wyszukiwania", "Szybki dostęp", "Przypięte", "Biblioteki", "Dyski", "Sieć", "Synchronizacja"];

pub fn toggle_side(s: &Shared, name: &str) {
    change(s, |p| match p.side_hidden.iter().position(|h| h == name) {
        Some(i) => {
            p.side_hidden.remove(i);
        }
        None => p.side_hidden.push(name.to_string()),
    });
    crate::render_side(s);
}

/// Sekcja ekranu „Komputer” włączona albo wyłączona (po nagłówku z listy w Ustawieniach).
pub fn toggle_home(s: &Shared, title: &str) {
    let Some(key) = crate::home::SECTIONS.iter().find(|(_, t)| *t == title).map(|(k, _)| k.to_string()) else { return };
    change(s, |p| match p.home_hidden.iter().position(|h| *h == key) {
        Some(i) => {
            p.home_hidden.remove(i);
        }
        None => p.home_hidden.push(key),
    });
    crate::home::render_sections(s);
}

pub fn set_window_opacity(s: &Shared, v: f32) {
    change(s, |p| p.window_opacity = v.clamp(20.0, 100.0));
    relook(s);
}

/// Sekcja Nakładka: przełączniki i stan pomiaru FPS.
pub fn render_overlay(s: &Shared) {
    use powerdesk_core::fpsmon::State;
    let (on, corner, full, game, limit) = {
        let b = s.borrow();
        (crate::overlay::is_open(), b.prefs.overlay_corner as i32, b.prefs.overlay_full, b.prefs.game_power, b.prefs.fps_limit as i32)
    };
    let st = powerdesk_core::fpsmon::state();
    let (text, need) = match st {
        State::Running => ("Działa.".to_string(), false),
        State::Off => ("Włącza się razem z nakładką.".to_string(), false),
        State::NoAccess => ("Brak uprawnień: pomiar FPS wymaga administratora albo członkostwa w grupie „Użytkownicy dzienników wydajności”. Reszta nakładki działa.".to_string(), true),
        State::Error(e) => (format!("Nie działa: {e}"), false),
    };
    with_ui(s, |g| {
        g.set_overlay_on(on);
        g.set_overlay_corner(corner);
        g.set_overlay_full(full);
        g.set_fps_status(text.into());
        g.set_fps_need_access(need);
        g.set_game_power(game);
        g.set_fps_limit(limit);
    });
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
    g.on_repair_integration(crate::ensure_integration);
    g.on_install(crate::setup::install_from_settings);
    let s = st.clone();
    g.on_set_fps_limit(move |f| {
        let fps = f.max(0) as u32;
        match powerdesk_core::gamemode::set_fps_limit(fps) {
            Ok(()) => {
                change(&s, |p| p.fps_limit = fps);
                let msg = if fps == 0 { "Limit klatek wyłączony".to_string() } else { format!("Limit klatek: {fps} – działa w grach uruchomionych od teraz") };
                crate::transfers::toast(&s, msg, crate::transfers::Kind::Ok, None);
            }
            Err(e) => {
                powerdesk_core::log::error(format!("limit klatek: {e}"));
                crate::transfers::toast(&s, format!("Nie udało się ustawić limitu klatek: {e}"), crate::transfers::Kind::Error, None);
            }
        }
    });
    let s = st.clone();
    g.on_set_zones_layout(move |k| change(&s, |p| p.zones_layout = k.clamp(0, 4) as u8));
    let s = st.clone();
    g.on_set_wall_mode(move |m| {
        let mode = ["", "slideshow", "bing"][m.clamp(0, 2) as usize];
        // Pokaz slajdów bez folderu – od razu wybór folderu (domyślnie Obrazy).
        if mode == "slideshow" && s.borrow().prefs.wallpaper_folder.is_empty() {
            let pics = dirs::picture_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            change(&s, |p| p.wallpaper_folder = pics);
        }
        crate::wallpapers::set_mode(mode);
        render(&s);
    });
    let s = st.clone();
    g.on_pick_wall_folder(move || {
        let cur = s.borrow().prefs.wallpaper_folder.clone();
        if let Some(p) = powerdesk_core::shell::pick_folder(crate::hwnd(&s), "Folder ze zdjęciami na tapetę", (!cur.is_empty()).then_some(cur.as_str())) {
            change(&s, |x| x.wallpaper_folder = p);
            crate::wallpapers::refresh();
        }
    });
    let s = st.clone();
    g.on_set_wall_minutes(move |m| {
        change(&s, |p| p.wallpaper_minutes = m.max(1) as u32);
        crate::wallpapers::start_timer();
    });
    let s = st.clone();
    g.on_report_problem(move || {
        let text = powerdesk_core::log::report(powerdesk_core::update::current());
        let _ = powerdesk_core::clip::set_text(&text);
        let _ = open::that_detached(powerdesk_core::log::dir());
        crate::transfers::toast(&s, "Raport skopiowany do schowka – wklej go w zgłoszeniu; obok otwarty folder z dziennikiem", crate::transfers::Kind::Ok, None);
    });
    let s = st.clone();
    g.on_set_overlay_corner(move |k| change(&s, |p| p.overlay_corner = k.clamp(0, 3) as u8));
    let s = st.clone();
    g.on_grant_fps(move || {
        match powerdesk_core::fpsmon::grant_access() {
            Ok(()) => crate::transfers::toast(&s, "Gotowe po ponownym zalogowaniu – wtedy nakładka pokaże FPS", crate::transfers::Kind::Ok, None),
            Err(e) => crate::transfers::toast(&s, e, crate::transfers::Kind::Error, None),
        }
    });
    g.on_uninstall(crate::setup::uninstall_from_settings);
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
    g.on_set_taskbar_opacity(move |v| set_taskbar_opacity(&s, v));
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
    let s = st.clone();
    g.on_toggle_side(move |n| toggle_side(&s, &n));
    let s = st.clone();
    g.on_toggle_home(move |n| toggle_home(&s, &n));
    let s = st.clone();
    g.on_set_window_opacity(move |v| set_window_opacity(&s, v));
    let s = st.clone();
    g.on_set_taskbar_height(move |h| {
        change(&s, |p| p.taskbar_height = (h as f32).clamp(32.0, 72.0));
        crate::taskbar::restyle();
    });
    let s = st.clone();
    g.on_set_taskbar_icon(move |px| {
        change(&s, |p| p.taskbar_icon = (px as f32).clamp(16.0, 48.0));
        crate::taskbar::restyle();
    });
    let s = st.clone();
    g.on_pick_start_image(move || {
        if let Some(path) = powerdesk_core::shell::pick_image(crate::hwnd(&s), "Obraz przycisku Start") {
            change(&s, |p| {
                p.start_image = path;
                p.start_frames = 0;
            });
            crate::taskbar::restyle();
        }
    });
    let s = st.clone();
    g.on_clear_start_image(move || {
        change(&s, |p| p.start_image.clear());
        crate::taskbar::restyle();
    });
    let s = st.clone();
    g.on_set_start_frames(move |n| {
        change(&s, |p| p.start_frames = n.clamp(0, 4) as u8);
        crate::taskbar::restyle();
    });
    let s = st.clone();
    g.on_set_desktop_icon_size(move |n| {
        change(&s, |p| p.desktop_icon_size = n.clamp(0, 2) as u8);
        crate::desktop::relayout();
    });
    let s = st.clone();
    g.on_edge_add(move |e| crate::edgepanels::pick_and_add(&s, 0, e.clamp(0, 2) as u8));
    let s = st.clone();
    g.on_edge_remove(move |i| crate::edgepanels::remove(&s, i as usize));
    let s = st.clone();
    g.on_edge_set_edge(move |i, e| crate::edgepanels::edit(&s, i as usize, |p| p.edge = e.clamp(0, 2) as u8));
    let s = st.clone();
    g.on_edge_set_monitor(move |i, m| crate::edgepanels::edit(&s, i as usize, |p| p.monitor = m.max(0) as usize));
    let s = st.clone();
    g.on_edge_set_width(move |i, w| crate::edgepanels::edit(&s, i as usize, |p| p.width = w.clamp(160.0, 900.0)));
    let s = st.clone();
    g.on_app_quit(move |what| app_quit(&s, what));
    g.on_open_prefs_folder(|| {
        if let Some(d) = dirs::data_dir() {
            let _ = open::that_detached(d.join("PowerDesk"));
        }
    });
}
