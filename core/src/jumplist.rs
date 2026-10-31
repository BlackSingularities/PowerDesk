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
