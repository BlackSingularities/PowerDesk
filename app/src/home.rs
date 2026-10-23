//! Ekran „Komputer” (Home.svelte): dyski, stan komputera,
//! rozmiary folderów użytkownika, narzędzia systemu i ostatnio zmienione pliki. Pokazuje się na karcie `ofp:home`
//! zamiast listy; dane o systemie odświeża co 3 s, tylko gdy ekran jest widoczny.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use chrono::Local;
use powerdesk_core::fsx::{self, Entry};
use powerdesk_core::{shell, sysmon};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::fmt::{self, Kind};
use crate::menus::{it, sep};
use crate::model::HOME;
use crate::{HomeActivity, HomeDay, HomeDrive, HomeFile, HomeFolder, HomeMetric, HomeSection, HomeState, HomeTool, Shared};

/// Ile ostatnich plików wczytujemy i pokazujemy.
const RECENT_LIMIT: usize = 120;
const SHOWN: usize = 30;
/// Dni na wykresie zmian.
const DAYS: usize = 14;

/// Sekcje ekranu pod powitaniem: klucz (w ustawieniach) i nagłówek, w domyślnej kolejności.
const SECTIONS: [(&str, &str); 6] = [
    ("overview", "Dyski i system"),
    ("tools", "Narzędzia systemu"),
    ("favorites", "Ulubione"),
    ("activity", "Aktywność"),
    ("changes", "Zmiany w 14 dniach"),
    ("recent", "Ostatnie pliki"),
];

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    /// Ostatnie pliki (wszystkie rodzaje) – filtr wybiera z nich bez ponownego czytania dysku.
    static FILES: RefCell<Option<Vec<Entry>>> = const { RefCell::new(None) };
    /// Rozmiary folderów użytkownika – liczone raz na uruchomienie.
    static SIZES: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
    static CPU: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    static GPU: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    /// Prędkość pobierania (B/s) z kolejnych odczytów i ostatni stan liczników sieci.
    static NET: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
    static NET_LAST: RefCell<Option<(std::time::Instant, u64, u64)>> = const { RefCell::new(None) };
    /// Liczba zmienionych plików w kolejnych dniach (od najstarszego, ostatni – dziś).
    static DAILY: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// Kolejność z ustawień: nieznane klucze pomijamy, brakujące (nowe sekcje) dopisujemy na końcu.
fn order(saved: &[String]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = saved.iter().filter_map(|k| SECTIONS.iter().find(|(x, _)| x == k).map(|(x, _)| *x)).collect();
    for (k, _) in SECTIONS {
        if !v.contains(&k) {
            v.push(k);
        }
    }
    v
}

fn render_sections(s: &Shared) {
    let (keys, shut) = {
        let b = s.borrow();
        (order(&b.prefs.home_order), b.prefs.home_shut.clone())
    };
    let n = keys.len();
    let rows: Vec<HomeSection> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| HomeSection {
            key: (*k).into(),
            title: SECTIONS.iter().find(|(x, _)| x == k).map(|(_, t)| *t).unwrap_or_default().into(),
            shut: shut.iter().any(|x| x == k),
            first: i == 0,
            last: i + 1 == n,
        })
        .collect();
    with_ui(s, |g| g.set_sections(ModelRc::new(VecModel::from(rows))));
}

/// Przesuwa sekcję o jedno miejsce w górę (-1) albo w dół (1).
pub fn move_section(s: &Shared, key: &str, dir: i32) {
    {
        let mut b = s.borrow_mut();
        let mut v: Vec<String> = order(&b.prefs.home_order).into_iter().map(String::from).collect();
        let Some(i) = v.iter().position(|k| k == key) else { return };
        let j = i as i32 + dir;
        if j < 0 || j as usize >= v.len() {
            return;
        }
        v.swap(i, j as usize);
        b.prefs.home_order = v;
        b.prefs.save();
    }
    render_sections(s);
}

