//! Dane widżetów z ustawieniami: zegary świata (strefy), pogoda (miejscowość), co teraz gra,
//! zdjęcia z folderu i przypomnienia. Ustawienia są wspólne dla wszystkich widżetów danego
//! rodzaju (prefs) – zmienia je menu widżetu (`config_items`).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use powerdesk_core::nowplaying::{self, Cmd, Playing};
use powerdesk_core::weather::{self, Weather};
use slint::{ModelRc, VecModel};

use crate::menus::{it, Item};
use crate::model::Reminder;
use crate::transfers::{self, Kind};
use crate::{Shared, WFc, WRem, WZone, WidgetData};

/// Strefy, gdy użytkownik nie wybrał własnych.
pub const DEFAULT_ZONES: [&str; 3] = ["Europe/London", "America/New_York", "Asia/Tokyo"];
pub const DEFAULT_CITY: &str = "Warszawa";

/// Polskie nazwy popularnych stref (reszta – ostatni człon nazwy strefy).
const CITIES: [(&str, &str); 24] = [
    ("Europe/London", "Londyn"),
    ("Europe/Paris", "Paryż"),
    ("Europe/Berlin", "Berlin"),
    ("Europe/Warsaw", "Warszawa"),
    ("Europe/Rome", "Rzym"),
    ("Europe/Madrid", "Madryt"),
    ("Europe/Kyiv", "Kijów"),
    ("Europe/Moscow", "Moskwa"),
    ("Europe/Istanbul", "Stambuł"),
    ("Europe/Lisbon", "Lizbona"),
    ("Europe/Athens", "Ateny"),
    ("America/New_York", "Nowy Jork"),
    ("America/Chicago", "Chicago"),
    ("America/Los_Angeles", "Los Angeles"),
    ("America/Toronto", "Toronto"),
    ("America/Sao_Paulo", "São Paulo"),
    ("America/Mexico_City", "Meksyk"),
    ("Asia/Tokyo", "Tokio"),
    ("Asia/Shanghai", "Szanghaj"),
    ("Asia/Dubai", "Dubaj"),
    ("Asia/Kolkata", "Delhi"),
    ("Asia/Singapore", "Singapur"),
    ("Australia/Sydney", "Sydney"),
    ("Pacific/Auckland", "Auckland"),
];

fn city_of(zone: &str) -> String {
    CITIES.iter().find(|c| c.0 == zone).map(|c| c.1.to_string()).unwrap_or_else(|| zone.rsplit('/').next().unwrap_or(zone).replace('_', " "))
}

/// Strefa z wpisu użytkownika: nazwa strefy (Europe/Paris) albo polska nazwa miasta z listy.
fn zone_of(entry: &str) -> Option<String> {
    let e = entry.trim();
    if e.parse::<chrono_tz::Tz>().is_ok() {
        return Some(e.to_string());
    }
    let low = crate::palette::fold(e);
    CITIES.iter().find(|c| crate::palette::fold(c.1) == low).map(|c| c.0.to_string())
}

#[derive(Default)]
struct Feeds {
    weather: Option<Result<Weather, String>>,
    weather_city: String,
    playing: Option<Playing>,
    photos: Vec<PathBuf>,
    photo_folder: String,
    photo_at: usize,
    photo: Option<(slint::SharedPixelBuffer<slint::Rgba8Pixel>, String)>,
}

thread_local! {
    static F: RefCell<Feeds> = RefCell::new(Feeds::default());
}
static MEDIA_BUSY: AtomicBool = AtomicBool::new(false);

fn prefs<R>(f: impl FnOnce(&crate::model::Prefs) -> R) -> Option<R> {
    let mut out = None;
    crate::with_state(|s| out = Some(f(&s.borrow().prefs)));
    out
}

fn edit_prefs(f: impl FnOnce(&mut crate::model::Prefs)) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        f(&mut b.prefs);
        b.prefs.save();
    });
}

