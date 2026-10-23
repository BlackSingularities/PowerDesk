//! Stan systemu dla zasobnika paska PowerDesk: głośność domyślnego wyjścia audio,
//! połączenie sieciowe i bateria. Odczyty są szybkie – pasek pyta co kilka sekund.

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::NetworkManagement::IpHelper::{GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_IEEE80211, IP_ADAPTER_ADDRESSES_LH};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// Głośność 0.0–1.0 i wyciszenie.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub level: f32,
    pub muted: bool,
}

fn endpoint() -> Option<IAudioEndpointVolume> {
    unsafe {
        // Wątek UI ma już COM (OLE); na innym wątku inicjalizujemy – błąd „już jest” nie szkodzi.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

/// None – brak urządzenia odtwarzającego.
pub fn volume() -> Option<Volume> {
    let ep = endpoint()?;
    unsafe { Some(Volume { level: ep.GetMasterVolumeLevelScalar().ok()?, muted: ep.GetMute().ok()?.as_bool() }) }
}

pub fn set_volume(level: f32) {
    if let Some(ep) = endpoint() {
        unsafe {
            let _ = ep.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
            // Przesunięcie suwaka zdejmuje wyciszenie – jak w Windows.
            if level > 0.0 {
                let _ = ep.SetMute(false, std::ptr::null());
            }
        }
    }
}

pub fn set_mute(muted: bool) {
    if let Some(ep) = endpoint() {
        unsafe {
            let _ = ep.SetMute(muted, std::ptr::null());
        }
    }
}

/// Głośność jednej aplikacji (sesje audio jednego procesu razem), jak w mikserze Windows.
#[derive(Clone, Debug, PartialEq)]
pub struct AppVolume {
    /// 0 – dźwięki systemowe.
    pub pid: u32,
    pub name: String,
    /// Plik programu – do ikony; pusty dla dźwięków systemowych.
    pub exe: String,
    pub level: f32,
    pub muted: bool,
}

fn exe_of_pid(pid: u32) -> String {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(p);
        if ok {
            String::from_utf16_lossy(&buf[..n as usize])
        } else {
            String::new()
        }
    }
}

/// Opis pliku z zasobów wersji (np. „Google Chrome”); bez niego nazwa pliku bez rozszerzenia.
fn app_name(exe: &str) -> String {
    let stem = std::path::Path::new(exe).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    file_description(exe).filter(|d| !d.trim().is_empty()).unwrap_or(stem)
}

/// FileDescription z zasobów wersji pliku (pierwsze tłumaczenie).
fn file_description(exe: &str) -> Option<String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    if exe.is_empty() {
        return None;
    }
    unsafe {
        let path = HSTRING::from(exe);
        let size = GetFileVersionInfoSizeW(&path, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(&path, None, size, buf.as_mut_ptr() as *mut _).ok()?;
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut n = 0u32;
        if !VerQueryValueW(buf.as_ptr() as *const _, &HSTRING::from("\\VarFileInfo\\Translation"), &mut p, &mut n).as_bool() || n < 4 {
            return None;
        }
        let lang = *(p as *const u16);
        let cp = *(p as *const u16).add(1);
        let key: Vec<u16> = format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\FileDescription\0").encode_utf16().collect();
        if !VerQueryValueW(buf.as_ptr() as *const _, PCWSTR(key.as_ptr()), &mut p, &mut n).as_bool() || n == 0 {
            return None;
        }
        let s = std::slice::from_raw_parts(p as *const u16, n as usize);
        Some(String::from_utf16_lossy(s).trim_end_matches('\0').to_string())
    }
}

/// Aktywne sesje audio domyślnego wyjścia: (pid, głośność sesji, nazwa sesji).
fn sessions() -> Vec<(u32, windows::Win32::Media::Audio::ISimpleAudioVolume, String)> {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume};
    let mut out = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return out };
        let Ok(dev) = en.GetDefaultAudioEndpoint(eRender, eMultimedia) else { return out };
        let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return out };
        let Ok(list) = mgr.GetSessionEnumerator() else { return out };
        let n = list.GetCount().unwrap_or(0);
        for i in 0..n {
            let Ok(c) = list.GetSession(i) else { continue };
            if c.GetState().is_ok_and(|s| s == AudioSessionStateExpired) {
                continue;
            }
            let Ok(c2) = c.cast::<IAudioSessionControl2>() else { continue };
            let Ok(vol) = c.cast::<ISimpleAudioVolume>() else { continue };
            let system = c2.IsSystemSoundsSession().0 == 0;
            let pid = if system { 0 } else { c2.GetProcessId().unwrap_or(0) };
            let name = c.GetDisplayName().ok().and_then(|p| {
                let s = p.to_string().ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
                s
            });
            out.push((pid, vol, name.unwrap_or_default()));
        }
    }
    out
}

