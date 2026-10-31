//! Szybkie ustawienia w zasobniku: Bluetooth i tryb samolotowy (Windows.Devices.Radios) oraz
//! jasność ekranu – wbudowany ekran laptopa przez WMI, monitory zewnętrzne przez DDC/CI.
//! Wszystko blokuje (WinRT, PowerShell) – wywoływać w wątku tła.

use windows::Devices::Radios::{Radio, RadioKind, RadioState};

fn radios() -> Vec<Radio> {
    let Ok(op) = Radio::GetRadiosAsync() else { return Vec::new() };
    let Ok(list) = op.get() else { return Vec::new() };
    (0..list.Size().unwrap_or(0)).filter_map(|i| list.GetAt(i).ok()).collect()
}

fn set(radio: &Radio, on: bool) -> Result<(), String> {
    let _ = Radio::RequestAccessAsync().and_then(|op| op.get());
    let state = if on { RadioState::On } else { RadioState::Off };
    radio.SetStateAsync(state).and_then(|op| op.get()).map(|_| ()).map_err(|e| e.message())
}

/// Stan radiów: (Bluetooth: None – brak karty, tryb samolotowy – wszystkie radia wyłączone).
pub fn radio_state() -> (Option<bool>, bool) {
    let list = radios();
    let bt = list.iter().find(|r| r.Kind().ok() == Some(RadioKind::Bluetooth)).map(|r| r.State().ok() == Some(RadioState::On));
    let airplane = !list.is_empty() && list.iter().all(|r| r.State().ok() != Some(RadioState::On));
    (bt, airplane)
}

pub fn set_bluetooth(on: bool) -> Result<(), String> {
    let list = radios();
    let Some(r) = list.iter().find(|r| r.Kind().ok() == Some(RadioKind::Bluetooth)) else { return Err("Brak karty Bluetooth".into()) };
    set(r, on)
}

/// Tryb samolotowy: wyłącza (albo z powrotem włącza) wszystkie radia – Wi-Fi, Bluetooth, sieć komórkową.
pub fn set_airplane(on: bool) -> Result<(), String> {
    let mut err = Ok(());
    for r in radios() {
        if let Err(e) = set(&r, !on) {
            err = Err(e);
        }
    }
    err
}

fn powershell(cmd: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", cmd]).creation_flags(0x0800_0000).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Monitory z DDC/CI (uchwyty fizycznych monitorów) – wywołujący zwalnia je `DestroyPhysicalMonitors`.
fn physical() -> Vec<windows::Win32::Devices::Display::PHYSICAL_MONITOR> {
    use windows::Win32::Devices::Display::{GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR, PHYSICAL_MONITOR};
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
        let v = &mut *(lp.0 as *mut Vec<HMONITOR>);
        v.push(m);
        BOOL(1)
    }
    let mut mons: Vec<HMONITOR> = Vec::new();
    let mut out = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut mons as *mut _ as isize));
        for m in mons {
            let mut n = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(m, &mut n).is_err() || n == 0 {
                continue;
            }
            let mut arr = vec![PHYSICAL_MONITOR::default(); n as usize];
            if GetPhysicalMonitorsFromHMONITOR(m, &mut arr).is_ok() {
                out.extend(arr);
            }
        }
    }
    out
}

/// Jasność 0–100: wbudowany ekran, a bez niego pierwszy monitor z DDC/CI. None – nie da się ustawić.
pub fn brightness() -> Option<u32> {
    if let Some(v) = powershell("(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction Stop | Select-Object -First 1).CurrentBrightness").and_then(|s| s.parse().ok()) {
        return Some(v);
    }
    use windows::Win32::Devices::Display::{DestroyPhysicalMonitors, GetMonitorBrightness};
    let mons = physical();
    let mut found = None;
    for m in &mons {
        let (mut lo, mut cur, mut hi) = (0u32, 0u32, 0u32);
        if unsafe { GetMonitorBrightness(m.hPhysicalMonitor, &mut lo, &mut cur, &mut hi) } != 0 && hi > lo {
            found = Some((cur - lo) * 100 / (hi - lo));
            break;
        }
    }
    let _ = unsafe { DestroyPhysicalMonitors(&mons) };
    found
}

/// Ustawia jasność wszystkich ekranów, które na to pozwalają.
pub fn set_brightness(level: u32) {
    let level = level.min(100);
    let _ = powershell(&format!("Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods -ErrorAction Stop | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=1;Brightness={level}}}"));
    use windows::Win32::Devices::Display::{DestroyPhysicalMonitors, GetMonitorBrightness, SetMonitorBrightness};
    let mons = physical();
    for m in &mons {
        let (mut lo, mut cur, mut hi) = (0u32, 0u32, 0u32);
        if unsafe { GetMonitorBrightness(m.hPhysicalMonitor, &mut lo, &mut cur, &mut hi) } != 0 && hi > lo {
            unsafe { SetMonitorBrightness(m.hPhysicalMonitor, lo + (hi - lo) * level / 100) };
        }
    }
    let _ = unsafe { DestroyPhysicalMonitors(&mons) };
}

/// Dyski wymienne (pendrive, karta pamięci) – litery z ukośnikiem, np. `E:\`. Szybkie, bez dotykania dysków.
pub fn removable() -> Vec<String> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .filter(|root| unsafe { GetDriveTypeW(&windows::core::HSTRING::from(root.as_str())) } == 2)
        .collect()
}

/// Tryb zasilania Windows (Ustawienia → Zasilanie): 0 – najlepsza wydajność energetyczna,
/// 1 – zrównoważony, 2 – najlepsza wydajność. Nakładki planu z powrprof.dll (bez publicznego
/// nagłówka, ale stabilne od Windows 10 1709 – tak przełącza też suwak baterii).
const POWER_MODES: [u128; 3] = [0x961cc777_2547_4f9d_8174_7d86181b8a7a, 0, 0xded574b5_45a0_4f42_8737_46345c09c238];

type GetOverlay = unsafe extern "system" fn(*mut windows::core::GUID) -> u32;
type SetOverlay = unsafe extern "system" fn(*const windows::core::GUID) -> u32;

fn powrprof(name: windows::core::PCSTR) -> Option<unsafe extern "system" fn() -> isize> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    unsafe {
        let lib = LoadLibraryW(windows::core::w!("powrprof.dll")).ok()?;
        GetProcAddress(lib, name)
    }
}

/// Bieżący tryb zasilania (None – system go nie obsługuje).
pub fn power_mode() -> Option<u8> {
    let f = powrprof(windows::core::s!("PowerGetEffectiveOverlayScheme"))?;
    let get: GetOverlay = unsafe { std::mem::transmute(f) };
    let mut g = windows::core::GUID::zeroed();
    if unsafe { get(&mut g) } != 0 {
        return None;
    }
    let v = g.to_u128();
    Some(POWER_MODES.iter().position(|&m| m == v).unwrap_or(1) as u8)
}

pub fn set_power_mode(mode: u8) -> Result<(), String> {
    let f = powrprof(windows::core::s!("PowerSetActiveOverlayScheme")).ok_or("Tryb zasilania niedostępny")?;
    let set: SetOverlay = unsafe { std::mem::transmute(f) };
    let g = windows::core::GUID::from_u128(POWER_MODES[mode.min(2) as usize]);
    match unsafe { set(&g) } {
        0 => Ok(()),
        e => Err(format!("Nie udało się zmienić trybu zasilania (kod {e})")),
    }
}