fn zones() -> Vec<String> {
    let z = prefs(|p| p.world_zones.clone()).unwrap_or_default();
    if z.is_empty() {
        DEFAULT_ZONES.iter().map(|s| s.to_string()).collect()
    } else {
        z
    }
}

fn city() -> String {
    prefs(|p| p.weather_city.clone()).filter(|c| !c.trim().is_empty()).unwrap_or_else(|| DEFAULT_CITY.into())
}

fn photo_folder() -> String {
    prefs(|p| p.photo_folder.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| dirs::picture_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default())
}

// ---------- Pobieranie w tle ----------

fn refresh_weather() {
    let c = city();
    F.with(|f| f.borrow_mut().weather_city = c.clone());
    std::thread::spawn(move || {
        let w = weather::fetch(&c);
        let _ = slint::invoke_from_event_loop(move || {
            F.with(|f| f.borrow_mut().weather = Some(w));
            crate::widgets::push_all();
        });
    });
}

fn refresh_media() {
    if MEDIA_BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let p = nowplaying::current();
        MEDIA_BUSY.store(false, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(move || {
            let changed = F.with(|f| {
                let mut f = f.borrow_mut();
                let c = f.playing != p;
                f.playing = p;
                c
            });
            if changed {
                crate::widgets::push_all();
            }
        });
    });
}

pub fn media(k: i32) {
    let c = match k {
        0 => Cmd::Previous,
        2 => Cmd::Next,
        _ => Cmd::PlayPause,
    };
    std::thread::spawn(move || {
        nowplaying::command(c);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = slint::invoke_from_event_loop(refresh_media);
    });
}

const PHOTO_EXT: [&str; 6] = ["jpg", "jpeg", "png", "webp", "bmp", "gif"];

/// Zdjęcia z folderu i jego podfolderów (do 2000, bez ukrytych).
fn scan(folder: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::from(folder)];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| PHOTO_EXT.contains(&x.to_string_lossy().to_lowercase().as_str())) {
                out.push(p);
                if out.len() >= 2000 {
                    return out;
                }
            }
        }
    }
    out
}

fn refresh_photos_list() {
    let folder = photo_folder();
    std::thread::spawn(move || {
        let mut list = scan(&folder);
        // Kolejność losowa, ale stała w obrębie listy (bez zależności od crate'a rand).
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        let mut x = seed | 1;
        for i in (1..list.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            list.swap(i, (x % (i as u64 + 1)) as usize);
        }
        let _ = slint::invoke_from_event_loop(move || {
            F.with(|f| {
                let mut f = f.borrow_mut();
                f.photos = list;
                f.photo_folder = folder;
                f.photo_at = 0;
            });
            next_photo();
        });
    });
}

/// Następne zdjęcie: zmniejszone w tle do 900 px.
pub fn next_photo() {
    let path = F.with(|f| {
        let mut f = f.borrow_mut();
        if f.photos.is_empty() {
            return None;
        }
        let p = f.photos[f.photo_at % f.photos.len()].clone();
        f.photo_at += 1;
        Some(p)
    });
    let Some(path) = path else {
        crate::widgets::push_all();
        return;
    };
    std::thread::spawn(move || {
        let Ok(img) = image::open(&path) else { return };
        let img = img.thumbnail(900, 900).to_rgba8();
        let buf = slint::SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height());
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let _ = slint::invoke_from_event_loop(move || {
            F.with(|f| f.borrow_mut().photo = Some((buf, name)));
            crate::widgets::push_all();
        });
    });
}

// ---------- Przypomnienia ----------