/// Aplikacje grające przez domyślne wyjście, po nazwie; dźwięki systemowe na końcu.
pub fn app_volumes() -> Vec<AppVolume> {
    let mut out: Vec<AppVolume> = Vec::new();
    for (pid, vol, name) in sessions() {
        if out.iter().any(|a| a.pid == pid) {
            continue;
        }
        let (level, muted) = unsafe { (vol.GetMasterVolume().unwrap_or(1.0), vol.GetMute().map(|m| m.as_bool()).unwrap_or(false)) };
        let (name, exe) = if pid == 0 {
            ("Dźwięki systemowe".to_string(), String::new())
        } else {
            let exe = exe_of_pid(pid);
            // Nazwy sesji w stylu „@%SystemRoot%\…” to odnośniki do zasobów – bierzemy program.
            let n = if name.is_empty() || name.starts_with('@') { app_name(&exe) } else { name };
            (n, exe)
        };
        out.push(AppVolume { pid, name, exe, level, muted });
    }
    out.sort_by_key(|a| (a.pid == 0, a.name.to_lowercase()));
    out
}

fn each_session(pid: u32, f: impl Fn(&windows::Win32::Media::Audio::ISimpleAudioVolume)) {
    for (p, vol, _) in sessions() {
        if p == pid {
            f(&vol);
        }
    }
}

pub fn set_app_volume(pid: u32, level: f32) {
    each_session(pid, |v| unsafe {
        let _ = v.SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null());
        if level > 0.0 {
            let _ = v.SetMute(false, std::ptr::null());
        }
    });
}

pub fn set_app_mute(pid: u32, muted: bool) {
    each_session(pid, |v| unsafe {
        let _ = v.SetMute(muted, std::ptr::null());
    });
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NetKind {
    None,
    Wifi,
    Wired,
}

/// Połączenie sieciowe: rodzaj i nazwa karty.
#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    pub kind: NetKind,
    pub name: String,
}

/// Pierwsza działająca karta z bramą domyślną – tędy idzie ruch do internetu.
/// Wi-Fi wygrywa z innymi tylko wtedy, gdy nie ma kabla (jak ikona w Windows).
pub fn network() -> Net {
    let mut best = Net { kind: NetKind::None, name: String::new() };
    unsafe {
        let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
        let mut size = 16 * 1024u32;
        let mut buf: Vec<u64> = Vec::new();
        for _ in 0..3 {
            buf = vec![0u64; size as usize / 8 + 1];
            let r = GetAdaptersAddresses(0, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut size);
            if r == 0 {
                break;
            }
            if r != 111 {
                // Inny błąd niż ERROR_BUFFER_OVERFLOW – brak danych.
                return best;
            }
        }
        let mut a = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !a.is_null() {
            let ad = &*a;
            if ad.OperStatus == IfOperStatusUp && !ad.FirstGatewayAddress.is_null() {
                let wifi = ad.IfType == IF_TYPE_IEEE80211;
                let name = ad.FriendlyName.to_string().unwrap_or_default();
                if !wifi {
                    return Net { kind: NetKind::Wired, name };
                }
                if best.kind == NetKind::None {
                    best = Net { kind: NetKind::Wifi, name };
                }
            }
            a = ad.Next;
        }
    }
    best
}

/// Bateria: procent i ładowanie. None – komputer bez baterii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

pub fn battery() -> Option<Battery> {
    let mut st = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut st).ok()? };
    // 128 – brak baterii, 255 – stan nieznany.
    if st.BatteryFlag & 128 != 0 || st.BatteryFlag == 255 || st.BatteryLifePercent > 100 {
        return None;
    }
    Some(Battery { percent: st.BatteryLifePercent, charging: st.ACLineStatus == 1 })
}

/// Otwiera stronę Ustawień Windows (ms-settings:…).
pub fn open_settings(page: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(format!("ms-settings:{page}")).spawn();
}
