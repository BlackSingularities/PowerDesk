//! Klatki na sekundę dowolnej aplikacji – ze zdarzeń systemowych Windows (ETW), jak PresentMon:
//! nic nie wstrzykujemy do programu. Źródła: DXGI (DirectX 10–12), D3D9 i jądro grafiki
//! (DxgKrnl – też Vulkan i OpenGL). Program, który ma zdarzenia DXGI/D3D9, liczymy z nich,
//! pozostałe – z jądra, żeby klatka nie liczyła się dwa razy.
//!
//! Sesja ETW wymaga administratora albo członkostwa w grupie „Użytkownicy dzienników
//! wydajności” (S-1-5-32-559) – `grant_access` dopisuje do niej bieżące konto (jednorazowo, UAC).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use windows::core::{GUID, HSTRING, PWSTR};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_SUCCESS};
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW, CONTROLTRACE_HANDLE, EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW,
    EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE, PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, WNODE_FLAG_TRACED_GUID,
};
use windows::Win32::System::Performance::QueryPerformanceFrequency;

const SESSION: &str = "PowerDeskFrames";
const DXGI: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
const D3D9: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
const DXGKRNL: GUID = GUID::from_u128(0x802EC45A_1E99_4B83_9920_87C98277BA9D);
/// Present_Start w DXGI i D3D9, Present_Info w DxgKrnl.
const DXGI_PRESENT: u16 = 42;
const D3D9_PRESENT: u16 = 1;
const KRNL_PRESENT: u16 = 184;
/// Słowo kluczowe „Present” DxgKrnl – bez niego jądro grafiki zasypałoby sesję zdarzeniami.
const KRNL_KEYWORD_PRESENT: u64 = 0x800_0000;

/// Ile czasu klatek trzymamy na proces (sekundy).
const WINDOW_SECS: f64 = 3.0;

#[derive(Default)]
struct Frames {
    /// Czasy klatek (sekundy QPC) z DXGI/D3D9 i z jądra.
    api: VecDeque<f64>,
    krnl: VecDeque<f64>,
}

static FRAMES: Mutex<Option<HashMap<u32, Frames>>> = Mutex::new(None);
static FREQ: AtomicU64 = AtomicU64::new(0);

/// Stan pomiaru.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Running,
    /// Brak uprawnień do sesji ETW.
    NoAccess,
    Error(String),
}

static STATE: Mutex<State> = Mutex::new(State::Off);

pub fn state() -> State {
    STATE.lock().clone()
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let r = &*rec;
    let h = &r.EventHeader;
    let id = h.EventDescriptor.Id;
    let api = (h.ProviderId == DXGI && id == DXGI_PRESENT) || (h.ProviderId == D3D9 && id == D3D9_PRESENT);
    let krnl = h.ProviderId == DXGKRNL && id == KRNL_PRESENT;
    if !api && !krnl {
        return;
    }
    let freq = FREQ.load(Ordering::Relaxed).max(1) as f64;
    let t = h.TimeStamp as f64 / freq;
    let mut g = FRAMES.lock();
    let map = g.get_or_insert_with(HashMap::new);
    let f = map.entry(h.ProcessId).or_default();
    let q = if api { &mut f.api } else { &mut f.krnl };
    q.push_back(t);
    while q.front().is_some_and(|&x| t - x > WINDOW_SECS) {
        q.pop_front();
    }
}

