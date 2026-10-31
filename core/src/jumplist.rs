//! Listy szybkiego dostępu (jump lists): ostatnie i częste pliki programu z list, które Windows
//! prowadzi dla każdego AppUserModelID (IApplicationDocumentLists – te same, co na pasku Windows).
//!
//! AppID bierzemy z okna (programy, które go ustawiają – Edge, Office, Teams), a bez okna albo
//! bez jawnego AppID – domyślny, który Windows wylicza ze ścieżki programu (znane foldery jako GUID).

use windows::core::{GUID, HSTRING, PWSTR};
use windows::Win32::Foundation::{HWND, PROPERTYKEY};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::Common::IObjectArray;
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{ApplicationDocumentLists, IApplicationDocumentLists, IShellItem, ADLT_FREQUENT, ADLT_RECENT, SIGDN_FILESYSPATH};

/// PKEY_AppUserModel_ID.
const APP_ID: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

/// Jawny AppID okna (pusty, gdy program go nie ustawia).
fn window_app_id(hwnd: isize) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(HWND(hwnd as *mut _)).ok()?;
        let v = store.GetValue(&APP_ID).ok()?;
        let s = v.to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// Domyślny AppID programu bez jawnego: ścieżka z prefiksem znanego folderu zamienionym na jego GUID.
fn default_app_id(exe: &str) -> String {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let root = env("SystemRoot");
    let known = [
        (format!(r"{root}\System32"), "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}"),
        (env("ProgramFiles(x86)"), "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}"),
        (env("ProgramW6432"), "{6D809377-6AF0-444B-8957-A3773F02200E}"),
        (root.clone(), "{F38BF404-1D43-42F2-9305-67DE0B28FC23}"),
    ];
    let low = exe.to_lowercase();
    for (dir, guid) in known {
        if dir.is_empty() {
            continue;
        }
        let prefix = format!("{}\\", dir.trim_end_matches('\\').to_lowercase());
        if low.starts_with(&prefix) {
            return format!("{guid}\\{}", &exe[prefix.len()..]);
        }
    }
    exe.to_string()
}

fn list(app: &str, kind: windows::Win32::UI::Shell::APPDOCLISTTYPE, n: u32) -> Vec<String> {
    unsafe {
        let Ok(lists) = CoCreateInstance::<_, IApplicationDocumentLists>(&ApplicationDocumentLists, None, CLSCTX_INPROC_SERVER) else { return Vec::new() };
        if lists.SetAppID(&HSTRING::from(app)).is_err() {
            return Vec::new();
        }
        let Ok(arr) = lists.GetList::<IObjectArray>(kind, n) else { return Vec::new() };
        let count = arr.GetCount().unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..count {
            let Ok(item) = arr.GetAt::<IShellItem>(i) else { continue };
            let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
            out.push(take(name));
        }
        out
    }
}

unsafe fn take(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

/// Ostatnie pliki programu (najwyżej `n`, istniejące): najpierw ostatnie, potem częste.
/// `hwnd` – okno programu, jeśli działa (jego AppID ma pierwszeństwo).
pub fn recent(hwnd: Option<isize>, exe: &str, n: usize) -> Vec<String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let mut ids: Vec<String> = hwnd.and_then(window_app_id).into_iter().collect();
    if !exe.is_empty() {
        ids.push(default_app_id(exe));
    }
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        for kind in [ADLT_RECENT, ADLT_FREQUENT] {
            for p in list(&id, kind, n as u32) {
                if out.len() < n && !out.iter().any(|o| o.eq_ignore_ascii_case(&p)) && std::path::Path::new(&p).exists() {
                    out.push(p);
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// Zadanie albo pozycja własnej kategorii z listy programu (np. „Nowe okno InPrivate” w Edge).
#[derive(Clone, Debug)]
pub struct Task {
    /// Kategoria programu; pusta – „Zadania”.
    pub category: String,
    pub title: String,
    pub path: String,
    pub args: String,
}

/// System.Title i System.AppUserModel.IsDestListSeparator.
const TITLE: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9), pid: 2 };
const SEPARATOR: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 6 };
/// CLSID_ShellLink w zapisie binarnym (GUID w porządku little-endian).
const SHELL_LINK: [u8; 16] = [0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];
const FOOTER: u32 = 0xBABF_FBAB;

fn wide(buf: &[u16]) -> String {
    String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())])
}

