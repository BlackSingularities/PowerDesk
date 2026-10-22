//! Jedna instancja: pierwsze okno słucha na potoku nazwanym, kolejne uruchomienia (Win+E,
//! „PowerDesk.exe C:\folder”) przekazują mu swoje argumenty i od razu się kończą.

use std::io::{Read, Write};
use std::os::windows::io::FromRawHandle;

use windows::core::HSTRING;
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_INBOUND;
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, IsIconic, SetForegroundWindow, ShowWindow, ASFW_ANY, SW_RESTORE};

/// Potok osobny dla każdego użytkownika – dwa konta na jednym komputerze się nie mieszają.
fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    format!(r"\\.\pipe\PowerDesk-{user}")
}

/// Przekazuje argumenty działającemu oknu; `false` – nikt nie słucha, trzeba otworzyć własne.
pub fn send_to_running(args: &[String]) -> bool {
    let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(pipe_name()) else { return false };
    // Działające okno może wtedy wyjść na wierzch (Windows blokuje to obcym procesom).
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
    f.write_all(args.join("\n").as_bytes()).is_ok()
}

/// Nasłuch w osobnym wątku; każde połączenie to lista argumentów (może być pusta).
pub fn serve(on_args: impl Fn(Vec<String>) + Send + 'static) {
    let name = HSTRING::from(pipe_name());
    std::thread::spawn(move || loop {
        let h = unsafe { CreateNamedPipeW(&name, PIPE_ACCESS_INBOUND, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, PIPE_UNLIMITED_INSTANCES, 0, 64 * 1024, 0, None) };
        if h.is_invalid() {
            return;
        }
        // Klient mógł połączyć się między utworzeniem a czekaniem – to też jest połączenie.
        let _ = unsafe { ConnectNamedPipe(h, None) };
        let mut f = unsafe { std::fs::File::from_raw_handle(h.0) };
        let mut buf = String::new();
        if f.read_to_string(&mut buf).is_ok() {
            on_args(buf.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect());
        }
    });
}

/// Przywraca zminimalizowane okno i wysuwa je na wierzch.
pub fn bring_to_front(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    let h = HWND(hwnd as *mut _);
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetForegroundWindow(h);
    }
}
