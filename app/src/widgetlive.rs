//! Dane widżetów z widgetcfg.rs i nowych rodzajów: zegary wg strefy, pogoda wg miejscowości,
//! foldery, ostatnie pliki, ulubione, aplikacje, karta graficzna, bateria, połączenie, głośność,
//! okładka utworu, obraz i cytat dnia, księżyc i postęp czasu. Wolne odczyty idą w tle;
//! wszystko tylko dla widżetów, które gdzieś stoją.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use powerdesk_core::nowplaying::Playing;
use powerdesk_core::weather::{self, Weather};
use slint::{Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::{fmt, WApp, WClock, WDayFc, WFile, WFolder, WProg, WWx, WidgetData};

type Pixels = SharedPixelBuffer<Rgba8Pixel>;

/// Pogoda jednej miejscowości: zapytanie, wynik, numer odczytu ostatniego pobrania.
#[derive(Default)]
struct Wx {
    query: String,
    data: Option<Result<Weather, String>>,
    at: u32,
    busy: bool,
}

/// Plik w widżecie folderu (bez ikony – ta z pamięci miniatur przy rysowaniu).
#[derive(Clone)]
struct Row {
    name: String,
    path: String,
    detail: String,
    dir: bool,
}

#[derive(Default)]
struct Folder {
    path: String,
    rows: Vec<Row>,
    status: String,
    at: u32,
    busy: bool,
}

#[derive(Default)]
struct Live {
    n: u32,
    wx: HashMap<String, Wx>,
    folders: HashMap<String, Folder>,
    recent: Vec<(String, String)>,
    app_names: HashMap<String, String>,
    apps_loaded: bool,
    gpu_name: String,
    gpu: powerdesk_core::gpumon::Gpu,
    gpu_hist: VecDeque<f32>,
    battery: Option<powerdesk_core::sysbar::Battery>,
    net_kind: i32,
    net_name: String,
    wifi_signal: f32,
    wifi_count: i32,
    vol: f32,
    muted: bool,
    cover: Option<Pixels>,
    cover_title: String,
    bing: Option<Pixels>,
    bing_day: String,
}

thread_local! {
    static L: RefCell<Live> = RefCell::new(Live::default());
}
static NET_BUSY: AtomicBool = AtomicBool::new(false);
static VOL_BUSY: AtomicBool = AtomicBool::new(false);

fn done() {
    crate::widgets::push_all();
}

// ---------- Źródła w użyciu ----------

/// Klucze źródeł grupy, które stoją na pulpicie albo w Starcie.
fn used_keys(g: &str) -> HashSet<String> {
    crate::widgets::placed().into_iter().filter(|(k, _)| crate::widgetcfg::group(k) == Some(g)).map(|(k, c)| crate::widgetcfg::key(&k, &c)).collect()
}

/// Zmiana źródła w „Dostosuj…” – nowe miejscowości i foldery pobierają się od razu.
pub fn sources_changed() {
    let n = L.with(|l| l.borrow().n);
    ensure(n, true);
}

fn wx_query(key: &str) -> String {
    if key.is_empty() {
        crate::widgetfeeds::city()
    } else {
        key.to_string()
    }
}

fn folder_path(key: &str) -> String {
    if key.is_empty() {
        dirs::download_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
    } else {
        key.to_string()
    }
}

/// Pogoda co 30 min, folder co 10 s; nowe źródło albo zmiana wspólnej miejscowości – od razu.
fn ensure(n: u32, now: bool) {
    for key in used_keys("city") {
        let q = wx_query(&key);
        let go = L.with(|l| {
            let mut l = l.borrow_mut();
            let e = l.wx.entry(key.clone()).or_default();
            let stale = e.query != q || e.data.is_none() || n.wrapping_sub(e.at) >= 1800;
            if e.busy || !(stale || now && e.data.is_none()) {
                return false;
            }
            e.busy = true;
            e.query = q.clone();
            e.at = n;
            true
        });
        if go {
            let k = key.clone();
            std::thread::spawn(move || {
                let w = weather::fetch(&q);
                let _ = slint::invoke_from_event_loop(move || {
                    L.with(|l| {
                        if let Some(e) = l.borrow_mut().wx.get_mut(&k) {
                            e.data = Some(w);
                            e.busy = false;
                        }
                    });
                    done();
                });
            });
        }
    }
    for key in used_keys("folder") {
        let path = folder_path(&key);
        let go = L.with(|l| {
            let mut l = l.borrow_mut();
            let e = l.folders.entry(key.clone()).or_default();
            if e.busy || (e.path == path && !e.status.is_empty() && n.wrapping_sub(e.at) < 10 && !now) {
                return false;
            }
            e.busy = true;
            e.at = n;
            true
        });
        if go {
            let k = key.clone();
            std::thread::spawn(move || {
                let (rows, status) = scan_folder(&path);
                let _ = slint::invoke_from_event_loop(move || {
                    L.with(|l| {
                        if let Some(e) = l.borrow_mut().folders.get_mut(&k) {
                            e.path = path;
                            e.rows = rows;
                            e.status = status;
                            e.busy = false;
                        }
                    });
                    done();
                });
            });
        }
    }
}

/// Zawartość folderu: najnowsze pierwsze (do 40), bez ukrytych.
fn scan_folder(path: &str) -> (Vec<Row>, String) {
    let Ok(rd) = std::fs::read_dir(path) else { return (Vec::new(), powerdesk_core::trf!("Nie można otworzyć folderu {path}")) };
    let mut v: Vec<(std::time::SystemTime, Row)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.eq_ignore_ascii_case("desktop.ini") {
                return None;
            }
            let md = e.metadata().ok()?;
            let t = md.modified().ok()?;
            let when = chrono::DateTime::<Local>::from(t).format("%d.%m").to_string();
            let detail = if md.is_dir() { when } else { format!("{} · {when}", fmt::size(md.len())) };
            Some((t, Row { name, path: e.path().to_string_lossy().to_string(), detail, dir: md.is_dir() }))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    let status = if v.is_empty() { crate::tr("Folder jest pusty").to_string() } else { String::new() };
    (v.into_iter().take(40).map(|x| x.1).collect(), status)
}

pub fn refresh_weather_all() {
    L.with(|l| l.borrow_mut().wx.values_mut().for_each(|e| e.data = None));
    sources_changed();
}

// ---------- Multimedia i obrazy ----------

fn decode(bytes: &[u8], max: u32) -> Option<Pixels> {
    let img = image::load_from_memory(bytes).ok()?.thumbnail(max, max).to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

/// Nowy utwór (widgetfeeds.rs) – okładka w tle.
pub fn media_changed(p: Option<Playing>) {
    let title = p.as_ref().map(|p| format!("{}\u{1}{}", p.title, p.artist)).unwrap_or_default();
    let same = L.with(|l| {
        let mut l = l.borrow_mut();
        let same = l.cover_title == title;
        l.cover_title = title.clone();
        if title.is_empty() {
            l.cover = None;
        }
        same
    });
    if same || title.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let img = powerdesk_core::nowplaying::cover().and_then(|b| decode(&b, 320));
        let _ = slint::invoke_from_event_loop(move || {
            L.with(|l| {
                let mut l = l.borrow_mut();
                if l.cover_title == title {
                    l.cover = img;
                }
            });
            done();
        });
    });
}

