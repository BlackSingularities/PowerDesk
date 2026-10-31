//! Hak powłoki (RegisterShellHookWindow): okna, które wołają o uwagę (FlashWindow – nowa
//! wiadomość, koniec pobierania). Pasek pokazuje je jak Windows, póki okno nie stanie się aktywne.
//!
//! To samo okno jest „oknem menedżera zadań” (SetTaskmanWindow): ITaskbarList3 programów
//! (postęp pobierania, plakietka na ikonie) wysyła komunikaty WM_USER+n właśnie do niego.
//! Zbieramy postęp i plakietki, a każdy komunikat przekazujemy też poprzedniemu oknu
//! (ukryty pasek Eksploratora), żeby jego stan został spójny, gdy PowerDesk się zamknie.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::core::{s, w};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IsWindow, RegisterClassW, RegisterShellHookWindow, RegisterWindowMessageW, SendMessageTimeoutW, HICON, MSG,
    SMTO_ABORTIFHUNG, WM_USER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

const HSHELL_WINDOWACTIVATED: usize = 4;
const HSHELL_WINDOWDESTROYED: usize = 2;
const HSHELL_RUDEAPPACTIVATED: usize = 0x8004;
const HSHELL_FLASH: usize = 0x8006;

/// Komunikaty ITaskbarList3 do okna menedżera zadań (wParam – okno programu).
const TBL_PROGRESS_VALUE: u32 = WM_USER + 64;
const TBL_PROGRESS_STATE: u32 = WM_USER + 65;
const TBL_OVERLAY_ICON: u32 = WM_USER + 75;
/// Wartość postępu przychodzi przeskalowana do 0..=0xFFFE.
const PROGRESS_MAX: f32 = 65534.0;

/// Stan paska postępu na przycisku (TBPFLAG).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    None,
    Indeterminate,
    Normal,
    Error,
    Paused,
}

/// Postęp okna: stan i wartość 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub state: ProgressState,
    pub value: f32,
}

static FLASHING: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static PROGRESS: Mutex<Vec<(isize, Progress)>> = Mutex::new(Vec::new());
/// Plakietki okien: obraz RGBA (szerokość, wysokość, piksele).
static OVERLAYS: Mutex<Vec<(isize, (u32, u32, Vec<u8>))>> = Mutex::new(Vec::new());
static MSG_ID: OnceLock<u32> = OnceLock::new();
static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// Nasze okno haka i okno menedżera zadań sprzed nas (Eksplorator).
static OWN: AtomicIsize = AtomicIsize::new(0);
static PREV_TASKMAN: AtomicIsize = AtomicIsize::new(0);

/// Okna, które teraz migają.
pub fn flashing() -> HashSet<isize> {
    FLASHING.lock().unwrap().iter().copied().collect()
}

/// Postęp zgłoszony przez okna (bez tych w stanie „brak”).
pub fn progress() -> HashMap<isize, Progress> {
    PROGRESS.lock().unwrap().iter().copied().collect()
}

/// Plakietki okien (obraz RGBA).
pub fn overlays() -> HashMap<isize, (u32, u32, Vec<u8>)> {
    OVERLAYS.lock().unwrap().iter().cloned().collect()
}

/// Okno zostało aktywowane z paska – przestaje migać od razu.
pub fn clear(hwnd: isize) {
    FLASHING.lock().unwrap().retain(|&h| h != hwnd);
}

fn changed() {
    if let Some(f) = ON_CHANGE.get() {
        f();
    }
}

type GetTaskman = unsafe extern "system" fn() -> HWND;
type SetTaskman = unsafe extern "system" fn(HWND) -> i32;

/// Nieudokumentowane GetTaskmanWindow/SetTaskmanWindow z user32.
unsafe fn taskman_fns() -> Option<(GetTaskman, SetTaskman)> {
    let user32 = GetModuleHandleW(w!("user32.dll")).ok()?;
    let get = GetProcAddress(user32, s!("GetTaskmanWindow"))?;
    let set = GetProcAddress(user32, s!("SetTaskmanWindow"))?;
    Some((std::mem::transmute::<_, GetTaskman>(get), std::mem::transmute::<_, SetTaskman>(set)))
}

/// Nasze okno odbiera ITaskbarList; poprzednie zapamiętujemy do przekazywania.
unsafe fn claim_taskman() {
    let own = OWN.load(Ordering::SeqCst);
    let Some((get, set)) = taskman_fns() else { return };
    let cur = get().0 as isize;
    if own == 0 || cur == own {
        return;
    }
    if cur != 0 {
        PREV_TASKMAN.store(cur, Ordering::SeqCst);
    }
    if set(HWND(own as *mut _)) == 0 {
        crate::log::error("hak powłoki: SetTaskmanWindow nie powiódł się");
    }
}

/// Oddaje okno menedżera zadań Eksploratorowi (przy zamykaniu PowerDesk).
pub fn release() {
    let prev = PREV_TASKMAN.load(Ordering::SeqCst);
    unsafe {
        let Some((get, set)) = taskman_fns() else { return };
        if prev != 0 && get().0 as isize == OWN.load(Ordering::SeqCst) && IsWindow(Some(HWND(prev as *mut _))).as_bool() {
            set(HWND(prev as *mut _));
        }
    }
}

