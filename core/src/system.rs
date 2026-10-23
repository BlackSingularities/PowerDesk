//! Integracja z Windows: domyślny menedżer plików, pozycja w menu Eksploratora,
//! instalacja aktualizacji prosto z aplikacji.

use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;

use serde::Serialize;
use sha2::{Digest, Sha256};
use crate::events::{AppHandle, Emitter};

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
            let ok = reg(&["add", &key, "/ve", "/d", "Otwórz w OpenFilePower", "/f"])
                && reg(&["add", &key, "/v", "Icon", "/d", &exe(), "/f"])
                && reg(&["add", &format!(r"{key}\command"), "/ve", "/d", &cmd, "/f"]);
            if !ok {
                return Err("Nie udało się zapisać ustawień w rejestrze".into());
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
                return Err("Nie udało się zapisać ustawień w rejestrze".into());
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

// ---------- Aktualizacje ----------

#[derive(Serialize, Clone)]
struct Progress {
    done: u64,
    total: u64,
}

fn release(repo: &str) -> Result<serde_json::Value, String> {
    ureq::get(&format!("https://api.github.com/repos/{repo}/releases/latest"))
        .set("User-Agent", "OpenFilePowerWin")
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())
}

/// Pobiera instalator najnowszej wersji i sprawdza jego sumę SHA-256 z pliku SHA256SUMS.txt w wydaniu.
pub fn download_update(app: &AppHandle, repo: &str) -> Result<String, String> {
    let r = release(repo)?;
    let assets = r["assets"].as_array().ok_or("Wydanie nie ma plików")?;
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find(|a| a["name"].as_str().map(|n| pred(&n.to_lowercase())).unwrap_or(false)).cloned()
    };
    let setup = find(&|n| n.ends_with("-setup.exe")).ok_or("W wydaniu nie ma instalatora")?;
    let name = setup["name"].as_str().unwrap_or("setup.exe").to_string();
    let url = setup["browser_download_url"].as_str().ok_or("Brak adresu pobierania")?;
    let total = setup["size"].as_u64().unwrap_or(0);

    let expected = find(&|n| n == "sha256sums.txt").and_then(|s| {
        let u = s["browser_download_url"].as_str()?.to_string();
        let body = ureq::get(&u).set("User-Agent", "OpenFilePowerWin").call().ok()?.into_string().ok()?;
        body.lines().find(|l| l.ends_with(&name)).and_then(|l| l.split_whitespace().next()).map(|h| h.to_lowercase())
    });

    let dir = std::env::temp_dir().join("OpenFilePower").join("update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path: PathBuf = dir.join(&name);
    let resp = ureq::get(url).set("User-Agent", "OpenFilePowerWin").call().map_err(|e| e.to_string())?;
    let mut rd = resp.into_reader();
    let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    let mut last = std::time::Instant::now();
    loop {
        let n = rd.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        h.update(&buf[..n]);
        done += n as u64;
        if last.elapsed().as_millis() > 150 {
            let _ = app.emit("update-progress", Progress { done, total });
            last = std::time::Instant::now();
        }
    }
    let _ = app.emit("update-progress", Progress { done, total: total.max(done) });
    let got = format!("{:x}", h.finalize());
    match expected {
        Some(e) if e != got => {
            let _ = std::fs::remove_file(&path);
            Err("Suma kontrolna pobranego instalatora się nie zgadza – aktualizacja przerwana".into())
        }
        _ => Ok(path.to_string_lossy().to_string()),
    }
}

/// Uruchamia instalator po cichu i po zakończeniu otwiera aplikację ponownie.
pub fn install_update(app: &AppHandle, setup: &str) -> Result<(), String> {
    let me = exe();
    let script = format!("start \"\" /wait \"{setup}\" /S & start \"\" \"{me}\"");
    std::process::Command::new("cmd").raw_arg(format!("/c {script}")).creation_flags(NO_WINDOW).spawn().map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
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
