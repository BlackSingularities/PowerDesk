//! Okna programów dla paska zadań PowerDesk: lista jak na pasku Windows, aktywowanie
//! i minimalizacja, zamykanie, menu podręczne, przycisk Start i okno paska na wierzchu.

use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_LWIN};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, PostMessageW, SetForegroundWindow, SetWindowPos, ShowWindow, TrackPopupMenu, GWL_EXSTYLE, GW_OWNER, HWND_TOPMOST, MF_STRING, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_MINIMIZE,
    SW_RESTORE, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

/// Okno widoczne na pasku zadań.
#[derive(Clone, Debug, PartialEq)]
pub struct Win {
    pub hwnd: isize,
    pub title: String,
    /// Plik programu – z niego bierzemy ikonę i grupujemy okna.
    pub exe: String,
    pub minimized: bool,
    /// Lewy górny róg monitora, na którym leży okno (do przypisania do paska).
    pub monitor: (i32, i32),
}

fn h(v: isize) -> HWND {
    HWND(v as *mut _)
}

unsafe fn title(w: HWND) -> String {
    let n = GetWindowTextLengthW(w);
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = GetWindowTextW(w, &mut buf);
    String::from_utf16_lossy(&buf[..got as usize])
}

/// Plik programu, do którego należy okno (pusty, gdy nie da się odczytać).
pub fn exe_of_window(hwnd: isize) -> String {
    unsafe { exe_of(h(hwnd)) }
}

unsafe fn exe_of(w: HWND) -> String {
    let mut pid = 0u32;
    GetWindowThreadProcessId(w, Some(&mut pid));
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

unsafe fn monitor_of(w: HWND) -> (i32, i32) {
    let m = MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if GetMonitorInfoW(m, &mut mi).as_bool() {
        (mi.rcMonitor.left, mi.rcMonitor.top)
    } else {
        (0, 0)
    }
}

/// Te same zasady co pasek Windows: widoczne, bez właściciela, nie narzędziowe (chyba że
/// wymuszają przycisk), z tytułem i nieukryte przez DWM (np. aplikacje z innych pulpitów wirtualnych).
unsafe fn on_taskbar(w: HWND) -> bool {
    if !IsWindowVisible(w).as_bool() {
        return false;
    }
    let ex = GetWindowLongPtrW(w, GWL_EXSTYLE) as u32;
    let owned = GetWindow(w, GW_OWNER).is_ok_and(|o| !o.is_invalid());
    if ex & WS_EX_APPWINDOW.0 == 0 && (owned || ex & WS_EX_TOOLWINDOW.0 != 0) {
        return false;
    }
    let mut cloaked = 0u32;
    if DwmGetWindowAttribute(w, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0 {
        return false;
    }
    GetWindowTextLengthW(w) > 0
}

unsafe extern "system" fn collect(w: HWND, lp: LPARAM) -> BOOL {
    let out = &mut *(lp.0 as *mut Vec<Win>);
    if on_taskbar(w) {
        out.push(Win { hwnd: w.0 as isize, title: title(w), exe: exe_of(w), minimized: IsIconic(w).as_bool(), monitor: monitor_of(w) });
    }
    BOOL(1)
}

/// Okna w kolejności z systemu (od najwyżej położonego); pasek układa je po swojemu.
pub fn list() -> Vec<Win> {
    let mut out: Vec<Win> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Klik w przycisk okna: aktywne – minimalizuje, inne – przywraca i wysuwa na wierzch.
pub fn click(hwnd: isize) {
    let w = h(hwnd);
    unsafe {
        if GetForegroundWindow() == w && !IsIconic(w).as_bool() {
            let _ = ShowWindow(w, SW_MINIMIZE);
            return;
        }
        if IsIconic(w).as_bool() {
            let _ = ShowWindow(w, SW_RESTORE);
        }
        let _ = SetForegroundWindow(w);
    }
}

/// Przywraca (jeśli zminimalizowane) i wysuwa na wierzch – bez przełączania jak `click`.
pub fn activate(hwnd: isize) {
    let w = h(hwnd);
    unsafe {
        if IsIconic(w).as_bool() {
            let _ = ShowWindow(w, SW_RESTORE);
        }
        let _ = SetForegroundWindow(w);
    }
}

pub fn minimize(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(h(hwnd), SW_MINIMIZE);
    }
}

pub fn close(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(h(hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// Systemowe menu podręczne w punkcie ekranu; zwraca numer wybranej pozycji.
pub fn pick(owner: isize, items: &[&str], x: i32, y: i32) -> Option<usize> {
    unsafe {
        let m = CreatePopupMenu().ok()?;
        let wide: Vec<Vec<u16>> = items.iter().map(|s| s.encode_utf16().chain(std::iter::once(0)).collect()).collect();
        for (i, s) in wide.iter().enumerate() {
            let _ = AppendMenuW(m, MF_STRING, i + 1, windows::core::PCWSTR(s.as_ptr()));
        }
        let _ = SetForegroundWindow(h(owner));
        let pt = POINT { x, y };
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), h(owner), None);
        let _ = DestroyMenu(m);
        (cmd.0 > 0).then(|| cmd.0 as usize - 1)
    }
}

/// Menu Start Windows (na razie systemowe – jak klawisz Win).
pub fn open_start() {
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_LWIN, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } },
    };
    unsafe {
        SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32);
    }
}

