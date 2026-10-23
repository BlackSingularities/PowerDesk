//! Karta graficzna dla nakładki: obciążenie i zajęta pamięć z liczników wydajności Windows
//! (te same co w Menedżerze zadań – każda karta), pojemność pamięci z DXGI, temperatura
//! z NVML (karty NVIDIA, biblioteka sterownika; inne karty – brak temperatury).
//! Liczniki „na sekundę” potrzebują dwóch odczytów, więc zapytanie żyje we własnym wątku.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use windows::core::{w, HSTRING, PCSTR};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct Gpu {
    /// Obciążenie 3D najbardziej zajętej karty (0–100).
    pub usage: f32,
    pub vram_used: u64,
    pub vram_total: u64,
    pub temp: Option<u32>,
}

static SNAP: Mutex<Gpu> = Mutex::new(Gpu { usage: 0.0, vram_used: 0, vram_total: 0, temp: None });
static STARTED: AtomicBool = AtomicBool::new(false);

pub fn snapshot() -> Gpu {
    *SNAP.lock()
}

/// Wartości licznika dla wszystkich instancji: (nazwa instancji, wartość).
unsafe fn values(c: PDH_HCOUNTER) -> Vec<(String, f64)> {
    let (mut size, mut count) = (0u32, 0u32);
    if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, None) != PDH_MORE_DATA {
        return Vec::new();
    }
    let mut buf = vec![0u8; size as usize];
    let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
    if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
        return Vec::new();
    }
    (0..count as usize)
        .map(|i| {
            let it = &*items.add(i);
            (it.szName.to_string().unwrap_or_default(), it.FmtValue.Anonymous.doubleValue)
        })
        .collect()
}

/// Karta z nazwy instancji („…luid_0x…_0x…_phys_0…”).
fn adapter(name: &str) -> String {
    let start = name.find("luid_").unwrap_or(0);
    let rest = &name[start..];
    let end = rest.find("_eng_").unwrap_or(rest.len());
    rest[..end].to_string()
}

/// Największa pamięć dedykowana kart (bez karty programowej).
/// Nazwa karty graficznej z największą pamięcią (bez kart programowych).
pub fn name() -> String {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| unsafe {
        let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return String::new() };
        let (mut best, mut name) = (0u64, String::new());
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            if let Ok(d) = a.GetDesc1() {
                if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 && (name.is_empty() || d.DedicatedVideoMemory as u64 > best) {
                    best = d.DedicatedVideoMemory as u64;
                    let n = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
                    name = String::from_utf16_lossy(&d.Description[..n]).trim().to_string();
                }
            }
            i += 1;
        }
        name
    })
    .clone()
}

fn vram_total() -> u64 {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    unsafe {
        let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return 0 };
        let mut best = 0u64;
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            if let Ok(d) = a.GetDesc1() {
                if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 {
                    best = best.max(d.DedicatedVideoMemory as u64);
                }
            }
            i += 1;
        }
        best
    }
}

type NvmlInit = unsafe extern "C" fn() -> i32;
type NvmlHandle = unsafe extern "C" fn(u32, *mut *mut std::ffi::c_void) -> i32;
type NvmlTemp = unsafe extern "C" fn(*mut std::ffi::c_void, u32, *mut u32) -> i32;

/// Temperatura pierwszej karty NVIDIA przez NVML; None – brak biblioteki albo karty.
struct Nvml {
    dev: *mut std::ffi::c_void,
    temp: NvmlTemp,
}

impl Nvml {
    unsafe fn open() -> Option<Nvml> {
        let lib = LoadLibraryW(w!("nvml.dll")).or_else(|_| LoadLibraryW(&HSTRING::from(r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll"))).ok()?;
        let sym = |n: &[u8]| GetProcAddress(lib, PCSTR(n.as_ptr()));
        let init: NvmlInit = std::mem::transmute(sym(b"nvmlInit_v2\0")?);
        let handle: NvmlHandle = std::mem::transmute(sym(b"nvmlDeviceGetHandleByIndex_v2\0")?);
        let temp: NvmlTemp = std::mem::transmute(sym(b"nvmlDeviceGetTemperature\0")?);
        if init() != 0 {
            return None;
        }
        let mut dev = std::ptr::null_mut();
        if handle(0, &mut dev) != 0 {
            return None;
        }
        Some(Nvml { dev, temp })
    }

    fn read(&self) -> Option<u32> {
        let mut t = 0u32;
        // 0 – czujnik rdzenia karty.
        (unsafe { (self.temp)(self.dev, 0, &mut t) } == 0).then_some(t)
    }
}

/// Wątek odczytów co sekundę (raz na cały program).
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| unsafe {
        let total = vram_total();
        let nvml = Nvml::open();
        let mut q = PDH_HQUERY::default();
        if PdhOpenQueryW(None, 0, &mut q) != 0 {
            return;
        }
        let (mut c_eng, mut c_mem) = (PDH_HCOUNTER::default(), PDH_HCOUNTER::default());
        let eng_ok = PdhAddEnglishCounterW(q, w!(r"\GPU Engine(*engtype_3D)\Utilization Percentage"), 0, &mut c_eng) == 0;
        let mem_ok = PdhAddEnglishCounterW(q, w!(r"\GPU Adapter Memory(*)\Dedicated Usage"), 0, &mut c_mem) == 0;
        let _ = PdhCollectQueryData(q);
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let _ = PdhCollectQueryData(q);
            let mut per: HashMap<String, f64> = HashMap::new();
            if eng_ok {
                for (name, v) in values(c_eng) {
                    *per.entry(adapter(&name)).or_default() += v;
                }
            }
            let usage = per.values().copied().fold(0.0, f64::max).clamp(0.0, 100.0) as f32;
            let used = if mem_ok { values(c_mem).into_iter().map(|(_, v)| v).fold(0.0, f64::max) as u64 } else { 0 };
            *SNAP.lock() = Gpu { usage, vram_used: used, vram_total: total, temp: nvml.as_ref().and_then(|n| n.read()) };
        }
    });
}
