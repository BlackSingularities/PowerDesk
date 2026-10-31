//! Okna „Otwórz” i „Zapisz jako” w innych programach: biblioteka `powerdesk-dialog` (skrzynka
//! `dialog/`, osadzona w pliku .exe przy budowie wydania) zarejestrowana dla bieżącego użytkownika
//! jako serwer klas CLSID_FileOpenDialog i CLSID_FileSaveDialog (`HKCU\Software\Classes` ma
//! pierwszeństwo przed HKLM). Biblioteka pokazuje okno PowerDesk (`powerdesk.exe --pick-file`),
//! a przy każdym problemie – oryginalne okno Windows.
//!
//! Każdy użytkownik ma własną kopię biblioteki w `%LOCALAPPDATA%\PowerDesk\FileDialog`: wpis
//! w rejestrze i plik powstają i znikają razem. Gdyby wpisy wskazywały plik w folderze programu,
//! deinstalacja (albo przez innego użytkownika) zostawiłaby wpisy bez pliku – a wtedy okna plików
//! przestają działać we wszystkich programach. Kopia bez `powerdesk.exe` po prostu oddaje okno
//! Windows.
//!
//! Programy 64-bitowe czytają `Software\Classes\CLSID`, 32-bitowe – `Software\Classes\WOW6432Node\CLSID`
//! (32-bitowa biblioteka, gdy wydanie ją zawiera). Procesy z uprawnieniami administratora wpisów
//! z HKCU nie widzą – mają zawsze okno Windows.

use std::path::{Path, PathBuf};

use crate::winkey::reg;

static DLL64: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dialog64.dll"));
static DLL32: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dialog32.dll"));

/// CLSID_FileOpenDialog i CLSID_FileSaveDialog.
const CLSIDS: [&str; 2] = ["{DC1C5A9C-E88A-4dde-A5A1-60F82A20AEF7}", "{C0B4E2F3-BA21-4773-8DBA-335EC946EB8B}"];
/// Tu biblioteka szuka `powerdesk.exe` (HKCU\Software nie jest przekierowywane dla 32 bitów).
const EXE_KEY: &str = r"HKCU\Software\PowerDesk";
const EXE_VALUE: &str = "DialogExe";
/// Po tym poznajemy własne wpisy – cudzych nadpisań tych klas nie ruszamy.
const MARK: &str = "powerdesk-dialog";

fn dir() -> PathBuf {
    crate::log::dir().join("FileDialog")
}

fn file(bits: u32) -> PathBuf {
    dir().join(format!("{MARK}{bits}.dll"))
}

fn embedded(bits: u32) -> &'static [u8] {
    if bits == 64 {
        DLL64
    } else {
        DLL32
    }
}

fn class_key(clsid: &str, bits: u32) -> String {
    if bits == 64 {
        format!(r"HKCU\Software\Classes\CLSID\{clsid}")
    } else {
        format!(r"HKCU\Software\Classes\WOW6432Node\CLSID\{clsid}")
    }
}

/// Wartość z wyniku `reg query` (nazwa wartości bywa przetłumaczona – „(Domyślna)”).
fn value(out: &str) -> Option<String> {
    out.lines().find(|l| l.contains("REG_")).and_then(|l| l.split("REG_").nth(1)).and_then(|v| v.split_once(char::is_whitespace)).map(|(_, v)| v.trim().to_string())
}

/// Biblioteka, którą wskazuje wpis klasy (dowolna – także cudza).
fn server(clsid: &str, bits: u32) -> Option<String> {
    reg(&["query", &format!(r"{}\InprocServer32", class_key(clsid, bits)), "/ve"]).ok().and_then(|o| value(&o))
}

