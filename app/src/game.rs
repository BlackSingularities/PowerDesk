//! Tryb gry: okno programu na cały monitor (gra, film) – plan zasilania „Najwyższa wydajność”,
//! po wyjściu – poprzedni plan. Sprawdzane co 3 s, gdy włączone w Ustawieniach → Nakładka.

use std::cell::Cell;
use std::time::Duration;

use powerdesk_core::{gamemode, taskwin};

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    /// Plan sprzed gry (do przywrócenia); None – nic nie zmienialiśmy.
    static PREV: Cell<Option<u128>> = const { Cell::new(None) };
}

fn enabled() -> bool {
    let mut on = false;
    crate::with_state(|s| on = s.borrow().prefs.game_power);
    on
}

/// Start albo zatrzymanie sprawdzania (zmiana ustawienia).
pub fn init() {
    if enabled() {
        TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(3), tick));
    } else {
        TIMER.with(|t| t.stop());
        restore();
    }
}

fn tick() {
    let playing = taskwin::foreground_app().is_some_and(|(pid, _, (x, y, w, h))| pid != std::process::id() && taskwin::covers(taskwin::foreground(), x, y, w, h));
    let boosted = PREV.with(|p| p.get()).is_some();
    if playing && !boosted {
        if let Some(prev) = gamemode::boost() {
            PREV.with(|p| p.set(Some(prev)));
            powerdesk_core::log::info("tryb gry: plan Najwyższa wydajność");
        }
    } else if !playing && boosted {
        restore();
    }
}

/// Poprzedni plan zasilania (koniec gry, wyłączenie trybu albo zamknięcie PowerDesk).
pub fn restore() {
    if let Some(g) = PREV.with(|p| p.take()) {
        gamemode::restore_scheme(g);
        powerdesk_core::log::info("tryb gry: poprzedni plan zasilania");
    }
}