fn refresh_bing(day: String) {
    L.with(|l| l.borrow_mut().bing_day = day);
    std::thread::spawn(|| {
        let img = powerdesk_core::bing::today().and_then(|p| std::fs::read(p).ok()).and_then(|b| decode(&b, 1280));
        let _ = slint::invoke_from_event_loop(move || {
            if img.is_some() {
                L.with(|l| l.borrow_mut().bing = img);
            } else {
                // Bez internetu – kolejna próba przy następnym odczycie.
                L.with(|l| l.borrow_mut().bing_day.clear());
            }
            done();
        });
    });
}

// ---------- System ----------

fn refresh_net(wifi: bool) {
    if NET_BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let net = powerdesk_core::sysbar::network();
        let nets = if wifi { powerdesk_core::wifi::networks() } else { None };
        NET_BUSY.store(false, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(move || {
            L.with(|l| {
                let mut l = l.borrow_mut();
                use powerdesk_core::sysbar::NetKind;
                l.net_kind = match net.kind {
                    NetKind::None => 0,
                    NetKind::Wifi => 1,
                    NetKind::Wired => 2,
                };
                l.net_name = net.name;
                if let Some(list) = nets {
                    l.wifi_count = list.len() as i32;
                    if let Some(c) = list.iter().find(|n| n.connected) {
                        l.wifi_signal = c.signal as f32 / 100.0;
                        l.net_name = c.ssid.clone();
                    }
                }
            });
            done();
        });
    });
}

