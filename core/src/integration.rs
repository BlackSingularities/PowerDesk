//! PowerDesk zastępuje Eksplorator: Win+E, otwieranie folderów i dysków, „Otwórz w PowerDesk”
//! w menu oraz start z Windows. Wpisy (tylko HKCU) odnawiamy przy każdym starcie – zawsze
//! wskazują bieżący plik .exe. Cofnięcie: `--remove-integration` (deinstalator).

use crate::{autostart, filemgr, shellmode, winkey};

/// Argument, który zdejmuje integrację i kończy program.
pub const REMOVE_FLAG: &str = "--remove-integration";

/// Stan poszczególnych elementów: nazwa, opis i czy działa.
pub fn status() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("Foldery i dyski", "Dwuklik na Pulpicie, skróty i „Pokaż w folderze” w innych programach", filemgr::default_enabled()),
        ("Win+E", "Skrót klawiszowy otwiera nowe okno PowerDesk", winkey::enabled()),
        ("Menu Eksploratora", "„Otwórz w PowerDesk” dla folderów, dysków i tła folderu", filemgr::menu_enabled()),
        ("Start z Windows", "Po zalogowaniu PowerDesk działa w tle, w zasobniku", autostart::enabled()),
        ("Powłoka", "Po zalogowaniu PowerDesk zamiast Eksploratora (pulpit, pasek, zasobnik)", shellmode::registered()),
    ]
}

/// Ustawia brakujące wpisy (te, które nie wskazują tego pliku .exe). Zwraca pierwszy błąd.
/// Zakłada wpisy tylko zainstalowana kopia. Plik z Pobranych albo z folderu budowania niczego
/// w Windows nie zmienia – foldery dalej otwiera Eksplorator (gdyby plik zniknął albo był
/// podmieniany, Windows zostałby z martwymi skojarzeniami). Zwraca Ok bez zmian, gdy to nie instalacja.
pub fn ensure() -> Result<(), String> {
    if !crate::install::running_installed() {
        return Ok(());
    }
    remove_legacy();
    let mut err = Ok(());
    let mut keep = |r: Result<(), String>| {
        if err.is_ok() {
            if let Err(e) = r {
                err = Err(e);
            }
        }
    };
    if !filemgr::default_enabled() {
        keep(filemgr::set_default(true));
    }
    if !winkey::enabled() {
        keep(winkey::set(true));
    }
    if !filemgr::menu_enabled() {
        keep(filemgr::set_menu(true));
    }
    if !autostart::enabled() {
        keep(autostart::set(true));
    }
    if !shellmode::registered() {
        keep(shellmode::register());
    }
    err
}

/// Przywraca Eksplorator: usuwa wszystkie nasze wpisy.
pub fn remove() -> Result<(), String> {
    remove_legacy();
    filemgr::set_default(false)?;
    winkey::set(false)?;
    filemgr::set_menu(false)?;
    shellmode::unregister()?;
    autostart::set(false)
}

/// Pozostałości po OpenFilePower: „Otwórz w OpenFilePower” w menu folderów i dysków, które po jego
/// odinstalowaniu wskazują nieistniejący plik.
pub fn remove_legacy() {
    for class in ["Directory", "Drive", r"Directory\Background"] {
        let key = format!(r"HKCU\Software\Classes\{class}\shell\OpenFilePower");
        if winkey::reg(&["query", &key]).is_ok() {
            let _ = winkey::reg(&["delete", &key, "/f"]);
        }
    }
}