/// Komunikat ITaskbarList dalej do Eksploratora.
unsafe fn forward(msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let prev = PREV_TASKMAN.load(Ordering::SeqCst);
    if prev == 0 || prev == OWN.load(Ordering::SeqCst) {
        return LRESULT(0);
    }
    let mut res = 0usize;
    if SendMessageTimeoutW(HWND(prev as *mut _), msg, wp, lp, SMTO_ABORTIFHUNG, 500, Some(&mut res)).0 == 0 {
        return LRESULT(0);
    }
    LRESULT(res as isize)
}

/// Okno zniknęło – bez migania, postępu i plakietki.
fn forget(win: isize) -> bool {
    let mut diff = false;
    let mut f = FLASHING.lock().unwrap();
    let n = f.len();
    f.retain(|&h| h != win);
    diff |= f.len() != n;
    drop(f);
    let mut p = PROGRESS.lock().unwrap();
    let n = p.len();
    p.retain(|(h, _)| *h != win);
    diff |= p.len() != n;
    drop(p);
    let mut o = OVERLAYS.lock().unwrap();
    let n = o.len();
    o.retain(|(h, _)| *h != win);
    diff | (o.len() != n)
}

fn set_progress(win: isize, f: impl FnOnce(&mut Progress)) {
    let mut list = PROGRESS.lock().unwrap();
    let before = list.iter().find(|(h, _)| *h == win).map(|(_, p)| *p);
    let mut p = before.unwrap_or(Progress { state: ProgressState::None, value: 0.0 });
    f(&mut p);
    list.retain(|(h, _)| *h != win);
    if p.state != ProgressState::None {
        list.push((win, p));
    }
    let after = list.iter().find(|(h, _)| *h == win).map(|(_, p)| *p);
    drop(list);
    if before != after {
        changed();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if MSG_ID.get() == Some(&msg) {
        let win = lp.0;
        let diff = match wp.0 & 0xFFFF_FFFF {
            HSHELL_FLASH => {
                let mut list = FLASHING.lock().unwrap();
                let new = !list.contains(&win);
                if new {
                    list.push(win);
                }
                new
            }
            HSHELL_WINDOWACTIVATED | HSHELL_RUDEAPPACTIVATED => {
                let mut list = FLASHING.lock().unwrap();
                let n = list.len();
                list.retain(|&h| h != win);
                list.len() != n
            }
            HSHELL_WINDOWDESTROYED => forget(win),
            _ => false,
        };
        if diff {
            changed();
        }
        return LRESULT(0);
    }
    match msg {
        TBL_PROGRESS_VALUE => {
            let v = (lp.0 as f32 / PROGRESS_MAX).clamp(0.0, 1.0);
            set_progress(wp.0 as isize, |p| p.value = v);
            forward(msg, wp, lp)
        }
        TBL_PROGRESS_STATE => {
            let state = match lp.0 {
                0x1 => ProgressState::Indeterminate,
                0x2 => ProgressState::Normal,
                0x4 => ProgressState::Error,
                0x8 => ProgressState::Paused,
                _ => ProgressState::None,
            };
            set_progress(wp.0 as isize, |p| p.state = state);
            forward(msg, wp, lp)
        }
        TBL_OVERLAY_ICON => {
            let win = wp.0 as isize;
            // HICON należy do programu, ale uchwyty ikon są wspólne – kopia obrazu od razu.
            let img = if lp.0 == 0 { None } else { crate::trayhost::rgba(HICON(lp.0 as *mut _)) };
            let mut list = OVERLAYS.lock().unwrap();
            let had = list.iter().any(|(h, _)| *h == win);
            list.retain(|(h, _)| *h != win);
            let has = img.is_some();
            if let Some(img) = img {
                list.push((win, img));
            }
            drop(list);
            if had || has {
                changed();
            }
            forward(msg, wp, lp)
        }
        // Pozostałe komunikaty ITaskbarList (karty, przyciski miniatur) obsługuje Eksplorator.
        m if (WM_USER..WM_USER + 100).contains(&m) => forward(msg, wp, lp),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Startuje nasłuch (własny wątek z ukrytym oknem); `on_change` – miganie, postęp albo plakietki się zmieniły.
pub fn start(on_change: impl Fn() + Send + Sync + 'static) {
    if ON_CHANGE.set(Box::new(on_change)).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe {
        let _ = MSG_ID.set(RegisterWindowMessageW(w!("SHELLHOOK")));
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), lpszClassName: w!("PowerDeskShellHook"), ..Default::default() };
        RegisterClassW(&class);
        // Okno najwyższego poziomu (niewidoczne) – okna tylko do komunikatów haka nie dostają.
        let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("PowerDeskShellHook"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, None, None) else { return };
        if !RegisterShellHookWindow(hwnd).as_bool() {
            crate::log::error("hak powłoki: RegisterShellHookWindow nie powiódł się");
            return;
        }
        OWN.store(hwnd.0 as isize, Ordering::SeqCst);
        claim_taskman();
        // Restart Eksploratora zabiera mu okno menedżera zadań z powrotem – odbieramy co kilka sekund.
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            unsafe { claim_taskman() };
            let dead: Vec<isize> = PROGRESS.lock().unwrap().iter().map(|(h, _)| *h).chain(OVERLAYS.lock().unwrap().iter().map(|(h, _)| *h)).filter(|&h| !unsafe { IsWindow(Some(HWND(h as *mut _))).as_bool() }).collect();
            if dead.into_iter().fold(false, |d, h| forget(h) | d) {
                changed();
            }
        });
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
}
