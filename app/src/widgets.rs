//! Widżety: zegar, kalendarz, system, dyski, notatka, zegary świata, multimedia, zdjęcia
//! i przypomnienia (te z ustawieniami – widgetfeeds.rs), sieć, schowek, zadania i kursy
//! (widgetmore.rs), wskaźniki, pliki, pogoda wg miejscowości i niebo (widgetlive.rs) oraz
//! narzędzia (widgettools.rs). Te same widżety stoją w kafelkach Startu (rozmiar kafelka)
//! i na pulpicie (miejsce i rozmiar wolne); wygląd każdego – widgetcfg.rs („Dostosuj…”).
//! Dane liczymy raz na sekundę i rozsyłamy do globalnego `WidgetData` każdego okna z widżetami.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, Timelike};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{fmt, WDay, WDrive, WidgetData};

/// Rodzaje widżetów: identyfikator, nazwa w menu, ikona menu i domyślny rozmiar kafelka
/// (0 – średni, 2 – szeroki, 3 – duży) oraz na pulpicie (szerokość, wysokość).
pub const KINDS: [(&str, &str, &str, u8, (f32, f32)); 36] = [
    ("clock", "Zegar", "history", 2, (260.0, 130.0)),
    ("analog", "Zegar analogowy", "history", 0, (200.0, 200.0)),
    ("calendar", "Kalendarz", "grid", 3, (260.0, 250.0)),
    ("clocks", "Zegary świata", "history", 2, (260.0, 180.0)),
    ("countdown", "Odliczanie", "history", 0, (220.0, 140.0)),
    ("stopwatch", "Stoper", "history", 3, (240.0, 260.0)),
    ("timer", "Minutnik", "history", 3, (250.0, 290.0)),
    ("progress", "Postęp czasu", "history", 2, (280.0, 160.0)),
    ("weather", "Pogoda", "cloud", 3, (280.0, 200.0)),
    ("forecast", "Prognoza", "cloud", 3, (300.0, 230.0)),
    ("moon", "Księżyc", "dot", 0, (250.0, 130.0)),
    ("media", "Co teraz gra", "play", 2, (320.0, 140.0)),
    ("photos", "Zdjęcia", "image", 3, (300.0, 220.0)),
    ("daypic", "Obraz dnia", "image", 3, (320.0, 200.0)),
    ("quote", "Cytat dnia", "doc", 2, (300.0, 150.0)),
    ("system", "System", "bolt", 2, (280.0, 200.0)),
    ("cpu", "Procesor", "bolt", 2, (280.0, 160.0)),
    ("ram", "Pamięć", "grid", 2, (280.0, 160.0)),
    ("gpu", "Karta graficzna", "grid", 2, (280.0, 160.0)),
    ("drives", "Dyski", "drive", 2, (280.0, 180.0)),
    ("network", "Sieć", "wifi", 2, (280.0, 150.0)),
    ("wifi", "Połączenie", "wifi", 2, (270.0, 150.0)),
    ("battery", "Bateria", "battery", 0, (240.0, 130.0)),
    ("volume", "Głośność", "volume", 2, (270.0, 110.0)),
    ("pcinfo", "Ten komputer", "pc", 2, (300.0, 170.0)),
    ("recent", "Ostatnie pliki", "history", 3, (280.0, 240.0)),
    ("favorites", "Ulubione", "star", 3, (260.0, 220.0)),
    ("folder", "Folder", "folder", 3, (300.0, 260.0)),
    ("launcher", "Aplikacje", "app", 3, (290.0, 210.0)),
    ("actions", "Szybkie akcje", "bolt", 2, (290.0, 150.0)),
    ("clipboard", "Schowek", "paste", 2, (280.0, 200.0)),
    ("note", "Notatka", "edit", 0, (220.0, 200.0)),
    ("todo", "Zadania", "check", 3, (260.0, 240.0)),
    ("reminders", "Przypomnienia", "check", 2, (260.0, 200.0)),
    ("calc", "Kalkulator", "hash", 3, (250.0, 320.0)),
    ("currency", "Kursy walut", "hash", 2, (240.0, 190.0)),
];

/// Grupy w menu „Dodaj widżet”: nazwa, ikona i rodzaje.
pub const GROUPS: [(&str, &str, &[&str]); 6] = [
    ("Czas i daty", "history", &["clock", "analog", "calendar", "clocks", "countdown", "stopwatch", "timer", "progress"]),
    ("Pogoda i niebo", "cloud", &["weather", "forecast", "moon"]),
    ("Multimedia i obrazy", "image", &["media", "photos", "daypic", "quote"]),
    ("System", "bolt", &["system", "cpu", "ram", "gpu", "drives", "network", "wifi", "battery", "volume", "pcinfo"]),
    ("Pliki i aplikacje", "folder", &["recent", "favorites", "folder", "launcher", "actions", "clipboard"]),
    ("Notatki i narzędzia", "edit", &["note", "todo", "reminders", "calc", "currency"]),
];

