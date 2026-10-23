//! Pasek zadań PowerDesk – na razie tryb testowy: na dole każdego monitora okno na wierzchu
//! (nad paskiem Windows, który działa dalej) z przyciskiem Start, oknami z tego monitora
//! i zegarem. Lista okien odświeża się co sekundę; okna jednego programu dzielą przycisk.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::time::Duration;

use chrono::{Datelike, Local, Months, NaiveDate};
use powerdesk_core::desktop as data;
use powerdesk_core::sysbar::{self, NetKind};
use powerdesk_core::taskwin::{self, Win};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, VecModel};

use crate::menus::{self, it, sep, Item};
use crate::{BarMenuWindow, CalDay, CalendarWindow, Shared, TaskButton, TaskbarWindow, Theme, VolumeWindow};

/// Flaga wiersza poleceń: start od razu z paskiem testowym.
pub const TEST_FLAG: &str = "--taskbar-test";
/// Wysokość paska w pikselach logicznych.
const HEIGHT: f32 = 48.0;

struct Bar {
    w: TaskbarWindow,
    /// Lewy górny róg monitora – po nim przypisujemy okna.
    origin: (i32, i32),
    /// Rozmiar monitora (piksele ekranu).
    mon: (i32, i32),
    /// Położenie okna paska na ekranie (do menu podręcznego).
    pos: (i32, i32),
    /// Ułożone na dole monitora i ukryte z paska Windows.
    placed: bool,
    /// Okna w kolejności pojawiania się na pasku.
    wins: Vec<Win>,
    /// Przyciski: najpierw przypięte programy, potem reszta okien w kolejności `wins`.
    groups: Vec<Group>,
    /// Przypięte programy z ostatniego odświeżenia – zmiana przebudowuje przyciski.
    pins: Vec<String>,
    fg: isize,
}

/// Przycisk paska: program i jego okna z tego monitora (przypięty może nie mieć żadnego).
#[derive(Clone, PartialEq)]
struct Group {
    exe: String,
    pinned: bool,
    wins: Vec<Win>,
}

thread_local! {
    static BARS: RefCell<Vec<Bar>> = const { RefCell::new(Vec::new()) };
    static TIMER: slint::Timer = slint::Timer::default();
    /// Otwarte menu paska i jego pozycje.
    static MENU: RefCell<Option<(BarMenuWindow, Vec<Item>)>> = const { RefCell::new(None) };
    /// Otwarte okienko zasobnika (głośność albo kalendarz).
    static POPUP: RefCell<Option<Popup>> = const { RefCell::new(None) };
    /// Kalendarz: przesunięcie w miesiącach od bieżącego.
    static CAL_SHIFT: Cell<i32> = const { Cell::new(0) };
    /// Sieć sprawdzamy rzadziej niż co sekundę – licznik obiegów i ostatni wynik.
    static TICK: Cell<u32> = const { Cell::new(0) };
    static NET: RefCell<Option<sysbar::Net>> = const { RefCell::new(None) };
}

enum Popup {
    Volume(VolumeWindow),
    Calendar(CalendarWindow),
}

pub fn is_open() -> bool {
    BARS.with(|b| !b.borrow().is_empty())
}

pub fn toggle(s: &Shared) {
    if is_open() {
        close();
    } else {
        open(s);
    }
}

pub fn open(_s: &Shared) {
    if is_open() {
        return;
    }
    std::thread::spawn(|| {
        let mons = data::monitors();
        let _ = slint::invoke_from_event_loop(move || crate::with_state(|s| build(s, mons)));
    });
}