fn refresh_volume() {
    if VOL_BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let v = powerdesk_core::sysbar::volume();
        VOL_BUSY.store(false, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(v) = v {
                L.with(|l| {
                    let mut l = l.borrow_mut();
                    l.vol = v.level;
                    l.muted = v.muted;
                });
            }
        });
    });
}

fn set_volume(v: f32) {
    L.with(|l| {
        let mut l = l.borrow_mut();
        l.vol = v;
        l.muted = false;
    });
    std::thread::spawn(move || {
        powerdesk_core::sysbar::set_volume(v);
        powerdesk_core::sysbar::set_mute(false);
    });
    done();
}

fn toggle_mute() {
    let m = L.with(|l| {
        let mut l = l.borrow_mut();
        l.muted = !l.muted;
        l.muted
    });
    std::thread::spawn(move || powerdesk_core::sysbar::set_mute(m));
    done();
}

fn load_apps() {
    L.with(|l| l.borrow_mut().apps_loaded = true);
    std::thread::spawn(|| {
        let names: HashMap<String, String> = powerdesk_core::startmenu::apps().into_iter().map(|a| (a.shell_path(), a.name)).collect();
        let _ = slint::invoke_from_event_loop(move || {
            L.with(|l| l.borrow_mut().app_names = names);
            done();
        });
    });
}

/// Co sekundę: źródła danych tylko dla postawionych rodzajów.
pub fn tick(n: u32, used: &HashSet<String>) {
    L.with(|l| l.borrow_mut().n = n);
    ensure(n, false);
    if used.contains("gpu") || used.contains("system") {
        powerdesk_core::gpumon::start();
        if n % 2 == 0 || L.with(|l| l.borrow().gpu_hist.is_empty()) {
            let g = powerdesk_core::gpumon::snapshot();
            L.with(|l| {
                let mut l = l.borrow_mut();
                l.gpu = g;
                l.gpu_hist.push_back((g.usage / 100.0).clamp(0.0, 1.0));
                while l.gpu_hist.len() > 40 {
                    l.gpu_hist.pop_front();
                }
            });
        }
        if L.with(|l| l.borrow().gpu_name.is_empty()) && n % 30 == 1 {
            std::thread::spawn(|| {
                let name = powerdesk_core::gpumon::name();
                let _ = slint::invoke_from_event_loop(move || L.with(|l| l.borrow_mut().gpu_name = name));
            });
        }
    }
    if used.contains("battery") && (n % 5 == 0 || n < 3) {
        let b = powerdesk_core::sysbar::battery();
        L.with(|l| l.borrow_mut().battery = b);
    }
    let wifi = used.contains("wifi");
    if (wifi || used.contains("network")) && (n % 20 == 1 || n % 5 == 0 && !wifi) {
        refresh_net(wifi);
    }
    if used.contains("volume") && n % 2 == 0 {
        refresh_volume();
    }
    if used.contains("recent") && (n % 10 == 1 || L.with(|l| l.borrow().recent.is_empty()) && n % 3 == 0) {
        let r = powerdesk_core::quickindex::recent(24);
        L.with(|l| l.borrow_mut().recent = r);
    }
    if used.contains("launcher") && !L.with(|l| l.borrow().apps_loaded) {
        load_apps();
    }
    let day = Local::now().format("%Y-%m-%d").to_string();
    if used.contains("daypic") && L.with(|l| l.borrow().bing_day != day) {
        refresh_bing(day);
    }
}

