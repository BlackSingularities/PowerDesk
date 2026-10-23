//! Pasek zadań PowerDesk zamiast paska Windows: na dole każdego monitora okno na wierzchu,
//! zarejestrowane jako AppBar; pasek Windows ukryty (core::shellswap, przywracany przy wyjściu).
//! Start, okna z tego monitora, zasobnik i zegar. Lista okien odświeża się co sekundę; okna jednego programu dzielą przycisk.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::time::Duration;

use chrono::{Datelike, Local, Months, NaiveDate};
use powerdesk_core::desktop as data;
use powerdesk_core::sysbar::{self, NetKind};
use powerdesk_core::wifi;
use powerdesk_core::taskwin::{self, Win};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, PhysicalSize, VecModel};

use crate::menus::{self, it, sep, Item};
use crate::{NoticesWindow, OutDev, AppVol, TrayApp, BarMenuWindow, CalDay, CalendarWindow, NetRow, NetworkWindow, Shared, TaskButton, TaskbarWindow, Theme, ThumbCard, ThumbWindow, VolumeWindow};

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
    /// Podglądy okien: otwarte okienko, przycisk pod kursorem (pasek, numer), kursor nad podglądami.
    static THUMBS: RefCell<Option<Thumbs>> = const { RefCell::new(None) };
    static HOVER: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
    static THUMB_HOVER: Cell<bool> = const { Cell::new(false) };
    /// Okienko głośności: miejsce nad paskiem, x i wysokość bez listy urządzeń.
    static VOL_GEOM: Cell<Option<(Spot, f32, f32)>> = const { Cell::new(None) };
}

enum Popup {
    Volume(VolumeWindow),
    Calendar(CalendarWindow),
    Network(NetworkWindow),
    Notices(NoticesWindow),
}

pub fn is_open() -> bool {
    BARS.with(|b| !b.borrow().is_empty())
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
    // Jeden pasek (tylko monitor główny) albo na każdym monitorze – Ustawienia → Pasek zadań.
    let all = s.borrow().prefs.taskbar_all_monitors;
    let mons: Vec<data::Monitor> = if all || mons.len() < 2 { mons } else { mons.into_iter().filter(|m| m.primary).collect() };
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
            t.set_popup_alpha(l.popup_alpha);
        }
        w.on_start(move || open_start(n));
        w.on_start_context(move |x| start_menu(n, x));
        w.on_bar_context(move |x| bar_menu(n, x));
        w.on_show_desktop(taskwin::show_desktop);
        w.on_activate(move |i| {
            close_thumbs();
            if let Some(g) = group_at(n, i as usize) {
                click(&g);
                // Stan (aktywne / zminimalizowane) od razu, bez czekania na zegar.
                slint::Timer::single_shot(Duration::from_millis(150), refresh);
            }
        });
        w.on_reorder(move |i, x, each| reorder(n, i as usize, x, each));
        w.on_context(move |i, x, y| {
            close_thumbs();
            context(n, i as usize, x, y)
        });
        w.on_hover(move |i, x| hover(n, i as usize, x));
        w.on_unhover(unhover);
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
        w.on_network_open(move |x| open_network(n, x));
        w.on_battery_open(|| sysbar::open_settings("batterysaver"));
        w.on_calendar_open(move |x| open_calendar(n, x));
        w.on_notices_open(move |x| open_notices(n, x));
        w.on_vd_menu(move |x| vd_menu(n, x));
        w.on_vd_wheel(|d| {
            std::thread::spawn(move || powerdesk_core::vdesk::step(d));
            slint::Timer::single_shot(Duration::from_millis(400), vd_status);
        });
        w.set_notice_count(crate::notices::unread());
        w.on_tray_app_click(|key, b| {
            use powerdesk_core::trayhost::{self, Click};
            let (x, y) = taskwin::cursor();
            let button = match b {
                1 => Click::Double,
                2 => Click::Right,
                _ => Click::Left,
            };
            trayhost::click(key.as_str(), button, x, y);
        });
        let _ = w.show();
        bars.push(Bar { w, origin: (m.x, m.y), mon: (m.width, m.height), pos: (m.x, m.y + m.height), placed: false, wins: Vec::new(), groups: Vec::new(), pins: Vec::new(), fg: 0 });
    }
    BARS.with(|b| *b.borrow_mut() = bars);
    // Zasobnik z ikonami innych programów (własny Shell_TrayWnd, przed paskiem Eksploratora).
    // `start` działa raz na proces – po przebudowie pasków (zmiana monitorów) tylko odświeżenie.
    powerdesk_core::trayhost::on_balloon(|title, text| {
        let _ = slint::invoke_from_event_loop(move || crate::notices::add("Program w zasobniku", &title, &text));
    });
    powerdesk_core::trayhost::start(|| {
        let _ = slint::invoke_from_event_loop(tray_apps);
    });
    tray_apps();
    // Tryb powłoki: Win+E – okno PowerDesk, Win+R – „Uruchom”, Win+D – pulpit, Win+Tab – widok zadań.
    powerdesk_core::startmenu::hook_shell_keys(|k, shift| {
        let ui = |f: fn()| {
            let _ = slint::invoke_from_event_loop(f);
        };
        let uri = |u: &'static str| {
            let _ = open::that_detached(u);
        };
        match k {
            b'E' => ui(|| crate::with_state(|s| crate::open_args(s, Vec::new()))),
            b'R' => powerdesk_core::shellmode::run_dialog(),
            b'D' | b'M' => taskwin::show_desktop(),
            0x09 => ui(crate::taskview::toggle),
            // Win+Shift+S i PrtScn – wycinek; Win+PrtScn – cały ekran do pliku i schowka.
            b'S' if shift => screenshot(false),
            0x2C => screenshot(shift),
            // Win+S – szukanie: Start z polem szukania.
            b'S' => ui(|| open_start(bar_under_cursor())),
            // Win+A – szybkie ustawienia (sieć), Win+N – powiadomienia, Win+X – menu Startu.
            b'A' => ui(|| {
                let bar = bar_under_cursor();
                let x = BARS.with(|b| b.borrow().get(bar).map(|b| b.mon.0 as f32 / b.w.window().scale_factor())).unwrap_or(0.0);
                open_network(bar, x - 300.0);
            }),
            b'N' => ui(crate::taskbar::open_notices_here),
            b'X' => ui(|| start_menu(bar_under_cursor(), 0.0)),
            b'I' => uri("ms-settings:"),
            b'P' => {
                let _ = std::process::Command::new("DisplaySwitch.exe").spawn();
            }
            b'K' => uri("ms-settings:connecteddevices"),
            // Win+1…9 – n-ty przycisk paska (jak w Windows: przełącza albo uruchamia przypięty).
            b'1'..=b'9' => {
                let n = (k - b'1') as usize;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(g) = group_at(bar_under_cursor(), n) {
                        click(&g);
                    }
                });
            }
            _ => {}
        }
    });
    // Samo Win otwiera Start PowerDesk na monitorze z kursorem.
    powerdesk_core::startmenu::hook_win_key(|| {
        let _ = slint::invoke_from_event_loop(|| open_start(bar_under_cursor()));
    });
    crate::start::preload();
    refresh();
    // Okno systemowe powstaje dopiero w pętli zdarzeń – ułożenie zaraz po pierwszym obiegu.
    slint::Timer::single_shot(Duration::from_millis(100), refresh);
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(1), refresh));
}

