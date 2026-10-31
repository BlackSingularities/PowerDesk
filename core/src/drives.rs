//! Dyski: lokalne, wymienne, optyczne i sieciowe (także zapamiętane, ale rozłączone).
//! Mapowanie dysków sieciowych przez WNet – hasło wpisuje się w systemowym oknie Windows.

use std::sync::mpsc;
use std::time::Duration;

use serde::Serialize;
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, WIN32_ERROR, NO_ERROR, ERROR_MORE_DATA};
use windows::Win32::NetworkManagement::WNet::{
    WNetAddConnection3W, WNetCancelConnection2W, WNetCloseEnum, WNetEnumResourceW,
    WNetGetConnectionW, WNetOpenEnumW, CONNECT_INTERACTIVE, CONNECT_PROMPT, CONNECT_UPDATE_PROFILE,
    NETRESOURCEW, NET_CONNECT_FLAGS, RESOURCETYPE_DISK, RESOURCE_REMEMBERED, RESOURCEUSAGE_ALL,
    NET_RESOURCE_SCOPE, NET_RESOURCE_TYPE, WNET_OPEN_ENUM_USAGE,
};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};

#[derive(Serialize, Clone, Debug)]
pub struct Drive {
    pub path: String,
    pub letter: String,
    pub label: String,
    pub kind: String, // fixed | removable | network | cdrom | ram | unknown
    pub fs: String,
    pub total: u64,
    pub free: u64,
    pub serial: u32,
    pub remote: Option<String>,
    pub online: bool,
}

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

pub struct VolInfo {
    pub label: String,
    pub fs: String,
    pub serial: u32,
    pub total: u64,
    pub free: u64,
}

fn volume_info_blocking(root: &str) -> Option<VolInfo> {
    unsafe {
        let r = HSTRING::from(root);
        let mut label = [0u16; 261];
        let mut fsname = [0u16; 64];
        let mut serial = 0u32;
        if GetVolumeInformationW(&r, Some(&mut label), Some(&mut serial), None, None, Some(&mut fsname)).is_err() {
            return None;
        }
        let (mut free, mut total) = (0u64, 0u64);
        let _ = GetDiskFreeSpaceExW(&r, Some(&mut free), Some(&mut total), None);
        Some(VolInfo {
            label: from_wide(&label),
            fs: from_wide(&fsname),
            serial,
            total,
            free,
        })
    }
}

/// Dysk sieciowy może „wisieć" kilkanaście sekund – pytamy w osobnym wątku z limitem czasu.
pub fn volume_info(root: &str, timeout: Duration) -> Option<VolInfo> {
    let (tx, rx) = mpsc::channel();
    let r = root.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(volume_info_blocking(&r));
    });
    rx.recv_timeout(timeout).ok().flatten()
}

pub fn path_reachable(path: &str, timeout: Duration) -> bool {
    let (tx, rx) = mpsc::channel();
    let p = path.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(std::fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false));
    });
    rx.recv_timeout(timeout).unwrap_or(false)
}

fn from_wide(b: &[u16]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..end])
}

fn remote_name(letter: &str) -> Option<String> {
    unsafe {
        let local = HSTRING::from(letter);
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let r = WNetGetConnectionW(&local, Some(PWSTR(buf.as_mut_ptr())), &mut len);
        if r == NO_ERROR {
            Some(from_wide(&buf))
        } else {
            None
        }
    }
}