// ---------- Teksty i niebo ----------

const QUOTES: [(&str, &str); 16] = [
    ("Wiem, że nic nie wiem.", "Sokrates"),
    ("Myślę, więc jestem.", "Kartezjusz"),
    ("Wszystko płynie.", "Heraklit"),
    ("Chwytaj dzień.", "Horacy"),
    ("Nie wszystek umrę.", "Horacy"),
    ("Poznaj samego siebie.", "napis w Delfach"),
    ("Człowiek jest miarą wszystkich rzeczy.", "Protagoras"),
    ("Kropla drąży skałę nie siłą, lecz częstym spadaniem.", "Owidiusz"),
    ("Śpiesz się powoli.", "Oktawian August"),
    ("Przyszedłem, zobaczyłem, zwyciężyłem.", "Juliusz Cezar"),
    ("Nauka jest jak niezmierne morze; im więcej jej pijesz, tym bardziej jesteś spragniony.", "Stanisław Staszic"),
    ("Ojczyzna to ziemia i groby. Narody, tracąc pamięć, tracą życie.", "Ferdynand Foch"),
    ("Bądź zmianą, którą pragniesz ujrzeć w świecie.", "przypisywane Gandhiemu"),
    ("Póki żyjemy, uczmy się żyć.", "Seneka"),
    ("Nie dlatego, że coś jest trudne, nie mamy odwagi; jest trudne, bo nie mamy odwagi.", "Seneka"),
    ("Szczęście zależy od nas samych.", "Arystoteles"),
];

const PROVERBS: [&str; 20] = [
    "Kto rano wstaje, temu Pan Bóg daje.",
    "Lepszy wróbel w garści niż gołąb na dachu.",
    "Nie chwal dnia przed zachodem słońca.",
    "Gdzie kucharek sześć, tam nie ma co jeść.",
    "Kto pyta, nie błądzi.",
    "Ćwiczenie czyni mistrza.",
    "Co nagle, to po diable.",
    "Prawdziwych przyjaciół poznaje się w biedzie.",
    "Darowanemu koniowi nie zagląda się w zęby.",
    "Kto mieczem wojuje, od miecza ginie.",
    "Nie ma tego złego, co by na dobre nie wyszło.",
    "Mądry Polak po szkodzie.",
    "Apetyt rośnie w miarę jedzenia.",
    "Cicha woda brzegi rwie.",
    "Wszędzie dobrze, ale w domu najlepiej.",
    "Bez pracy nie ma kołaczy.",
    "Fortuna kołem się toczy.",
    "Kto pod kim dołki kopie, ten sam w nie wpada.",
    "Grosz do grosza, a będzie kokosza.",
    "Czego Jaś się nie nauczy, tego Jan nie będzie umiał.",
];

/// Faza księżyca 0–1 (0 – nów) z miesiąca synodycznego od nowiu 6.01.2000 18:14 UTC.
fn moon_phase(ts: i64) -> f64 {
    const SYN: f64 = 29.530588853;
    let days = (ts - 947_182_440) as f64 / 86400.0;
    (days / SYN).rem_euclid(1.0)
}