pub fn name(kind: &str) -> &'static str {
    KINDS.iter().find(|k| k.0 == kind).map_or(crate::tr("Widżet"), |k| crate::tr(k.1))
}

/// Rodzaj z listy `KINDS` (nazwa, ikona, rozmiary domyślne).
pub fn kind(id: &str) -> Option<(&'static str, &'static str, &'static str, u8, (f32, f32))> {
    KINDS.iter().find(|k| k.0 == id).copied()
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
    today_row: i32,
    week_no: String,
    cpu: f32,
    ram: f32,
    cpu_text: String,
    ram_text: String,
    ram_free: String,
    ram_total: String,
    cpu_name: String,
    cpu_mhz: String,
    cpu_cores: String,
    host: String,
    os: String,
    uptime: String,
    hist: VecDeque<f32>,
    ram_hist: VecDeque<f32>,
    drives: Vec<WDrive>,
}

thread_local! {
    static DATA: RefCell<Data> = RefCell::new(Data::default());
    static TIMER: slint::Timer = slint::Timer::default();
    static TICKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub const WEEKDAYS: [&str; 7] = ["poniedziałek", "wtorek", "środa", "czwartek", "piątek", "sobota", "niedziela"];
const MONTHS: [&str; 12] = ["Styczeń", "Luty", "Marzec", "Kwiecień", "Maj", "Czerwiec", "Lipiec", "Sierpień", "Wrzesień", "Październik", "Listopad", "Grudzień"];
pub const MONTHS_OF: [&str; 12] = ["stycznia", "lutego", "marca", "kwietnia", "maja", "czerwca", "lipca", "sierpnia", "września", "października", "listopada", "grudnia"];

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
        powerdesk_core::trf!("{d} d {h} h")
    } else if h > 0 {
        powerdesk_core::trf!("{h} h {m} min")
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
            .take(6)
            .map(|d| {
                let name = if d.label.is_empty() { powerdesk_core::trf!("Dysk ({})", d.letter) } else { format!("{} ({})", d.label, d.letter) };
                let used = d.total.saturating_sub(d.free) as f32 / d.total as f32;
                (name, used, powerdesk_core::trf!("{} wolne z {}", fmt::size(d.free), fmt::size(d.total)))
            })
            .collect();
        let _ = slint::invoke_from_event_loop(move || {
            DATA.with(|x| x.borrow_mut().drives = list.into_iter().map(|(n, u, t)| WDrive { name: n.into(), used: u, detail: t.into() }).collect());
            push_all();
        });
    });
}

/// Rodzaje widżetów, które gdzieś stoją (pulpit albo kafelki Startu) – dane liczymy tylko dla nich.
pub fn in_use() -> std::collections::HashSet<String> {
    placed().into_iter().map(|p| p.0).collect()
}

/// Wszystkie postawione widżety: rodzaj i ustawienia (źródła danych dla widgetlive.rs).
pub fn placed() -> Vec<(String, crate::model::WidgetCfg)> {
    let mut out = Vec::new();
    crate::with_state(|s| {
        let b = s.borrow();
        out.extend(b.prefs.desktop_widgets.iter().map(|w| (w.kind.clone(), w.cfg.clone())));
        for g in &b.prefs.start_groups {
            out.extend(g.tiles.iter().filter(|t| !t.widget.is_empty()).map(|t| (t.widget.clone(), t.cfg.clone())));
        }
    });
    out
}

/// Ścieżki wykresu z historii (0–1, najnowsza na końcu) w polu 0–100 × 0–100: linia i wypełnienie.
/// Krótsza historia (zaraz po starcie) dochodzi od prawej – brakujące punkty to zera.
pub fn spark(values: &[f32], n: usize) -> (String, String) {
    if values.is_empty() || n < 2 {
        return (String::new(), String::new());
    }
    let pad = n.saturating_sub(values.len());
    let skip = values.len().saturating_sub(n);
    let mut line = String::new();
    for i in 0..n {
        let v = if i < pad { 0.0 } else { values[skip + i - pad] };
        let (x, y) = (i as f32 * 100.0 / (n - 1) as f32, 98.0 - v.clamp(0.0, 1.0) * 96.0);
        line.push_str(&format!("{}{x:.1} {y:.1} ", if i == 0 { "M " } else { "L " }));
    }
    let line = line.trim_end().to_string();
    let area = format!("{line} L 100 100 L 0 100 Z");
    (line, area)
}