pub fn list() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut handles = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = format!("{}:", (b'A' + i as u8) as char);
        handles.push(std::thread::spawn(move || {
            let root = format!("{letter}\\");
            let t = unsafe { GetDriveTypeW(&HSTRING::from(root.as_str())) };
            let kind = match t {
                DRIVE_REMOVABLE => "removable",
                DRIVE_FIXED => "fixed",
                DRIVE_REMOTE => "network",
                DRIVE_CDROM => "cdrom",
                DRIVE_RAMDISK => "ram",
                _ => "unknown",
            };
            let timeout = if kind == "network" { Duration::from_millis(1500) } else { Duration::from_secs(4) };
            let info = volume_info(&root, timeout);
            let remote = if kind == "network" { remote_name(&letter) } else { None };
            Drive {
                path: root,
                letter: letter.clone(),
                label: info.as_ref().map(|v| v.label.clone()).unwrap_or_default(),
                kind: kind.into(),
                fs: info.as_ref().map(|v| v.fs.clone()).unwrap_or_default(),
                total: info.as_ref().map(|v| v.total).unwrap_or(0),
                free: info.as_ref().map(|v| v.free).unwrap_or(0),
                serial: info.as_ref().map(|v| v.serial).unwrap_or(0),
                remote,
                online: info.is_some(),
            }
        }));
    }
    let mut out: Vec<Drive> = handles.into_iter().filter_map(|h| h.join().ok()).collect();
    // Zapamiętane połączenia, których litery chwilowo nie ma (np. NAS wyłączony).
    for (local, remote) in remembered() {
        if !local.is_empty() && !out.iter().any(|d| d.letter.eq_ignore_ascii_case(&local)) {
            out.push(Drive {
                path: format!("{local}\\"),
                letter: local,
                label: String::new(),
                kind: "network".into(),
                fs: String::new(),
                total: 0,
                free: 0,
                serial: 0,
                remote: Some(remote),
                online: false,
            });
        }
    }
    out.sort_by(|a, b| a.letter.cmp(&b.letter));
    out
}

fn remembered() -> Vec<(String, String)> {
    let mut out = Vec::new();
    unsafe {
        let mut h = Default::default();
        if WNetOpenEnumW(
            NET_RESOURCE_SCOPE(RESOURCE_REMEMBERED.0),
            NET_RESOURCE_TYPE(RESOURCETYPE_DISK.0),
            WNET_OPEN_ENUM_USAGE(RESOURCEUSAGE_ALL.0),
            None,
            &mut h,
        ) != NO_ERROR
        {
            return out;
        }
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut count = u32::MAX;
            let mut size = buf.len() as u32;
            let r: WIN32_ERROR = WNetEnumResourceW(h, &mut count, buf.as_mut_ptr() as *mut _, &mut size);
            if r == ERROR_MORE_DATA {
                buf.resize(size as usize + 1024, 0);
                continue;
            }
            if r != NO_ERROR {
                break;
            }
            let items = std::slice::from_raw_parts(buf.as_ptr() as *const NETRESOURCEW, count as usize);
            for it in items {
                let l = it.lpLocalName.to_string().unwrap_or_default();
                let rn = it.lpRemoteName.to_string().unwrap_or_default();
                out.push((l, rn));
            }
        }
        let _ = WNetCloseEnum(h);
    }
    out
}