pub fn close() {
    close_thumbs();
    close_menu();
    close_popup();
    TIMER.with(|t| t.stop());
    let bars = BARS.with(|b| std::mem::take(&mut *b.borrow_mut()));
    for b in bars {
        if powerdesk_core::shellmode::active() {
            let (x, y) = b.origin;
            powerdesk_core::shellmode::clear_bar(x, y, x + b.mon.0, y + b.mon.1);
        }
        let _ = b.w.hide();
    }
}

/// Prawy przycisk na pustym miejscu paska – jak w Windows: narzędzia, ustawienia paska,
/// pokaż pulpit, zasilanie i zakończenie PowerDesk.
fn bar_menu(bar: usize, x: f32) {
    let mut items = vec![
        it("Menedżer zadań", "app", |_| {
            let _ = std::process::Command::new("taskmgr.exe").spawn();
        })
        .kbd("Ctrl+Shift+Esc"),
        it("Ustawienia paska zadań", "settings", |s| {
            crate::settings::open_section(s, crate::settings::SECTION_TASKBAR);
            crate::show_window(s);
        }),
        it("Pokaż pulpit", "desktop", |_| taskwin::show_desktop()).kbd("Win+D"),
        sep(),
    ];
    items.extend(crate::start::power_items());
    items.extend(quit_items());
    show_menu(bar, items, x);
}

/// Prawy przycisk na Start: PowerDesk i narzędzia systemowe.
fn start_menu(bar: usize, x: f32) {
    let run = |cmd: &'static str| {
        move |_: &Shared| {
            let _ = std::process::Command::new(cmd).spawn();
        }
    };
    let mut items = vec![
        it("Otwórz PowerDesk", "folder", |s| crate::show_window(s)),
        it("Ustawienia paska zadań", "settings", |s| {
            crate::settings::open_section(s, crate::settings::SECTION_TASKBAR);
            crate::show_window(s);
        }),
        sep(),
        it("Menedżer zadań", "app", run("taskmgr.exe")),
        it("Ustawienia Windows", "settings", |_| sysbar::open_settings("")),
        it("Sprawdź aktualizacje", "refresh", |_| crate::updates::check(true)),
        // Okno „Uruchom” wprost z shell32 – działa także bez Eksploratora.
        it("Uruchom…", "terminal", |_| powerdesk_core::shellmode::run_dialog()).kbd("Win+R"),
    ];
    items.extend(quit_items());
    show_menu(bar, items, x);
}