fn same_path(a: &str, b: &Path) -> bool {
    a.trim_matches('"').eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Ta kopia programu zawiera bibliotekę (wydanie z CI; budowa lokalna zwykle nie).
pub fn bundled() -> bool {
    !DLL64.is_empty()
}

pub fn has_32bit() -> bool {
    !DLL32.is_empty()
}

fn ours_ok(bits: u32) -> bool {
    file(bits).is_file() && CLSIDS.iter().all(|c| server(c, bits).is_some_and(|s| same_path(&s, &file(bits))))
}

/// Okna plików prowadzą do tego programu (64 bity; 32 bity, gdy wydanie ma taką bibliotekę).
pub fn enabled() -> bool {
    let exe = crate::winkey::exe().unwrap_or_default();
    let exe_ok = reg(&["query", EXE_KEY, "/v", EXE_VALUE]).ok().and_then(|o| value(&o)).is_some_and(|v| v.eq_ignore_ascii_case(&exe));
    exe_ok && ours_ok(64) && (!has_32bit() || ours_ok(32))
}

/// Czy w programach 32-bitowych też jest okno PowerDesk.
pub fn enabled_32bit() -> bool {
    has_32bit() && ours_ok(32)
}

/// Zapis pliku tak, żeby wpis nigdy nie wskazywał połowy biblioteki: najpierw `.new`, potem
/// zamiana. Stara wersja wczytana w działających programach daje się przemianować, nie nadpisać.
fn put(path: &Path, data: &[u8]) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|d| d == data) {
        return Ok(());
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| crate::trf!("Nie można utworzyć {}: {e}", d.display()))?;
    }
    let tmp = path.with_extension("dll.new");
    std::fs::write(&tmp, data).map_err(|e| crate::trf!("Nie można zapisać {}: {e}", tmp.display()))?;
    if std::fs::rename(&tmp, path).is_err() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let old = path.with_extension(format!("dll.old{stamp}"));
        std::fs::rename(path, &old).map_err(|e| crate::trf!("Nie można zastąpić {}: {e}", path.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| crate::trf!("Nie można zastąpić {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Stare wersje, których już nikt nie trzyma (wczytane w programach – zostają do następnego razu).
fn sweep() {
    let Ok(rd) = std::fs::read_dir(dir()) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.contains(".dll.old") || name.ends_with(".dll.new") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn register(bits: u32) -> Result<(), String> {
    let path = file(bits).to_string_lossy().to_string();
    for c in CLSIDS {
        let key = format!(r"{}\InprocServer32", class_key(c, bits));
        reg(&["add", &key, "/ve", "/d", &path, "/f"])?;
        reg(&["add", &key, "/v", "ThreadingModel", "/d", crate::lang::tr("Apartment"), "/f"])?;
    }
    Ok(())
}

/// Zdejmuje tylko własne wpisy (wskazujące `powerdesk-dialog*.dll`).
fn unregister(bits: u32) -> Result<(), String> {
    for c in CLSIDS {
        if server(c, bits).is_some_and(|s| s.to_lowercase().contains(MARK)) {
            reg(&["delete", &class_key(c, bits), "/f"])?;
        }
    }
    Ok(())
}

/// Wypakowuje bibliotekę (gdy zmieniła się z wersją programu) i ustawia wpisy. Kopia bez osadzonej
/// biblioteki niczego nie zakłada, ale zdejmuje wpisy wskazujące brakujący plik.
pub fn ensure() -> Result<(), String> {
    sweep();
    if !bundled() {
        for bits in [64, 32] {
            if !file(bits).is_file() {
                unregister(bits)?;
            }
        }
        return Ok(());
    }
    let exe = crate::winkey::exe()?;
    for bits in [64, 32] {
        if !embedded(bits).is_empty() {
            put(&file(bits), embedded(bits))?;
        }
    }
    // Plik i ścieżka programu przed wpisami klas – wpis nigdy nie wskazuje czegoś, czego nie ma.
    reg(&["add", EXE_KEY, "/v", EXE_VALUE, "/d", &exe, "/f"])?;
    if !ours_ok(64) {
        register(64)?;
    }
    if has_32bit() {
        if !ours_ok(32) {
            register(32)?;
        }
    } else if !file(32).is_file() {
        unregister(32)?;
    }
    Ok(())
}

/// Przywraca okna Windows: najpierw wpisy, potem pliki (wczytane w programach zostają do ich końca –
/// bez wpisów nikt ich już nie użyje).
pub fn remove() -> Result<(), String> {
    unregister(64)?;
    unregister(32)?;
    if reg(&["query", EXE_KEY, "/v", EXE_VALUE]).is_ok() {
        let _ = reg(&["delete", EXE_KEY, "/v", EXE_VALUE, "/f"]);
    }
    for bits in [64, 32] {
        let _ = std::fs::remove_file(file(bits));
    }
    sweep();
    let _ = std::fs::remove_dir(dir());
    Ok(())
}

// ---------- Okno wyboru pliku (`powerdesk.exe --pick-file`) ----------

/// Prostokąt okna (fizyczne piksele) wyśrodkowany nad oknem programu `owner` i mieszczący się
/// na jego monitorze; `w`, `h` – rozmiar w pikselach logicznych. Zwraca też skalę monitora.
pub fn place_over(owner: isize, w: f32, h: f32) -> Option<(i32, i32, u32, u32, f32)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindow};
    if owner == 0 {
        return None;
    }
    let h_owner = HWND(owner as *mut _);
    unsafe {
        if !IsWindow(Some(h_owner)).as_bool() {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(h_owner, &mut r).ok()?;
        let scale = match GetDpiForWindow(h_owner) {
            0 => 1.0,
            d => d as f32 / 96.0,
        };
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mon = MonitorFromWindow(h_owner, MONITOR_DEFAULTTONEAREST);
        let work = if GetMonitorInfoW(mon, &mut mi).as_bool() { mi.rcWork } else { r };
        let pw = ((w * scale) as i32).min(work.right - work.left);
        let ph = ((h * scale) as i32).min(work.bottom - work.top);
        let cx = (r.left + r.right) / 2;
        let cy = (r.top + r.bottom) / 2;
        let x = (cx - pw / 2).clamp(work.left, work.right - pw);
        let y = (cy - ph / 2).clamp(work.top, work.bottom - ph);
        Some((x, y, pw as u32, ph as u32, scale))
    }
}

/// Okno wyboru jako okno należące do okna programu (zawsze nad nim, minimalizuje się razem
/// z nim) i ciemny pasek tytułu w ciemnym motywie.
pub fn attach(hwnd: isize, owner: isize, dark: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow, SetWindowLongPtrW, GWLP_HWNDPARENT};
    if hwnd == 0 {
        return;
    }
    let h = HWND(hwnd as *mut _);
    unsafe {
        let on: i32 = dark as i32;
        let _ = DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const i32 as *const _, 4);
        if owner != 0 && IsWindow(Some(HWND(owner as *mut _))).as_bool() {
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, owner);
        }
        let _ = SetForegroundWindow(h);
    }
}

/// Po wyborze: okno programu z powrotem na wierzch (ten proces jeszcze może to zrobić).
pub fn return_focus(owner: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
    if owner == 0 {
        return;
    }
    unsafe {
        let h = HWND(owner as *mut _);
        if IsWindow(Some(h)).as_bool() {
            let _ = SetForegroundWindow(h);
        }
    }
}
