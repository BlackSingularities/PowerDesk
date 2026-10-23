//! Sieci Wi-Fi przez Native Wifi (wlanapi): lista dostępnych sieci, skanowanie, łączenie
//! (zapisany profil albo nowy z hasłem) i rozłączanie – dla okienka sieci na pasku PowerDesk.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_infrastructure, wlan_connection_mode_profile, WlanCloseHandle, WlanConnect, WlanDisconnect, WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList, WlanOpenHandle, WlanScan,
    WlanSetProfile, DOT11_AUTH_ALGO_WPA3_SAE, WLAN_AVAILABLE_NETWORK, WLAN_AVAILABLE_NETWORK_CONNECTED, WLAN_AVAILABLE_NETWORK_HAS_PROFILE,
    WLAN_AVAILABLE_NETWORK_LIST, WLAN_CONNECTION_PARAMETERS, WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST,
};

/// Sieć widoczna w okolicy.
#[derive(Clone, Debug, PartialEq)]
pub struct WifiNet {
    pub ssid: String,
    /// Jakość sygnału 0–100.
    pub signal: u32,
    pub secured: bool,
    pub connected: bool,
    /// Windows ma zapisany profil – łączy się bez pytania o hasło.
    pub known: bool,
    /// Nazwa zapisanego profilu (zwykle równa SSID).
    profile: String,
    sae: bool,
}

/// Uchwyt wlanapi i pierwsza karta Wi-Fi; zamykany przy wyjściu z zakresu.
struct Wlan {
    h: HANDLE,
    iface: windows::core::GUID,
}

impl Wlan {
    fn open() -> Option<Wlan> {
        unsafe {
            let mut ver = 0u32;
            let mut h = HANDLE::default();
            if WlanOpenHandle(2, None, &mut ver, &mut h) != 0 {
                return None;
            }
            let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
            if WlanEnumInterfaces(h, None, &mut list) != 0 || list.is_null() {
                WlanCloseHandle(h, None);
                return None;
            }
            let n = (*list).dwNumberOfItems as usize;
            let first = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr() as *const WLAN_INTERFACE_INFO, n).first().map(|i| i.InterfaceGuid);
            WlanFreeMemory(list as *const _);
            match first {
                Some(iface) => Some(Wlan { h, iface }),
                None => {
                    WlanCloseHandle(h, None);
                    None
                }
            }
        }
    }
}

impl Drop for Wlan {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.h, None);
        }
    }
}

fn wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// None – komputer nie ma karty Wi-Fi. Sieci bez nazwy (ukryte) pomijamy; każda nazwa raz,
/// z najlepszym sygnałem. Połączona na górze, potem od najsilniejszej.
pub fn networks() -> Option<Vec<WifiNet>> {
    let w = Wlan::open()?;
    let mut out: Vec<WifiNet> = Vec::new();
    unsafe {
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        if WlanGetAvailableNetworkList(w.h, &w.iface, 0, None, &mut list) != 0 || list.is_null() {
            return Some(out);
        }
        let n = (*list).dwNumberOfItems as usize;
        let items = std::slice::from_raw_parts((*list).Network.as_ptr() as *const WLAN_AVAILABLE_NETWORK, n);
        for it in items {
            let len = (it.dot11Ssid.uSSIDLength as usize).min(32);
            let ssid = String::from_utf8_lossy(&it.dot11Ssid.ucSSID[..len]).into_owned();
            if ssid.is_empty() {
                continue;
            }
            let net = WifiNet {
                ssid,
                signal: it.wlanSignalQuality,
                secured: it.bSecurityEnabled.as_bool(),
                connected: it.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                known: it.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0,
                profile: wide(&it.strProfileName),
                sae: it.dot11DefaultAuthAlgorithm == DOT11_AUTH_ALGO_WPA3_SAE,
            };
            match out.iter_mut().find(|o| o.ssid == net.ssid) {
                // Ta sama sieć na kilku wpisach (z profilem i bez) – łączymy informacje.
                Some(o) => {
                    o.signal = o.signal.max(net.signal);
                    o.connected |= net.connected;
                    if net.known && !o.known {
                        o.known = true;
                        o.profile = net.profile;
                    }
                }
                None => out.push(net),
            }
        }
        WlanFreeMemory(list as *const _);
    }
    out.sort_by_key(|n| (!n.connected, std::cmp::Reverse(n.signal)));
    Some(out)
}