/// „14:30 tekst” (dziś albo jutro), „24.12 18:00 tekst”, „za 15 min tekst”, „za 2 h tekst”.
fn parse_reminder(input: &str) -> Option<Reminder> {
    let t = input.trim();
    let now = Local::now();
    let mut words = t.split_whitespace();
    let first = words.next()?;
    let rest: Vec<&str> = words.collect();
    let at: chrono::DateTime<Local>;
    let text: String;
    if first.eq_ignore_ascii_case("za") && rest.len() >= 2 {
        let n: i64 = rest[0].parse().ok()?;
        let unit = rest[1].to_lowercase();
        let secs = if unit.starts_with('h') || unit.starts_with("godz") { n * 3600 } else { n * 60 };
        at = now + chrono::Duration::seconds(secs);
        text = rest[2..].join(" ");
    } else if let Ok(tm) = NaiveTime::parse_from_str(first, "%H:%M") {
        let mut d = now.date_naive().and_time(tm);
        if d <= now.naive_local() {
            d += chrono::Duration::days(1);
        }
        at = Local.from_local_datetime(&d).single()?;
        text = rest.join(" ");
    } else {
        let (dd, mm) = first.split_once('.')?;
        let (dd, mm): (u32, u32) = (dd.parse().ok()?, mm.trim_end_matches('.').parse().ok()?);
        let tm = rest.first().and_then(|r| NaiveTime::parse_from_str(r, "%H:%M").ok());
        let (tm, skip) = match tm {
            Some(tm) => (tm, 1),
            None => (NaiveTime::from_hms_opt(9, 0, 0)?, 0),
        };
        let mut date = NaiveDate::from_ymd_opt(now.year(), mm, dd)?;
        if NaiveDateTime::new(date, tm) <= now.naive_local() {
            date = NaiveDate::from_ymd_opt(now.year() + 1, mm, dd)?;
        }
        at = Local.from_local_datetime(&NaiveDateTime::new(date, tm)).single()?;
        text = rest[skip..].join(" ");
    }
    let text = if text.trim().is_empty() { "Przypomnienie".to_string() } else { text };
    Some(Reminder { at: at.timestamp(), text })
}

fn when(at: i64) -> (String, bool) {
    let Some(d) = Local.timestamp_opt(at, 0).single() else { return (String::new(), false) };
    let now = Local::now();
    let left = at - now.timestamp();
    let soon = left < 3600;
    let day = if d.date_naive() == now.date_naive() {
        "dziś".to_string()
    } else if d.date_naive() == now.date_naive() + chrono::Duration::days(1) {
        "jutro".to_string()
    } else {
        d.format("%d.%m").to_string()
    };
    let rel = if left < 3600 { format!(" · za {} min", (left / 60).max(0)) } else { String::new() };
    (format!("{day} {}{rel}", d.format("%H:%M")), soon)
}

/// Przypomnienia, których czas minął: powiadomienie, dymek i usunięcie z listy.
fn fire_due(s: &Shared) {
    let now = Local::now().timestamp();
    let due: Vec<Reminder> = {
        let mut b = s.borrow_mut();
        let (due, keep): (Vec<Reminder>, Vec<Reminder>) = b.prefs.reminders.drain(..).partition(|r| r.at <= now);
        b.prefs.reminders = keep;
        if !due.is_empty() {
            b.prefs.save();
        }
        due
    };
    for r in due {
        powerdesk_core::tray::balloon("Przypomnienie", &r.text);
        transfers::toast(s, format!("Przypomnienie: {}", r.text), Kind::Info, None);
    }
}

// ---------- Odświeżanie i dane ----------

/// Co sekundę (`n` – licznik): przypomnienia; co 2 s – multimedia; co 15 s – zdjęcie;
/// co 30 min – pogoda; zmiana miejscowości albo folderu – od razu.
/// Pobieranie tylko dla widżetów, które gdzieś stoją (`used`); przypomnienia – zawsze.
pub fn tick(n: u32, used: &std::collections::HashSet<String>) {
    crate::with_state(fire_due);
    if n % 2 == 0 && used.contains("media") {
        refresh_media();
    }
    let (fresh_w, city_changed, folder_changed, no_photos) = F.with(|f| {
        let f = f.borrow();
        (f.weather.is_none(), f.weather_city != city(), f.photo_folder != photo_folder(), f.photo_folder.is_empty())
    });
    if used.contains("weather") && (fresh_w || city_changed || n % 1800 == 0) {
        refresh_weather();
    }
    if used.contains("photos") {
        if no_photos || folder_changed || n % 600 == 0 {
            refresh_photos_list();
        } else if n % 15 == 0 {
            next_photo();
        }
    }
}