pub fn toggle_section(s: &Shared, key: &str) {
    {
        let mut b = s.borrow_mut();
        let shut = &mut b.prefs.home_shut;
        match shut.iter().position(|k| k == key) {
            Some(i) => {
                shut.remove(i);
            }
            None => shut.push(key.to_string()),
        }
        b.prefs.save();
    }
    render_sections(s);
}

/// Przypięte foldery (te same co w panelu bocznym).
fn render_favorites(s: &Shared) {
    let rows: Vec<HomeFolder> = s
        .borrow()
        .prefs
        .favorites
        .iter()
        .map(|p| {
            let img = crate::thumbs::get(p, px(s, 20.0), true);
            HomeFolder {
                name: std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone()).into(),
                icon: ic("pin"),
                size: std::path::Path::new(p).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default().into(),
                ratio: -1.0,
                path: p.clone().into(),
                has_img: img.is_some(),
                img: img.unwrap_or_default(),
            }
        })
        .collect();
    with_ui(s, |g| g.set_favorites(ModelRc::new(VecModel::from(rows))));
}

/// Transfery w toku i stan synchronizacji – odświeżane razem z zegarem.
fn render_activity(s: &Shared) {
    let mut rows: Vec<HomeActivity> = crate::transfers::running(s)
        .into_iter()
        .map(|(title, detail, p)| HomeActivity { icon: ic("copy"), title: title.into(), detail: detail.into(), progress: p, path: Default::default() })
        .collect();
    let (profiles, status) = powerdesk_core::sync::profiles();
    for (p, st) in profiles.iter().zip(status.iter()) {
        let name = if p.name.is_empty() { p.source.clone() } else { p.name.clone() };
        let (detail, progress) = match st.state.as_str() {
            "syncing" => (format!("synchronizuję {:.0}%", st.progress * 100.0), st.progress),
            "scanning" => ("porównuję…".into(), -1.0),
            "offline" => ("cel odłączony".into(), -1.0),
            "error" => ("błąd".into(), -1.0),
            _ if !p.enabled => ("wstrzymana".into(), -1.0),
            _ => (format!("{} → {}", p.source, p.target.path), -1.0),
        };
        rows.push(HomeActivity { icon: ic("sync"), title: name.into(), detail: detail.into(), progress, path: crate::syncview::TAB.into() });
    }
    let shelf = s.borrow().shelf.items.len();
    if shelf > 0 {
        rows.push(HomeActivity { icon: ic("archive"), title: "Półka".into(), detail: fmt::items(shelf).into(), progress: -1.0, path: Default::default() });
    }
    with_ui(s, |g| g.set_activities(ModelRc::new(VecModel::from(rows))));
}

/// Słupki dni: „pn 13”, liczba plików i wysokość względem największego dnia.
fn render_days(s: &Shared) {
    use chrono::Datelike;
    const WD: [&str; 7] = ["pn", "wt", "śr", "cz", "pt", "so", "nd"];
    let daily = DAILY.with(|d| d.borrow().clone());
    let max = daily.iter().copied().max().unwrap_or(0).max(1);
    let today = Local::now().date_naive();
    let rows: Vec<HomeDay> = daily
        .iter()
        .enumerate()
        .map(|(i, &n)| {
            let d = today - chrono::Days::new((daily.len() - 1 - i) as u64);
            HomeDay {
                label: format!("{} {}", WD[d.weekday().num_days_from_monday() as usize], d.day()).into(),
                count: n as i32,
                ratio: n as f32 / max as f32,
                today: i + 1 == daily.len(),
            }
        })
        .collect();
    let total: usize = daily.iter().sum();
    with_ui(s, |g| {
        g.set_days(ModelRc::new(VecModel::from(rows)));
        g.set_changes_note(if daily.is_empty() { String::new() } else { format!("{} w Twoich folderach", fmt::items(total)) }.into());
    });
}

