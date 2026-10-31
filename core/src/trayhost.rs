//! Zasobnik: własne okno klasy `Shell_TrayWnd`. `Shell_NotifyIcon` innych programów szuka okna
//! tej klasy (FindWindow – pierwsze w kolejności okien) i wysyła mu WM_COPYDATA – zbieramy ikony,
//! a klik na pasku odsyłamy do programu jego komunikatem zwrotnym.
//!
//! Z Eksploratorem w tle nasze okno stoi przed jego paskiem (najwyżej wśród okien „zawsze na
//! wierzchu”), a każdy komunikat przekazujemy też jemu – AppBary, obszar roboczy i jego okienka
//! działają jak dotąd, a ikony zostają u niego, gdy PowerDesk się zamknie.

use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

use parking_lot::Mutex;
use windows::core::{w, GUID};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, CreateWindowExW, DefWindowProcW, DestroyIcon, DispatchMessageW, GetIconInfo, GetMessageW, GetWindowThreadProcessId, IsWindow, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SendMessageTimeoutW, SetWindowPos, TranslateMessage, HICON, HWND_BROADCAST, HWND_TOPMOST, ICONINFO, MSG, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, WINDOW_EX_STYLE, WM_CONTEXTMENU, WM_COPYDATA, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// Ikona programu w zasobniku.
#[derive(Clone, Debug)]
pub struct Icon {
    pub hwnd: isize,
    pub uid: u32,
    pub guid: Option<GUID>,
    pub callback: u32,
    /// Wersja protokołu (NIM_SETVERSION) – od 4 inny układ parametrów komunikatu zwrotnego.
    pub version: u32,
    pub tip: String,
    pub hidden: bool,
    /// Plik programu, który dodał ikonę (do trwałego identyfikatora).
    pub exe: String,
    /// Obraz RGBA (szerokość, wysokość, piksele).
    pub image: Option<(u32, u32, Vec<u8>)>,
    hicon: isize,
}

impl Icon {
    /// Klucz ikony: identyfikator GUID albo para (okno, numer).
    pub fn key(&self) -> String {
        match self.guid {
            Some(g) => format!("{g:?}"),
            None => format!("{}:{}", self.hwnd, self.uid),
        }
    }

    /// Identyfikator stały między uruchomieniami (okno programu zmienia się co start):
    /// GUID albo nazwa pliku programu z numerem ikony.
    pub fn id(&self) -> String {
        match self.guid {
            Some(g) => format!("{g:?}"),
            None => {
                let name = std::path::Path::new(&self.exe).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                format!("{name}|{}", self.uid)
            }
        }
    }
}

/// Układ NOTIFYICONDATA, jaki powłoka dostaje przez WM_COPYDATA – uchwyty zawsze 32-bitowe.
#[repr(C)]
struct NotifyData32 {
    cb_size: u32,
    hwnd: u32,
    uid: u32,
    flags: u32,
    callback: u32,
    hicon: u32,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: GUID,
    balloon_icon: u32,
}

#[repr(C)]
struct TrayData {
    signature: u32,
    message: u32,
    nid: NotifyData32,
}

const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 0x1;
const NIF_ICON: u32 = 0x2;
const NIF_TIP: u32 = 0x4;
const NIF_STATE: u32 = 0x8;
const NIF_INFO: u32 = 0x10;
const NIF_GUID: u32 = 0x20;
const NIS_HIDDEN: u32 = 0x1;
const NIN_SELECT: u32 = WM_USER;

static ICONS: Mutex<Vec<Icon>> = Mutex::new(Vec::new());
static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// Dymek programu (NIF_INFO): tytuł i treść.
static ON_BALLOON: OnceLock<Box<dyn Fn(String, String) + Send + Sync>> = OnceLock::new();
/// Ostatni klik na ikonie (ekran) – programy pytają o położenie ikony, żeby ustawić przy niej okienko.
static LAST_CLICK: Mutex<(i32, i32)> = Mutex::new((0, 0));
/// Miejsce zasobnika na pasku (x, y, szerokość, wysokość – ekran), gdy jeszcze nic nie kliknięto.
static ANCHOR: Mutex<(i32, i32, i32, i32)> = Mutex::new((0, 0, 0, 0));

