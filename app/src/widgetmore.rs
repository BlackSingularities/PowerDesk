//! Widżety: sieć (prędkość), schowek (ostatnie teksty), zadania (lista do odhaczania), kursy
//! walut NBP i odliczanie do daty. Dane do `WidgetData` jak w widgets.rs / widgetfeeds.rs.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use slint::{ModelRc, SharedString, VecModel};

use crate::menus::{it, Item};
use crate::model::Todo;
use crate::transfers::{self, Kind};
use crate::{fmt, WFx, WTodo, WidgetData};

/// Waluty w widżecie (kolejność wyświetlania).
const CODES: [&str; 5] = ["EUR", "USD", "GBP", "CHF", "CZK"];

#[derive(Default)]
struct More {
    down: f64,
    up: f64,
    hist: VecDeque<f32>,
    up_hist: VecDeque<f32>,
    /// Kursy: kod, bieżący, poprzedni; data tabeli.
    fx: Vec<(String, f64, f64)>,
    fx_date: String,
    clip_ids: Vec<u64>,
}

thread_local! {
    static M: RefCell<More> = RefCell::new(More::default());
}

fn edit_prefs(f: impl FnOnce(&mut crate::model::Prefs)) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        f(&mut b.prefs);
        b.prefs.save();
    });
}

fn speed(bps: f64) -> String {
    if bps < 1024.0 {
        format!("{bps:.0} B/s")
    } else {
        format!("{}/s", fmt::size(bps as u64))
    }
}

/// Kursy z dwóch ostatnich tabel A NBP (do zmiany dzień do dnia).
fn refresh_fx() {
    std::thread::spawn(|| {
        let res = powerdesk_core::weather::get_json("https://api.nbp.pl/api/exchangerates/tables/A/last/2/?format=json");
        let Ok(v) = res else { return };
        let rate = |t: &serde_json::Value, code: &str| t["rates"].as_array().and_then(|r| r.iter().find(|x| x["code"] == code)).and_then(|x| x["mid"].as_f64());
        let (prev, last) = (&v[0], &v[1]);
        let fx: Vec<(String, f64, f64)> = CODES.iter().filter_map(|c| Some((c.to_string(), rate(last, c)?, rate(prev, c).unwrap_or(0.0)))).collect();
        let date = last["effectiveDate"].as_str().unwrap_or_default().to_string();
        let _ = slint::invoke_from_event_loop(move || {
            M.with(|m| {
                let mut m = m.borrow_mut();
                m.fx = fx;
                m.fx_date = date;
            });
            crate::widgets::push_all();
        });
    });
}

/// Co sekundę: sieć; kursy – przy pierwszym użyciu i co godzinę.
pub fn tick(n: u32, used: &std::collections::HashSet<String>) {
    if used.contains("network") || used.contains("wifi") {
        let (d, u) = powerdesk_core::netspeed::sample();
        M.with(|m| {
            let mut m = m.borrow_mut();
            m.down = d;
            m.up = u;
            m.hist.push_back(d as f32);
            m.up_hist.push_back(u as f32);
            while m.hist.len() > 40 {
                m.hist.pop_front();
            }
            while m.up_hist.len() > 40 {
                m.up_hist.pop_front();
            }
        });
    }
    if used.contains("currency") && (n % 3600 == 0 || M.with(|m| m.borrow().fx.is_empty()) && n % 30 == 1) {
        refresh_fx();
    }
}