/// Pliki z 14 dni rozłożone na dni (najstarszy pierwszy).
fn daily_counts(list: &[Entry]) -> Vec<usize> {
    let today = Local::now().date_naive();
    let mut v = vec![0usize; DAYS];
    for e in list {
        let Some(d) = chrono::DateTime::from_timestamp_millis(e.modified).map(|t| t.with_timezone(&Local).date_naive()) else { continue };
        let back = (today - d).num_days();
        if (0..DAYS as i64).contains(&back) {
            v[DAYS - 1 - back as usize] += 1;
        }
    }
    v
}

fn with_ui(s: &Shared, f: impl FnOnce(&HomeState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<HomeState>());
    }
}

fn known() -> Vec<(&'static str, PathBuf, &'static str)> {
    [
        ("Pulpit", dirs::desktop_dir(), "desktop"),
        ("Pobrane", dirs::download_dir(), "download"),
        ("Dokumenty", dirs::document_dir(), "doc"),
        ("Obrazy", dirs::picture_dir(), "image"),
        ("Wideo", dirs::video_dir(), "video"),
        ("Muzyka", dirs::audio_dir(), "music"),
    ]
    .into_iter()
    .filter_map(|(n, p, i)| p.filter(|p| p.is_dir()).map(|p| (n, p, i)))
    .collect()
}

fn ic(name: &str) -> slint::SharedString {
    crate::icons::icon(name).into()
}

/// Narzędzia systemu: nazwa, opis, ikona liniowa (zanim dotrze systemowa), źródło ikony
/// systemowej (`sys:` – plik w System32, `app:` – aplikacja z menu Start) i polecenie
/// (ShellExecute; `!map`, `!env`, `!rel` – obsługiwane osobno).
const TOOLS: [(&str, &str, &str, &str, &str); 27] = [
    ("Ustawienia Windows", "Wszystkie ustawienia systemu", "settings", SETTINGS_APP, "ms-settings:"),
    ("Informacje o systemie", "Wersja, sprzęt, nazwa komputera", "info", "sys:msinfo32.exe", "msinfo32.exe"),
    ("Aplikacje", "Instalowanie i odinstalowywanie", "app", "sys:appwiz.cpl", "ms-settings:appsfeatures"),
    ("Aplikacje domyślne", "Czym otwierać pliki i linki", "open", SETTINGS_APP, "ms-settings:defaultapps"),
    ("Panel sterowania", "Klasyczne ustawienia", "list", "sys:control.exe", "control.exe"),
    ("Menedżer zadań", "Programy i wydajność", "bolt", "sys:taskmgr.exe", "taskmgr.exe"),
    ("Monitor zasobów", "Procesor, dysk, sieć na żywo", "bolt", "sys:resmon.exe", "resmon.exe"),
    ("Menedżer urządzeń", "Sterowniki i sprzęt", "usb", "sys:devmgmt.msc", "devmgmt.msc"),
    ("Zarządzanie dyskami", "Partycje i woluminy", "drive", "sys:diskmgmt.msc", "diskmgmt.msc"),
    ("Zarządzanie komputerem", "Usługi, dzienniki, dyski", "pc", "sys:compmgmt.msc", "compmgmt.msc"),
    ("Usługi", "Usługi systemu Windows", "settings", "sys:services.msc", "services.msc"),
    ("Podgląd zdarzeń", "Dzienniki błędów i ostrzeżeń", "history", "sys:eventvwr.msc", "eventvwr.msc"),
    ("Monitor niezawodności", "Awarie, błędy i zmiany w czasie", "shield", "sys:perfmon.exe", "!rel"),
    ("Harmonogram zadań", "Zadania uruchamiane o czasie", "history", "sys:taskschd.msc", "taskschd.msc"),
    ("Edytor rejestru", "Ustawienia zapisane w rejestrze", "code", "sys:..\\regedit.exe", "regedit.exe"),
    ("Konfiguracja systemu", "Uruchamianie i rozruch", "settings", "sys:msconfig.exe", "msconfig.exe"),
    ("Właściwości systemu", "Nazwa, wydajność, ochrona", "pc", "sys:sysdm.cpl", "sysdm.cpl"),
    ("Zmienne środowiskowe", "PATH i inne zmienne", "code", "sys:sysdm.cpl", "!env"),
    ("Połączenia sieciowe", "Karty sieciowe", "network", "sys:ncpa.cpl", "ncpa.cpl"),
    ("Mapuj dysk sieciowy", "Udział sieciowy jako litera dysku", "link", "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", "!map"),
    ("Dźwięk", "Urządzenia odtwarzania i nagrywania", "volume", "sys:mmsys.cpl", "mmsys.cpl"),
    ("Opcje zasilania", "Plany zasilania i uśpienie", "power", "sys:powercfg.cpl", "powercfg.cpl"),
    ("Oczyszczanie dysku", "Pliki tymczasowe i kosz", "trash", "sys:cleanmgr.exe", "cleanmgr.exe"),
    ("Wiersz polecenia", "cmd.exe", "terminal", "sys:cmd.exe", "cmd.exe"),
    ("PowerShell", "Powłoka i skrypty", "terminal", "sys:WindowsPowerShell\\v1.0\\powershell.exe", "powershell.exe"),
    ("Windows Update", "Aktualizacje systemu", "refresh", SETTINGS_APP, "ms-settings:windowsupdate"),
    ("Zabezpieczenia Windows", "Antywirus i zapora", "shield", "app:Microsoft.SecHealthUI_8wekyb3d8bbwe!SecHealthUI", "windowsdefender:"),
];

