//! Integracja z Windows: domyślny menedżer plików, pozycja w menu Eksploratora,
//! okna, tło i kolory z Windows.

use std::os::windows::process::CommandExt;

use serde::Serialize;

const NO_WINDOW: u32 = 0x0800_0000;
const VERB: &str = "OpenFilePower";
/// Klucze, pod którymi Windows szuka akcji dla folderów i dysków (tylko bieżący użytkownik).
const TARGETS: [&str; 2] = [r"HKCU\Software\Classes\Directory\shell", r"HKCU\Software\Classes\Drive\shell"];
/// Instalacja dla wszystkich (HKLM); starsze wersje instalowały się tylko dla użytkownika (HKCU).
const UNINSTALL: [&str; 2] = [
    r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenFilePower",
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenFilePower",
];

fn reg(args: &[&str]) -> bool {
    std::process::Command::new("reg").args(args).creation_flags(NO_WINDOW).output().map(|o| o.status.success()).unwrap_or(false)
}

fn reg_query(key: &str, value: Option<&str>) -> Option<String> {
    let mut args = vec!["query", key];
    match value {
        Some(v) => args.extend(["/v", v]),
        None => args.push("/ve"),
    }
    let out = std::process::Command::new("reg").args(&args).creation_flags(NO_WINDOW).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    text.lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
}

#[derive(Serialize)]
pub struct Integration {
    /// Zainstalowana instalatorem (można się aktualizować sama), a nie wersja przenośna.
    installed: bool,
    default_fm: bool,
    context_menu: bool,
    exe: String,
}

pub fn integration() -> Integration {
    // Zainstalowana = uruchomiona z folderu instalacji (przenośna kopia obok instalacji nie może jej nadpisywać).
    let installed = UNINSTALL
        .iter()
        .find_map(|k| reg_query(k, Some("UninstallString")))
        .map(|u| u.trim_matches('"').to_lowercase())
        .and_then(|u| std::path::Path::new(&u).parent().map(|p| p.to_path_buf()))
        .map(|dir| std::path::Path::new(&exe().to_lowercase()).parent().map(|p| p == dir).unwrap_or(false))
        .unwrap_or(false);
    let ours = |k: &str| reg_query(&format!(r"{k}\{VERB}\command"), None).map(|c| c.to_lowercase().contains(&exe().to_lowercase())).unwrap_or(false);
    let context_menu = TARGETS.iter().all(|k| ours(k));
    let default_fm = context_menu && TARGETS.iter().all(|k| reg_query(k, None).as_deref() == Some(VERB));
    Integration { installed, default_fm, context_menu, exe: exe() }
}

/// Pozycja „Otwórz w OpenFilePower” w menu folderów i dysków.
pub fn set_context_menu(on: bool) -> Result<(), String> {
    for k in TARGETS {
        let key = format!(r"{k}\{VERB}");
        if on {
            let cmd = format!("\"{}\" \"%V\"", exe());
            let ok = reg(&["add", &key, "/ve", "/d", crate::lang::tr("Otwórz w OpenFilePower"), "/f"])
                && reg(&["add", &key, "/v", crate::lang::tr("Icon"), "/d", &exe(), "/f"])
                && reg(&["add", &format!(r"{key}\command"), "/ve", "/d", &cmd, "/f"]);
            if !ok {
                return Err(crate::lang::tr("Nie udało się zapisać ustawień w rejestrze").into());
            }
        } else {
            reg(&["delete", &key, "/f"]);
        }
    }
    Ok(())
}

/// Domyślna akcja dla folderów i dysków: otwieranie w tej aplikacji (Win+E nadal otwiera Eksplorator).
pub fn set_default(on: bool) -> Result<(), String> {
    if on {
        set_context_menu(true)?;
        for k in TARGETS {
            if !reg(&["add", k, "/ve", "/d", VERB, "/f"]) {
                return Err(crate::lang::tr("Nie udało się zapisać ustawień w rejestrze").into());
            }
        }
    } else {
        for k in TARGETS {
            // Pusta wartość domyślna = Windows wraca do Eksploratora.
            reg(&["delete", k, "/ve", "/f"]);
        }
    }
    Ok(())
}

/// Kolor akcentu Windows (#rrggbb) z ustawień personalizacji.
pub fn accent_color() -> Option<String> {
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\DWM", "/v", "AccentColor"])
        .creation_flags(NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let hex = text.lines().find_map(|l| l.split_once("REG_DWORD").map(|(_, v)| v.trim().trim_start_matches("0x").to_string()))?;
    // Zapis w rejestrze: 0xAABBGGRR.
    let v = u32::from_str_radix(&hex, 16).ok()?;
    Some(format!("#{:02x}{:02x}{:02x}", v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff))
}

/// Tło okna z Windows 11: 0 – pełne (bez efektu), 1 – Mica, 2 – Acrylic.
/// Okno musi rysować półprzezroczyste tło, żeby efekt było widać.
pub fn set_backdrop(hwnd: isize, kind: u32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE};
    use windows::Win32::UI::Controls::MARGINS;
    if hwnd == 0 {
        return;
    }
    let h = HWND(hwnd as *mut _);
    // DWMSBT_NONE = 1, DWMSBT_MAINWINDOW (Mica) = 2, DWMSBT_TRANSIENTWINDOW (Acrylic) = 3.
    let t: i32 = match kind {
        1 => 2,
        2 => 3,
        _ => 1,
    };
    let m = if kind == 0 { MARGINS::default() } else { MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 } };
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(h, &m);
        let _ = DwmSetWindowAttribute(h, DWMWA_SYSTEMBACKDROP_TYPE, &t as *const i32 as *const _, 4);
    }
}