fn moon_name(p: f64) -> &'static str {
    match p {
        p if !(0.0339..0.9661).contains(&p) => crate::tr("Nów"),
        p if p < 0.2161 => crate::tr("Przybywający sierp"),
        p if p < 0.2839 => crate::tr("Pierwsza kwadra"),
        p if p < 0.4661 => crate::tr("Przybywający garb"),
        p if p < 0.5339 => crate::tr("Pełnia"),
        p if p < 0.7161 => crate::tr("Ubywający garb"),
        p if p < 0.7839 => crate::tr("Ostatnia kwadra"),
        _ => crate::tr("Ubywający sierp"),
    }
}

/// Postęp dnia, tygodnia, miesiąca i roku.
fn progress() -> Vec<WProg> {
    let now = Local::now();
    let secs = now.num_seconds_from_midnight() as f64;
    let day = secs / 86400.0;
    let week = (now.weekday().num_days_from_monday() as f64 + day) / 7.0;
    let first = |y: i32, m: u32| NaiveDate::from_ymd_opt(y, m, 1).unwrap_or_default();
    let (y, m) = (now.year(), now.month());
    let next = if m == 12 { first(y + 1, 1) } else { first(y, m + 1) };
    let mdays = (next - first(y, m)).num_days() as f64;
    let month = (now.day0() as f64 + day) / mdays;
    let ydays = (first(y + 1, 1) - first(y, 1)).num_days() as f64;
    let year = (now.ordinal0() as f64 + day) / ydays;
    [(crate::tr("Dzień"), day), (crate::tr("Tydzień"), week), (crate::tr("Miesiąc"), month), (crate::tr("Rok"), year)]
        .into_iter()
        .map(|(n, f)| WProg { name: n.into(), frac: f as f32, text: format!("{:.0}%", (f * 100.0).floor()).into() })
        .collect()
}

// ---------- Zegary ----------

fn clock_of(zone: &str) -> WClock {
    let utc = chrono::Utc::now();
    let local = Local::now();
    let (h, m, s, date, city, note) = match zone.parse::<chrono_tz::Tz>() {
        Ok(tz) if !zone.is_empty() => {
            let t = utc.with_timezone(&tz);
            let off = chrono::Offset::fix(t.offset()).local_minus_utc();
            let diff = (off - local.offset().local_minus_utc()) / 60;
            let mut note = if diff == 0 { crate::tr("jak tutaj").to_string() } else if diff % 60 == 0 { format!("{:+} h", diff / 60) } else { powerdesk_core::trf!("{:+}:{:02} h", diff / 60, (diff % 60).abs()) };
            if t.date_naive() > local.date_naive() {
                note.push_str(" · jutro");
            } else if t.date_naive() < local.date_naive() {
                note.push_str(" · wczoraj");
            }
            (t.hour(), t.minute(), t.second(), t.date_naive(), crate::widgetfeeds::city_of(zone), note)
        }
        _ => (local.hour(), local.minute(), local.second(), local.date_naive(), String::new(), String::new()),
    };
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    WClock {
        t24: format!("{h:02}:{m:02}").into(),
        t12: format!("{h12}:{m:02}").into(),
        hh24: format!("{h:02}").into(),
        hh12: h12.to_string().into(),
        mm: format!("{m:02}").into(),
        ampm: if h < 12 { "AM" } else { "PM" }.into(),
        sec: format!("{s:02}").into(),
        ha: ((h % 12) as f32 + m as f32 / 60.0) * 30.0,
        ma: (m as f32 + s as f32 / 60.0) * 6.0,
        sa: s as f32 * 6.0,
        weekday: crate::tr(crate::widgets::WEEKDAYS[date.weekday().num_days_from_monday() as usize]).into(),
        date: format!("{} {} {}", date.day(), crate::tr(crate::widgets::MONTHS_OF[date.month0() as usize]), date.year()).into(),
        city: city.into(),
        note: note.into(),
    }
}

// ---------- Dane do okna ----------

const DAYS: [&str; 7] = ["pn", "wt", "śr", "cz", "pt", "so", "nd"];