const SETTINGS_APP: &str = "app:windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel";

/// Ścieżka, z której powłoka da ikonę narzędzia.
fn tool_icon_path(src: &str) -> String {
    let sys = || std::env::var("WINDIR").map(|w| format!(r"{w}\System32")).unwrap_or_else(|_| r"C:\Windows\System32".into());
    if let Some(f) = src.strip_prefix("sys:") {
        return format!(r"{}\{f}", sys());
    }
    if let Some(id) = src.strip_prefix("app:") {
        return format!(r"shell:AppsFolder\{id}");
    }
    src.to_string()
}

/// Uruchamia narzędzie z kafelka (klucz – polecenie z `TOOLS`).
pub fn run_tool(s: &Shared, key: &str) {
    match key {
        "!map" | "map" => powerdesk_core::drives::map_dialog(crate::hwnd(s)),
        "!rel" => {
            let _ = std::process::Command::new("perfmon.exe").arg("/rel").spawn();
        }
        "!env" => {
            let _ = std::process::Command::new("rundll32.exe").args(["sysdm.cpl,EditEnvironmentVariables"]).spawn();
        }
        cmd => {
            let _ = open::that_detached(cmd);
        }
    }
}

fn px(s: &Shared, logical: f32) -> u32 {
    s.borrow().ui.upgrade().map(|u| (logical * u.window().scale_factor()) as u32).unwrap_or(logical as u32)
}

fn render_tools(s: &Shared) {
    let px = px(s, 24.0);
    let rows: Vec<HomeTool> = TOOLS
        .iter()
        .map(|(n, h, i, src, run)| {
            let img = crate::thumbs::get(&tool_icon_path(src), px, true);
            HomeTool { name: (*n).into(), hint: (*h).into(), icon: ic(i), key: (*run).into(), has_img: img.is_some(), img: img.unwrap_or_default() }
        })
        .collect();
    with_ui(s, |g| g.set_tools(ModelRc::new(VecModel::from(rows))));
}

/// Dyski podłączone albo odłączone – ekran od razu, jeśli jest otwarty.
pub fn drives_changed(s: &Shared) {
    if s.borrow().ui.upgrade().is_some_and(|u| u.global::<HomeState>().get_open()) {
        render_drives(s);
    }
}

