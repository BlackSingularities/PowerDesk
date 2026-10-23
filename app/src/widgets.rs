//! Widżety: zegar, kalendarz, system, dyski i notatka. Te same widżety stoją w kafelkach Startu
//! (rozmiar kafelka) i na pulpicie (miejsce i rozmiar wolne). Dane liczymy raz na sekundę
//! i rozsyłamy do globalnego `WidgetData` każdego okna, które widżety pokazuje.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, Timelike};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{fmt, WDay, WDrive, WidgetData};

/// Rodzaje widżetów: identyfikator, nazwa w menu, ikona menu i domyślny rozmiar kafelka
/// (0 – średni, 2 – szeroki, 3 – duży) oraz na pulpicie (szerokość, wysokość).
pub const KINDS: [(&str, &str, &str, u8, (f32, f32)); 5] = [
    ("clock", "Zegar", "history", 2, (260.0, 130.0)),
    ("calendar", "Kalendarz", "grid", 3, (260.0, 250.0)),
    ("system", "System", "bolt", 2, (280.0, 200.0)),
    ("drives", "Dyski", "drive", 2, (280.0, 180.0)),
    ("note", "Notatka", "edit", 0, (220.0, 200.0)),
];

pub fn name(kind: &str) -> &'static str {
    KINDS.iter().find(|k| k.0 == kind).map_or("Widżet", |k| k.1)
}

#[derive(Clone, Default)]
struct Data {
    time: String,
    seconds: String,
    sec_frac: f32,
    weekday: String,
    date: String,
    month: String,
    day: i32,
    days: Vec<WDay>,
    cpu: f32,
    ram: f32,
    cpu_text: String,
    ram_text: String,
    uptime: String,
    hist: VecDeque<f32>,
    drives: Vec<WDrive>,
}

thread_local! {
    static DATA: RefCell<Data> = RefCell::new(Data::default());
    static TIMER: slint::Timer = slint::Timer::default();
    static TICKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

const WEEKDAYS: [&str; 7] = ["poniedziałek", "wtorek", "środa", "czwartek", "piątek", "sobota", "niedziela"];
const MONTHS: [&str; 12] = ["Styczeń", "Luty", "Marzec", "Kwiecień", "Maj", "Czerwiec", "Lipiec", "Sierpień", "Wrzesień", "Październik", "Listopad", "Grudzień"];
const MONTHS_OF: [&str; 12] = ["stycznia", "lutego", "marca", "kwietnia", "maja", "czerwca", "lipca", "sierpnia", "września", "października", "listopada", "grudnia"];

/// Siatka miesiąca: 6 tygodni od poniedziałku, dni sąsiednich miesięcy przygaszone.
fn month_grid(today: NaiveDate) -> Vec<WDay> {
    let first = today.with_day(1).unwrap_or(today);
    let start = first - chrono::Duration::days(first.weekday().num_days_from_monday() as i64);
    (0..42)
        .map(|i| {
            let d = start + chrono::Duration::days(i);
            WDay { day: d.day() as i32, cur: d.month() == today.month(), today: d == today }
        })
        .collect()
}

fn uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else {
        format!("{m} min")
    }
}

/// Dyski w tle (lista potrafi czekać na dyski sieciowe) – co 30 s.
fn refresh_drives() {
    std::thread::spawn(|| {
        let list: Vec<(String, f32, String)> = powerdesk_core::drives::list()
            .into_iter()
            .filter(|d| d.online && d.total > 0 && d.kind != "network")
            .take(5)
            .map(|d| {
                let name = if d.label.is_empty() { format!("Dysk ({})", d.letter) } else { format!("{} ({})", d.label, d.letter) };
                let used = d.total.saturating_sub(d.free) as f32 / d.total as f32;
                (name, used, format!("{} wolne z {}", fmt::size(d.free), fmt::size(d.total)))
            })
            .collect();
        let _ = slint::invoke_from_event_loop(move || {
            DATA.with(|x| x.borrow_mut().drives = list.into_iter().map(|(n, u, t)| WDrive { name: n.into(), used: u, detail: t.into() }).collect());
            push_all();
        });
    });
}

fn tick() {
    let now = Local::now();
    let n = TICKS.with(|t| {
        t.set(t.get().wrapping_add(1));
        t.get()
    });
    DATA.with(|x| {
        let mut d = x.borrow_mut();
        d.time = now.format("%H:%M").to_string();
        d.seconds = now.format("%S").to_string();
        d.sec_frac = now.second() as f32 / 60.0;
        let date = now.date_naive();
        d.weekday = WEEKDAYS[date.weekday().num_days_from_monday() as usize].to_string();
        d.date = format!("{} {} {}", date.day(), MONTHS_OF[date.month0() as usize], date.year());
        d.month = format!("{} {}", MONTHS[date.month0() as usize], date.year());
        if d.day != date.day() as i32 {
            d.day = date.day() as i32;
            d.days = month_grid(date);
        }
        // Procesor i pamięć co 2 s – odczyt jest dokładny z różnicy między wywołaniami.
        if n % 2 == 1 {
            let info = powerdesk_core::sysmon::info();
            let cpu = (info.cpu_usage / 100.0).clamp(0.0, 1.0);
            d.cpu = cpu;
            d.cpu_text = format!("{:.0}%", info.cpu_usage);
            d.ram = if info.mem_total > 0 { info.mem_used as f32 / info.mem_total as f32 } else { 0.0 };
            d.ram_text = format!("{} / {}", fmt::size(info.mem_used), fmt::size(info.mem_total));
            d.uptime = uptime(info.uptime);
            d.hist.push_back(cpu);
            while d.hist.len() > 40 {
                d.hist.pop_front();
            }
        }
    });
    if n % 30 == 1 {
        refresh_drives();
    }
    push_all();
}

/// Dane do globalnego `WidgetData` jednego okna (także zaraz po jego utworzeniu).
pub fn apply(g: &WidgetData) {
    DATA.with(|x| {
        let d = x.borrow();
        g.set_time(d.time.clone().into());
        g.set_seconds(d.seconds.clone().into());
        g.set_sec_frac(d.sec_frac);
        g.set_weekday(d.weekday.clone().into());
        g.set_date(d.date.clone().into());
        g.set_month(d.month.clone().into());
        g.set_day(d.day);
        g.set_days(ModelRc::new(VecModel::from(d.days.clone())));
        g.set_cpu(d.cpu);
        g.set_ram(d.ram);
        g.set_cpu_text(SharedString::from(d.cpu_text.as_str()));
        g.set_ram_text(SharedString::from(d.ram_text.as_str()));
        g.set_uptime(d.uptime.clone().into());
        // Krótsza historia (start) – słupki dochodzą od prawej.
        let mut hist: Vec<f32> = vec![0.0; 40 - d.hist.len().min(40)];
        hist.extend(d.hist.iter().copied());
        g.set_cpu_hist(ModelRc::new(VecModel::from(hist)));
        g.set_drives(ModelRc::new(VecModel::from(d.drives.clone())));
    });
}

fn push_all() {
    crate::desktop::each_window(|w| apply(&w.global::<WidgetData>()));
    crate::start::each_window(|w| apply(&w.global::<WidgetData>()));
}

/// Odświeżanie co sekundę – od startu pulpitu (widżety są tam zawsze dostępne).
pub fn start() {
    if TIMER.with(|t| t.running()) {
        return;
    }
    tick();
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(1), tick));
}
