//! Instalacja i deinstalacja z punktu widzenia użytkownika: okna potwierdzenia, przeniesienie
//! ustawień z OpenFilePower, zamknięcie działającej kopii, przywrócenie Windows. Pliki i rejestr
//! obsługuje core::install.

use powerdesk_core::install::{self, message as ask, Answer, Ask, ALL_USERS_FLAG, INSTALL_FLAG, NO_LAUNCH_FLAG, QUIET_FLAG, UNINSTALL_FLAG, UPDATE_FLAG};

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

/// `--install [--all-users] [--quiet] [--update] [--no-launch]`: kopia do folderu programu,
/// ustawienia ze starej wersji, propozycja odinstalowania OpenFilePower i start zainstalowanej
/// kopii. Dla wszystkich użytkowników bez uprawnień – ta sama instalacja w kopii z UAC, a ten
/// proces czeka na nią i uruchamia program już bez uprawnień administratora. Zwraca kod wyjścia.
fn install_flow(all_users: bool, quiet: bool, update: bool, launch: bool) -> i32 {
    if all_users && !install::is_admin() {
        let Ok(me) = std::env::current_exe() else { return 1 };
        let mut args = format!("{INSTALL_FLAG} {ALL_USERS_FLAG} {NO_LAUNCH_FLAG}");
        if quiet {
            args.push_str(&format!(" {QUIET_FLAG}"));
        }
        if update {
            args.push_str(&format!(" {UPDATE_FLAG}"));
        }
        return match install::run_wait(&me, &args, true) {
            Ok(0) => {
                if launch {
                    finish(quiet);
                }
                0
            }
            Ok(code) => code as i32,
            Err(e) => {
                if !quiet {
                    ask(&e, Ask::Warning);
                }
                1
            }
        };
    }
    // Aktualizacja: działająca kopia czeka na nas i zamknie się sama (stary plik zostaje
    // przemianowany na .old, więc można go podmienić w trakcie działania).
    if !update {
        stop_running();
    }
    let exe = match install::install(all_users, version()) {
        Ok(p) => p,
        Err(e) => {
            if !quiet {
                ask(&format!("Instalacja nie powiodła się:\n{e}"), Ask::Warning);
            }
            return 1;
        }
    };
    if update {
        return 0;
    }
    migrate_settings();
    let ofp = install::has_openfilepower();
    if ofp && (quiet || ask("Znaleziono OpenFilePower – poprzednią wersję programu. Ustawienia zostały przeniesione.\n\nOdinstalować OpenFilePower?", Ask::YesNo) == Answer::Yes) {
        install::remove_openfilepower();
    }
    if launch {
        if !quiet {
            ask_autostart();
        }
        let _ = std::process::Command::new(&exe).spawn();
        if !quiet {
            done_info(&exe);
        }
    }
    0
}

/// Pytanie przy instalacji: start razem z Windows. Zapis w ustawieniach – wpis w rejestrze
/// zakłada (albo zdejmuje) zainstalowany program przy starcie, już jako zwykły użytkownik.
fn ask_autostart() {
    let on = ask(
        "Uruchamiać PowerDesk razem z Windows?

Tak – po zalogowaniu PowerDesk od razu zastępuje pulpit, pasek zadań i menu Start
Nie – uruchomisz go sam z menu Start

Zmienisz to w Ustawieniach PowerDesk → System.",
        Ask::YesNo,
    ) == Answer::Yes;
    let mut p = Prefs::load();
    p.autostart = on;
    p.save();
}

fn done_info(exe: &std::path::Path) {
    info(&format!(
        "PowerDesk {} zainstalowano w:\n{}\n\nPo zalogowaniu PowerDesk zastępuje Eksplorator: pulpit, pasek zadań i menu Start. Aktualizacje instalują się w tym samym miejscu (Windows zapyta o uprawnienia administratora).\nOdinstalujesz go w Ustawieniach Windows → Aplikacje albo w Ustawieniach PowerDesk → System.",
        version(),
        exe.parent().map(|d| d.display().to_string()).unwrap_or_default()
    ));
}

/// Po instalacji w kopii z uprawnieniami: start zainstalowanego programu jako zwykły użytkownik.
fn finish(quiet: bool) {
    if let Some((exe, _)) = install::installed() {
        if !quiet {
            ask_autostart();
        }
        let _ = std::process::Command::new(&exe).spawn();
        if !quiet {
            done_info(&exe);
        }
    }
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
        let update = has(UPDATE_FLAG);
        return Some(install_flow(has(ALL_USERS_FLAG), quiet || update, update, !has(NO_LAUNCH_FLAG) && !update));
    }
    if has(UNINSTALL_FLAG) {
        return Some(uninstall_flow(quiet));
    }
    None
}

/// Uruchomiony pobrany plik (instalator `PowerDesk-Setup-*.exe`): propozycja instalacji, a gdy
/// PowerDesk już jest – aktualizacji w tym samym miejscu. Domyślnie do Program Files (Windows
/// pyta o uprawnienia administratora). Zwraca Some(kod) – program ma się zakończyć.
pub fn offer_install(args: &[String]) -> Option<i32> {
    // Z autostartu, jako powłoka i z argumentami (ścieżki) – bez pytania.
    if args.len() > 1 {
        return None;
    }
    let me = std::env::current_exe().ok()?;
    // Kopia z budowania (cargo, folder target) i kopia w folderze programu – bez pytania.
    let built = me.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target"));
    if built || install::running_installed() || me.parent().is_some_and(|d| d == install::install_dir(false) || d == install::install_dir(true)) {
        return None;
    }
    if let Some((exe, all)) = install::installed() {
        let have = install::installed_version().unwrap_or_else(|| "?".into());
        let text = format!(
            "PowerDesk {have} jest już zainstalowany w:\n{}\n\nZainstalować wersję {} w tym samym miejscu?{}",
            exe.parent().map(|d| d.display().to_string()).unwrap_or_default(),
            version(),
            if all { "\n\nWindows zapyta o uprawnienia administratora." } else { "" }
        );
        return match ask(&text, Ask::YesNo) {
            Answer::Yes => Some(install_flow(all, false, false, true)),
            _ => Some(0),
        };
    }
    // Bez trybu przenośnego: pobrany plik to instalator – instaluje albo kończy.
    match ask(
        "Zainstalować PowerDesk?\n\nPowerDesk zainstaluje się w Program Files (Windows zapyta o uprawnienia administratora) i zastąpi Eksplorator: pulpit, pasek zadań i menu Start. Aktualizacje instalują się w tym samym miejscu.",
        Ask::YesNo,
    ) {
        Answer::Yes => Some(install_flow(true, false, false, true)),
        _ => Some(0),
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