/// Wołane po każdym przełączeniu karty: włącza albo wyłącza ekran.
pub fn sync(s: &Shared) {
    let on = crate::tab_mut(s, |t| t.path == HOME && t.viewer.is_none() && t.search.is_none());
    let was = s.borrow().ui.upgrade().is_some_and(|u| u.global::<HomeState>().get_open());
    with_ui(s, |g| g.set_open(on));
    if !on {
        TIMER.with(|t| t.stop());
        return;
    }
    if was {
        return;
    }
    // Karta graficzna: liczniki odczytywane co sekundę we własnym wątku (raz na program).
    powerdesk_core::gpumon::start();
    render_static(s);
    tick(s);
    let s2 = s.clone();
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(3), move || tick(&s2)));
    if FILES.with(|f| f.borrow().is_none()) {
        load_files(s);
    } else {
        render_files(s);
    }
    count_folders();
}

/// Rodzaj dysku w opisie: „USB”, „sieciowy”, „napęd optyczny”.
fn drive_kind(kind: &str) -> &'static str {
    match kind {
        "removable" => "USB · ",
        "network" => "sieciowy · ",
        "cdrom" => "napęd optyczny · ",
        "ram" => "RAM-dysk · ",
        _ => "",
    }
}

/// Dyski z wolnym miejscem i suma dla dysków lokalnych.
fn render_drives(s: &Shared) {
    let px = px(s, 20.0);
    let drives: Vec<HomeDrive> = s
        .borrow()
        .drives
        .iter()
        .filter(|d| d.online && d.total > 0)
        .map(|d| HomeDrive {
            name: if d.label.is_empty() { format!("Dysk ({})", d.letter) } else { format!("{} ({})", d.label, d.letter) }.into(),
            path: d.path.clone().into(),
            detail: format!("{}{} wolne z {}{}", drive_kind(&d.kind), fmt::size(d.free), fmt::size(d.total), if d.fs.is_empty() { String::new() } else { format!(" · {}", d.fs) }).into(),
            usage: 1.0 - d.free as f32 / d.total as f32,
            icon: ic(match d.kind.as_str() {
                "network" => "network",
                "removable" => "usb",
                "cdrom" => "disc",
                _ => "drive",
            }),
            img: crate::thumbs::get(&d.path, px, true).unwrap_or_default(),
            has_img: crate::thumbs::get(&d.path, px, true).is_some(),
        })
        .collect();
    let (total, free) = s.borrow().drives.iter().filter(|d| d.online && d.kind != "network").fold((0u64, 0u64), |(t, f), d| (t + d.total, f + d.free));
    with_ui(s, |g| {
        g.set_drives(ModelRc::new(VecModel::from(drives)));
        g.set_storage(if total > 0 { format!("{} wolne z {}", fmt::size(free), fmt::size(total)) } else { String::new() }.into());
        g.set_storage_usage(if total > 0 { 1.0 - free as f32 / total as f32 } else { 0.0 });
    });
}

/// Dyski, foldery i narzędzia – to, co zmienia się rzadko.
fn render_static(s: &Shared) {
    render_drives(s);
    render_tools(s);
    render_folders(s);
    render_sections(s);
    render_favorites(s);
    render_days(s);
}

/// Dopisuje odczyt do historii wykresu (ostatnie 20 – minuta przy odczycie co 3 s).
fn push<T: Copy>(h: &'static std::thread::LocalKey<RefCell<Vec<T>>>, v: T) -> Vec<T> {
    h.with(|h| {
        let mut h = h.borrow_mut();
        h.push(v);
        let n = h.len();
        if n > 20 {
            h.drain(..n - 20);
        }
        h.clone()
    })
}

/// „1,2 MB/s”.
fn rate(bps: f64) -> String {
    format!("{}/s", fmt::size(bps.max(0.0) as u64))
}

