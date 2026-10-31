//! PowerDesk zastępuje Eksplorator: Win+E, otwieranie folderów i dysków, „Otwórz w PowerDesk”
//! w menu, okna „Otwórz” i „Zapisz jako” w innych programach oraz start z Windows. Wpisy (tylko HKCU) odnawiamy przy każdym starcie – zawsze
//! wskazują bieżący plik .exe. Cofnięcie: `--remove-integration` (deinstalator).

use crate::{autostart, filedlg, filemgr, shellmode, winkey};

/// Argument, który zdejmuje integrację i kończy program.
pub const REMOVE_FLAG: &str = "--remove-integration";

/// Stan poszczególnych elementów: nazwa, opis i czy działa.
pub fn status() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        (crate::lang::tr("Foldery, dyski i ZIP"), crate::lang::tr("Dwuklik na Pulpicie, skróty, „Pokaż w folderze” i pobrane archiwa ZIP w innych programach"), filemgr::default_enabled()),
        ("Win+E", crate::lang::tr("Skrót klawiszowy otwiera nowe okno PowerDesk"), winkey::enabled()),
        (crate::lang::tr("Menu Eksploratora"), crate::lang::tr("„Otwórz w PowerDesk” dla folderów, dysków i tła folderu"), filemgr::menu_enabled()),
        (crate::lang::tr("Okna „Otwórz” i „Zapisz jako”"), file_dialogs_detail(), filedlg::enabled()),
    ]
}

/// Opis stanu okien plików w innych programach.
fn file_dialogs_detail() -> &'static str {
    if !filedlg::bundled() {
        crate::lang::tr("Ta kopia PowerDesk nie zawiera biblioteki okien plików (budowa bez niej) – programy mają okna Windows")
    } else if filedlg::enabled_32bit() {
        crate::lang::tr("W programach 64- i 32-bitowych; programy uruchomione jako administrator i przy każdym problemie – okno Windows")
    } else {
        crate::lang::tr("W programach 64-bitowych; 32-bitowe, uruchomione jako administrator i przy każdym problemie – okno Windows")
    }
}

/// Ustawia brakujące wpisy (te, które nie wskazują tego pliku .exe). Zwraca pierwszy błąd.
/// Zakłada wpisy tylko zainstalowana kopia. Plik z Pobranych albo z folderu budowania niczego
/// w Windows nie zmienia – foldery dalej otwiera Eksplorator (gdyby plik zniknął albo był
/// podmieniany, Windows zostałby z martwymi skojarzeniami). Zwraca Ok bez zmian, gdy to nie instalacja.
/// `autostart` – start z Windows (pytanie przy instalacji, przełącznik w Ustawieniach → System).
pub fn ensure(autostart: bool, file_dialogs: bool) -> Result<(), String> {
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
    // Okna plików: wypakowanie nowej wersji biblioteki i wpisy – co start, tanie, gdy nic się nie zmieniło.
    // Wyłączone w Ustawieniach – zdejmujemy wpisy, programy mają okna Windows.
    if file_dialogs {
        keep(filedlg::ensure());
    } else if filedlg::enabled() {
        keep(filedlg::remove());
    }
    if autostart::enabled() != autostart {
        keep(autostart::set(autostart));
    }
    // Starsze wersje zastępowały Eksplorator po zalogowaniu – teraz działa on w tle pod PowerDesk.
    if shellmode::registered() {
        keep(shellmode::unregister());
    }
    err
}

/// Przywraca Eksplorator: usuwa wszystkie nasze wpisy.
pub fn remove() -> Result<(), String> {
    remove_legacy();
    // Najpierw okna plików – ich wpisy muszą zniknąć, zanim deinstalator usunie pliki programu.
    filedlg::remove()?;
    filemgr::set_default(false)?;
    winkey::set(false)?;
    filemgr::set_menu(false)?;
    shellmode::unregister()?;
    autostart::set(false)
}

/// Pozostałości po OpenFilePower: „Otwórz w OpenFilePower” w menu folderów i dysków, które po jego
/// odinstalowaniu wskazują nieistniejący plik.
pub fn remove_legacy() {
    for class in [crate::lang::tr("Directory"), crate::lang::tr("Drive"), r"Directory\Background"] {
        let key = format!(r"HKCU\Software\Classes\{class}\shell\OpenFilePower");
        if winkey::reg(&["query", &key]).is_ok() {
            let _ = winkey::reg(&["delete", &key, "/f"]);
        }
    }
}