/// Koniec PowerDesk. Jako powłoka: restart (strażnik uruchamia go od nowa) albo przejście
/// na Eksplorator (wpis zdjęty – strażnik uruchamia `explorer.exe`); inaczej zwykłe wyjście.
fn quit_items() -> Vec<Item> {
    if !powerdesk_core::shellmode::active() {
        return vec![sep(), it("Zakończ PowerDesk (przywróć pasek Windows)", "close", |s| crate::quit(s)).danger()];
    }
    vec![
        sep(),
        it("Uruchom ponownie PowerDesk", "refresh", |s| crate::quit(s)),
        it("Przełącz na Eksplorator Windows", "close", |s| {
            if let Err(e) = powerdesk_core::shellmode::unregister() {
                crate::transfers::toast(s, format!("Nie udało się zdjąć wpisu powłoki: {e}"), crate::transfers::Kind::Error, None);
                return;
            }
            crate::quit(s)
        })
        .danger(),
    ]
}

/// Menu Start nad lewym końcem paska `bar`.
fn open_start(bar: usize) {
    close_thumbs();
    close_menu();
    close_popup();
    let Some(((x, y), _, _, scale)) = spot(bar) else { return };
    let Some((light, accent, alpha)) = BARS.with(|b| {
        b.borrow().get(bar).map(|b| {
            let t = b.w.global::<Theme>();
            (t.get_light(), t.get_accent_override(), t.get_popup_alpha())
        })
    }) else {
        return;
    };
    crate::start::toggle(x, y, scale, light, accent, alpha);
}

/// Dla okienek spoza paska (historia schowka): motyw, krycie, miejsce nad paskiem monitora
/// z kursorem (lewa krawędź, góra paska, szerokość monitora) i skala.
pub fn popup_place() -> Option<(bool, slint::Color, f32, (i32, i32, i32), f32)> {
    let bar = bar_under_cursor();
    BARS.with(|b| {
        b.borrow().get(bar).map(|b| {
            let t = b.w.global::<Theme>();
            (t.get_light(), t.get_accent_override(), t.get_popup_alpha(), (b.origin.0, b.pos.1, b.mon.0), b.w.window().scale_factor())
        })
    })
}

/// Pasek na monitorze, na którym jest kursor (pierwszy, gdy nie da się ustalić).
/// Monitor pod kursorem: lewy górny róg, rozmiar (piksele ekranu) i skala.
pub fn monitor_under_cursor() -> Option<((i32, i32), (i32, i32), f32)> {
    let i = bar_under_cursor();
    BARS.with(|b| b.borrow().get(i).map(|b| (b.origin, b.mon, b.w.window().scale_factor())))
}

fn bar_under_cursor() -> usize {
    let (cx, cy) = taskwin::cursor();
    BARS.with(|b| b.borrow().iter().position(|bar| cx >= bar.origin.0 && cx < bar.origin.0 + bar.mon.0 && cy >= bar.origin.1 && cy < bar.origin.1 + bar.mon.1).unwrap_or(0))
}

/// Przypina program do paska (z menu Start); już przypięty zostaje.
pub fn pin_program(exe: &str) {
    if !pinned(exe) {
        toggle_pin(exe);
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

/// Przypięty przeciągnięty na miejsce innego przypiętego (x – w pasie przycisków). Przypięte
/// stoją na początku paska w kolejności z ustawień, więc numer przycisku to numer na liście.
fn reorder(bar: usize, from: usize, x: f32, each: f32) {
    let Some(groups) = BARS.with(|b| b.borrow().get(bar).map(|b| b.groups.clone())) else { return };
    // Te same szerokości co w taskbar.slint: przypięty bez okien 44 px, reszta `each`, odstęp 4 px.
    let mut left = 0.0;
    let mut to = groups.len().saturating_sub(1);
    for (i, g) in groups.iter().enumerate() {
        let w = if g.wins.is_empty() { 44.0 } else { each };
        if x < left + w + 2.0 {
            to = i;
            break;
        }
        left += w + 4.0;
    }
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        let pins = &mut b.prefs.taskbar_pins;
        if from >= pins.len() || from == to {
            return;
        }
        let to = to.min(pins.len() - 1);
        let p = pins.remove(from);
        pins.insert(to, p);
        b.prefs.save();
    });
    refresh();
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
    close_thumbs();
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
            t.set_popup_alpha(src.get_popup_alpha());
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
    popup_look(hwnd);
    taskwin::activate(hwnd);
}

/// Wygląd okienka jak menu kontekstowe: zaokrąglone rogi, obwódka w kolorze linii motywu,
/// przezroczystość okienek z ustawień (tło okna łączy się z tym, co pod spodem).
pub fn popup_look(hwnd: isize) {
    let line = BARS.with(|b| b.borrow().first().map(|b| b.w.global::<Theme>().get_line()));
    let c = line.unwrap_or(slint::Color::from_rgb_u8(60, 60, 60));
    powerdesk_core::system::style_popup(hwnd, Some((c.red(), c.green(), c.blue())));
    powerdesk_core::system::see_through(hwnd);
}

