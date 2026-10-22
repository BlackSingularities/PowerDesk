//! Start razem z Windows: wpis „PowerDesk” w HKCU\…\Run (tylko bieżący użytkownik).
//! Program startuje wtedy z `--tray` – sama ikona w zasobniku, bez okna.

use crate::winkey::{exe, reg};

const RUN: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const NAME: &str = "PowerDesk";
/// Argument, z którym uruchamia Windows: okno zostaje schowane.
pub const FLAG: &str = "--tray";

/// Wpis istnieje i wskazuje ten plik .exe.
pub fn enabled() -> bool {
    let Ok(exe) = exe() else { return false };
    reg(&["query", RUN, "/v", NAME]).is_ok_and(|o| o.to_lowercase().contains(&exe.to_lowercase()))
}

pub fn set(on: bool) -> Result<(), String> {
    if on {
        reg(&["add", RUN, "/v", NAME, "/d", &format!("\"{}\" {FLAG}", exe()?), "/f"])?;
    } else if reg(&["query", RUN, "/v", NAME]).is_ok() {
        reg(&["delete", RUN, "/v", NAME, "/f"])?;
    }
    Ok(())
}
