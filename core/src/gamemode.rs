//! Tryb gry: plan zasilania „Najwyższa wydajność” na czas gry (okno na cały monitor) i limit
//! klatek w profilu sterownika NVIDIA (globalnie, jak „Maksymalna liczba klatek” w Panelu
//! sterowania NVIDIA) – bez wstrzykiwania czegokolwiek do gier.

use windows::core::GUID;
use windows::Win32::System::Power::{PowerGetActiveScheme, PowerSetActiveScheme};

/// Plan „Wysoka wydajność” Windows (zawsze dostępny).
const HIGH_PERFORMANCE: GUID = GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c);

/// Bieżący plan zasilania.
pub fn active_scheme() -> Option<GUID> {
    unsafe {
        let mut p: *mut GUID = std::ptr::null_mut();
        if PowerGetActiveScheme(None, &mut p).is_err() || p.is_null() {
            return None;
        }
        let g = *p;
        let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(p as *mut _)));
        Some(g)
    }
}

pub fn set_scheme(g: &GUID) -> bool {
    unsafe { PowerSetActiveScheme(None, Some(g)).is_ok() }
}

/// Włącza plan wysokiej wydajności; zwraca poprzedni plan (do przywrócenia po grze).
pub fn boost() -> Option<u128> {
    let prev = active_scheme()?;
    if prev == HIGH_PERFORMANCE {
        return None;
    }
    set_scheme(&HIGH_PERFORMANCE).then_some(prev.to_u128())
}

/// Przywraca plan zapisany przez `boost`.
pub fn restore_scheme(g: u128) -> bool {
    set_scheme(&GUID::from_u128(g))
}

// ---------- Limit klatek NVIDIA (NVAPI, profil globalny) ----------

const ID_INIT: u32 = 0x0150_E828;
const ID_CREATE: u32 = 0x0694_D52E;
const ID_LOAD: u32 = 0x375D_BD6B;
const ID_BASE: u32 = 0xDA84_66A0;
const ID_SET: u32 = 0x577D_D202;
const ID_DELETE: u32 = 0xE4A2_6362;
const ID_SAVE: u32 = 0xFCBC_7E14;
const ID_DESTROY: u32 = 0xDAD9_CFF8;
/// Ustawienie „Maksymalna liczba klatek” (FRL_FPS).
const FRL_FPS: u32 = 0x1083_5002;
/// NVDRS_SETTING w wersji 1: rozmiar 0x3020, wersja w górnym słowie.
const SETTING_SIZE: usize = 0x3020;

type QueryInterface = unsafe extern "C" fn(u32) -> *const std::ffi::c_void;
type Init = unsafe extern "C" fn() -> i32;
type Create = unsafe extern "C" fn(*mut *mut std::ffi::c_void) -> i32;
type Session = unsafe extern "C" fn(*mut std::ffi::c_void) -> i32;
type Base = unsafe extern "C" fn(*mut std::ffi::c_void, *mut *mut std::ffi::c_void) -> i32;
type Set = unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut u8) -> i32;
type Delete = unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, u32) -> i32;

/// Limit klatek wszystkich gier na kartach NVIDIA; 0 – bez limitu. Zapis profilu sterownika
/// może wymagać uprawnień administratora.
pub fn set_fps_limit(fps: u32) -> Result<(), String> {
    use windows::core::{s, w};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    unsafe {
        let lib = LoadLibraryW(w!("nvapi64.dll")).map_err(|_| crate::lang::tr("Brak sterownika NVIDIA (nvapi64.dll)").to_string())?;
        let q: QueryInterface = std::mem::transmute(GetProcAddress(lib, s!("nvapi_QueryInterface")).ok_or("NVAPI bez nvapi_QueryInterface")?);
        let f = |id: u32| {
            let p = q(id);
            (!p.is_null()).then_some(p)
        };
        let init: Init = std::mem::transmute(f(ID_INIT).ok_or(crate::lang::tr("NVAPI: brak Initialize"))?);
        let create: Create = std::mem::transmute(f(ID_CREATE).ok_or(crate::lang::tr("NVAPI: brak CreateSession"))?);
        let load: Session = std::mem::transmute(f(ID_LOAD).ok_or(crate::lang::tr("NVAPI: brak LoadSettings"))?);
        let base: Base = std::mem::transmute(f(ID_BASE).ok_or(crate::lang::tr("NVAPI: brak GetBaseProfile"))?);
        let set: Set = std::mem::transmute(f(ID_SET).ok_or(crate::lang::tr("NVAPI: brak SetSetting"))?);
        let delete: Delete = std::mem::transmute(f(ID_DELETE).ok_or(crate::lang::tr("NVAPI: brak DeleteProfileSetting"))?);
        let save: Session = std::mem::transmute(f(ID_SAVE).ok_or(crate::lang::tr("NVAPI: brak SaveSettings"))?);
        let destroy: Session = std::mem::transmute(f(ID_DESTROY).ok_or(crate::lang::tr("NVAPI: brak DestroySession"))?);
        let check = |r: i32, what: &str| if r == 0 { Ok(()) } else { Err(crate::trf!("NVAPI {what}: błąd {r}")) };
        check(init(), "Initialize")?;
        let mut session = std::ptr::null_mut();
        check(create(&mut session), "CreateSession")?;
        let result = (|| {
            check(load(session), "LoadSettings")?;
            let mut profile = std::ptr::null_mut();
            check(base(session, &mut profile), "GetBaseProfile")?;
            if fps == 0 {
                // Brak ustawienia w profilu = brak limitu; „nie znaleziono” też jest w porządku.
                let r = delete(session, profile, FRL_FPS);
                if r != 0 && r != -160 {
                    return Err(crate::trf!("NVAPI DeleteProfileSetting: błąd {r}"));
                }
            } else {
                let mut buf = vec![0u8; SETTING_SIZE];
                let version = (SETTING_SIZE as u32) | (1 << 16);
                buf[0..4].copy_from_slice(&version.to_le_bytes());
                buf[4100..4104].copy_from_slice(&FRL_FPS.to_le_bytes());
                // Typ: liczba (0); bieżąca wartość za wartością predefiniowaną.
                buf[8220..8224].copy_from_slice(&fps.to_le_bytes());
                check(set(session, profile, buf.as_mut_ptr()), "SetSetting")?;
            }
            check(save(session), crate::lang::tr("SaveSettings (może wymagać administratora)"))
        })();
        destroy(session);
        result
    }
}