fn build(s: &Shared, mons: Vec<data::Monitor>) {
    if mons.is_empty() {
        crate::transfers::toast(s, "Nie udało się odczytać monitorów", crate::transfers::Kind::Error, None);
        return;
    }
    let look = s.borrow().ui.upgrade().map(|u| crate::settings::look(&u));
    let mut bars = Vec::new();
    for (n, m) in mons.iter().enumerate() {
        let Ok(w) = TaskbarWindow::new() else { continue };
        if let Some(l) = &look {
            let t = w.global::<Theme>();
            t.set_light(l.light);
            t.set_accent_override(l.accent);
        }
        w.on_start(taskwin::open_start);
        w.on_activate(move |i| {
            if let Some(g) = group_at(n, i as usize) {
                click(&g);
                // Stan (aktywne / zminimalizowane) od razu, bez czekania na zegar.
                slint::Timer::single_shot(Duration::from_millis(150), refresh);
            }
        });
        w.on_context(move |i, x, y| context(n, i as usize, x, y));
        w.on_close_test(|| slint::Timer::single_shot(Duration::ZERO, close));
        w.on_volume_open(move |x| open_volume(n, x));
        w.on_volume_wheel(|d| {
            if let Some(v) = sysbar::volume() {
                sysbar::set_volume(((v.level * 50.0).round() + d as f32) / 50.0);
                tray_status();
            }
        });
        w.on_toggle_mute(|| {
            if let Some(v) = sysbar::volume() {
                sysbar::set_mute(!v.muted);
                tray_status();
            }
        });
        w.on_network_open(|| sysbar::open_settings("network-status"));
        w.on_battery_open(|| sysbar::open_settings("batterysaver"));
        w.on_calendar_open(move |x| open_calendar(n, x));
        let _ = w.show();
        bars.push(Bar { w, origin: (m.x, m.y), mon: (m.width, m.height), pos: (m.x, m.y + m.height), placed: false, wins: Vec::new(), groups: Vec::new(), pins: Vec::new(), fg: 0 });
    }
    BARS.with(|b| *b.borrow_mut() = bars);
    refresh();
    // Okno systemowe powstaje dopiero w pętli zdarzeń – ułożenie zaraz po pierwszym obiegu.
    slint::Timer::single_shot(Duration::from_millis(100), refresh);
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(1), refresh));
}

pub fn close() {
    close_menu();
    close_popup();
    TIMER.with(|t| t.stop());
    let bars = BARS.with(|b| std::mem::take(&mut *b.borrow_mut()));
    for b in bars {
        let _ = b.w.hide();
    }
}

fn group_at(bar: usize, i: usize) -> Option<Group> {
    BARS.with(|b| b.borrow().get(bar).and_then(|b| b.groups.get(i).cloned()))
}

fn launch(exe: &str) {
    let _ = std::process::Command::new(exe).spawn();
}

/// Przypięty program bez okien – klik go uruchamia.
fn click(g: &Group) {
    if g.wins.is_empty() {
        launch(&g.exe);
    } else {
        click_group(&g.wins);
    }
}

fn pinned(exe: &str) -> bool {
    let mut on = false;
    crate::with_state(|s| on = s.borrow().prefs.taskbar_pins.iter().any(|p| p.eq_ignore_ascii_case(exe)));
    on
}

/// Przypina albo odpina program; paski przebudują się przy odświeżeniu.
fn toggle_pin(exe: &str) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        let pins = &mut b.prefs.taskbar_pins;
        if let Some(i) = pins.iter().position(|p| p.eq_ignore_ascii_case(exe)) {
            pins.remove(i);
        } else {
            pins.push(exe.to_string());
        }
        b.prefs.save();
    });
    refresh();
}

/// Klik w przycisk: jedno okno – jak na pasku Windows (przełącz / zminimalizuj). Kilka okien –
/// gdy aktywne jest jedno z nich, przechodzi do następnego w kolejności z paska; inaczej
/// wysuwa to, które było ostatnio na wierzchu.
fn click_group(group: &[Win]) {
    match group {
        [] => {}
        [one] => taskwin::click(one.hwnd),
        _ => {
            let fg = taskwin::foreground();
            if let Some(i) = group.iter().position(|w| w.hwnd == fg) {
                taskwin::activate(group[(i + 1) % group.len()].hwnd);
            } else {
                // list() idzie od góry – pierwsze z grupy było ostatnio najwyżej.
                let z = taskwin::list();
                let top = z.iter().find(|w| group.iter().any(|g| g.hwnd == w.hwnd)).map_or(group[0].hwnd, |w| w.hwnd);
                taskwin::activate(top);
            }
        }
    }
}

