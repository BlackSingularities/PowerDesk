//! Instalacja i deinstalacja z punktu widzenia użytkownika: okna potwierdzenia, przeniesienie
//! ustawień z OpenFilePower, zamknięcie działającej kopii, przywrócenie Windows. Pliki i rejestr
//! obsługuje core::install.

use powerdesk_core::install::{self, message as ask, Answer, Ask, ALL_USERS_FLAG, INSTALL_FLAG, QUIET_FLAG, UNINSTALL_FLAG};

use crate::model::Prefs;

/// Argument przekazany działającej kopii: zakończ się (deinstalacja, aktualizacja instalatorem).
pub const QUIT_ARG: &str = "ofp:quit";

fn info(text: &str) {
    ask(text, Ask::Info);
}

fn version() -> &'static str {
    powerdesk_core::update::current()
}

/// Zamyka działającą kopię PowerDesk (jeśli jest) i czeka chwilę na jej koniec.
fn stop_running() {
    if powerdesk_core::instance::send_to_running(&[QUIT_ARG.to_string()]) {
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Ustawienia z OpenFilePower – raz (znacznik w ustawieniach PowerDesk).
pub fn migrate_settings() {
    let mut p = Prefs::load();
    crate::migrate::from_openfilepower(&mut p);
    p.save();
}

/// `--install [--all-users] [--quiet]`: kopia do folderu programu, ustawienia ze starej wersji,
/// propozycja odinstalowania OpenFilePower i start zainstalowanej kopii. Zwraca kod wyjścia.
fn install_flow(all_users: bool, quiet: bool) -> i32 {
    if all_users && !install::is_admin() {
        let mut args = format!("{INSTALL_FLAG} {ALL_USERS_FLAG}");
        if quiet {
            args.push_str(&format!(" {QUIET_FLAG}"));
        }
        return match install::relaunch_elevated(&args) {
            Ok(()) => 0,
            Err(e) => {
                if !quiet {
                    ask(&e, Ask::Warning);
                }
                1
            }
        };
    }
    stop_running();
    let exe = match install::install(all_users, version()) {
        Ok(p) => p,
        Err(e) => {
            if !quiet {
                ask(&format!("Instalacja nie powiodła się:\n{e}"), Ask::Warning);
            }
            return 1;
        }
    };
    migrate_settings();
    let ofp = install::has_openfilepower();
    if ofp && (quiet || ask("Znaleziono OpenFilePower – poprzednią wersję programu. Ustawienia zostały przeniesione.\n\nOdinstalować OpenFilePower?", Ask::YesNo) == Answer::Yes) {
        install::remove_openfilepower();
    }
    let _ = std::process::Command::new(&exe).spawn();
    if !quiet {
        info(&format!(
            "PowerDesk {} zainstalowano w:\n{}\n\nPo zalogowaniu PowerDesk zastępuje Eksplorator: pulpit, pasek zadań i menu Start.\nOdinstalujesz go w Ustawieniach Windows → Aplikacje albo w Ustawieniach PowerDesk → System.",
            version(),
            exe.parent().map(|d| d.display().to_string()).unwrap_or_default()
        ));
    }
    0
}

/// `--uninstall [--quiet]`: potwierdzenie, zamknięcie programu, przywrócenie pulpitu, paska
/// i powłoki Windows, usunięcie wpisów i plików (ustawienia – na życzenie).
fn uninstall_flow(quiet: bool) -> i32 {
    if !quiet && ask("Odinstalować PowerDesk?\n\nPulpit, pasek zadań i Eksplorator Windows wrócą na swoje miejsce.", Ask::YesNo) != Answer::Yes {
        return 0;
    }
    if install::installed().is_some_and(|(_, all)| all) && !install::is_admin() {
        // Potwierdzenie już było – kopia z uprawnieniami pyta tylko o ustawienia, więc bez --quiet.
        return install::relaunch_elevated(&format!("{UNINSTALL_FLAG} {QUIET_FLAG}")).map_or(1, |_| 0);
    }
    let remove_settings = !quiet && ask("Usunąć też ustawienia PowerDesk (wygląd, kafelki, widżety, przypięte foldery)?", Ask::YesNo) == Answer::Yes;
    stop_running();
    // Kolejność jak przy `--remove-integration`: Windows wraca, zanim znikną pliki.
    powerdesk_core::tray::hide();
    powerdesk_core::shellswap::restore();
    let _ = powerdesk_core::integration::remove();
    if powerdesk_core::shellmode::detect() {
        powerdesk_core::shellmode::fallback_to_explorer();
    }
    match install::uninstall(remove_settings) {
        Ok(()) => {
            if !quiet {
                info("PowerDesk został odinstalowany.");
            }
            0
        }
        Err(e) => {
            if !quiet {
                ask(&format!("Deinstalacja nie powiodła się:\n{e}"), Ask::Warning);
            }
            1
        }
    }
}

/// Polecenia instalatora z wiersza poleceń. Zwraca kod wyjścia, gdy to było jedno z nich.
pub fn handle_args(args: &[String]) -> Option<i32> {
    let has = |f: &str| args.iter().any(|a| a == f);
    let quiet = has(QUIET_FLAG);
    if has(INSTALL_FLAG) {
        return Some(install_flow(has(ALL_USERS_FLAG), quiet));
    }
    if has(UNINSTALL_FLAG) {
        return Some(uninstall_flow(quiet));
    }
    None
}

/// Pierwsze uruchomienie pobranego pliku (PowerDesk nie jest zainstalowany): propozycja instalacji.
/// Tak – dla bieżącego użytkownika, Nie – uruchom bez instalacji, Anuluj – zakończ.
/// Zwraca Some(kod) – program ma się zakończyć.
pub fn offer_install(args: &[String]) -> Option<i32> {
    // Z autostartu, jako powłoka i z argumentami (ścieżki) – bez pytania.
    if args.len() > 1 || install::installed().is_some() {
        return None;
    }
    // Uruchomiony z folderu programu bez wpisu (np. skopiowany ręcznie) – też bez pytania.
    let me = std::env::current_exe().ok()?;
    // Kopia z budowania (cargo, folder target) – też bez pytania.
    let built = me.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target"));
    if built || me.parent().is_some_and(|d| d == install::install_dir(false) || d == install::install_dir(true)) {
        return None;
    }
    match ask(
        "Zainstalować PowerDesk?\n\nTak – zainstaluj dla mnie (bez uprawnień administratora)\nNie – uruchom bez instalacji\n\nPo instalacji PowerDesk zastąpi Eksplorator: pulpit, pasek zadań i menu Start.",
        Ask::YesNoCancel,
    ) {
        Answer::Yes => Some(install_flow(false, false)),
        Answer::No => None,
        Answer::Other => Some(0),
    }
}

/// Z Ustawień: instalacja (dla mnie / dla wszystkich) – osobny proces z flagą, potem koniec tej kopii.
pub fn install_from_settings(all_users: bool) {
    let Ok(me) = std::env::current_exe() else { return };
    let mut c = std::process::Command::new(me);
    c.arg(INSTALL_FLAG);
    if all_users {
        c.arg(ALL_USERS_FLAG);
    }
    let _ = c.spawn();
}

pub fn uninstall_from_settings() {
    let exe = install::installed().map(|(e, _)| e).or_else(|| std::env::current_exe().ok());
    if let Some(exe) = exe {
        let _ = std::process::Command::new(exe).arg(UNINSTALL_FLAG).spawn();
    }
}