/// Pasek na wierzchu, w podanym miejscu (piksele ekranu), bez kradzenia fokusu.
pub fn place_topmost(hwnd: isize, x: i32, y: i32, width: i32, height: i32) {
    unsafe {
        let _ = SetWindowPos(h(hwnd), Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

/// Podgląd okna (miniatura DWM) rysowany przez system w oknie `dest`; zwraca uchwyt podglądu
/// i rozmiar okna źródłowego w pikselach.
pub fn thumb_register(dest: isize, src: isize) -> Option<(isize, (i32, i32))> {
    use windows::Win32::Graphics::Dwm::{DwmQueryThumbnailSourceSize, DwmRegisterThumbnail};
    unsafe {
        let id = DwmRegisterThumbnail(h(dest), h(src)).ok()?;
        let size = DwmQueryThumbnailSourceSize(id).map(|s| (s.cx, s.cy)).unwrap_or((0, 0));
        Some((id, size))
    }
}

/// Miejsce podglądu w oknie docelowym (piksele obszaru klienta).
pub fn thumb_place(id: isize, x: i32, y: i32, width: i32, height: i32) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE};
    let p = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_SOURCECLIENTAREAONLY,
        rcDestination: RECT { left: x, top: y, right: x + width, bottom: y + height },
        rcSource: RECT::default(),
        opacity: 255,
        fVisible: BOOL(1),
        fSourceClientAreaOnly: BOOL(0),
    };
    unsafe {
        let _ = DwmUpdateThumbnailProperties(id, &p);
    }
}

pub fn thumb_unregister(id: isize) {
    unsafe {
        let _ = windows::Win32::Graphics::Dwm::DwmUnregisterThumbnail(id);
    }
}

/// Obraz wnętrza okna (RGBA), zmniejszony do `max_w`×`max_h` z zachowaniem proporcji.
/// PrintWindow z PW_RENDERFULLCONTENT oddaje też treść rysowaną przez GPU (przeglądarki,
/// Electron). Zminimalizowane okno nie ma treści – None. Wołać poza wątkiem interfejsu.
pub fn snapshot(hwnd: isize, max_w: u32, max_h: u32) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let w = h(hwnd);
    unsafe {
        if IsIconic(w).as_bool() {
            return None;
        }
        let mut rc = RECT::default();
        GetClientRect(w, &mut rc).ok()?;
        let (cw, ch) = (rc.right - rc.left, rc.bottom - rc.top);
        if cw <= 0 || ch <= 0 {
            return None;
        }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, cw, ch);
        let old = SelectObject(mem, bmp.into());
        // PW_CLIENTONLY | PW_RENDERFULLCONTENT.
        let ok = PrintWindow(w, mem, PRINT_WINDOW_FLAGS(3)).as_bool();
        SelectObject(mem, old);
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: cw,
                biHeight: -ch,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (cw * ch * 4) as usize];
        let lines = if ok { GetDIBits(mem, bmp, 0, ch as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS) } else { 0 };
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if lines == 0 {
            return None;
        }
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        let img = image::RgbaImage::from_raw(cw as u32, ch as u32, px)?;
        let scale = (max_w as f32 / cw as f32).min(max_h as f32 / ch as f32).min(1.0);
        let (tw, th) = (((cw as f32 * scale) as u32).max(1), ((ch as f32 * scale) as u32).max(1));
        let small = image::imageops::thumbnail(&img, tw, th);
        Some((tw, th, small.into_raw()))
    }
}

/// Pokaż pulpit (Win+D, przycisk na końcu paska): minimalizuje okna programów, ponownie – przywraca.
pub fn show_desktop() {
    // Własne minimalizowanie i przywracanie: bez Eksploratora systemowe Win+D nic nie robi,
    // a z nim wysunęłoby pulpit Windows nad pulpit PowerDesk.
    crate::shellmode::toggle_desktop();
}

/// Czy okno zajmuje cały monitor (gra, film na pełnym ekranie) – wtedy pasek chowa się pod nie.
/// Pulpit (Progman / WorkerW) się nie liczy.
pub fn covers(hwnd: isize, x: i32, y: i32, width: i32, height: i32) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowRect};
    if hwnd == 0 {
        return false;
    }
    unsafe {
        let mut buf = [0u16; 32];
        let n = GetClassNameW(h(hwnd), &mut buf);
        let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        if class == "Progman" || class == "WorkerW" {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(h(hwnd), &mut r).is_err() {
            return false;
        }
        r.left <= x && r.top <= y && r.right >= x + width && r.bottom >= y + height
    }
}

/// Pasek bez „zawsze na wierzchu” – pod oknem pełnoekranowym.
pub fn place_normal(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{HWND_NOTOPMOST, SWP_NOMOVE, SWP_NOSIZE};
    unsafe {
        let _ = SetWindowPos(h(hwnd), Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Kursor w pikselach ekranu.
pub fn cursor() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Aktywny program dla nakładki: proces, nazwa pliku (bez .exe) i prostokąt jego monitora
/// (lewo, góra, szerokość, wysokość). None – brak okna na pierwszym planie.
pub fn foreground_app() -> Option<(u32, String, (i32, i32, i32, i32))> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    unsafe {
        let w = GetForegroundWindow();
        if w.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, Some(&mut pid));
        let exe = exe_of(w);
        let name = std::path::Path::new(&exe).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let m = MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let r = if GetMonitorInfoW(m, &mut mi).as_bool() { mi.rcMonitor } else { windows::Win32::Foundation::RECT::default() };
        Some((pid, name, (r.left, r.top, r.right - r.left, r.bottom - r.top)))
    }
}

/// Okno zawsze na samym wierzchu, bez zmiany położenia i bez aktywowania (nakładka nad grą).
pub fn keep_topmost(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};
    unsafe {
        let _ = SetWindowPos(h(hwnd), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Czy okno należy do tego procesu (własne okna PowerDesk).
pub fn is_own(hwnd: isize) -> bool {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h(hwnd), Some(&mut pid));
    }
    pid == std::process::id()
}