/// Otoczenie sieciowe: `None` – komputery w grupie roboczej/domenie (`\\NAZWA`), `Some("\\NAZWA")` –
/// foldery udostępnione przez ten komputer (`\\NAZWA\udział`). Blokuje (sieć) – nawet kilka sekund.
/// Nowsze Windows bez usługi Przeglądarka komputerów mogą nie zwrócić żadnego komputera;
/// udziały znanego komputera działają zawsze.
pub fn network(server: Option<&str>) -> Result<Vec<String>, String> {
    use windows::Win32::NetworkManagement::WNet::{RESOURCETYPE_ANY, RESOURCE_CONTEXT, RESOURCE_GLOBALNET};
    // RESOURCEDISPLAYTYPE_SERVER i RESOURCEDISPLAYTYPE_SHARE.
    const SERVER: u32 = 2;
    const SHARE: u32 = 3;
    let mut out = Vec::new();
    unsafe {
        let remote = server.map(|s| HSTRING::from(s.trim_end_matches('\\')));
        let mut nr = NETRESOURCEW::default();
        let (scope, kind) = match &remote {
            Some(r) => {
                nr.lpRemoteName = PWSTR(r.as_ptr() as *mut _);
                (RESOURCE_GLOBALNET.0, RESOURCETYPE_DISK.0)
            }
            None => (RESOURCE_CONTEXT.0, RESOURCETYPE_ANY.0),
        };
        let mut h = Default::default();
        let r = WNetOpenEnumW(NET_RESOURCE_SCOPE(scope), NET_RESOURCE_TYPE(kind), WNET_OPEN_ENUM_USAGE(0), if remote.is_some() { Some(&nr as *const NETRESOURCEW) } else { None }, &mut h);
        if r != NO_ERROR {
            return Err(win_err(r.0));
        }
        let want = if remote.is_some() { SHARE } else { SERVER };
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut count = u32::MAX;
            let mut size = buf.len() as u32;
            let r: WIN32_ERROR = WNetEnumResourceW(h, &mut count, buf.as_mut_ptr() as *mut _, &mut size);
            if r == ERROR_MORE_DATA {
                buf.resize(size as usize + 1024, 0);
                continue;
            }
            if r != NO_ERROR {
                break;
            }
            let items = std::slice::from_raw_parts(buf.as_ptr() as *const NETRESOURCEW, count as usize);
            for it in items.iter().filter(|it| it.dwDisplayType == want) {
                let rn = it.lpRemoteName.to_string().unwrap_or_default();
                if !rn.is_empty() {
                    out.push(rn);
                }
            }
        }
        let _ = WNetCloseEnum(h);
    }
    out.sort_by_key(|s| s.to_lowercase());
    out.dedup();
    Ok(out)
}

/// Mapuje udział sieciowy. Gdy potrzebne są dane logowania, Windows sam pokazuje
/// swoje okno – aplikacja nigdy nie widzi hasła.
pub fn map(hwnd: isize, letter: Option<String>, remote: &str, persistent: bool) -> Result<String, String> {
    let local = letter.map(|l| {
        let l = l.trim_end_matches(['\\', ':']).to_uppercase();
        format!("{l}:")
    });
    let remote_h = HSTRING::from(remote.trim_end_matches('\\'));
    let local_h = local.as_deref().map(HSTRING::from);
    let nr = NETRESOURCEW {
        dwType: RESOURCETYPE_DISK,
        lpLocalName: match &local_h {
            Some(h) => PWSTR(h.as_ptr() as *mut _),
            None => PWSTR::null(),
        },
        lpRemoteName: PWSTR(remote_h.as_ptr() as *mut _),
        ..Default::default()
    };
    let mut flags = CONNECT_INTERACTIVE.0 | CONNECT_PROMPT.0;
    if persistent && local.is_some() {
        flags |= CONNECT_UPDATE_PROFILE.0;
    }
    let r = unsafe { WNetAddConnection3W(Some(HWND(hwnd as *mut _)), &nr, PCWSTR::null(), PCWSTR::null(), NET_CONNECT_FLAGS(flags)) };
    if r == NO_ERROR {
        Ok(local.map(|l| format!("{l}\\")).unwrap_or_else(|| remote.to_string()))
    } else {
        Err(win_err(r.0))
    }
}

pub fn unmap(letter: &str, forget: bool) -> Result<(), String> {
    let l = HSTRING::from(letter.trim_end_matches('\\'));
    let flags = if forget { CONNECT_UPDATE_PROFILE } else { NET_CONNECT_FLAGS(0) };
    let r = unsafe { WNetCancelConnection2W(&l, flags, true) };
    if r == NO_ERROR {
        Ok(())
    } else {
        Err(win_err(r.0))
    }
}

/// Systemowe okno „Mapuj dysk sieciowy” (wybór litery, ścieżki i logowania robi Windows).
pub fn map_dialog(hwnd: isize) {
    unsafe {
        let _ = windows::Win32::NetworkManagement::WNet::WNetConnectionDialog(HWND(hwnd as *mut _), RESOURCETYPE_DISK.0);
    }
}

