//! Aktualizacje: sprawdzanie w tle (minutę po starcie, potem co 6 godzin), dymek w zasobniku
//! i powiadomienie z przyciskiem „Zaktualizuj”. Pobranie i podmiana – core::update.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use powerdesk_core::update::{self, Release};

use crate::transfers::{self, Kind};
use crate::Shared;

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    /// Znaleziona wersja – „Zaktualizuj” jej używa.
    static FOUND: RefCell<Option<Release>> = const { RefCell::new(None) };
    /// O tej wersji już powiadomiliśmy – dymek nie wraca co 6 godzin.
    static TOLD: RefCell<String> = const { RefCell::new(String::new()) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

pub fn start() {
    if !update::has_key() {
        return;
    }
    slint::Timer::single_shot(Duration::from_secs(60), || check(false));
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(6 * 3600), || check(false)));
}

/// Sprawdza wydania. `manual` – z menu: zgłaszamy też „brak nowej wersji” i błędy.
pub fn check(manual: bool) {
    if BUSY.with(|b| b.replace(true)) {
        return;
    }
    std::thread::spawn(move || {
        let res = update::check();
        let _ = slint::invoke_from_event_loop(move || {
            BUSY.with(|b| b.set(false));
            crate::with_state(|s| found(s, res, manual));
        });
    });
}

fn found(s: &Shared, res: Result<Option<Release>, String>, manual: bool) {
    match res {
        Ok(Some(rel)) => {
            let v = rel.version.clone();
            FOUND.with(|f| *f.borrow_mut() = Some(rel));
            let new = TOLD.with(|t| t.replace(v.clone()) != v);
            if new || manual {
                crate::notices::add("PowerDesk", "Jest nowa wersja", &format!("Wersja {v} – „Zaktualizuj” w oknie PowerDesk albo w menu Start → Sprawdź aktualizacje."));
                transfers::toast(s, format!("Jest nowa wersja PowerDesk: {v} (masz {})", update::current()), Kind::Info, Some(("Zaktualizuj", std::rc::Rc::new(install))));
            }
        }
        Ok(None) if manual => transfers::toast(s, format!("Masz najnowszą wersję PowerDesk ({})", update::current()), Kind::Ok, None),
        Err(e) => {
            powerdesk_core::log::error(format!("aktualizacje: {e}"));
            if manual {
                transfers::toast(s, format!("Nie udało się sprawdzić aktualizacji: {e}"), Kind::Error, None);
            }
        }
        _ => {}
    }
}

/// Pobiera wersję, sprawdza sumę, podmienia program i go zamyka (skrypt uruchomi nowy).
pub fn install(s: &Shared) {
    let Some(rel) = FOUND.with(|f| f.borrow().clone()) else { return };
    if BUSY.with(|b| b.replace(true)) {
        return;
    }
    transfers::toast(s, format!("Pobieranie PowerDesk {}…", rel.version), Kind::Info, None);
    std::thread::spawn(move || {
        let res = update::download(&rel, |_, _| {}).and_then(|p| update::apply(&p));
        let _ = slint::invoke_from_event_loop(move || {
            BUSY.with(|b| b.set(false));
            crate::with_state(|s| match res {
                Ok(()) => crate::quit(s),
                Err(e) => {
                    powerdesk_core::log::error(format!("aktualizacja: {e}"));
                    transfers::toast(s, format!("Aktualizacja nie powiodła się: {e}"), Kind::Error, None);
                }
            });
        });
    });
}