/// Bufor EVENT_TRACE_PROPERTIES z nazwą sesji za strukturą.
fn props() -> Vec<u8> {
    let name: Vec<u16> = SESSION.encode_utf16().chain(Some(0)).collect();
    let size = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + name.len() * 2;
    let mut buf = vec![0u8; size];
    unsafe {
        let p = &mut *(buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
        p.Wnode.BufferSize = size as u32;
        p.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        // Znaczniki czasu z QueryPerformanceCounter.
        p.Wnode.ClientContext = 1;
        p.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        p.BufferSize = 64;
        p.MinimumBuffers = 4;
        p.MaximumBuffers = 32;
        p.FlushTimer = 1;
        p.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    }
    buf
}

fn stop_session() {
    let mut buf = props();
    unsafe {
        let _ = ControlTraceW(CONTROLTRACE_HANDLE::default(), &HSTRING::from(SESSION), buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP);
    }
}

/// Uruchamia sesję ETW i wątek, który ją czyta. Drugie wywołanie przy działającej – nic.
pub fn start() {
    if *STATE.lock() == State::Running {
        return;
    }
    let mut f = 0i64;
    unsafe {
        let _ = QueryPerformanceFrequency(&mut f);
    }
    FREQ.store(f as u64, Ordering::Relaxed);
    // Sesja po poprzednim uruchomieniu (np. po awarii) – zamykamy i zakładamy od nowa.
    stop_session();
    let mut buf = props();
    let mut handle = CONTROLTRACE_HANDLE::default();
    let r = unsafe { StartTraceW(&mut handle, &HSTRING::from(SESSION), buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
    if r == ERROR_ACCESS_DENIED {
        *STATE.lock() = State::NoAccess;
        return;
    }
    if r != ERROR_SUCCESS && r != ERROR_ALREADY_EXISTS {
        *STATE.lock() = State::Error(format!("StartTrace: {}", r.0));
        return;
    }
    unsafe {
        for (guid, keyword) in [(DXGI, 0u64), (D3D9, 0), (DXGKRNL, KRNL_KEYWORD_PRESENT)] {
            let _ = EnableTraceEx2(handle, &guid, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, 4, keyword, 0, 0, None);
        }
    }
    *STATE.lock() = State::Running;
    std::thread::spawn(|| unsafe {
        let mut name: Vec<u16> = SESSION.encode_utf16().chain(Some(0)).collect();
        let mut log = EVENT_TRACE_LOGFILEW { LoggerName: PWSTR(name.as_mut_ptr()), ..Default::default() };
        log.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        log.Anonymous2.EventRecordCallback = Some(on_event);
        let h = OpenTraceW(&mut log);
        if h.Value == u64::MAX {
            *STATE.lock() = State::Error("OpenTrace".into());
            return;
        }
        // Blokuje do zamknięcia sesji.
        let _ = ProcessTrace(&[h], None, None);
        let _ = CloseTrace(h);
        let mut s = STATE.lock();
        if *s == State::Running {
            *s = State::Off;
        }
    });
}

/// Zatrzymuje pomiar (sesja ETW zamknięta – wątek czytający się kończy).
pub fn stop() {
    stop_session();
    *STATE.lock() = State::Off;
    *FRAMES.lock() = None;
}

/// Klatki procesu: FPS z ostatniej sekundy, średni i najdłuższy czas klatki (ms) oraz
/// czasy ostatnich klatek (ms, najstarsze pierwsze, do `hist`).
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub fps: f64,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub times_ms: Vec<f32>,
}

pub fn stats(pid: u32, hist: usize) -> Option<Stats> {
    let g = FRAMES.lock();
    let f = g.as_ref()?.get(&pid)?;
    let q = if f.api.len() >= 2 { &f.api } else { &f.krnl };
    if q.len() < 2 {
        return None;
    }
    let last = *q.back()?;
    // Bieżący czas – z ostatniej klatki dowolnego procesu nie wiemy; program, który przestał
    // rysować, rozpoznajemy po tym, że jego ostatnia klatka jest stara względem innych.
    let now = g.as_ref()?.values().flat_map(|x| x.api.back().into_iter().chain(x.krnl.back())).fold(last, |a, &b| a.max(b));
    if now - last > 1.0 {
        return Some(Stats::default());
    }
    let recent: Vec<f64> = q.iter().copied().filter(|&t| last - t <= 1.0).collect();
    let deltas: Vec<f64> = q.iter().zip(q.iter().skip(1)).map(|(a, b)| (b - a) * 1000.0).collect();
    let tail = &deltas[deltas.len().saturating_sub(hist)..];
    let span = recent.last().zip(recent.first()).map_or(0.0, |(b, a)| b - a);
    let fps = if span > 0.0 { (recent.len() - 1) as f64 / span } else { 0.0 };
    let avg = if tail.is_empty() { 0.0 } else { tail.iter().sum::<f64>() / tail.len() as f64 };
    let max = tail.iter().copied().fold(0.0, f64::max);
    Some(Stats { fps, avg_ms: avg, max_ms: max, times_ms: tail.iter().map(|&x| x as f32).collect() })
}

/// Dopisuje bieżące konto do grupy „Użytkownicy dzienników wydajności” (nazwa grupy zależy od
/// języka Windows – szukamy po SID). Okno UAC; działa po ponownym zalogowaniu.
pub fn grant_access() -> Result<(), String> {
    use windows::core::w;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
    let cmd = format!("-NoProfile -WindowStyle Hidden -Command \"Add-LocalGroupMember -SID 'S-1-5-32-559' -Member '{}' -ErrorAction SilentlyContinue\"", user.replace('\'', "''"));
    let r = unsafe { ShellExecuteW(None, w!("runas"), w!("powershell.exe"), &HSTRING::from(cmd), None, SW_HIDE) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err("Brak zgody na uprawnienia administratora".into())
    }
}
