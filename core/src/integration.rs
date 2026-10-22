//! PowerDesk zastępuje Eksplorator: Win+E, otwieranie folderów i dysków, „Otwórz w PowerDesk”
//! w menu oraz start z Windows. Wpisy (tylko HKCU) odnawiamy przy każdym starcie – zawsze
//! wskazują bieżący plik .exe. Cofnięcie: `--remove-integration` (deinstalator).

use crate::{autostart, filemgr, winkey};

/// Argument, który zdejmuje integrację i kończy program.
pub const REMOVE_FLAG: &str = "--remove-integration";

/// Stan poszczególnych elementów: nazwa, opis i czy działa.
pub fn status() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("Foldery i dyski", "Dwuklik na Pulpicie, skróty i „Pokaż w folderze” w innych programach", filemgr::default_enabled()),
        ("Win+E", "Skrót klawiszowy otwiera nowe okno PowerDesk", winkey::enabled()),
        ("Menu Eksploratora", "„Otwórz w PowerDesk” dla folderów, dysków i tła folderu", filemgr::menu_enabled()),
        ("Start z Windows", "Po zalogowaniu PowerDesk działa w tle, w zasobniku", autostart::enabled()),
    ]
}

/// Ustawia brakujące wpisy (te, które nie wskazują tego pliku .exe). Zwraca pierwszy błąd.
pub fn ensure() -> Result<(), String> {
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
    err
}

/// Przywraca Eksplorator: usuwa wszystkie nasze wpisy.
pub fn remove() -> Result<(), String> {
    filemgr::set_default(false)?;
    winkey::set(false)?;
    filemgr::set_menu(false)?;
    autostart::set(false)
}
