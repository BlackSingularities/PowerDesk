//! Widżety-narzędzia: stoper (z okrążeniami), minutnik (powiadomienie po końcu), kalkulator
//! (działania z core::calc) i szybkie akcje systemu. Stan wspólny dla wszystkich takich widżetów;
//! gdy stoper albo minutnik chodzi, czas odświeża się co 100 ms (`FAST`).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use powerdesk_core::startmenu::{self, Power};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::WidgetData;

struct Tools {
    /// Stoper: zmierzony czas przed bieżącym startem, chwila startu, okrążenia (najnowsze pierwsze).
    sw_acc: Duration,
    sw_start: Option<Instant>,
    laps: Vec<Duration>,
    /// Minutnik: ustawiony czas, pozostały przy pauzie, koniec przy chodzeniu, czy minął.
    tm_total: Duration,
    tm_left: Duration,
    tm_end: Option<Instant>,
    tm_done: bool,
    /// Kalkulator: wpisane działanie (znaki jak na klawiszach).
    expr: String,
}

impl Default for Tools {
    fn default() -> Self {
        let five = Duration::from_secs(300);
        Tools { sw_acc: Duration::ZERO, sw_start: None, laps: Vec::new(), tm_total: five, tm_left: five, tm_end: None, tm_done: false, expr: String::new() }
    }
}

thread_local! {
    static T: RefCell<Tools> = RefCell::new(Tools::default());
    static FAST: slint::Timer = slint::Timer::default();
}

fn sw_elapsed(t: &Tools) -> Duration {
    t.sw_acc + t.sw_start.map_or(Duration::ZERO, |s| s.elapsed())
}

fn tm_left(t: &Tools) -> Duration {
    match t.tm_end {
        Some(e) => e.saturating_duration_since(Instant::now()),
        None => t.tm_left,
    }
}

/// „MM:SS” albo „H:MM:SS”.
fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// Ładniejsze znaki z klawiszy → działanie dla core::calc.
fn plain(expr: &str) -> String {
    expr.replace('×', "*").replace('÷', "/").replace('−', "-")
}

/// Wynik na żywo (pusto, gdy działanie niepełne albo to sama liczba).
fn result(expr: &str) -> String {
    powerdesk_core::calc::evaluate(&plain(expr)).map(|(_, n)| n.replace('.', ",")).unwrap_or_default()
}

/// Działanie do wyświetlenia – długie skraca się od lewej (koniec zostaje widoczny).
fn shown(expr: &str) -> String {
    let n = expr.chars().count();
    if n > 26 {
        format!("…{}", expr.chars().skip(n - 25).collect::<String>())
    } else {
        expr.to_string()
    }
}

/// Koniec minutnika: powiadomienie i stan „czas minął”.
fn check_timer() {
    let done = T.with(|t| {
        let mut t = t.borrow_mut();
        if t.tm_end.is_some() && tm_left(&t).is_zero() {
            t.tm_end = None;
            t.tm_left = Duration::ZERO;
            t.tm_done = true;
            Some(clock(t.tm_total))
        } else {
            None
        }
    });
    if let Some(total) = done {
        crate::notices::add(crate::tr("Minutnik"), crate::tr("Minutnik"), &powerdesk_core::trf!("Minął czas: {total}"));
    }
}

fn running() -> bool {
    T.with(|t| {
        let t = t.borrow();
        t.sw_start.is_some() || t.tm_end.is_some()
    })
}

/// Szybkie odświeżanie, gdy coś chodzi – tylko dane narzędzi, bez reszty widżetów.
fn fast() {
    let on = running();
    FAST.with(|f| {
        if on && !f.running() {
            f.start(slint::TimerMode::Repeated, Duration::from_millis(100), || {
                check_timer();
                crate::desktop::each_window(|w| apply(&w.global::<WidgetData>()));
                crate::start::each_window(|w| apply(&w.global::<WidgetData>()));
                if !running() {
                    FAST.with(|f| f.stop());
                }
            });
        }
    });
}

fn push() {
    fast();
    crate::desktop::each_window(|w| apply(&w.global::<WidgetData>()));
    crate::start::each_window(|w| apply(&w.global::<WidgetData>()));
}

/// Stoper: 0 – start/stop, 1 – okrążenie (chodzi) albo zerowanie (stoi).
pub fn stopwatch(k: i32) {
    T.with(|t| {
        let mut t = t.borrow_mut();
        match (k, t.sw_start) {
            (0, Some(s)) => {
                t.sw_acc += s.elapsed();
                t.sw_start = None;
            }
            (0, None) => t.sw_start = Some(Instant::now()),
            (_, Some(_)) => {
                let e = sw_elapsed(&t);
                t.laps.insert(0, e);
                t.laps.truncate(50);
            }
            (_, None) => {
                t.sw_acc = Duration::ZERO;
                t.laps.clear();
            }
        }
    });
    push();
}