fn wx_of(e: Option<&Wx>) -> WWx {
    let empty = |status: String| WWx { ok: false, status: status.into(), city: SharedString::new(), temp: 0.0, feels: 0.0, desc: SharedString::new(), kind: 2, wind: SharedString::new(), humidity: SharedString::new(), sunrise: SharedString::new(), sunset: SharedString::new(), days: ModelRc::default(), hours: ModelRc::default(), hmin: 0.0, hmax: 0.0, hour0: 0, hline: SharedString::new(), harea: SharedString::new(), dmin: 0.0, dmax: 1.0 };
    let Some(e) = e else { return empty(crate::tr("Pobieranie pogody…").into()) };
    match &e.data {
        None => empty(crate::tr("Pobieranie pogody…").into()),
        Some(Err(err)) => empty(powerdesk_core::trf!("Pogoda niedostępna: {err}")),
        Some(Ok(w)) => {
            let days: Vec<WDayFc> = w
                .days
                .iter()
                .skip(1)
                .map(|d| {
                    let name = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d").map(|x| crate::tr(DAYS[x.weekday().num_days_from_monday() as usize])).unwrap_or("");
                    WDayFc { name: name.into(), kind: weather::kind(d.code) as i32, hi: d.max as f32, lo: d.min as f32 }
                })
                .collect();
            let dmin = days.iter().map(|d| d.lo).fold(f32::MAX, f32::min);
            let dmax = days.iter().map(|d| d.hi).fold(f32::MIN, f32::max);
            let hmin = w.hours.iter().copied().fold(f64::MAX, f64::min);
            let hmax = w.hours.iter().copied().fold(f64::MIN, f64::max);
            let span = (hmax - hmin).max(1.0);
            let norm: Vec<f32> = w.hours.iter().map(|t| ((t - hmin) / span) as f32).collect();
            let (hline, harea) = crate::widgets::spark(&norm, norm.len().max(2));
            WWx {
                ok: true,
                status: SharedString::new(),
                city: w.place.clone().into(),
                temp: w.temp as f32,
                feels: w.feels as f32,
                desc: weather::describe(w.code).into(),
                kind: weather::kind(w.code) as i32,
                wind: format!("{:.0} km/h", w.wind).into(),
                humidity: format!("{:.0}%", w.humidity).into(),
                sunrise: w.sunrise.clone().into(),
                sunset: w.sunset.clone().into(),
                days: ModelRc::new(VecModel::from(days)),
                hours: ModelRc::new(VecModel::from(w.hours.iter().map(|t| *t as f32).collect::<Vec<_>>())),
                hmin: if w.hours.is_empty() { 0.0 } else { hmin as f32 },
                hmax: if w.hours.is_empty() { 0.0 } else { hmax as f32 },
                hour0: w.hour0 as i32,
                hline: hline.into(),
                harea: harea.into(),
                dmin: if dmin == f32::MAX { 0.0 } else { dmin },
                dmax: if dmax == f32::MIN { 1.0 } else { dmax },
            }
        }
    }
}

/// Ikona pliku z pamięci miniatur (pierwszy raz zamawia ją w tle).
fn file(name: &str, path: &str, detail: &str, dir: bool) -> WFile {
    let icon = crate::thumbs::get(path, 32, true);
    WFile { name: name.into(), path: path.into(), detail: detail.into(), dir, has_icon: icon.is_some(), icon: icon.unwrap_or_default() }
}

/// Nowy model tylko po zmianie – inaczej przewijane listy i podświetlenia skakałyby co sekundę.
fn same<T: Clone + PartialEq + 'static>(m: &ModelRc<T>, v: &[T]) -> bool {
    m.row_count() == v.len() && m.iter().zip(v).all(|(a, b)| a == *b)
}