/// Pomiary na karcie „Komputer”: procesor, pamięć, karta graficzna, sieć i bateria.
fn metrics(i: &sysmon::Info, cpu_hist: Vec<f32>) -> Vec<HomeMetric> {
    let spark = |v: Vec<f32>| ModelRc::new(VecModel::from(v));
    let mut out = Vec::new();
    let ghz = if i.cpu_mhz > 0 { format!(" · {:.1} GHz", i.cpu_mhz as f64 / 1000.0).replace('.', ",") } else { String::new() };
    out.push(HomeMetric {
        icon: ic("bolt"),
        label: "Procesor".into(),
        value: format!("{:.0}%", i.cpu_usage).into(),
        detail: format!("{} · {} rdz. / {} wątków{ghz}", i.cpu, i.cores, i.threads).into(),
        bar: -1.0,
        spark: spark(cpu_hist),
    });
    let mem = if i.mem_total > 0 { i.mem_used as f32 / i.mem_total as f32 } else { 0.0 };
    out.push(HomeMetric {
        icon: ic("app"),
        label: "Pamięć RAM".into(),
        value: format!("{} / {}", fmt::size(i.mem_used), fmt::size(i.mem_total)).into(),
        detail: format!("{:.0}% zajęte · wolne {}", mem * 100.0, fmt::size(i.mem_total.saturating_sub(i.mem_used))).into(),
        bar: mem,
        spark: Default::default(),
    });
    let g = powerdesk_core::gpumon::snapshot();
    let gname = powerdesk_core::gpumon::name();
    if !gname.is_empty() || g.vram_total > 0 {
        let hist = push(&GPU, g.usage);
        let mut detail = gname;
        if g.vram_total > 0 {
            detail.push_str(&format!(" · VRAM {} / {}", fmt::size(g.vram_used), fmt::size(g.vram_total)));
        }
        if let Some(t) = g.temp {
            detail.push_str(&format!(" · {t}°C"));
        }
        out.push(HomeMetric { icon: ic("image"), label: "Karta graficzna".into(), value: format!("{:.0}%", g.usage).into(), detail: detail.trim_start_matches(" · ").to_string().into(), bar: -1.0, spark: spark(hist) });
    }
    // Sieć: własny pomiar (widżet ma swój), wykres względem najszybszej chwili z minuty.
    if let Some((d, u)) = powerdesk_core::netspeed::totals() {
        let now = std::time::Instant::now();
        let (down, up) = NET_LAST.with(|l| {
            let prev = l.borrow_mut().replace((now, d, u));
            prev.map_or((0.0, 0.0), |(t, pd, pu)| {
                let secs = now.duration_since(t).as_secs_f64().max(0.001);
                (d.saturating_sub(pd) as f64 / secs, u.saturating_sub(pu) as f64 / secs)
            })
        });
        let hist = push(&NET, down);
        let top = hist.iter().copied().fold(1.0, f64::max);
        let net = powerdesk_core::sysbar::network();
        out.push(HomeMetric {
            icon: ic("network"),
            label: "Sieć".into(),
            value: format!("↓ {}  ↑ {}", rate(down), rate(up)).into(),
            detail: if net.name.is_empty() { "Brak połączenia".to_string() } else { net.name }.into(),
            bar: -1.0,
            spark: spark(hist.iter().map(|&v| (v / top * 100.0) as f32).collect()),
        });
    }
    if let Some(b) = powerdesk_core::sysbar::battery() {
        out.push(HomeMetric {
            icon: ic("battery"),
            label: "Bateria".into(),
            value: format!("{}%", b.percent).into(),
            detail: if b.charging { "Zasilacz podłączony" } else { "Na baterii" }.into(),
            bar: b.percent as f32 / 100.0,
            spark: Default::default(),
        });
    }
    out
}

/// Zegar, pomiary komputera, czas pracy i aktywność.
fn tick(s: &Shared) {
    let i = sysmon::info();
    let hist = push(&CPU, i.cpu_usage);
    let up = i.uptime;
    let uptime = match (up / 86_400, up / 3600 % 24, up / 60 % 60) {
        (0, 0, m) => format!("{m} min"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, h, _) => format!("{d} d {h} h"),
    };
    render_activity(s);
    let rows = metrics(&i, hist.clone());
    with_ui(s, |g| {
        g.set_metrics(ModelRc::new(VecModel::from(rows)));
        g.set_sys_note(format!("{} · działa od {uptime}", i.os).into());
        g.set_host(i.host.clone().into());
        g.set_os(i.os.clone().into());
        g.set_cpu(format!("{} · {} rdz. / {} wątków", i.cpu, i.cores, i.threads).into());
        g.set_cpu_usage(i.cpu_usage);
        g.set_cpu_hist(ModelRc::new(VecModel::from(hist)));
        g.set_mem(format!("{} / {}", fmt::size(i.mem_used), fmt::size(i.mem_total)).into());
        g.set_mem_usage(if i.mem_total > 0 { i.mem_used as f32 / i.mem_total as f32 } else { 0.0 });
        g.set_uptime(uptime.into());
    });
}