pub fn apply(g: &WidgetData) {
    let now = chrono::Utc::now();
    let local_off = Local::now().offset().local_minus_utc();
    let today = Local::now().date_naive();
    let z: Vec<WZone> = zones()
        .iter()
        .filter_map(|name| {
            let tz: chrono_tz::Tz = name.parse().ok()?;
            let t = now.with_timezone(&tz);
            let off = chrono::Offset::fix(t.offset()).local_minus_utc();
            let diff = (off - local_off) / 60;
            let mut note = if diff == 0 { "jak tutaj".to_string() } else if diff % 60 == 0 { format!("{:+} h", diff / 60) } else { format!("{:+}:{:02} h", diff / 60, (diff % 60).abs()) };
            let d = t.date_naive();
            if d > today {
                note.push_str(" · jutro");
            } else if d < today {
                note.push_str(" · wczoraj");
            }
            Some(WZone { city: city_of(name).into(), time: format!("{:02}:{:02}", t.hour(), t.minute()).into(), note: note.into() })
        })
        .collect();
    g.set_zones(ModelRc::new(VecModel::from(z)));
    F.with(|f| {
        let f = f.borrow();
        match &f.weather {
            Some(Ok(w)) => {
                g.set_w_ok(true);
                g.set_w_city(w.place.clone().into());
                g.set_w_temp(format!("{:.0}°", w.temp).into());
                g.set_w_desc(weather::describe(w.code).into());
                g.set_w_kind(weather::kind(w.code) as i32);
                g.set_w_wind(format!("{:.0} km/h", w.wind).into());
                const DAYS: [&str; 7] = ["pn", "wt", "śr", "cz", "pt", "so", "nd"];
                let fc: Vec<WFc> = w
                    .days
                    .iter()
                    .skip(1)
                    .take(4)
                    .map(|d| {
                        let name = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d").map(|x| DAYS[x.weekday().num_days_from_monday() as usize]).unwrap_or("");
                        WFc { name: name.into(), kind: weather::kind(d.code) as i32, hi: format!("{:.0}°", d.max).into(), lo: format!("{:.0}°", d.min).into() }
                    })
                    .collect();
                g.set_w_days(ModelRc::new(VecModel::from(fc)));
            }
            Some(Err(e)) => {
                g.set_w_ok(false);
                g.set_w_status(format!("Pogoda niedostępna: {e}").into());
            }
            None => {
                g.set_w_ok(false);
                g.set_w_status("Pobieranie pogody…".into());
            }
        }
        match &f.playing {
            Some(p) => {
                g.set_m_has(true);
                g.set_m_title(p.title.clone().into());
                g.set_m_artist(p.artist.clone().into());
                g.set_m_playing(p.playing);
            }
            None => g.set_m_has(false),
        }
        match &f.photo {
            Some((buf, name)) => {
                g.set_p_has(true);
                g.set_p_img(slint::Image::from_rgba8(buf.clone()));
                g.set_p_name(name.clone().into());
            }
            None => {
                g.set_p_has(false);
                g.set_p_status(if f.photo_folder.is_empty() { "Wczytywanie zdjęć…".into() } else { format!("Brak zdjęć w folderze {}", f.photo_folder).into() });
            }
        }
    });
    let mut rems = prefs(|p| p.reminders.clone()).unwrap_or_default();
    rems.sort_by_key(|r| r.at);
    let rows: Vec<WRem> = rems
        .iter()
        .take(5)
        .map(|r| {
            let (w, soon) = when(r.at);
            WRem { when: w.into(), text: r.text.clone().into(), soon }
        })
        .collect();
    g.set_rems(ModelRc::new(VecModel::from(rows)));
    g.on_media(media);
}