pub fn apply(g: &WidgetData) {
    let zones: Vec<WClock> = crate::widgetcfg::sources("zone").iter().map(|z| clock_of(z)).collect();
    g.set_clk(ModelRc::new(VecModel::from(zones)));
    L.with(|l| {
        let l = l.borrow();
        let wx: Vec<WWx> = crate::widgetcfg::sources("city").iter().map(|k| wx_of(l.wx.get(k))).collect();
        g.set_wx(ModelRc::new(VecModel::from(wx)));
        let folders: Vec<WFolder> = crate::widgetcfg::sources("folder")
            .iter()
            .map(|k| {
                let path = folder_path(k);
                let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
                match l.folders.get(k) {
                    Some(f) => {
                        let files: Vec<WFile> = f.rows.iter().map(|r| file(&r.name, &r.path, &r.detail, r.dir)).collect();
                        WFolder { name: name.into(), path: path.clone().into(), status: f.status.clone().into(), files: ModelRc::new(VecModel::from(files)) }
                    }
                    None => WFolder { name: name.into(), path: path.into(), status: "Wczytywanie…".into(), files: ModelRc::default() },
                }
            })
            .collect();
        // Foldery: nowy model tylko po zmianie (lista plików w środku jest modelem).
        let cur = g.get_folders();
        let changed = cur.row_count() != folders.len()
            || cur.iter().zip(&folders).any(|(a, b)| a.name != b.name || a.path != b.path || a.status != b.status || !same(&a.files, &b.files.iter().collect::<Vec<_>>()));
        if changed {
            g.set_folders(ModelRc::new(VecModel::from(folders)));
        }
        let recent: Vec<WFile> = l.recent.iter().map(|(n, p)| file(n, p, "", false)).collect();
        if !same(&g.get_recent(), &recent) {
            g.set_recent(ModelRc::new(VecModel::from(recent)));
        }
        // Aplikacje: najczęściej uruchamiane z menu Start.
        let mut launches: Vec<(String, u32)> = Vec::new();
        let mut favs: Vec<String> = Vec::new();
        crate::with_state(|s| {
            let b = s.borrow();
            launches = b.prefs.app_launches.iter().map(|(k, v)| (k.clone(), *v)).collect();
            favs = b.prefs.favorites.clone();
        });
        launches.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let apps: Vec<WApp> = launches
            .iter()
            .filter(|(p, _)| l.app_names.is_empty() || l.app_names.contains_key(p))
            .take(16)
            .map(|(p, _)| {
                let icon = crate::thumbs::get(p, 32, true);
                let name = l.app_names.get(p).cloned().unwrap_or_else(|| p.rsplit(['\\', '!']).next().unwrap_or(p).to_string());
                WApp { name: name.into(), path: p.clone().into(), has_icon: icon.is_some(), icon: icon.unwrap_or_default() }
            })
            .collect();
        if !same(&g.get_apps(), &apps) {
            g.set_apps(ModelRc::new(VecModel::from(apps)));
        }
        let favs: Vec<WFile> = favs
            .iter()
            .map(|p| {
                let name = Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
                file(&name, p, "", Path::new(p).is_dir())
            })
            .collect();
        if !same(&g.get_favs(), &favs) {
            g.set_favs(ModelRc::new(VecModel::from(favs)));
        }
        // Karta graficzna.
        let gpu = l.gpu;
        g.set_gpu_has(!l.gpu_name.is_empty() || gpu.vram_total > 0);
        g.set_gpu_name(l.gpu_name.clone().into());
        g.set_gpu((gpu.usage / 100.0).clamp(0.0, 1.0));
        g.set_gpu_text(format!("{:.0}%", gpu.usage).into());
        g.set_vram(if gpu.vram_total > 0 { gpu.vram_used as f32 / gpu.vram_total as f32 } else { 0.0 });
        g.set_vram_text(if gpu.vram_total > 0 { format!("Pamięć {} / {}", fmt::size(gpu.vram_used), fmt::size(gpu.vram_total)) } else { String::new() }.into());
        g.set_gpu_temp(gpu.temp.map(|t| format!("{t} °C")).unwrap_or_default().into());
        let hist: Vec<f32> = l.gpu_hist.iter().copied().collect();
        let (line, area) = crate::widgets::spark(&hist, 40);
        g.set_gpu_line(line.into());
        g.set_gpu_area(area.into());
        let mut bars = vec![0.0; 40 - hist.len().min(40)];
        bars.extend(hist);
        g.set_gpu_hist(ModelRc::new(VecModel::from(bars)));
        // Bateria, połączenie, głośność.
        g.set_bat_has(l.battery.is_some());
        if let Some(b) = l.battery {
            g.set_bat(b.percent as f32 / 100.0);
            g.set_bat_charging(b.charging);
            g.set_bat_text(if b.charging { if b.percent >= 100 { crate::tr("Naładowana") } else { crate::tr("Ładowanie") } } else { crate::tr("Na baterii") }.into());
        }
        g.set_net_kind(l.net_kind);
        g.set_net_name(l.net_name.clone().into());
        g.set_wifi_signal(l.wifi_signal);
        g.set_wifi_count(l.wifi_count);
        g.set_vol(l.vol);
        g.set_vol_muted(l.muted);
        // Okładka i obraz dnia.
        g.set_m_cover_has(l.cover.is_some());
        if let Some(c) = &l.cover {
            g.set_m_cover(slint::Image::from_rgba8(c.clone()));
        }
        g.set_bing_has(l.bing.is_some());
        if let Some(b) = &l.bing {
            g.set_bing_img(slint::Image::from_rgba8(b.clone()));
        }
    });
    // Cytat i przysłowie – jedno na dzień.
    let now = Local::now();
    let d = now.ordinal() as usize + now.year() as usize;
    let (q, by) = QUOTES[d % QUOTES.len()];
    g.set_quote(q.into());
    g.set_quote_by(by.into());
    g.set_proverb(PROVERBS[d % PROVERBS.len()].into());
    // Księżyc.
    let ts = now.timestamp();
    let p = moon_phase(ts);
    g.set_moon(p as f32);
    g.set_moon_name(moon_name(p).into());
    g.set_moon_light(format!("{:.0}%", (1.0 - (p * std::f64::consts::TAU).cos()) / 2.0 * 100.0).into());
    let (target, label) = if p < 0.5 { (0.5, crate::tr("Pełnia")) } else { (1.0, crate::tr("Nów")) };
    let days = (target - p) * 29.530588853;
    let when = Local.timestamp_opt(ts + (days * 86400.0) as i64, 0).single().map(|t| t.format("%d.%m").to_string()).unwrap_or_default();
    g.set_moon_next(powerdesk_core::trf!("{label} {when} (za {:.0} d)", days.ceil()).into());
    g.set_prog(ModelRc::new(VecModel::from(progress())));
    // Przyciski.
    g.on_open_file(|path, dir| {
        let path = path.to_string();
        if dir {
            crate::with_state(|s| crate::open_args(s, vec![path]));
        } else {
            let _ = open::that_detached(&path);
        }
    });
    g.on_launch(|p| powerdesk_core::startmenu::launch(p.as_str()));
    g.on_vol_set(set_volume);
    g.on_vol_mute(toggle_mute);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_known_dates() {
        // Pełnia 13.01.2025 22:27 UTC, nów 29.01.2025 12:36 UTC.
        let full = chrono::Utc.with_ymd_and_hms(2025, 1, 13, 22, 27, 0).unwrap().timestamp();
        assert!((moon_phase(full) - 0.5).abs() < 0.03);
        let new = chrono::Utc.with_ymd_and_hms(2025, 1, 29, 12, 36, 0).unwrap().timestamp();
        let p = moon_phase(new);
        assert!(p < 0.03 || p > 0.97);
        assert_eq!(moon_name(0.5), "Pełnia");
    }

    #[test]
    fn progress_rows() {
        let p = progress();
        assert_eq!(p.len(), 4);
        assert!(p.iter().all(|r| (0.0..=1.0).contains(&r.frac)));
    }
}