/// Prosi kartę o nowe skanowanie; wyniki są w `networks()` po ok. 4 sekundach.
pub fn scan() {
    if let Some(w) = Wlan::open() {
        unsafe {
            WlanScan(w.h, &w.iface, None, None, None);
        }
    }
}

pub fn disconnect() {
    if let Some(w) = Wlan::open() {
        unsafe {
            WlanDisconnect(w.h, &w.iface, None);
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Profil jak zapisywany przez Windows: automatyczne łączenie, WPA2/WPA3-Personal albo otwarta sieć.
fn profile_xml(net: &WifiNet, password: &str) -> String {
    let name = xml_escape(&net.ssid);
    let hex: String = net.ssid.bytes().map(|b| format!("{b:02X}")).collect();
    let security = if !net.secured {
        "<authEncryption><authentication>open</authentication><encryption>none</encryption><useOneX>false</useOneX></authEncryption>".to_string()
    } else {
        let auth = if net.sae { "WPA3SAE" } else { "WPA2PSK" };
        format!(
            "<authEncryption><authentication>{auth}</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption>\
             <sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>",
            xml_escape(password)
        )
    };
    format!(
        "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
         <name>{name}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID></SSIDConfig>\
         <connectionType>ESS</connectionType><connectionMode>auto</connectionMode>\
         <MSM><security>{security}</security></MSM></WLANProfile>"
    )
}

/// Łączy z siecią: zapisany profil wprost, nowa sieć – najpierw profil (z hasłem, jeśli
/// zabezpieczona). Wynik połączenia przychodzi później – widać go w `networks()`.
pub fn connect(ssid: &str, password: Option<&str>) -> Result<(), String> {
    let w = Wlan::open().ok_or("Brak karty Wi-Fi")?;
    let net = networks().and_then(|l| l.into_iter().find(|n| n.ssid == ssid)).ok_or("Sieć jest poza zasięgiem")?;
    let profile = match password {
        Some(pass) => {
            if net.secured && !(8..=63).contains(&pass.chars().count()) {
                return Err("Hasło musi mieć od 8 do 63 znaków".into());
            }
            let xml = HSTRING::from(profile_xml(&net, pass));
            let mut reason = 0u32;
            let r = unsafe { WlanSetProfile(w.h, &w.iface, 0, &xml, PCWSTR::null(), true, None, &mut reason) };
            if r != 0 {
                return Err(format!("Nie udało się zapisać sieci (błąd {r}, powód {reason})"));
            }
            net.ssid.clone()
        }
        None if net.known => net.profile.clone(),
        None if !net.secured => {
            // Otwarta sieć bez profilu – zapisujemy profil bez hasła.
            return connect(ssid, Some(""));
        }
        None => return Err("Podaj hasło sieci".into()),
    };
    let name = HSTRING::from(profile);
    let params = WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: wlan_connection_mode_profile,
        strProfile: PCWSTR(name.as_ptr()),
        pDot11Ssid: std::ptr::null_mut(),
        pDesiredBssidList: std::ptr::null_mut(),
        dot11BssType: dot11_BSS_type_infrastructure,
        dwFlags: 0,
    };
    let r = unsafe { WlanConnect(w.h, &w.iface, &params, None) };
    if r != 0 {
        return Err(format!("Nie udało się połączyć (błąd {r})"));
    }
    Ok(())
}