/// Motyw okienka jak motyw paska (jasny/ciemny, akcent).
macro_rules! same_theme {
    ($bar:expr, $w:expr) => {
        BARS.with(|b| {
            if let Some(bar) = b.borrow().get($bar) {
                let (t, src) = ($w.global::<Theme>(), bar.w.global::<Theme>());
                t.set_light(src.get_light());
                t.set_accent_override(src.get_accent_override());
            t.set_popup_alpha(src.get_popup_alpha());
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

fn own_popups() -> Vec<isize> {
    let mut out = Vec::new();
    MENU.with(|m| out.extend(m.borrow().as_ref().map(|(w, _)| crate::window_hwnd(w.window()))));
    POPUP.with(|p| {
        out.extend(p.borrow().as_ref().map(|p| match p {
            Popup::Volume(w) => crate::window_hwnd(w.window()),
            Popup::Calendar(w) => crate::window_hwnd(w.window()),
            Popup::Network(w) => crate::window_hwnd(w.window()),
            Popup::Notices(w) => crate::window_hwnd(w.window()),
        }))
    });
    THUMBS.with(|t| out.extend(t.borrow().as_ref().map(|t| crate::window_hwnd(t.w.window()))));
    out.extend(crate::start::hwnd());
    out.extend(crate::cliphistory::hwnd());
    out
}

fn close_popup() {
    match POPUP.with(|p| p.borrow_mut().take()) {
        Some(Popup::Volume(w)) => {
            let _ = w.hide();
        }
        Some(Popup::Calendar(w)) => {
            let _ = w.hide();
        }
        Some(Popup::Network(w)) => {
            let _ = w.hide();
        }
        Some(Popup::Notices(w)) => {
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
    let px = (20.0 * w.window().scale_factor()) as u32;
    let apps = app_rows(px);
    w.on_app_volume(|pid, v| {
        sysbar::set_app_volume(pid as u32, v);
        apps_status();
    });
    w.on_app_mute(|pid| {
        let muted = sysbar::app_volumes().iter().any(|a| a.pid == pid as u32 && a.muted);
        sysbar::set_app_mute(pid as u32, !muted);
        apps_status();
    });
    // Wysokość: głośność ogólna, nagłówek „Aplikacje” i wiersze miksera, odnośnik.
    let n = apps.len() as f32;
    // Urządzenie (32 + odstęp), głośność ogólna, aplikacje, odnośnik.
    let height = 28.0 + 38.0 + 36.0 + if n > 0.0 { 6.0 + 24.0 + n * 50.0 } else { 0.0 } + 6.0 + 24.0;
    w.set_apps(ModelRc::new(VecModel::from(apps)));
    fill_outputs(&w);
    w.on_toggle_devices(|| {
        with_volume(|w| {
            w.set_devices_open(!w.get_devices_open());
            fill_outputs(w);
            relayout_volume(w);
        })
    });
    w.on_pick_output(|i| {
        let id = sysbar::outputs().get(i as usize).map(|o| o.id.clone());
        if let Some(id) = id {
            sysbar::set_output(&id);
        }
        with_volume(|w| {
            w.set_devices_open(false);
            fill_outputs(w);
            relayout_volume(w);
            // Głośność i mikser należą do nowego urządzenia.
            if let Some(v) = sysbar::volume() {
                w.set_volume(v.level);
                w.set_muted(v.muted);
            }
        });
        tray_status();
    });
    VOL_GEOM.with(|g| g.set(Some((spot, x + 16.0 - 160.0, height))));
    // Okienko 320 px wyśrodkowane nad ikoną (32 px).
    above_bar(w.window(), spot, 320.0, height, x + 16.0 - 160.0);
    close_on_blur(w.window());
    show_above(w.window());
    POPUP.with(|p| *p.borrow_mut() = Some(Popup::Volume(w)));
}

/// Kalendarz nad zegarem, dosunięty do prawej krawędzi monitora.
/// Przełącznik pulpitów wirtualnych na wszystkich paskach (z Eksploratorem; w trybie powłoki – ukryty).
fn vd_status() {
    let mut on = false;
    crate::with_state(|s| on = s.borrow().prefs.vd_button);
    let vd = if !on || powerdesk_core::shellmode::active() { None } else { powerdesk_core::vdesk::list() };
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            match &vd {
                Some((names, cur)) => {
                    bar.w.set_vd_show(true);
                    bar.w.set_vd_name(names.get(*cur).cloned().unwrap_or_default().into());
                    bar.w.set_vd_count(format!("{}/{}", cur + 1, names.len()).into());
                }
                None => bar.w.set_vd_show(false),
            }
        }
    });
}

/// Lista pulpitów: przejście, nowy pulpit, zamknięcie bieżącego.
fn vd_menu(bar: usize, x: f32) {
    let Some((names, cur)) = powerdesk_core::vdesk::list() else { return };
    let mut items: Vec<Item> = names
        .into_iter()
        .enumerate()
        .map(|(i, n)| {
            it(n, "desktop", move |_| {
                std::thread::spawn(move || powerdesk_core::vdesk::go(i));
                slint::Timer::single_shot(Duration::from_millis(500), vd_status);
            })
            .checked(i == cur)
        })
        .collect();
    items.extend([
        sep(),
        it("Nowy pulpit", "plus", |_| {
            powerdesk_core::vdesk::add();
            slint::Timer::single_shot(Duration::from_millis(500), vd_status);
        })
        .kbd("Win+Ctrl+D"),
        it("Zamknij ten pulpit", "close", |_| {
            powerdesk_core::vdesk::close_current();
            slint::Timer::single_shot(Duration::from_millis(500), vd_status);
        })
        .kbd("Win+Ctrl+F4"),
    ]);
    show_menu(bar, items, x);
}

/// Zrzut ekranu: `full` – cały ekran do Obrazy\Zrzuty ekranu i do schowka (z powiadomieniem),
/// inaczej wycinek w systemowym Wycinaniu.
fn screenshot(full: bool) {
    std::thread::spawn(move || {
        let res = if full { powerdesk_core::capture::full().map(Some) } else { powerdesk_core::capture::snip() };
        let _ = slint::invoke_from_event_loop(move || match res {
            Ok(Some(p)) => crate::notices::add("Zrzut ekranu", "Zapisano i skopiowano do schowka", &p.display().to_string()),
            Ok(None) => {}
            Err(e) => crate::notices::add("Zrzut ekranu", "Nie udało się", &e),
        });
    });
}

/// Licznik nieprzeczytanych na wszystkich paskach i odświeżona lista, jeśli otwarta.
pub fn notices_changed() {
    let n = crate::notices::unread();
    BARS.with(|b| b.borrow().iter().for_each(|bar| bar.w.set_notice_count(n)));
    POPUP.with(|p| {
        if let Some(Popup::Notices(w)) = &*p.borrow() {
            w.set_rows(slint::ModelRc::new(slint::VecModel::from(crate::notices::rows())));
        }
    });
}

fn open_notices(bar: usize, x: f32) {
    let was = POPUP.with(|p| matches!(*p.borrow(), Some(Popup::Notices(_))));
    close_menu();
    close_popup();
    if was {
        return;
    }
    let Some(spot) = spot(bar) else { return };
    let Ok(w) = NoticesWindow::new() else { return };
    same_theme!(bar, w);
    w.set_rows(slint::ModelRc::new(slint::VecModel::from(crate::notices::rows())));
    w.on_clear(crate::notices::clear);
    above_bar(w.window(), spot, 340.0, 460.0, x - 300.0);
    close_on_blur(w.window());
    show_above(w.window());
    POPUP.with(|p| *p.borrow_mut() = Some(Popup::Notices(w)));
    crate::notices::mark_read();
}

/// Historia powiadomień z dymka – na pasku monitora z kursorem, przy prawej krawędzi.
pub fn open_notices_here() {
    let bar = bar_under_cursor();
    let x = BARS.with(|b| b.borrow().get(bar).map(|b| b.mon.0 as f32 / b.w.window().scale_factor())).unwrap_or(0.0);
    open_notices(bar, x);
}

/// Dymek powiadomienia nad paskiem monitora z kursorem, przy prawej krawędzi.
pub fn place_toast(win: &slint::Window, width: f32, height: f32) {
    let bar = bar_under_cursor();
    if let Some(spot) = spot(bar) {
        let right = spot.2 .0 as f32 / spot.3;
        above_bar(win, spot, width, height, right - width - 8.0);
    }
}

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

/// Wiersze miksera: aplikacje grające przez domyślne wyjście, z ikonami programów.
fn app_rows(px: u32) -> Vec<AppVol> {
    sysbar::app_volumes()
        .into_iter()
        .map(|a| {
            let img = if a.exe.is_empty() { None } else { crate::thumbs::get(&a.exe, px, true) };
            AppVol { pid: a.pid as i32, name: a.name.into(), has_img: img.is_some(), img: img.unwrap_or_default(), level: a.level, muted: a.muted }
        })
        .collect()
}

fn with_volume(f: impl FnOnce(&VolumeWindow)) {
    POPUP.with(|p| {
        if let Some(Popup::Volume(w)) = &*p.borrow() {
            f(w);
        }
    });
}

fn fill_outputs(w: &VolumeWindow) {
    let outs = sysbar::outputs();
    let name = outs.iter().find(|o| o.default).map(|o| o.name.clone()).unwrap_or_else(|| "Brak urządzenia odtwarzającego".into());
    w.set_current_name(name.into());
    let rows: Vec<OutDev> = outs.into_iter().map(|o| OutDev { name: o.name.into(), current: o.default }).collect();
    w.set_outputs(ModelRc::new(VecModel::from(rows)));
}

/// Wysokość okienka z listą urządzeń albo bez – dół zostaje nad paskiem.
fn relayout_volume(w: &VolumeWindow) {
    let Some((spot, x, base)) = VOL_GEOM.with(|g| g.get()) else { return };
    let list = if w.get_devices_open() { w.get_outputs().row_count() as f32 * 34.0 + 6.0 } else { 0.0 };
    above_bar(w.window(), spot, 320.0, base + list, x);
}

/// Odświeża poziomy w otwartym mikserze (lista aplikacji zostaje – wysokość okienka stała).
fn apps_status() {
    POPUP.with(|p| {
        if let Some(Popup::Volume(w)) = &*p.borrow() {
            let now = sysbar::app_volumes();
            let rows = w.get_apps();
            for i in 0..rows.row_count() {
                let Some(mut r) = rows.row_data(i) else { continue };
                if let Some(a) = now.iter().find(|a| a.pid as i32 == r.pid) {
                    if r.level != a.level || r.muted != a.muted {
                        r.level = a.level;
                        r.muted = a.muted;
                        rows.set_row_data(i, r);
                    }
                }
            }
        }
    });
}

/// Okienko sieci nad ikoną: kabel albo lista Wi-Fi z łączeniem; przy otwarciu skanuje.
fn open_network(bar: usize, x: f32) {
    let was = POPUP.with(|p| matches!(*p.borrow(), Some(Popup::Network(_))));
    close_menu();
    close_popup();
    if was {
        return;
    }
    let Some(spot) = spot(bar) else { return };
    let Ok(w) = NetworkWindow::new() else { return };
    same_theme!(bar, w);
    fill_network(&w);
    w.on_rescan(rescan);
    w.on_connect(|ssid, pass| {
        let pass = pass.to_string();
        let res = wifi::connect(&ssid, (!pass.is_empty()).then_some(pass.as_str()));
        with_network(|w| {
            match &res {
                Ok(()) => w.set_note("Łączenie…".into()),
                Err(e) => w.set_note(e.clone().into()),
            }
            if res.is_ok() {
                w.set_pass("".into());
            }
        });
        if res.is_ok() {
            // Wynik połączenia przychodzi po chwili – dwa odczyty, potem sieć na pasku.
            for ms in [2500, 6000] {
                slint::Timer::single_shot(Duration::from_millis(ms), || {
                    with_network(|w| {
                        fill_network(w);
                        if net_connected(w) {
                            w.set_note("".into());
                            w.set_open(-1);
                        }
                    });
                    NET.with(|n| *n.borrow_mut() = None);
                    tray_status();
                });
            }
        }
    });
    w.on_disconnect(|| {
        wifi::disconnect();
        with_network(|w| w.set_open(-1));
        slint::Timer::single_shot(Duration::from_millis(1500), || {
            with_network(fill_network);
            NET.with(|n| *n.borrow_mut() = None);
            tray_status();
        });
    });
    w.on_settings(|| {
        slint::Timer::single_shot(Duration::ZERO, close_popup);
        sysbar::open_settings("network-status");
    });
    // Okienko 340 px wyśrodkowane nad ikoną (32 px).
    above_bar(w.window(), spot, 340.0, 440.0, x + 16.0 - 170.0);
    close_on_blur(w.window());
    show_above(w.window());
    POPUP.with(|p| *p.borrow_mut() = Some(Popup::Network(w)));
    rescan();
}

fn with_network(f: impl FnOnce(&NetworkWindow)) {
    POPUP.with(|p| {
        if let Some(Popup::Network(w)) = &*p.borrow() {
            f(w);
        }
    });
}

fn net_connected(w: &NetworkWindow) -> bool {
    let open = w.get_open();
    open >= 0 && w.get_nets().row_data(open as usize).is_some_and(|n| n.connected)
}

/// Skanowanie trwa kilka sekund – lista odświeża się po nim.
fn rescan() {
    wifi::scan();
    with_network(|w| w.set_scanning(true));
    slint::Timer::single_shot(Duration::from_secs(4), || {
        with_network(|w| {
            fill_network(w);
            w.set_scanning(false);
        })
    });
}

/// Lista sieci; rozwinięta zostaje przy tej samej nazwie, nawet gdy zmieni się kolejność.
fn fill_network(w: &NetworkWindow) {
    let net = sysbar::network();
    w.set_wired(if net.kind == NetKind::Wired { net.name.into() } else { "".into() });
    let nets = wifi::networks();
    w.set_has_wifi(nets.is_some());
    let open_ssid = w.get_nets().row_data(w.get_open().max(0) as usize).filter(|_| w.get_open() >= 0).map(|n| n.ssid);
    let rows: Vec<NetRow> = nets
        .unwrap_or_default()
        .into_iter()
        .map(|n| NetRow { ssid: n.ssid.into(), signal: n.signal as i32, secured: n.secured, connected: n.connected, known: n.known })
        .collect();
    let open = open_ssid.and_then(|s| rows.iter().position(|r| r.ssid == s)).map_or(-1, |i| i as i32);
    w.set_nets(ModelRc::new(VecModel::from(rows)));
    w.set_open(open);
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
    apps_status();
}

/// Otwarte podglądy okien: okienko, grupa, uchwyty podglądów DWM.
struct Thumbs {
    w: ThumbWindow,
    bar: usize,
    index: usize,
    hwnds: Vec<isize>,
    ids: Vec<isize>,
}

/// Wymiary karty podglądu – te same co w ThumbWindow (taskbar.slint).
const CARD_W: f32 = 220.0;
const CARD_H: f32 = 160.0;
const GAP: f32 = 8.0;

/// Kursor nad przyciskiem: gdy podglądy są już otwarte – od razu przełączamy grupę,
/// inaczej po chwili (jak w Windows, żeby przejechanie po pasku ich nie otwierało).
fn hover(bar: usize, i: usize, x: f32) {
    HOVER.with(|h| h.set(Some((bar, i))));
    let open = THUMBS.with(|t| t.borrow().as_ref().map(|t| (t.bar, t.index)));
    if open == Some((bar, i)) {
        return;
    }
    let delay = if open.is_some() { 0 } else { 450 };
    slint::Timer::single_shot(Duration::from_millis(delay), move || {
        // Kursor dalej nad tym samym przyciskiem i nie ma menu.
        if HOVER.with(|h| h.get()) == Some((bar, i)) && MENU.with(|m| m.borrow().is_none()) {
            show_thumbs(bar, i, x);
        }
    });
}

fn unhover() {
    HOVER.with(|h| h.set(None));
    later_close_thumbs();
}

/// Zamknięcie po chwili, jeśli kursor nie wrócił na przycisk ani nie wszedł w podglądy.
fn later_close_thumbs() {
    slint::Timer::single_shot(Duration::from_millis(300), || {
        if HOVER.with(|h| h.get()).is_none() && !THUMB_HOVER.with(|h| h.get()) {
            close_thumbs();
        }
    });
}

fn show_thumbs(bar: usize, i: usize, x: f32) {
    close_thumbs();
    let Some(g) = group_at(bar, i) else { return };
    if g.wins.is_empty() {
        return;
    }
    let Some(spot) = spot(bar) else { return };
    let Ok(w) = ThumbWindow::new() else { return };
    same_theme!(bar, w);
    let fg = taskwin::foreground();
    let px = (16.0 * spot.3) as u32;
    let img = if g.exe.is_empty() { None } else { crate::thumbs::get(&g.exe, px, true) };
    let cards: Vec<ThumbCard> = g
        .wins
        .iter()
        .map(|win| ThumbCard { title: win.title.clone().into(), has_img: img.is_some(), img: img.clone().unwrap_or_default(), active: win.hwnd == fg, minimized: win.minimized })
        .collect();
    let n = cards.len() as f32;
    w.set_cards(ModelRc::new(VecModel::from(cards)));
    let (width, height) = (n * (CARD_W + GAP) + GAP, CARD_H + 2.0 * GAP);
    above_bar(w.window(), spot, width, height, x - width / 2.0);
    w.on_activate(|i| {
        let h = THUMBS.with(|t| t.borrow().as_ref().and_then(|t| t.hwnds.get(i as usize).copied()));
        slint::Timer::single_shot(Duration::ZERO, close_thumbs);
        if let Some(h) = h {
            taskwin::activate(h);
            slint::Timer::single_shot(Duration::from_millis(150), refresh);
        }
    });
    w.on_close_win(|i| {
        let h = THUMBS.with(|t| t.borrow().as_ref().and_then(|t| t.hwnds.get(i as usize).copied()));
        slint::Timer::single_shot(Duration::ZERO, close_thumbs);
        if let Some(h) = h {
            taskwin::close(h);
            slint::Timer::single_shot(Duration::from_millis(300), refresh);
        }
    });
    // Wejście i wyjście kursora – podglądy zostają, póki kursor jest nad nimi.
    w.window().on_winit_window_event(|_, ev| {
        use slint::winit_030::winit::event::WindowEvent;
        match ev {
            WindowEvent::CursorEntered { .. } => THUMB_HOVER.with(|h| h.set(true)),
            WindowEvent::CursorLeft { .. } => {
                THUMB_HOVER.with(|h| h.set(false));
                later_close_thumbs();
            }
            _ => {}
        }
        slint::winit_030::EventResult::Propagate
    });
    // Bez fokusu – okno pod spodem zostaje aktywne, a podglądy nie trafiają na listę okien.
    let _ = w.show();
    let dest = crate::window_hwnd(w.window());
    data::hide_from_taskbar(dest);
    popup_look(dest);
    if fg != 0 {
        taskwin::activate(fg);
    }
    let scale = spot.3;
    let mut ids = Vec::new();
    for (k, win) in g.wins.iter().enumerate() {
        let Some((id, (sw, sh))) = taskwin::thumb_register(dest, win.hwnd) else { continue };
        // Podgląd wpisany w miejsce karty z zachowaniem proporcji, wyśrodkowany.
        let (bx, by) = (GAP + k as f32 * (CARD_W + GAP) + 8.0, GAP + 34.0);
        let (bw, bh) = (CARD_W - 16.0, CARD_H - 42.0);
        let ratio = if sw > 0 && sh > 0 { sw as f32 / sh as f32 } else { bw / bh };
        let (tw, th) = if ratio > bw / bh { (bw, bw / ratio) } else { (bh * ratio, bh) };
        let (tx, ty) = (bx + (bw - tw) / 2.0, by + (bh - th) / 2.0);
        if !win.minimized {
            taskwin::thumb_place(id, (tx * scale) as i32, (ty * scale) as i32, (tw * scale) as i32, (th * scale) as i32);
        }
        ids.push(id);
    }
    THUMB_HOVER.with(|h| h.set(false));
    THUMBS.with(|t| *t.borrow_mut() = Some(Thumbs { w, bar, index: i, hwnds: g.wins.iter().map(|w| w.hwnd).collect(), ids }));
}

fn close_thumbs() {
    if let Some(t) = THUMBS.with(|t| t.borrow_mut().take()) {
        for id in &t.ids {
            taskwin::thumb_unregister(*id);
        }
        let _ = t.w.hide();
    }
    THUMB_HOVER.with(|h| h.set(false));
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
        // Tło paska może być półprzezroczyste (krycie z ustawień) – DWM pokazuje, co pod spodem.
        powerdesk_core::system::see_through(hwnd);
        powerdesk_core::system::no_frame_border(hwnd);
        let (x, y) = bar.origin;
        // Bez Eksploratora nikt nie zostawi miejsca na pasek – obszar roboczy ustawiamy sami.
        if powerdesk_core::shellmode::active() {
            powerdesk_core::shellmode::set_bar(x, y, x + bar.mon.0, y + bar.mon.1, h);
        }
        if bar.origin == (0, 0) {
            // Zasobnik stoi przy zegarze – przybliżone miejsce dla programów pytających o ikonę.
            let w = (240.0 * scale) as i32;
            powerdesk_core::trayhost::set_anchor(x + bar.mon.0 - w, bar.pos.1, w, h);
        }
        bar.placed = true;
    }
    // Okno na cały monitor (gra, film) – pasek pod nim; inaczej na wierzchu.
    if taskwin::covers(taskwin::foreground(), bar.origin.0, bar.origin.1, bar.mon.0, bar.mon.1) {
        taskwin::place_normal(hwnd);
    } else {
        taskwin::place_topmost(hwnd, bar.pos.0, bar.pos.1, bar.mon.0, h);
    }
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
    // Pasek Windows potrafi wrócić (Start, restart Eksploratora) – chowamy go co obieg.
    powerdesk_core::shellswap::hide_taskbar();
    // Własne okienka (menu, głośność, kalendarz) nie są oknami programów.
    let own = own_popups();
    let all: Vec<Win> = taskwin::list().into_iter().filter(|w| !own.contains(&w.hwnd)).collect();
    let fg = taskwin::foreground();
    let now = Local::now();
    let (time, date) = (now.format("%H:%M").to_string(), now.format("%d.%m.%Y").to_string());
    let mut pins = Vec::new();
    let (mut opacity, mut clear) = (100.0, false);
    crate::with_state(|s| {
        let b = s.borrow();
        pins = b.prefs.taskbar_pins.clone();
        opacity = b.prefs.taskbar_opacity;
        clear = b.prefs.taskbar_clear_desktop;
    });
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
            // Na pulpicie (żadne okno tego monitora nie jest otwarte) – całkiem przezroczysty.
            let desktop = clear && bar.wins.iter().all(|w| w.minimized);
            bar.w.set_bg_alpha(if desktop { 0.0 } else { (opacity / 100.0f32).clamp(0.0, 1.0) });
            bar.w.set_time(time.clone().into());
            bar.w.set_date(date.clone().into());
        }
    });
    tray_status();
    vd_status();
}

/// Ikony programów w zasobniku na wszystkich paskach.
fn tray_apps() {
    let apps: Vec<TrayApp> = powerdesk_core::trayhost::icons()
        .into_iter()
        .map(|i| TrayApp {
            key: i.key().into(),
            tip: i.tip.clone().into(),
            img: i
                .image
                .as_ref()
                .map(|(w, h, px)| slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(px, *w, *h)))
                .unwrap_or_default(),
        })
        .collect();
    // Ikony programów tylko na pasku głównego monitora (jak w Windows).
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            let here = if bar.origin == (0, 0) { apps.clone() } else { Vec::new() };
            bar.w.set_tray_apps(ModelRc::new(VecModel::from(here)));
        }
    });
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