/// Prawy przycisk na przycisku – menu PowerDesk nad paskiem. Przy kilku oknach programu
/// najpierw lista okien do wyboru, potem polecenia dla całej grupy.
fn context(bar: usize, i: usize, x: f32, _y: f32) {
    let Some(g) = group_at(bar, i) else { return };
    let mut items = Vec::new();
    let pin_item = |exe: String| {
        if pinned(&exe) {
            it("Odepnij od paska", "pin", move |_| toggle_pin(&exe))
        } else {
            it("Przypnij do paska", "pin", move |_| toggle_pin(&exe))
        }
    };
    let Some(first) = g.wins.first().cloned() else {
        // Przypięty program bez okien.
        let exe = g.exe.clone();
        items.push(it(&format!("Uruchom {}", program_name(&g.exe)), "open", move |_| launch(&exe)));
        items.push(sep());
        items.push(pin_item(g.exe.clone()));
        show_menu(bar, items, x);
        return;
    };
    let group = g.wins;
    if group.len() == 1 {
        let target = first.hwnd;
        items.push(it("Przełącz na to okno", "open", move |_| taskwin::activate(target)));
        items.push(if first.minimized {
            it("Przywróć", "eye", move |_| taskwin::activate(target))
        } else {
            it("Zminimalizuj", "minus", move |_| taskwin::minimize(target))
        });
    } else {
        for w in &group {
            let target = w.hwnd;
            items.push(it(&w.title, if w.minimized { "minus" } else { "open" }, move |_| taskwin::activate(target)));
        }
        items.push(sep());
        let all: Vec<isize> = group.iter().map(|w| w.hwnd).collect();
        items.push(it("Zminimalizuj wszystkie", "minus", move |_| all.iter().for_each(|&h| taskwin::minimize(h))));
    }
    if !first.exe.is_empty() {
        let exe = first.exe.clone();
        // Nowe okno tego samego programu (jak środkowy przycisk na pasku Windows).
        items.push(it("Nowe okno programu", "plus", move |_| launch(&exe)));
        items.push(pin_item(first.exe.clone()));
    }
    items.push(sep());
    if group.len() == 1 {
        let target = first.hwnd;
        items.push(it("Zamknij okno", "close", move |_| taskwin::close(target)).danger());
    } else {
        let all: Vec<isize> = group.iter().map(|w| w.hwnd).collect();
        let label = format!("Zamknij wszystkie okna ({})", all.len());
        items.push(it(&label, "close", move |_| all.iter().for_each(|&h| taskwin::close(h))).danger());
    }
    show_menu(bar, items, x);
}

/// Menu w osobnym oknie, tuż nad paskiem, w miejscu kliknięcia (w granicach monitora).
fn show_menu(bar: usize, items: Vec<Item>, x: f32) {
    close_menu();
    close_popup();
    let Some((pos, origin, mon, scale)) = BARS.with(|b| b.borrow().get(bar).map(|b| (b.pos, b.origin, b.mon, b.w.window().scale_factor()))) else { return };
    let Ok(w) = BarMenuWindow::new() else { return };
    BARS.with(|b| {
        if let Some(bar) = b.borrow().get(bar) {
            let t = w.global::<Theme>();
            let src = bar.w.global::<Theme>();
            t.set_light(src.get_light());
            t.set_accent_override(src.get_accent_override());
        }
    });
    let (entries, width, _, _) = menus::layout(&items, 0.0, 0.0, slint::LogicalSize::new(f32::MAX, f32::MAX));
    let height: f32 = items.iter().map(|i| if i.is_sep() { 11.0 } else { 32.0 }).sum::<f32>() + 14.0;
    w.set_entries(ModelRc::new(VecModel::from(entries)));
    w.set_menu_width(width);
    above_bar(w.window(), (pos, origin, mon, scale), width, height, x);
    w.on_chosen(|i| {
        let item = MENU.with(|m| m.borrow().as_ref().and_then(|(_, items)| items.get(i as usize).cloned()));
        // Najpierw zamykamy menu – polecenie może przenieść fokus na inne okno.
        slint::Timer::single_shot(Duration::ZERO, close_menu);
        if let Some(item) = item {
            crate::with_state(|s| menus::run(s, &item));
        }
        slint::Timer::single_shot(Duration::from_millis(300), refresh);
    });
    // Klik obok menu (utrata fokusu) – zamknięcie, jak przy menu w oknie.
    w.window().on_winit_window_event(|_, ev| {
        if let slint::winit_030::winit::event::WindowEvent::Focused(false) = ev {
            slint::Timer::single_shot(Duration::ZERO, close_menu);
        }
        slint::winit_030::EventResult::Propagate
    });
    show_above(w.window());
    MENU.with(|m| *m.borrow_mut() = Some((w, items)));
}

type Spot = ((i32, i32), (i32, i32), (i32, i32), f32);

/// Położenie paska, monitor i skala – do ustawiania okienek nad paskiem.
fn spot(bar: usize) -> Option<Spot> {
    BARS.with(|b| b.borrow().get(bar).map(|b| (b.pos, b.origin, b.mon, b.w.window().scale_factor())))
}