fn tick() {
    let now = Local::now();
    let used = in_use();
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
        d.weekday = crate::tr(WEEKDAYS[date.weekday().num_days_from_monday() as usize]).to_string();
        d.date = format!("{} {} {}", date.day(), crate::tr(MONTHS_OF[date.month0() as usize]), date.year());
        d.month = format!("{} {}", crate::tr(MONTHS[date.month0() as usize]), date.year());
        if d.day != date.day() as i32 {
            d.day = date.day() as i32;
            d.days = month_grid(date);
            d.today_row = d.days.iter().position(|x| x.today).map_or(0, |i| i / 7) as i32;
            d.week_no = date.iso_week().week().to_string();
        }
        // Procesor i pamięć co 2 s – odczyt jest dokładny z różnicy między wywołaniami.
        let sys = ["system", "cpu", "ram", "pcinfo"].iter().any(|k| used.contains(*k));
        if sys && (n % 2 == 1 || d.cpu_text.is_empty()) {
            let info = powerdesk_core::sysmon::info();
            let cpu = (info.cpu_usage / 100.0).clamp(0.0, 1.0);
            d.cpu = cpu;
            d.cpu_text = format!("{:.0}%", info.cpu_usage);
            d.ram = if info.mem_total > 0 { info.mem_used as f32 / info.mem_total as f32 } else { 0.0 };
            d.ram_text = format!("{} / {}", fmt::size(info.mem_used), fmt::size(info.mem_total));
            d.ram_free = powerdesk_core::trf!("Wolne {}", fmt::size(info.mem_total.saturating_sub(info.mem_used)));
            d.ram_total = fmt::size(info.mem_total);
            d.uptime = uptime(info.uptime);
            d.cpu_name = info.cpu;
            d.cpu_mhz = if info.cpu_mhz > 0 { format!("{:.2} GHz", info.cpu_mhz as f64 / 1000.0).replace('.', ",") } else { String::new() };
            d.cpu_cores = powerdesk_core::trf!("{} rdzeni, {} wątków", info.cores, info.threads);
            d.host = info.host;
            d.os = info.os;
            let ram = d.ram;
            d.hist.push_back(cpu);
            d.ram_hist.push_back(ram);
            while d.hist.len() > 40 {
                d.hist.pop_front();
            }
            while d.ram_hist.len() > 40 {
                d.ram_hist.pop_front();
            }
        }
    });
    // Dyski co 30 s; nowy widżet dysków – od razu.
    let no_drives = DATA.with(|x| x.borrow().drives.is_empty());
    if used.contains("drives") && (n % 30 == 1 || (no_drives && n % 5 == 0)) {
        refresh_drives();
    }
    crate::widgetfeeds::tick(n, &used);
    crate::widgetmore::tick(n, &used);
    crate::widgetlive::tick(n, &used);
    crate::widgettools::tick(&used);
    // Bez widżetów nie ma czego przerysowywać (przypomnienia sprawdza widgetfeeds::tick).
    if !used.is_empty() {
        push_all();
    }
}

/// Historia 0–1 dopełniona zerami do 40 słupków (słupki dochodzą od prawej).
fn bars(h: &VecDeque<f32>) -> Vec<f32> {
    let mut out: Vec<f32> = vec![0.0; 40 - h.len().min(40)];
    out.extend(h.iter().copied());
    out
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
        g.set_today_row(d.today_row);
        g.set_week_no(d.week_no.clone().into());
        g.set_cpu(d.cpu);
        g.set_ram(d.ram);
        g.set_cpu_text(SharedString::from(d.cpu_text.as_str()));
        g.set_ram_text(SharedString::from(d.ram_text.as_str()));
        g.set_ram_pct(format!("{:.0}%", d.ram * 100.0).into());
        g.set_ram_free(d.ram_free.clone().into());
        g.set_ram_total(d.ram_total.clone().into());
        g.set_cpu_name(d.cpu_name.clone().into());
        g.set_cpu_mhz(d.cpu_mhz.clone().into());
        g.set_cpu_cores(d.cpu_cores.clone().into());
        g.set_sys_host(d.host.clone().into());
        g.set_sys_os(d.os.clone().into());
        g.set_uptime(d.uptime.clone().into());
        let cpu: Vec<f32> = d.hist.iter().copied().collect();
        let (line, area) = spark(&cpu, 40);
        g.set_cpu_line(line.into());
        g.set_cpu_area(area.into());
        g.set_cpu_hist(ModelRc::new(VecModel::from(bars(&d.hist))));
        let ram: Vec<f32> = d.ram_hist.iter().copied().collect();
        let (line, area) = spark(&ram, 40);
        g.set_ram_line(line.into());
        g.set_ram_area(area.into());
        g.set_ram_hist(ModelRc::new(VecModel::from(bars(&d.ram_hist))));
        g.set_drives(ModelRc::new(VecModel::from(d.drives.clone())));
    });
    crate::widgetfeeds::apply(g);
    crate::widgetmore::apply(g);
    crate::widgetlive::apply(g);
    crate::widgettools::apply(g);
}

pub fn push_all() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spark_pads_from_left() {
        let (line, area) = spark(&[1.0], 3);
        assert!(line.starts_with("M 0.0 98.0 L 50.0 98.0 L 100.0 2.0"));
        assert!(area.ends_with("Z"));
    }

    #[test]
    fn groups_cover_kinds() {
        for k in KINDS {
            assert!(GROUPS.iter().any(|g| g.2.contains(&k.0)), "{} bez grupy", k.0);
        }
    }
}