fn render_folders(s: &Shared) {
    let sizes = SIZES.with(|x| x.borrow().clone());
    let max = sizes.values().copied().max().unwrap_or(0).max(1);
    let px = px(s, 20.0);
    let rows: Vec<HomeFolder> = known()
        .into_iter()
        .map(|(n, p, icon)| {
            let p = p.to_string_lossy().to_string();
            let sz = sizes.get(&p).copied();
            let img = crate::thumbs::get(&p, px, true);
            HomeFolder {
                name: n.into(),
                icon: ic(icon),
                size: sz.map(fmt::size).unwrap_or_default().into(),
                ratio: sz.map(|v| v as f32 / max as f32).unwrap_or(-1.0),
                path: p.into(),
                has_img: img.is_some(),
                img: img.unwrap_or_default(),
            }
        })
        .collect();
    with_ui(s, |g| g.set_folders(ModelRc::new(VecModel::from(rows))));
}

/// Rozmiary folderów w tle, po kolei; każdy wynik od razu na ekran.
fn count_folders() {
    let todo: Vec<String> = known().into_iter().map(|(_, p, _)| p.to_string_lossy().to_string()).filter(|p| SIZES.with(|x| !x.borrow().contains_key(p))).collect();
    if todo.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for p in todo {
            let size = fsx::dir_size(&p).size;
            let _ = slint::invoke_from_event_loop(move || {
                SIZES.with(|x| x.borrow_mut().insert(p, size));
                crate::with_state(render_folders);
            });
        }
    });
}

fn load_files(s: &Shared) {
    with_ui(s, |g| g.set_files_loading(true));
    std::thread::spawn(|| {
        let dirs: Vec<String> = known().into_iter().map(|(_, p, _)| p.to_string_lossy().to_string()).collect();
        // Wszystkie pliki z 14 dni – do wykresu; na kafelki tylko najnowsze.
        let mut list = fsx::recent_files(&dirs, DAYS as u64, usize::MAX);
        let daily = daily_counts(&list);
        list.truncate(RECENT_LIMIT);
        let _ = slint::invoke_from_event_loop(move || {
            FILES.with(|f| *f.borrow_mut() = Some(list));
            DAILY.with(|d| *d.borrow_mut() = daily);
            crate::with_state(|s| {
                with_ui(s, |g| g.set_files_loading(false));
                render_files(s);
                render_days(s);
            });
        });
    });
}

/// „5 min temu”, „wczoraj”, „3 dni temu”.
fn ago(ms: i64) -> String {
    let mins = (Local::now().timestamp_millis() - ms).max(0) / 60_000;
    match mins {
        0 => "przed chwilą".into(),
        1..=59 => format!("{mins} min temu"),
        60..=1439 => format!("{} godz. temu", mins / 60),
        1440..=2879 => "wczoraj".into(),
        _ => format!("{} dni temu", mins / 1440),
    }
}

fn shown(s: &Shared) -> Vec<Entry> {
    let mut kind = 0;
    with_ui(s, |g| kind = g.get_kind());
    FILES.with(|f| {
        f.borrow()
            .iter()
            .flatten()
            .filter(|e| match kind {
                1 => matches!(fmt::kind_of(e), Kind::Image | Kind::Raw),
                2 => matches!(fmt::kind_of(e), Kind::Doc | Kind::Sheet | Kind::Pdf | Kind::Text),
                3 => fmt::kind_of(e) == Kind::Video,
                _ => true,
            })
            .take(SHOWN)
            .cloned()
            .collect()
    })
}