/// Okienko tuż nad paskiem, od miejsca `x` (w oknie paska), w granicach monitora.
fn above_bar(win: &slint::Window, (pos, origin, mon, scale): Spot, width: f32, height: f32, x: f32) {
    let (pw, ph) = ((width * scale) as i32, (height * scale) as i32);
    let sx = (pos.0 + (x * scale) as i32).clamp(origin.0 + 4, (origin.0 + mon.0 - pw - 4).max(origin.0 + 4));
    let sy = pos.1 - ph - (6.0 * scale) as i32;
    win.set_position(PhysicalPosition::new(sx, sy));
    win.set_size(PhysicalSize::new(pw as u32, ph as u32));
}

/// Pokazuje okienko bez przycisku na pasku Windows i z fokusem (jego utrata je zamyka).
fn show_above(win: &slint::Window) {
    let _ = win.show();
    let hwnd = crate::window_hwnd(win);
    data::hide_from_taskbar(hwnd);
    taskwin::activate(hwnd);
}

/// Motyw okienka jak motyw paska (jasny/ciemny, akcent).
macro_rules! same_theme {
    ($bar:expr, $w:expr) => {
        BARS.with(|b| {
            if let Some(bar) = b.borrow().get($bar) {
                let (t, src) = ($w.global::<Theme>(), bar.w.global::<Theme>());
                t.set_light(src.get_light());
                t.set_accent_override(src.get_accent_override());
            }
        })
    };
}

/// Klik obok okienka (utrata fokusu) – zamknięcie.
fn close_on_blur(win: &slint::Window) {
    win.on_winit_window_event(|_, ev| {
        if let slint::winit_030::winit::event::WindowEvent::Focused(false) = ev {
            slint::Timer::single_shot(Duration::ZERO, close_popup);
        }
        slint::winit_030::EventResult::Propagate
    });
}

fn close_popup() {
    match POPUP.with(|p| p.borrow_mut().take()) {
        Some(Popup::Volume(w)) => {
            let _ = w.hide();
        }
        Some(Popup::Calendar(w)) => {
            let _ = w.hide();
        }
        None => {}
    }
}

/// Okienko głośności nad ikoną; drugi klik w ikonę je zamyka.
fn open_volume(bar: usize, x: f32) {
    let was = POPUP.with(|p| matches!(*p.borrow(), Some(Popup::Volume(_))));
    close_menu();
    close_popup();
    if was {
        return;
    }
    let Some(spot) = spot(bar) else { return };
    let Ok(w) = VolumeWindow::new() else { return };
    same_theme!(bar, w);
    if let Some(v) = sysbar::volume() {
        w.set_volume(v.level);
        w.set_muted(v.muted);
    }
    w.on_set_volume(|v| {
        sysbar::set_volume(v);
        tray_status();
    });
    w.on_toggle_mute(|| {
        if let Some(v) = sysbar::volume() {
            sysbar::set_mute(!v.muted);
            tray_status();
        }
    });
    w.on_settings(|| {
        slint::Timer::single_shot(Duration::ZERO, close_popup);
        sysbar::open_settings("sound");
    });
    // Okienko 300 px wyśrodkowane nad ikoną (32 px).
    above_bar(w.window(), spot, 300.0, 96.0, x + 16.0 - 150.0);
    close_on_blur(w.window());
    show_above(w.window());
    POPUP.with(|p| *p.borrow_mut() = Some(Popup::Volume(w)));
}

/// Kalendarz nad zegarem, dosunięty do prawej krawędzi monitora.
fn open_calendar(bar: usize, x: f32) {
    let was = POPUP.with(|p| matches!(*p.borrow(), Some(Popup::Calendar(_))));
    close_menu();
    close_popup();
    if was {
        return;
    }
    let Some(spot) = spot(bar) else { return };
    let Ok(w) = CalendarWindow::new() else { return };
    same_theme!(bar, w);
    CAL_SHIFT.with(|c| c.set(0));
    fill_calendar(&w);
    w.on_shift(|d| {
        CAL_SHIFT.with(|c| c.set(if d == 0 { 0 } else { c.get() + d }));
        POPUP.with(|p| {
            if let Some(Popup::Calendar(w)) = &*p.borrow() {
                fill_calendar(w);
            }
        });
    });
    w.on_settings(|| {
        slint::Timer::single_shot(Duration::ZERO, close_popup);
        sysbar::open_settings("dateandtime");
    });
    above_bar(w.window(), spot, 316.0, 372.0, x);
    close_on_blur(w.window());
    show_above(w.window());
    POPUP.with(|p| *p.borrow_mut() = Some(Popup::Calendar(w)));
}