// ---------- Menu widżetu ----------

/// Pozycje ustawień w menu widżetu danego rodzaju (w Starcie i na pulpicie).
pub fn config_items(kind: &str) -> Vec<Item> {
    match kind {
        "clocks" => vec![it("Zmień strefy…", "history", |s| {
            let cur = zones().join(", ");
            crate::show_window(s);
            transfers::prompt(
                s,
                "Strefy czasowe (np. Londyn, Tokio, America/Denver)",
                &cur,
                "Zapisz",
                Rc::new(|s, text| {
                    let (ok, bad): (Vec<_>, Vec<_>) = text.split(',').filter(|e| !e.trim().is_empty()).map(|e| (e.trim().to_string(), zone_of(e))).partition(|(_, z)| z.is_some());
                    let list: Vec<String> = ok.into_iter().filter_map(|(_, z)| z).collect();
                    edit_prefs(|p| p.world_zones = list);
                    if !bad.is_empty() {
                        let names: Vec<String> = bad.into_iter().map(|(e, _)| e).collect();
                        transfers::toast(s, format!("Nieznane strefy: {}", names.join(", ")), Kind::Error, None);
                    }
                    crate::widgets::push_all();
                }),
            );
        })],
        "weather" => vec![
            it("Zmień miejscowość…", "cloud", |s| {
                crate::show_window(s);
                transfers::prompt(
                    s,
                    "Miejscowość dla pogody",
                    &city(),
                    "Zapisz",
                    Rc::new(|_, text| {
                        let t = text.trim().to_string();
                        edit_prefs(|p| p.weather_city = t);
                        F.with(|f| f.borrow_mut().weather = None);
                        refresh_weather();
                        crate::widgets::push_all();
                    }),
                );
            }),
            it("Odśwież pogodę", "refresh", |_| refresh_weather()),
        ],
        "photos" => vec![
            it("Wybierz folder ze zdjęciami…", "folder", |s| {
                let cur = photo_folder();
                if let Some(p) = powerdesk_core::shell::pick_folder(crate::hwnd(s), "Folder ze zdjęciami", Some(cur.as_str())) {
                    edit_prefs(|x| x.photo_folder = p);
                    refresh_photos_list();
                }
            }),
            it("Następne zdjęcie", "fwd", |_| next_photo()),
        ],
        "reminders" => vec![
            it("Dodaj przypomnienie…", "plus", |s| {
                crate::show_window(s);
                transfers::prompt(
                    s,
                    "Przypomnienie: „14:30 tekst”, „24.12 18:00 tekst” albo „za 15 min tekst”",
                    "",
                    "Dodaj",
                    Rc::new(|s, text| match parse_reminder(text) {
                        Some(r) => {
                            edit_prefs(|p| p.reminders.push(r));
                            crate::widgets::push_all();
                        }
                        None => transfers::toast(s, "Nie rozpoznano czasu – np. „14:30 spotkanie” albo „za 20 min herbata”", Kind::Error, None),
                    }),
                );
            }),
            it("Usuń wszystkie przypomnienia", "trash", |_| {
                edit_prefs(|p| p.reminders.clear());
                crate::widgets::push_all();
            })
            .danger(),
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminder_in_minutes() {
        let r = parse_reminder("za 15 min herbata").unwrap();
        let left = r.at - Local::now().timestamp();
        assert!((890..=901).contains(&left));
        assert_eq!(r.text, "herbata");
    }

    #[test]
    fn reminder_at_time() {
        let r = parse_reminder("23:59 sen").unwrap();
        assert_eq!(r.text, "sen");
        assert!(r.at > Local::now().timestamp());
    }

    #[test]
    fn zones_by_polish_name() {
        assert_eq!(zone_of("Tokio").as_deref(), Some("Asia/Tokyo"));
        assert_eq!(zone_of("America/Denver").as_deref(), Some("America/Denver"));
        assert!(zone_of("Atlantyda").is_none());
    }
}