/// Wysuwa dysk wymienny albo płytę – czasownik powłoki „Wysuń” (jak w Eksploratorze).
pub fn eject(hwnd: isize, root: &str) -> Result<(), String> {
    crate::shell::invoke_verb(hwnd, root, "eject")
}

/// Okno formatowania Windows dla dysku o tej literze (np. „E:”).
pub fn format_dialog(hwnd: isize, letter: &str) {
    use windows::Win32::UI::Shell::{SHFormatDrive, SHFMT_ID_DEFAULT, SHFMT_OPT_NONE};
    let Some(c) = letter.chars().next().map(|c| c.to_ascii_uppercase()).filter(|c| c.is_ascii_uppercase()) else { return };
    unsafe {
        SHFormatDrive(HWND(hwnd as *mut _), (c as u8 - b'A') as u32, SHFMT_ID_DEFAULT, SHFMT_OPT_NONE);
    }
}

/// Panel BitLockera z Panelu sterowania.
pub fn bitlocker() {
    let _ = std::process::Command::new("control.exe").args(["/name", "Microsoft.BitLockerDriveEncryption"]).spawn();
}

/// Oczyszczanie dysku Windows dla tej litery.
pub fn cleanup(letter: &str) {
    let l = letter.trim_end_matches(['\\', ':']);
    let _ = std::process::Command::new("cleanmgr.exe").arg(format!("/d{l}")).spawn();
}

pub fn win_err(code: u32) -> String {
    match code {
        5 => "Brak dostępu".into(),
        53 | 67 => "Nie znaleziono ścieżki sieciowej".into(),
        85 => "Ta litera dysku jest już zajęta".into(),
        86 | 1326 => "Nieprawidłowa nazwa użytkownika lub hasło".into(),
        1219 => "Istnieje już połączenie z tym serwerem na innych danych logowania".into(),
        1223 => "Anulowano".into(),
        2250 => "To połączenie nie istnieje".into(),
        _ => std::io::Error::from_raw_os_error(code as i32).to_string(),
    }
}

/// Podłączenie/odłączenie dysku lub włożenie karty do czytnika – zdarzenie „drives-changed”
/// od razu, bez czekania na okresowe odświeżenie.
pub fn init_watch(app: crate::events::AppHandle) {
    use crate::events::Emitter;
    std::thread::Builder::new()
        .name("drives".into())
        .spawn(move || {
            let mut last = String::new();
            loop {
                let mask = unsafe { GetLogicalDrives() };
                let mut sig = format!("{mask:x}");
                for i in 0..26u32 {
                    if mask & (1 << i) == 0 {
                        continue;
                    }
                    let root = format!("{}:\\", (b'A' + i as u8) as char);
                    if unsafe { GetDriveTypeW(&HSTRING::from(root.as_str())) } == DRIVE_REMOVABLE {
                        let ready = volume_info(&root, Duration::from_millis(400)).is_some();
                        sig.push(if ready { '1' } else { '0' });
                    }
                }
                if !last.is_empty() && sig != last {
                    let _ = app.emit("drives-changed", ());
                }
                last = sig;
                std::thread::sleep(Duration::from_millis(1200));
            }
        })
        .ok();
}

/// Urządzenia przenośne w „Ten komputer” (telefon, aparat przez MTP): nazwa i ścieżka powłoki.
/// Nie są dyskami z literą – otwiera je okno powłoki Windows. PowerShell – wywoływać w tle.
pub fn portable() -> Vec<(String, String)> {
    use std::os::windows::process::CommandExt;
    let script = "[Console]::OutputEncoding=[Text.Encoding]::UTF8;\
        (New-Object -ComObject Shell.Application).Namespace(17).Items() | Where-Object { -not $_.IsFileSystem -and $_.Path -like '*usb#*' } | ForEach-Object { $_.Name + '|' + $_.Path }";
    let Ok(out) = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]).creation_flags(0x0800_0000).output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().split_once('|').map(|(n, p)| (n.to_string(), p.to_string()))).collect()
}

/// Otwiera urządzenie przenośne w oknie powłoki Windows.
pub fn open_portable(path: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}