/// Nasze okno zasobnika.
static OWN: AtomicIsize = AtomicIsize::new(0);
/// Pasek Eksploratora, któremu przekazujemy komunikaty (0 – nie działa albo jeszcze nie znaleziony).
static EXPLORER: AtomicIsize = AtomicIsize::new(0);
/// Komunikat „TaskbarCreated” i chwila (ms od 1970) naszego ostatniego rozgłoszenia.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static BROADCAST_AT: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Pasek Eksploratora działającego w tle (None w trybie powłoki).
fn explorer() -> Option<HWND> {
    if crate::shellmode::active() {
        return None;
    }
    let cached = EXPLORER.load(Ordering::SeqCst);
    if cached != 0 && unsafe { IsWindow(Some(HWND(cached as *mut _))).as_bool() } {
        return Some(HWND(cached as *mut _));
    }
    let found = crate::shellswap::explorer_tray()?;
    EXPLORER.store(found, Ordering::SeqCst);
    Some(HWND(found as *mut _))
}

/// Komunikat dla paska Eksploratora; wynik jego obsługi (0, gdy nie działa albo nie odpowiada).
unsafe fn forward(to: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let mut res = 0usize;
    if SendMessageTimeoutW(to, msg, wp, lp, SMTO_ABORTIFHUNG, 2000, Some(&mut res)).0 == 0 {
        return LRESULT(0);
    }
    LRESULT(res as isize)
}