fn render_files(s: &Shared) {
    let px = s.borrow().ui.upgrade().map(|u| (110.0 * u.window().scale_factor()) as u32).unwrap_or(110);
    let rows: Vec<HomeFile> = shown(s)
        .into_iter()
        .map(|e| {
            let media = fmt::is_media(&e);
            let img = crate::thumbs::get(&e.path, px, false);
            HomeFile { name: e.name.clone().into(), path: e.path.clone().into(), when: ago(e.modified).into(), has_img: img.is_some(), img: img.unwrap_or_default(), media }
        })
        .collect();
    with_ui(s, |g| g.set_files(ModelRc::new(VecModel::from(rows))));
}

pub fn thumbs_arrived(s: &Shared, paths: &HashSet<String>) {
    let open = s.borrow().ui.upgrade().is_some_and(|u| u.global::<HomeState>().get_open());
    if !open {
        return;
    }
    if shown(s).iter().any(|e| paths.contains(&e.path)) {
        render_files(s);
    }
    // Ikony systemowe dysków, folderów i narzędzi.
    if s.borrow().drives.iter().any(|d| paths.contains(&d.path)) {
        render_drives(s);
    }
    if known().iter().any(|(_, p, _)| paths.contains(&p.to_string_lossy().to_string())) {
        render_folders(s);
    }
    if s.borrow().prefs.favorites.iter().any(|p| paths.contains(p)) {
        render_favorites(s);
    }
    if TOOLS.iter().any(|t| paths.contains(&tool_icon_path(t.3))) {
        render_tools(s);
    }
}

pub fn set_kind(s: &Shared, k: i32) {
    with_ui(s, |g| g.set_kind(k));
    render_files(s);
}

pub fn refresh_files(s: &Shared) {
    FILES.with(|f| *f.borrow_mut() = None);
    with_ui(s, |g| g.set_files(ModelRc::default()));
    load_files(s);
}

fn reveal(s: &Shared, path: &str) {
    if let Some(dir) = std::path::Path::new(path).parent() {
        crate::navigate(s, &dir.to_string_lossy(), true);
        crate::select_path(s, path);
    }
}

pub fn file_menu(s: &Shared, path: String, x: f32, y: f32) {
    let (p1, p2, p3, p4) = (path.clone(), path.clone(), path.clone(), path);
    let items = vec![
        it("Otwórz", "open", move |_| {
            let _ = open::that_detached(&p1);
        }),
        it("Pokaż w folderze", "folder", move |s| reveal(s, &p2)),
        sep(),
        it("Kopiuj ścieżkę", "copy", move |s| crate::props::copy(s, &p3)),
        it("Właściwości systemowe", "info", move |s| {
            shell::show_properties(crate::hwnd(s), &p4);
        }),
    ];
    crate::menus::show(s, items, x, y);
}

pub fn bind(ui: &crate::MainWindow, st: &Shared) {
    let g = ui.global::<HomeState>();
    let s = st.clone();
    g.on_go(move |p| crate::navigate(&s, &p, true));
    let s = st.clone();
    g.on_go_new_tab(move |p| crate::new_tab_at(&s, &p, false));
    g.on_open_file(|p| {
        let _ = open::that_detached(p.as_str());
    });
    let s = st.clone();
    g.on_file_menu(move |p, x, y| file_menu(&s, p.into(), x, y));
    let s = st.clone();
    g.on_set_kind(move |k| set_kind(&s, k));
    let s = st.clone();
    g.on_refresh_files(move || refresh_files(&s));
    let s = st.clone();
    g.on_move_section(move |k, d| move_section(&s, &k, d));
    let s = st.clone();
    g.on_toggle_section(move |k| toggle_section(&s, &k));
    let s = st.clone();
    g.on_tool(move |k| run_tool(&s, &k));
    let s = st.clone();
    g.on_drive_menu(move |p, x, y| crate::menus::side_menu(&s, p.into(), x, y));
}