/// Minutnik: 0 – start/pauza, 1 – zerowanie.
pub fn timer(k: i32) {
    T.with(|t| {
        let mut t = t.borrow_mut();
        if k == 0 {
            if t.tm_end.is_some() {
                t.tm_left = tm_left(&t);
                t.tm_end = None;
            } else {
                if t.tm_done || t.tm_left.is_zero() {
                    t.tm_left = t.tm_total;
                    t.tm_done = false;
                }
                t.tm_end = Some(Instant::now() + t.tm_left);
            }
        } else {
            t.tm_end = None;
            t.tm_left = t.tm_total;
            t.tm_done = false;
        }
    });
    push();
}

pub fn timer_set(minutes: i32) {
    T.with(|t| {
        let mut t = t.borrow_mut();
        t.tm_total = Duration::from_secs(minutes.clamp(1, 600) as u64 * 60);
        t.tm_left = t.tm_total;
        t.tm_end = None;
        t.tm_done = false;
    });
    push();
}

pub fn calc_key(k: &str) {
    T.with(|t| {
        let mut t = t.borrow_mut();
        match k {
            "C" => t.expr.clear(),
            "⌫" => {
                t.expr.pop();
            }
            "=" => {
                let r = result(&t.expr);
                if !r.is_empty() {
                    t.expr = r;
                }
            }
            _ if t.expr.chars().count() < 120 => t.expr.push_str(k),
            _ => {}
        }
    });
    push();
}

/// Szybka akcja z widżetu (identyfikatory jak w ActionsBody).
pub fn action(id: &str) {
    match id {
        "lock" => startmenu::power(Power::Lock),
        "sleep" => startmenu::power(Power::Sleep),
        "settings" => powerdesk_core::sysbar::open_settings(""),
        "taskmgr" => {
            let _ = std::process::Command::new("taskmgr.exe").spawn();
        }
        "snip" => {
            let _ = open::that_detached("ms-screenclip:");
        }
        "terminal" => {
            if std::process::Command::new("wt.exe").spawn().is_err() {
                let _ = std::process::Command::new("cmd.exe").spawn();
            }
        }
        "trash" => {
            let _ = open::that_detached("shell:RecycleBinFolder");
        }
        "downloads" => {
            if let Some(d) = dirs::download_dir() {
                crate::with_state(|s| crate::open_args(s, vec![d.to_string_lossy().to_string()]));
            }
        }
        _ => {}
    }
}

/// Co sekundę – koniec minutnika także bez szybkiego odświeżania.
pub fn tick(used: &std::collections::HashSet<String>) {
    let _ = used;
    check_timer();
}

pub fn apply(g: &WidgetData) {
    T.with(|t| {
        let t = t.borrow();
        let e = sw_elapsed(&t);
        g.set_sw_text(clock(e).into());
        g.set_sw_cs(format!(",{:02}", e.subsec_millis() / 10).into());
        g.set_sw_run(t.sw_start.is_some());
        g.set_sw_frac((e.as_millis() % 60_000) as f32 / 60_000.0);
        let n = t.laps.len();
        let laps: Vec<SharedString> = t
            .laps
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let prev = t.laps.get(i + 1).copied().unwrap_or(Duration::ZERO);
                powerdesk_core::trf!("Okrążenie {} · {} (+{})", n - i, clock(*l), clock(l.saturating_sub(prev))).into()
            })
            .collect();
        g.set_sw_laps(ModelRc::new(VecModel::from(laps)));
        let left = tm_left(&t);
        // W górę do pełnej sekundy – „00:00” dopiero na końcu.
        let up = if left.subsec_millis() > 0 && !left.is_zero() { left + Duration::from_secs(1) - Duration::from_millis(left.subsec_millis() as u64) } else { left };
        g.set_tm_text(clock(up).into());
        g.set_tm_total(clock(t.tm_total).into());
        g.set_tm_run(t.tm_end.is_some());
        g.set_tm_done(t.tm_done);
        g.set_tm_frac(if t.tm_total.is_zero() { 0.0 } else { left.as_secs_f32() / t.tm_total.as_secs_f32() });
        g.set_calc_expr(shown(&t.expr).into());
        g.set_calc_result(result(&t.expr).into());
    });
    g.on_sw(stopwatch);
    g.on_tm(timer);
    g.on_tm_set(timer_set);
    g.on_calc_key(|k| calc_key(k.as_str()));
    g.on_action(|id| action(id.as_str()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_symbols() {
        assert_eq!(result("2×3−1"), "5");
        assert_eq!(result("7÷2"), "3,5");
        assert_eq!(result("12"), "");
    }

    #[test]
    fn clock_format() {
        assert_eq!(clock(Duration::from_secs(65)), "01:05");
        assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
    }
}
