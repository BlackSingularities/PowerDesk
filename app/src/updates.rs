//! Aktualizacje: sprawdzanie w tle (minutę po starcie, potem co 6 godzin – przełącznik
//! w Ustawieniach → System), dymek w zasobniku i powiadomienie z przyciskiem „Zaktualizuj”,
//! albo pobranie i instalacja bez pytania (drugi przełącznik). Stan widać w Ustawieniach.
//! Pobranie i podmiana – core::update.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use powerdesk_core::update::{self, Release};

use crate::transfers::{self, Kind};
use crate::Shared;

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    static FIRST: slint::Timer = slint::Timer::default();
    /// Znaleziona wersja – „Zaktualizuj” jej używa.
    static FOUND: RefCell<Option<Release>> = const { RefCell::new(None) };
    /// O tej wersji już powiadomiliśmy – dymek nie wraca co 6 godzin.
    static TOLD: RefCell<String> = const { RefCell::new(String::new()) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// Opis stanu dla Ustawień.
    static STATUS: RefCell<String> = RefCell::new(format!("Wersja {} · jeszcze nie sprawdzano", update::current()));
}

fn prefs() -> (bool, bool) {
    let mut v = (true, false);
    crate::with_state(|s| {
        let p = &s.borrow().prefs;
        v = (p.update_check, p.update_auto);
    });
    v
}

/// Włącza albo wyłącza sprawdzanie w tle – zgodnie z ustawieniem.
pub fn start() {
    let (on, _) = prefs();
    if !on {
        FIRST.with(|t| t.stop());
        TIMER.with(|t| t.stop());
        return;
    }
    FIRST.with(|t| t.start(slint::TimerMode::SingleShot, Duration::from_secs(60), || check(false)));
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(6 * 3600), || check(false)));
}

fn set_status(text: String) {
    STATUS.with(|st| *st.borrow_mut() = text);
    show_status();
}

/// Stan dla Ustawień: opis i czy jest wersja do zainstalowania.
pub fn status() -> (String, bool) {
    (STATUS.with(|st| st.borrow().clone()), FOUND.with(|f| f.borrow().is_some()))
}

/// Przepisuje stan do Ustawień.
fn show_status() {
    let (text, ready) = status();
    crate::with_state(|s| {
        crate::settings::with_ui(s, |g| {
            g.set_update_status(text.into());
            g.set_update_ready(ready);
        })
    });
}

/// Sprawdza wydania. `manual` – z menu albo Ustawień: zgłaszamy też „brak nowej wersji” i błędy.
pub fn check(manual: bool) {
    if BUSY.with(|b| b.replace(true)) {
        return;
    }
    set_status("Sprawdzanie aktualizacji…".into());
    std::thread::spawn(move || {
        let res = update::check();
        let _ = slint::invoke_from_event_loop(move || {
            BUSY.with(|b| b.set(false));
            crate::with_state(|s| found(s, res, manual));
        });
    });
}

fn found(s: &Shared, res: Result<Option<Release>, String>, manual: bool) {
    let when = chrono::Local::now().format("%H:%M");
    match res {
        Ok(Some(rel)) => {
            let v = rel.version.clone();
            FOUND.with(|f| *f.borrow_mut() = Some(rel));
            set_status(format!("Dostępna wersja {v} (masz {}) · sprawdzono o {when}", update::current()));
            if prefs().1 {
                install(s);
                return;
            }
            let new = TOLD.with(|t| t.replace(v.clone()) != v);
            if new || manual {
                crate::notices::add("PowerDesk", "Jest nowa wersja", &format!("Wersja {v} – „Zaktualizuj” w oknie PowerDesk albo Ustawienia → System → Aktualizacje."));
                transfers::toast(s, format!("Jest nowa wersja PowerDesk: {v} (masz {})", update::current()), Kind::Info, Some(("Zaktualizuj", std::rc::Rc::new(install))));
            }
        }
        Ok(None) => {
            set_status(format!("Masz najnowszą wersję ({}) · sprawdzono o {when}", update::current()));
            if manual {
                transfers::toast(s, format!("Masz najnowszą wersję PowerDesk ({})", update::current()), Kind::Ok, None);
            }
        }
        Err(e) => {
            powerdesk_core::log::error(format!("aktualizacje: {e}"));
            set_status(format!("Nie udało się sprawdzić ({when}): {e}"));
            if manual {
                transfers::toast(s, format!("Nie udało się sprawdzić aktualizacji: {e}"), Kind::Error, None);
            }
        }
    }
}

/// Pobiera wersję, sprawdza sumę, podmienia program i go zamyka (skrypt uruchomi nowy).
pub fn install(s: &Shared) {
    let Some(rel) = FOUND.with(|f| f.borrow().clone()) else { return };
    if BUSY.with(|b| b.replace(true)) {
        return;
    }
    transfers::toast(s, format!("Pobieranie PowerDesk {}…", rel.version), Kind::Info, None);
    set_status(format!("Pobieranie wersji {}…", rel.version));
    let v = rel.version.clone();
    std::thread::spawn(move || {
        // Postęp do Ustawień – co pełny procent, żeby nie zalać pętli zdarzeń.
        let last = Cell::new(u64::MAX);
        let progress = |done: u64, total: u64| {
            let pct = if total > 0 { done * 100 / total } else { 0 };
            if last.replace(pct) != pct {
                let v = v.clone();
                let _ = slint::invoke_from_event_loop(move || set_status(format!("Pobieranie wersji {v}… {pct}%")));
            }
        };
        // Zainstalowany – przez instalator w to samo miejsce (UAC przy Program Files);
        // wersja bez instalacji – zwykły instalator.
        let res = update::download(&rel, progress).and_then(|p| update::apply_installer(&p));
        let _ = slint::invoke_from_event_loop(move || {
            BUSY.with(|b| b.set(false));
            crate::with_state(|s| match res {
                Ok(()) => crate::quit(s),
                Err(e) => {
                    powerdesk_core::log::error(format!("aktualizacja: {e}"));
                    set_status(format!("Aktualizacja nie powiodła się: {e}"));
                    transfers::toast(s, format!("Aktualizacja nie powiodła się: {e}"), Kind::Error, None);
                }
            });
        });
    });
}