const MONTHS: [&str; 12] = ["styczeń", "luty", "marzec", "kwiecień", "maj", "czerwiec", "lipiec", "sierpień", "wrzesień", "październik", "listopad", "grudzień"];
const MONTHS_OF: [&str; 12] = ["stycznia", "lutego", "marca", "kwietnia", "maja", "czerwca", "lipca", "sierpnia", "września", "października", "listopada", "grudnia"];
const WEEKDAYS: [&str; 7] = ["poniedziałek", "wtorek", "środa", "czwartek", "piątek", "sobota", "niedziela"];

/// Miesiąc przesunięty o CAL_SHIFT: nagłówek i 42 dni od poniedziałku.
fn fill_calendar(w: &CalendarWindow) {
    let today = Local::now().date_naive();
    let shift = CAL_SHIFT.with(|c| c.get());
    let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let first = if shift >= 0 { first.checked_add_months(Months::new(shift as u32)) } else { first.checked_sub_months(Months::new((-shift) as u32)) }.unwrap_or(first);
    let start = first - chrono::Duration::days(first.weekday().num_days_from_monday() as i64);
    let days: Vec<CalDay> = (0..42)
        .map(|i| {
            let d = start + chrono::Duration::days(i);
            CalDay { num: d.day() as i32, today: d == today, other: d.month() != first.month() }
        })
        .collect();
    let m0 = today.month0() as usize;
    w.set_long_date(format!("{}, {} {} {}", WEEKDAYS[today.weekday().num_days_from_monday() as usize], today.day(), MONTHS_OF[m0], today.year()).into());
    w.set_month(format!("{} {}", MONTHS[first.month0() as usize], first.year()).into());
    w.set_days(ModelRc::new(VecModel::from(days)));
}

/// Głośność, sieć i bateria na wszystkich paskach (sieć co 5 s, reszta przy każdym wywołaniu).
fn tray_status() {
    let vol = sysbar::volume();
    let bat = sysbar::battery();
    let tick = TICK.with(|t| {
        t.set(t.get().wrapping_add(1));
        t.get()
    });
    if tick % 5 == 1 || NET.with(|n| n.borrow().is_none()) {
        let net = sysbar::network();
        NET.with(|n| *n.borrow_mut() = Some(net));
    }
    let net = NET.with(|n| n.borrow().clone()).unwrap_or(sysbar::Net { kind: NetKind::None, name: String::new() });
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            let w = &bar.w;
            w.set_has_volume(vol.is_some());
            if let Some(v) = vol {
                w.set_volume(v.level);
                w.set_muted(v.muted);
            }
            w.set_net_kind(match net.kind {
                NetKind::None => 0,
                NetKind::Wifi => 1,
                NetKind::Wired => 2,
            });
            w.set_net_name(net.name.clone().into());
            w.set_has_battery(bat.is_some());
            if let Some(bt) = bat {
                w.set_battery(bt.percent as i32);
                w.set_charging(bt.charging);
            }
        }
    });
    // Otwarte okienko głośności idzie za zmianami z klawiatury i innych programów.
    POPUP.with(|p| {
        if let (Some(Popup::Volume(w)), Some(v)) = (&*p.borrow(), vol) {
            w.set_volume(v.level);
            w.set_muted(v.muted);
        }
    });
}

fn close_menu() {
    if let Some((w, _)) = MENU.with(|m| m.borrow_mut().take()) {
        let _ = w.hide();
    }
}

/// Pasek na dole swojego monitora i na wierzchu. Za pierwszym razem także rozmiar i styl
/// (bez przycisku na pasku Windows); potem co sekundę już tylko wierzch – pasek Windows
/// potrafi wysunąć się nad nasz po kliknięciu.
fn place(bar: &mut Bar) {
    let hwnd = crate::window_hwnd(bar.w.window());
    if hwnd == 0 {
        return;
    }
    let scale = bar.w.window().scale_factor();
    let h = (HEIGHT * scale).round() as i32;
    bar.pos = (bar.origin.0, bar.origin.1 + bar.mon.1 - h);
    if !bar.placed {
        data::hide_from_taskbar(hwnd);
        bar.w.window().set_position(slint::PhysicalPosition::new(bar.pos.0, bar.pos.1));
        bar.w.window().set_size(slint::PhysicalSize::new(bar.mon.0 as u32, h as u32));
        bar.placed = true;
    }
    taskwin::place_topmost(hwnd, bar.pos.0, bar.pos.1, bar.mon.0, h);
}