/// Odliczanie: (opis, liczba, jednostka, reszta) do chwili `at`.
fn countdown(label: &str, at: i64) -> (String, String, String, String) {
    if at == 0 {
        return (crate::tr("Odliczanie").into(), "–".into(), String::new(), crate::tr("Prawy przycisk → „Ustaw datę…”").into());
    }
    let left = at - Local::now().timestamp();
    let when = Local.timestamp_opt(at, 0).single().map(|d| d.format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default();
    if left <= 0 {
        return (label.to_string(), "0".into(), "dni".into(), powerdesk_core::trf!("Minęło · {when}"));
    }
    let (d, h, m) = (left / 86400, left / 3600 % 24, left / 60 % 60);
    let (big, unit, rest) = if d >= 1 {
        (d.to_string(), if d == 1 { crate::tr("dzień") } else { "dni" }.to_string(), powerdesk_core::trf!("i {h} h {m} min · {when}"))
    } else if h >= 1 {
        (h.to_string(), "h".to_string(), powerdesk_core::trf!("i {m} min · {when}"))
    } else {
        (m.max(1).to_string(), "min".to_string(), when)
    };
    (label.to_string(), big, unit, rest)
}

pub fn apply(g: &WidgetData) {
    M.with(|m| {
        let m = m.borrow();
        g.set_net_down(speed(m.down).into());
        g.set_net_up(speed(m.up).into());
        let top = m.hist.iter().copied().fold(1024.0f32, f32::max);
        let mut hist: Vec<f32> = vec![0.0; 40 - m.hist.len().min(40)];
        hist.extend(m.hist.iter().map(|v| v / top));
        g.set_net_hist(ModelRc::new(VecModel::from(hist)));
        // Wykres: pobieranie i wysyłanie w jednej skali.
        let top = m.up_hist.iter().copied().fold(top, f32::max);
        let down: Vec<f32> = m.hist.iter().map(|v| v / top).collect();
        let up: Vec<f32> = m.up_hist.iter().map(|v| v / top).collect();
        let (line, area) = crate::widgets::spark(&down, 40);
        g.set_net_line(line.into());
        g.set_net_area(area.into());
        g.set_net_up_line(crate::widgets::spark(&up, 40).0.into());
        let mut up_bars: Vec<f32> = vec![0.0; 40 - up.len().min(40)];
        up_bars.extend(up);
        g.set_net_up_hist(ModelRc::new(VecModel::from(up_bars)));
        let fx: Vec<WFx> = m
            .fx
            .iter()
            .map(|(c, v, p)| {
                let ch = if *p > 0.0 { v - p } else { 0.0 };
                WFx { code: c.clone().into(), value: powerdesk_core::trf!("{v:.4} zł").replace('.', ",").into(), change: format!("{ch:+.4}").replace('.', ",").into(), up: ch >= 0.0 }
            })
            .collect();
        g.set_fx(ModelRc::new(VecModel::from(fx)));
        g.set_fx_date(m.fx_date.clone().into());
    });
    // Schowek: ostatnie teksty z historii PowerDesk (Win+V).
    let texts: Vec<(u64, String)> = powerdesk_core::cliphist::list()
        .into_iter()
        .filter_map(|e| match e.kind {
            powerdesk_core::cliphist::Kind::Text(t) => Some((e.id, t.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string())),
            _ => None,
        })
        .take(6)
        .collect();
    M.with(|m| m.borrow_mut().clip_ids = texts.iter().map(|t| t.0).collect());
    g.set_clips(ModelRc::new(VecModel::from(texts.into_iter().map(|t| SharedString::from(t.1)).collect::<Vec<_>>())));
    g.on_clip_pick(|i| {
        if let Some(id) = M.with(|m| m.borrow().clip_ids.get(i as usize).copied()) {
            powerdesk_core::cliphist::restore(id);
        }
    });
    let (todos, label, at) = {
        let mut out = (Vec::new(), String::new(), 0);
        crate::with_state(|s| {
            let b = s.borrow();
            out = (b.prefs.todos.clone(), b.prefs.countdown_label.clone(), b.prefs.countdown_at);
        });
        out
    };
    g.set_todos(ModelRc::new(VecModel::from(todos.iter().map(|t| WTodo { text: t.text.clone().into(), done: t.done }).collect::<Vec<_>>())));
    g.on_todo_toggle(|i| {
        edit_prefs(|p| {
            if let Some(t) = p.todos.get_mut(i as usize) {
                t.done = !t.done;
            }
        });
        crate::widgets::push_all();
    });
    let (l, big, unit, rest) = countdown(&label, at);
    g.set_cd_label(l.into());
    g.set_cd_big(big.into());
    g.set_cd_unit(unit.into());
    g.set_cd_rest(rest.into());
}

/// „24.12.2026”, „24.12.2026 18:00” albo „24.12” (najbliższy taki dzień), potem opis.
fn parse_date(input: &str) -> Option<(i64, String)> {
    let mut words = input.split_whitespace();
    let d = words.next()?;
    let rest: Vec<&str> = words.collect();
    let now = Local::now();
    let parts: Vec<&str> = d.trim_end_matches('.').split('.').collect();
    let (dd, mm): (u32, u32) = (parts.first()?.parse().ok()?, parts.get(1)?.parse().ok()?);
    let year: i32 = parts.get(2).and_then(|y| y.parse().ok()).unwrap_or(now.year_ce().1 as i32);
    let (time, skip) = match rest.first().and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok()) {
        Some(t) => (t, 1),
        None => (NaiveTime::from_hms_opt(0, 0, 0)?, 0),
    };
    let mut date = NaiveDate::from_ymd_opt(year, mm, dd)?;
    if parts.get(2).is_none() && NaiveDateTime::new(date, time) < now.naive_local() {
        date = NaiveDate::from_ymd_opt(year + 1, mm, dd)?;
    }
    let at = Local.from_local_datetime(&NaiveDateTime::new(date, time)).single()?.timestamp();
    let label = rest[skip..].join(" ");
    Some((at, if label.is_empty() { format!("Do {}", date.format("%d.%m.%Y")) } else { label }))
}

use chrono::Datelike;

pub fn config_items(kind: &str) -> Vec<Item> {
    match kind {
        "todo" => vec![
            it(crate::tr("Dodaj zadanie…"), "plus", |s| {
                crate::show_window(s);
                transfers::prompt(
                    s,
                    crate::tr("Nowe zadanie"),
                    "",
                    crate::tr("Dodaj"),
                    Rc::new(|_, text| {
                        let t = text.trim().to_string();
                        if !t.is_empty() {
                            edit_prefs(|p| p.todos.push(Todo { text: t, done: false }));
                            crate::widgets::push_all();
                        }
                    }),
                );
            }),
            it(crate::tr("Usuń zrobione"), "trash", |_| {
                edit_prefs(|p| p.todos.retain(|t| !t.done));
                crate::widgets::push_all();
            }),
        ],
        "countdown" => vec![it(crate::tr("Ustaw datę…"), "history", |s| {
            crate::show_window(s);
            transfers::prompt(
                s,
                crate::tr("Odliczanie do: „24.12.2026 18:00 Wigilia” albo „1.05 Urlop”"),
                "",
                crate::tr("Ustaw"),
                Rc::new(|s, text| match parse_date(text) {
                    Some((at, label)) => {
                        edit_prefs(|p| {
                            p.countdown_at = at;
                            p.countdown_label = label;
                        });
                        crate::widgets::push_all();
                    }
                    None => transfers::toast(s, crate::tr("Nie rozpoznano daty – np. „24.12.2026 Wigilia”"), Kind::Error, None),
                }),
            );
        })],
        "currency" => vec![it(crate::tr("Odśwież kursy"), "refresh", |_| refresh_fx())],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_date() {
        let (at, label) = parse_date("24.12.2099 18:00 Wigilia").unwrap();
        assert_eq!(label, "Wigilia");
        assert!(at > Local::now().timestamp());
        let (_, l2) = parse_date("1.05").unwrap();
        assert!(l2.starts_with("Do 01.05."));
    }
}
