//! Integracja z Eksploratorem (tylko HKCU, bez uprawnień administratora):
//! - PowerDesk jako domyślny menedżer folderów – domyślne polecenie folderów i dysków
//!   (Pulpit, skróty, „Otwórz folder” w innych programach);
//! - „Otwórz w PowerDesk” w menu kontekstowym folderów, dysków i tła folderu.
//! Wyłączenie usuwa tylko nasze klucze.

use crate::winkey::{exe, reg};

const CLASSES: &str = r"HKCU\Software\Classes";
/// Polecenie domyślne – nazwa klucza w `…\shell`.
const DEFAULT_VERB: &str = "PowerDesk";
/// Pozycja menu – osobny klucz, żeby menu działało także bez domyślnego menedżera.
const MENU_VERB: &str = "PowerDesk.open";

/// Klasy powłoki i parametr ścieżki w ich poleceniu (tło folderu podaje ścieżkę przez %V).
const MENU_TARGETS: [(&str, &str); 3] = [("Directory", "%1"), ("Drive", "%1"), (r"Directory\Background", "%V")];
/// `CompressedFolder` – archiwa ZIP (otwierane z przeglądarki po pobraniu, dwuklikiem itd.)
/// przegląda PowerDesk jak folder, zamiast ukrytego w tle Eksploratora.
const DEFAULT_TARGETS: [&str; 3] = ["Directory", "Drive", "CompressedFolder"];

fn command(arg: &str) -> Result<String, String> {
    Ok(format!("\"{}\" \"{arg}\"", exe()?))
}

fn points_here(key: &str) -> bool {
    let Ok(exe) = exe() else { return false };
    reg(&["query", key, "/ve"]).is_ok_and(|o| o.to_lowercase().contains(&exe.to_lowercase()))
}

/// Foldery, dyski i archiwa ZIP otwierają się w PowerDesk.
pub fn default_enabled() -> bool {
    DEFAULT_TARGETS.iter().all(|c| {
        let shell = format!(r"{CLASSES}\{c}\shell");
        reg(&["query", &shell, "/ve"]).is_ok_and(|o| o.contains(DEFAULT_VERB) && !o.contains(MENU_VERB)) && points_here(&format!(r"{shell}\{DEFAULT_VERB}\command"))
    })
}

pub fn set_default(on: bool) -> Result<(), String> {
    for c in DEFAULT_TARGETS {
        let shell = format!(r"{CLASSES}\{c}\shell");
        let verb = format!(r"{shell}\{DEFAULT_VERB}");
        if on {
            reg(&["add", &verb, "/ve", "/d", crate::lang::tr("Otwórz w PowerDesk"), "/f"])?;
            reg(&["add", &format!(r"{verb}\command"), "/ve", "/d", &command("%1")?, "/f"])?;
            // Domyślne polecenie klasy – to ono rusza po dwukliku i z innych programów.
            reg(&["add", &shell, "/ve", "/d", DEFAULT_VERB, "/f"])?;
        } else {
            if reg(&["query", &shell, "/ve"]).is_ok_and(|o| o.contains(DEFAULT_VERB) && !o.contains(MENU_VERB)) {
                reg(&["delete", &shell, "/ve", "/f"])?;
            }
            if reg(&["query", &verb]).is_ok() {
                reg(&["delete", &verb, "/f"])?;
            }
        }
    }
    Ok(())
}

/// „Otwórz w PowerDesk” w menu kontekstowym Eksploratora.
pub fn menu_enabled() -> bool {
    MENU_TARGETS.iter().all(|(c, _)| points_here(&format!(r"{CLASSES}\{c}\shell\{MENU_VERB}\command")))
}

pub fn set_menu(on: bool) -> Result<(), String> {
    for (c, arg) in MENU_TARGETS {
        let verb = format!(r"{CLASSES}\{c}\shell\{MENU_VERB}");
        if on {
            reg(&["add", &verb, "/v", "MUIVerb", "/d", crate::lang::tr("Otwórz w PowerDesk"), "/f"])?;
            reg(&["add", &verb, "/v", crate::lang::tr("Icon"), "/d", &format!("\"{}\",0", exe()?), "/f"])?;
            reg(&["add", &format!(r"{verb}\command"), "/ve", "/d", &command(arg)?, "/f"])?;
        } else if reg(&["query", &verb]).is_ok() {
            reg(&["delete", &verb, "/f"])?;
        }
    }
    Ok(())
}