/// Napis „@plik.dll,-123” albo „ms-resource:” – tłumaczenie z zasobów programu.
fn indirect(s: String) -> String {
    if !s.starts_with('@') {
        return s;
    }
    let mut out = vec![0u16; 512];
    match unsafe { windows::Win32::UI::Shell::SHLoadIndirectString(&HSTRING::from(s.as_str()), &mut out, None) } {
        Ok(()) => wide(&out),
        Err(_) => String::new(),
    }
}

/// Skrót zapisany w pliku listy od `at`: (zadanie bez kategorii, długość w bajtach).
fn read_link(data: &[u8]) -> Option<(Option<Task>, usize)> {
    use windows::Win32::System::Com::{IPersistStream, STREAM_SEEK_CUR};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    unsafe {
        let stream = windows::Win32::UI::Shell::SHCreateMemStream(Some(data))?;
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistStream = windows::core::Interface::cast(&link).ok()?;
        persist.Load(&stream).ok()?;
        let mut pos = 0u64;
        stream.Seek(0, STREAM_SEEK_CUR, Some(&mut pos)).ok()?;
        let store: Option<IPropertyStore> = windows::core::Interface::cast(&link).ok();
        let prop = |k: &PROPERTYKEY| store.as_ref().and_then(|s| s.GetValue(k).ok()).map(|v| v.to_string()).unwrap_or_default();
        if matches!(prop(&SEPARATOR).to_lowercase().as_str(), "true" | "1" | "-1") {
            return Some((None, pos as usize));
        }
        let mut path = vec![0u16; 1024];
        let _ = link.GetPath(&mut path, std::ptr::null_mut(), 0);
        let mut args = vec![0u16; 4096];
        let _ = link.GetArguments(&mut args);
        let mut title = indirect(prop(&TITLE));
        if title.is_empty() {
            let mut d = vec![0u16; 512];
            let _ = link.GetDescription(&mut d);
            title = indirect(wide(&d));
        }
        let path = wide(&path);
        let task = (!path.is_empty() && !title.is_empty()).then(|| Task { category: String::new(), title, path, args: wide(&args) });
        Some((task, pos as usize))
    }
}

/// Zadania i własne kategorie z jednego pliku `.customDestinations-ms`.
fn parse(data: &[u8]) -> Vec<Task> {
    let u32_at = |p: usize| data.get(p..p + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let mut out = Vec::new();
    let Some(cats) = u32_at(4) else { return out };
    let mut p = 12;
    for _ in 0..cats.min(32) {
        let Some(kind) = u32_at(p) else { break };
        p += 4;
        let mut category = String::new();
        let count = match kind {
            0 => {
                let Some(len) = data.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize) else { break };
                let Some(name) = data.get(p + 2..p + 2 + len * 2) else { break };
                category = indirect(String::from_utf16_lossy(&name.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()));
                p += 2 + len * 2;
                let Some(n) = u32_at(p) else { break };
                p += 4;
                n
            }
            // Kategoria znana (Ostatnie/Częste) – same pliki są w listach Windows (recent).
            1 => {
                p += 4;
                0
            }
            2 => {
                let Some(n) = u32_at(p) else { break };
                p += 4;
                n
            }
            _ => break,
        };
        for _ in 0..count.min(64) {
            if data.get(p..p + 16) != Some(&SHELL_LINK[..]) {
                return out;
            }
            p += 16;
            let Some((task, len)) = data.get(p..).and_then(read_link) else { return out };
            p += len;
            if let Some(mut t) = task {
                t.category = category.clone();
                out.push(t);
            }
        }
        if u32_at(p) != Some(FOOTER) {
            break;
        }
        p += 4;
    }
    out
}

/// Zadania programu `exe` (np. „Nowe okno InPrivate”): pliki list, w których skróty wskazują ten program.
pub fn tasks(exe: &str) -> Vec<Task> {
    if exe.is_empty() {
        return Vec::new();
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let Ok(appdata) = std::env::var("APPDATA") else { return Vec::new() };
    let dir = std::path::Path::new(&appdata).join(r"Microsoft\Windows\Recent\CustomDestinations");
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<_> = rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".customDestinations-ms")).collect();
    // Najświeższy plik pierwszy – program mógł zostawić starsze listy pod innym AppID.
    files.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).ok()));
    for f in files {
        let Ok(data) = std::fs::read(f.path()) else { continue };
        let list = parse(&data);
        if list.iter().any(|t| t.path.eq_ignore_ascii_case(exe)) {
            return list.into_iter().take(20).collect();
        }
    }
    Vec::new()
}