/// Nasze okno na początek okien „zawsze na wierzchu” – przed paskiem Eksploratora.
fn raise() {
    let own = OWN.load(Ordering::SeqCst);
    if own != 0 {
        unsafe {
            let _ = SetWindowPos(HWND(own as *mut _), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

/// Programy dodają ikony od nowa – do nas (okno przed paskiem Eksploratora).
fn broadcast_created() {
    raise();
    BROADCAST_AT.store(now_ms(), Ordering::SeqCst);
    unsafe {
        let _ = PostMessageW(Some(HWND_BROADCAST), TASKBAR_CREATED.load(Ordering::SeqCst), WPARAM(0), LPARAM(0));
    }
}

/// Dymki programów z zasobnika (wywołać przed `start`).
pub fn on_balloon(f: impl Fn(String, String) + Send + Sync + 'static) {
    let _ = ON_BALLOON.set(Box::new(f));
}

/// Położenie zasobnika na pasku – odpowiedź dla programów, zanim ktoś kliknie ikonę.
pub fn set_anchor(x: i32, y: i32, width: i32, height: i32) {
    *ANCHOR.lock() = (x, y, width, height);
}

/// Ikony widoczne w zasobniku, w kolejności dodania.
pub fn icons() -> Vec<Icon> {
    ICONS.lock().iter().filter(|i| !i.hidden).cloned().collect()
}

fn changed() {
    if let Some(f) = ON_CHANGE.get() {
        f();
    }
}

fn wide(s: &[u16]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}

/// HICON na RGBA (kolor z kanałem alfa; stare ikony bez alfy – z maski).
pub(crate) unsafe fn rgba(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    let mut info = ICONINFO::default();
    GetIconInfo(icon, &mut info).ok()?;
    let color = info.hbmColor;
    let mask = info.hbmMask;
    let cleanup = || {
        if !color.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(color.0));
        }
        if !mask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(mask.0));
        }
    };
    if color.is_invalid() {
        cleanup();
        return None;
    }
    let mut bm = BITMAP::default();
    if GetObjectW(HGDIOBJ(color.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
        cleanup();
        return None;
    }
    let (wd, ht) = (bm.bmWidth.max(1) as u32, bm.bmHeight.max(1) as u32);
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: wd as i32,
            biHeight: -(ht as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let dc = GetDC(None);
    let mut px = vec![0u8; (wd * ht * 4) as usize];
    let got = GetDIBits(dc, color, 0, ht, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
    let mut mask_px = vec![0u8; (wd * ht * 4) as usize];
    let has_mask = !mask.is_invalid() && GetDIBits(dc, mask, 0, ht, Some(mask_px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS) != 0;
    ReleaseDC(None, dc);
    cleanup();
    if got == 0 {
        return None;
    }
    let alpha = px.chunks(4).any(|p| p[3] != 0);
    for (i, p) in px.chunks_mut(4).enumerate() {
        p.swap(0, 2);
        if !alpha {
            p[3] = if has_mask && mask_px[i * 4] != 0 { 0 } else { 255 };
        }
    }
    Some((wd, ht, px))
}

fn same(i: &Icon, n: &NotifyData32) -> bool {
    if n.flags & NIF_GUID != 0 {
        return i.guid == Some(n.guid);
    }
    i.hwnd == n.hwnd as i32 as isize && i.uid == n.uid
}

unsafe fn apply(i: &mut Icon, n: &NotifyData32) {
    if n.flags & NIF_MESSAGE != 0 {
        i.callback = n.callback;
    }
    if n.flags & NIF_TIP != 0 {
        i.tip = wide(&n.tip);
    }
    if n.flags & NIF_STATE != 0 && n.state_mask & NIS_HIDDEN != 0 {
        i.hidden = n.state & NIS_HIDDEN != 0;
    }
    if n.flags & NIF_INFO != 0 {
        let (title, text) = (wide(&n.info_title), wide(&n.info));
        if !text.is_empty() {
            if let Some(f) = ON_BALLOON.get() {
                f(title, text);
            }
        }
    }
    if n.flags & NIF_ICON != 0 {
        if i.hicon != 0 {
            let _ = DestroyIcon(HICON(i.hicon as *mut _));
            i.hicon = 0;
        }
        let src = HICON(n.hicon as i32 as isize as *mut _);
        if let Ok(c) = CopyIcon(src) {
            i.image = rgba(c);
            i.hicon = c.0 as isize;
        }
    }
}

/// Komunikat od Shell_NotifyIcon. Zwraca TRUE, gdy przyjęty.
unsafe fn notify(data: &TrayData) -> bool {
    let n = &data.nid;
    let mut icons = ICONS.lock();
    let pos = icons.iter().position(|i| same(i, n));
    match data.message {
        NIM_ADD => {
            if pos.is_some() {
                return false;
            }
            let mut i = Icon {
                hwnd: n.hwnd as i32 as isize,
                uid: n.uid,
                guid: (n.flags & NIF_GUID != 0).then_some(n.guid),
                callback: 0,
                version: 0,
                tip: String::new(),
                hidden: false,
                exe: crate::taskwin::exe_of_window(n.hwnd as i32 as isize),
                image: None,
                hicon: 0,
            };
            apply(&mut i, n);
            icons.push(i);
        }
        NIM_MODIFY => {
            let Some(p) = pos else { return false };
            apply(&mut icons[p], n);
        }
        NIM_DELETE => {
            let Some(p) = pos else { return false };
            let i = icons.remove(p);
            if i.hicon != 0 {
                let _ = DestroyIcon(HICON(i.hicon as *mut _));
            }
        }
        NIM_SETVERSION => {
            let Some(p) = pos else { return false };
            icons[p].version = n.version;
        }
        _ => return true,
    }
    drop(icons);
    changed();
    true
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let created = TASKBAR_CREATED.load(Ordering::SeqCst);
    if msg == created && created != 0 {
        // Eksplorator wstał od nowa i stanął przed nami – programy dodały ikony jemu. Po chwili
        // wracamy na początek i prosimy o nie jeszcze raz (własne rozgłoszenie pomijamy).
        if !crate::shellmode::active() && now_ms().saturating_sub(BROADCAST_AT.load(Ordering::SeqCst)) > 5000 {
            BROADCAST_AT.store(now_ms(), Ordering::SeqCst);
            EXPLORER.store(0, Ordering::SeqCst);
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                broadcast_created();
            });
        }
        return LRESULT(0);
    }
    if msg == WM_COPYDATA {
        let cds = &*(lp.0 as *const COPYDATASTRUCT);
        let ex = explorer();
        // 1 – ikona zasobnika (nasza i – w tle – kopia u Eksploratora).
        if cds.dwData == 1 && cds.cbData as usize >= 8 + 24 {
            if let Some(e) = ex {
                forward(e, msg, wp, lp);
            }
            let mut buf = vec![0u8; std::mem::size_of::<TrayData>()];
            let n = (cds.cbData as usize).min(buf.len());
            std::ptr::copy_nonoverlapping(cds.lpData as *const u8, buf.as_mut_ptr(), n);
            let data = &*(buf.as_ptr() as *const TrayData);
            return LRESULT(notify(data) as isize);
        }
        // Shell_NotifyIconGetRect: 1 – lewy górny róg, 2 – prawy dolny (po dwa słowa 16-bitowe).
        if cds.dwData == 3 && cds.cbData >= 8 {
            let what = *(cds.lpData as *const u32).add(1);
            let (x, y, w, h) = icon_rect();
            let (a, b) = if what == 1 { (x, y) } else { (x + w, y + h) };
            return LRESULT((((b as u16) as isize) << 16) | (a as u16 as isize));
        }
        // AppBary i reszta: z Eksploratorem – jego sprawa (on liczy obszar roboczy).
        if let Some(e) = ex {
            return forward(e, msg, wp, lp);
        }
        if cds.dwData == 0 {
            return LRESULT(appbar(cds.lpData as *const u8, cds.cbData as usize));
        }
        return LRESULT(0);
    }
    // Komunikaty programów do paska (WM_USER i zarejestrowane) – trafiły do nas zamiast do Eksploratora.
    if msg >= WM_USER {
        if let Some(e) = explorer() {
            return forward(e, msg, wp, lp);
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// SHAppBarMessage od innego programu. Dwa układy danych: 64-bitowy z wyrównaniem (80 bajtów,
/// Windows 10/11 x64) i spakowany z 32-bitowymi uchwytami (52 bajty). Wynik (prostokąt) wraca
/// przez pamięć współdzieloną nadawcy (SHLockShared).
unsafe fn appbar(p: *const u8, len: usize) -> isize {
    use crate::shellmode;
    use windows::Win32::Foundation::{HANDLE, RECT};
    use windows::Win32::UI::Shell::{SHLockShared, SHUnlockShared};
    let rd32 = |off: usize| std::ptr::read_unaligned(p.add(off) as *const u32);
    let rd64 = |off: usize| std::ptr::read_unaligned(p.add(off) as *const u64);
    let rect = |off: usize| std::ptr::read_unaligned(p.add(off) as *const RECT);
    // (okno, komunikat zwrotny, krawędź, prostokąt, lParam, wiadomość, pamięć, proces, przesunięcia krawędzi i rc w APPBARDATA)
    let (hwnd, cb, edge, rc, lparam, msg, shared, pid, edge_off, rc_off) = if len >= 80 {
        (rd64(8) as isize, rd32(16), rd32(24), rect(32), rd64(48) as isize, rd32(56), rd64(64) as isize, rd32(72), 24usize, 32usize)
    } else if len >= 52 {
        (rd32(4) as i32 as isize, rd32(8), rd32(12), rect(16), rd64(32) as isize, rd32(40), rd32(44) as i32 as isize, rd32(48), 12usize, 16usize)
    } else {
        return 0;
    };
    let _ = lparam;
    // Zapis prostokąta (i krawędzi) do APPBARDATA nadawcy.
    let write_back = |rc: RECT, edge: Option<u32>| {
        if shared == 0 {
            return;
        }
        let m = SHLockShared(HANDLE(shared as *mut _), pid) as *mut u8;
        if m.is_null() {
            return;
        }
        std::ptr::write_unaligned(m.add(rc_off) as *mut RECT, rc);
        if let Some(e) = edge {
            std::ptr::write_unaligned(m.add(edge_off) as *mut u32, e);
        }
        let _ = SHUnlockShared(m as *const _);
    };
    match msg {
        0 => shellmode::appbar_new(hwnd, cb) as isize,
        1 => {
            shellmode::appbar_remove(hwnd);
            1
        }
        2 => {
            write_back(shellmode::appbar_fit(hwnd, edge, rc), None);
            1
        }
        3 => {
            let fit = shellmode::appbar_fit(hwnd, edge, rc);
            shellmode::appbar_set(hwnd, edge, fit);
            write_back(fit, None);
            1
        }
        // ABM_GETSTATE: zawsze na wierzchu, bez autoukrywania.
        4 => 2,
        5 => match shellmode::taskbar_rect() {
            Some(r) => {
                write_back(r, Some(3));
                1
            }
            None => 0,
        },
        _ => 1,
    }
}

/// Położenie ikony na ekranie: przy ostatnim kliku, inaczej zasobnik na pasku.
fn icon_rect() -> (i32, i32, i32, i32) {
    let (cx, cy) = *LAST_CLICK.lock();
    if cx != 0 || cy != 0 {
        return (cx - 12, cy - 12, 24, 24);
    }
    *ANCHOR.lock()
}

/// Okno zasobnika i wezwanie programów (TaskbarCreated), żeby dodały ikony od nowa.
/// `on_change` woła się z wątku zasobnika po każdej zmianie ikon.
pub fn start(on_change: impl Fn() + Send + Sync + 'static) {
    if ON_CHANGE.set(Box::new(on_change)).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: w!("Shell_TrayWnd"), ..Default::default() };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0), w!("Shell_TrayWnd"), None, WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None) else { return };
        OWN.store(hwnd.0 as isize, Ordering::SeqCst);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::SeqCst);
        broadcast_created();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
    // Program zakończony bez NIM_DELETE – ikona znika przy najbliższym przeglądzie.
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let mut icons = ICONS.lock();
        let before = icons.len();
        icons.retain(|i| unsafe { IsWindow(Some(HWND(i.hwnd as *mut _))).as_bool() });
        let gone = icons.len() != before;
        drop(icons);
        if crate::shellmode::active() {
            crate::shellmode::appbar_prune();
        } else {
            // Eksplorator potrafi wysunąć swój pasek przed nasze okno (zmiana ustawień, monitorów).
            raise();
        }
        if gone {
            changed();
        }
    });
}