/// Stała kolejność: okna już na pasku zostają na swoich miejscach, nowe dochodzą na koniec.
fn keep_order(prev: &[Win], now: Vec<Win>) -> Vec<Win> {
    let mut out: Vec<Win> = prev.iter().filter_map(|p| now.iter().find(|w| w.hwnd == p.hwnd).cloned()).collect();
    for w in now {
        if !out.iter().any(|o| o.hwnd == w.hwnd) {
            out.push(w);
        }
    }
    out
}

/// Nazwa programu z pliku (bez folderu i rozszerzenia) – podpis przypiętego bez okien.
fn program_name(exe: &str) -> String {
    std::path::Path::new(exe).file_stem().map_or_else(|| exe.to_string(), |n| n.to_string_lossy().into_owned())
}

/// Najpierw przypięte programy (także bez okien), potem okna tego samego programu razem –
/// grupa stoi tam, gdzie jej pierwsze okno. Okna bez znanego pliku programu zostają osobno.
fn group(pins: &[String], wins: &[Win]) -> Vec<Group> {
    let mut out: Vec<Group> = pins.iter().map(|p| Group { exe: p.clone(), pinned: true, wins: Vec::new() }).collect();
    for w in wins {
        match out.iter_mut().find(|g| !w.exe.is_empty() && g.exe.eq_ignore_ascii_case(&w.exe)) {
            Some(g) => g.wins.push(w.clone()),
            None => out.push(Group { exe: w.exe.clone(), pinned: false, wins: vec![w.clone()] }),
        }
    }
    out
}

fn buttons(groups: &[Group], fg: isize, px: u32) -> Vec<TaskButton> {
    let mut idle = 0;
    groups
        .iter()
        .map(|g| {
            let idle_before = idle;
            if g.wins.is_empty() {
                idle += 1;
            }
            let img = if g.exe.is_empty() { None } else { crate::thumbs::get(&g.exe, px, true) };
            let w = g.wins.iter().find(|w| w.hwnd == fg).or(g.wins.first());
            TaskButton {
                title: w.map_or_else(|| program_name(&g.exe), |w| w.title.clone()).into(),
                has_img: img.is_some(),
                img: img.unwrap_or_default(),
                active: w.is_some_and(|w| w.hwnd == fg),
                minimized: g.wins.iter().all(|w| w.minimized),
                count: g.wins.len() as i32,
                pinned: g.pinned,
                idle_before,
            }
        })
        .collect()
}

fn refresh() {
    let all = taskwin::list();
    let fg = taskwin::foreground();
    let now = Local::now();
    let (time, date) = (now.format("%H:%M").to_string(), now.format("%d.%m.%Y").to_string());
    let mut pins = Vec::new();
    crate::with_state(|s| pins = s.borrow().prefs.taskbar_pins.clone());
    BARS.with(|b| {
        for bar in b.borrow_mut().iter_mut() {
            place(bar);
            let mine: Vec<Win> = all.iter().filter(|w| w.monitor == bar.origin).cloned().collect();
            let wins = keep_order(&bar.wins, mine);
            // Model tylko przy zmianie – inaczej co sekundę migałyby podpowiedzi.
            if wins != bar.wins || fg != bar.fg || pins != bar.pins {
                let px = (20.0 * bar.w.window().scale_factor()) as u32;
                bar.groups = group(&pins, &wins);
                bar.pins = pins.clone();
                bar.w.set_buttons(ModelRc::new(VecModel::from(buttons(&bar.groups, fg, px))));
                bar.w.set_idle(bar.groups.iter().filter(|g| g.wins.is_empty()).count() as i32);
                bar.wins = wins;
                bar.fg = fg;
            }
            bar.w.set_time(time.clone().into());
            bar.w.set_date(date.clone().into());
        }
    });
    tray_status();
}

/// Ikony programów dochodzą w tle – przerysowujemy paski, na których są.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            if bar.groups.iter().any(|g| paths.contains(&g.exe)) {
                let px = (20.0 * bar.w.window().scale_factor()) as u32;
                bar.w.set_buttons(ModelRc::new(VecModel::from(buttons(&bar.groups, bar.fg, px))));
            }
        }
    });
}