/// Rozmycie tego, co pod oknem (akryl), niezależnie od ramki i fokusu – `SetWindowCompositionAttribute`
/// z user32 (nieopisane w dokumentacji, ale używane przez pasek i menu Windows). Tło systemowe
/// (DWMWA_SYSTEMBACKDROP_TYPE) na oknach bez ramki i bez fokusu nie działa – stąd ta droga.
/// Okno rysuje półprzezroczyste tło; kolor przyciemnienia daje okno, akcent ma tylko minimalny odcień.
pub fn blur_behind(hwnd: isize, on: bool) {
    use windows::core::s;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, GetModuleHandleW};
    #[repr(C)]
    struct Accent {
        state: u32,
        flags: u32,
        gradient: u32,
        animation: u32,
    }
    #[repr(C)]
    struct Data {
        attrib: u32,
        data: *mut core::ffi::c_void,
        size: usize,
    }
    type SetWca = unsafe extern "system" fn(HWND, *mut Data) -> i32;
    if hwnd == 0 {
        return;
    }
    unsafe {
        let Ok(user) = GetModuleHandleW(windows::core::w!("user32.dll")) else { return };
        let Some(f) = GetProcAddress(user, s!("SetWindowCompositionAttribute")) else { return };
        let f: SetWca = std::mem::transmute(f);
        // ACCENT_ENABLE_ACRYLICBLURBEHIND = 4, ACCENT_DISABLED = 0; gradient 0xAABBGGRR – prawie
        // przezroczysty, bo przyciemnienie rysuje samo okno (krycie okienek z ustawień).
        // Flaga 2 – rozmycie na całym oknie (także pod zaokrąglonymi rogami DWM).
        let mut a = Accent { state: if on { 4 } else { 0 }, flags: 2, gradient: 0x0100_0000, animation: 0 };
        // WCA_ACCENT_POLICY = 19.
        let mut d = Data { attrib: 19, data: &mut a as *mut Accent as *mut _, size: std::mem::size_of::<Accent>() };
        f(HWND(hwnd as *mut _), &mut d);
    }
}

/// Kosz: liczba elementów i łączny rozmiar (wszystkie dyski).
pub fn recycle_bin() -> (i64, i64) {
    use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    unsafe {
        if SHQueryRecycleBinW(windows::core::PCWSTR::null(), &mut info).is_ok() {
            (info.i64NumItems, info.i64Size)
        } else {
            (0, 0)
        }
    }
}

/// Opróżnia Kosz – Windows pyta o potwierdzenie i pokazuje postęp.
pub fn empty_recycle_bin() {
    use windows::Win32::UI::Shell::SHEmptyRecycleBinW;
    unsafe {
        let _ = SHEmptyRecycleBinW(None, windows::core::PCWSTR::null(), 0);
    }
}

/// Okno z przezroczystym tłem: DWM łączy kanał alfa okna z tym, co pod spodem (bez rozmycia).
pub fn see_through(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
    use windows::Win32::UI::Controls::MARGINS;
    if hwnd == 0 {
        return;
    }
    let m = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(HWND(hwnd as *mut _), &m);
    }
}

/// Wygląd okienek PowerDesk (menu, głośność, kalendarz, Start…) jak menu w oknie: zaokrąglone
/// rogi i cienka obwódka w kolorze linii (Windows 11 – DWM; starsze systemy pomijają).
/// `None` – bez obwódki, samo tło.
pub fn style_popup(hwnd: isize, border: Option<(u8, u8, u8)>) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE};
    if hwnd == 0 {
        return;
    }
    let h = HWND(hwnd as *mut _);
    // DWMWCP_ROUND = 2; kolor jako COLORREF (0x00BBGGRR).
    let corner: i32 = 2;
    // DWMWA_COLOR_NONE = 0xFFFFFFFE – DWM nie rysuje obwódki.
    let color: u32 = border.map_or(0xFFFF_FFFE, |b| b.0 as u32 | (b.1 as u32) << 8 | (b.2 as u32) << 16);
    unsafe {
        let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const i32 as *const _, 4);
        let _ = DwmSetWindowAttribute(h, DWMWA_BORDER_COLOR, &color as *const u32 as *const _, 4);
    }
}

/// Okno bez ramki DWM i z prostymi rogami (pasek zadań, pulpit) – Windows 11 rysuje wokół okien
/// jednopikselową obwódkę i zaokrągla rogi, co na pasku przy krawędzi ekranu widać jako linię.
pub fn no_frame_border(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE};
    if hwnd == 0 {
        return;
    }
    let h = HWND(hwnd as *mut _);
    // DWMWCP_DONOTROUND = 1, DWMWA_COLOR_NONE = 0xFFFFFFFE.
    let (corner, color): (i32, u32) = (1, 0xFFFF_FFFE);
    unsafe {
        let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const i32 as *const _, 4);
        let _ = DwmSetWindowAttribute(h, DWMWA_BORDER_COLOR, &color as *const u32 as *const _, 4);
    }
}
