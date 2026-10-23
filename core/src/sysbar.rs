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