/// Przycisk myszy na ikonie zasobnika.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Left,
    Double,
    Right,
}

/// Klik na ikonie (x, y – kursor na ekranie): komunikat zwrotny do programu jak od Eksploratora.
pub fn click(key: &str, button: Click, x: i32, y: i32) {
    let Some(i) = ICONS.lock().iter().find(|i| i.key() == key).cloned() else { return };
    if i.callback == 0 {
        return;
    }
    *LAST_CLICK.lock() = (x, y);
    let hwnd = HWND(i.hwnd as *mut _);
    unsafe {
        // Program może wysunąć swoje menu albo okno na wierzch.
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
        let send = |m: u32| {
            let (wp, lp) = if i.version >= 4 {
                let xy = ((y as u16 as usize) << 16) | (x as u16 as usize);
                (WPARAM(xy), LPARAM((((i.uid as u16) as isize) << 16) | (m as u16 as isize)))
            } else {
                (WPARAM(i.uid as usize), LPARAM(m as isize))
            };
            let _ = PostMessageW(Some(hwnd), i.callback, wp, lp);
        };
        match button {
            Click::Left => {
                send(WM_LBUTTONDOWN);
                send(WM_LBUTTONUP);
                if i.version >= 4 {
                    send(NIN_SELECT);
                }
            }
            Click::Double => send(WM_LBUTTONDBLCLK),
            Click::Right => {
                send(WM_RBUTTONDOWN);
                send(WM_RBUTTONUP);
                if i.version >= 4 {
                    send(WM_CONTEXTMENU);
                }
            }
        }
    }
}
