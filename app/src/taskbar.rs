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
use crate::{NoticesWindow, OutDev, AppVol, TrayApp, CalDay, CalendarWindow, NetRow, NetworkWindow, Shared, TaskButton, TaskbarWindow, Theme, ThumbCard, ThumbWindow, TipWindow, TrayOverflowWindow, VolumeWindow};

/// Wysokość paska w pikselach logicznych – z ustawień (32–72, domyślnie 48).
pub fn height() -> f32 {
    BAR_H.with(|h| h.get())
}

/// Ikona programu na przycisku i szerokość przycisku z samą ikoną – jak w taskbar.slint.
fn icon_size() -> f32 {
    let ih = (height() - 8.0).clamp(24.0, 64.0);
    ICON.with(|i| i.get()).min(ih - 6.0)
}

fn pin_w() -> f32 {
    (icon_size() + 18.0).max(44.0)
}

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
    /// Rozmycie pod paskiem włączone (zmieniamy tylko przy zmianie stanu).
    blurred: Option<bool>,
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
    /// Ostatnie aktywne okno programu – klik w pasek zabiera mu fokus, zanim dojdzie do `click`.
    static LAST_APP: Cell<isize> = const { Cell::new(0) };
    static TIMER: slint::Timer = slint::Timer::default();
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
    /// Po kliknięciu przycisku podglądy się nie otwierają, dopóki kursor nie zjedzie z paska.
    static NO_THUMBS: Cell<bool> = const { Cell::new(false) };
    /// Odświeżanie obrazów okien w otwartych podglądach i numer otwarcia (stare wyniki odpadają).
    static SHOT_TIMER: slint::Timer = slint::Timer::default();
    static THUMB_GEN: Cell<u64> = const { Cell::new(0) };
    /// Okienko głośności: miejsce nad paskiem, x i wysokość bez listy urządzeń.
    static VOL_GEOM: Cell<Option<(Spot, f32, f32)>> = const { Cell::new(None) };
    /// Dymek podpowiedzi (jeden na proces) i numer ostatniej zmiany podpowiedzi.
    static TIP: RefCell<Option<TipWindow>> = const { RefCell::new(None) };
    static TIP_SEQ: Cell<u64> = const { Cell::new(0) };
    /// Ostatnio zamknięte okienko: rodzaj i chwila zamknięcia.
    static CLOSED: Cell<Option<(u8, std::time::Instant)>> = const { Cell::new(None) };
    /// Trwa przeciąganie ikony zasobnika – listy ikon nie zmieniamy, dopóki jej nie puszczono.
    static TRAY_DRAG: Cell<bool> = const { Cell::new(false) };
    /// Wysokość paska i ikon programów (piksele logiczne) z ustawień.
    static BAR_H: Cell<f32> = const { Cell::new(48.0) };
    static ICON: Cell<f32> = const { Cell::new(28.0) };
}

enum Popup {
    Volume(VolumeWindow),
    Calendar(CalendarWindow),
    Network(NetworkWindow),
    Notices(NoticesWindow),
    /// Ukryte ikony zasobnika: okienko, pasek i x strzałki (do ponownego ułożenia).
    Tray(TrayOverflowWindow, usize, f32),
}

pub fn is_open() -> bool {
    BARS.with(|b| !b.borrow().is_empty())
}

pub fn open(_s: &Shared) {
    if is_open() {
        return;
    }
    // Okna wołające o uwagę – przycisk miga od razu, nie dopiero przy następnym obiegu zegara.
    powerdesk_core::shellhook::start(|| {
        let _ = slint::invoke_from_event_loop(refresh);
    });
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
        w.on_start(move || open_start(n, false));
        w.on_start_context(move |x| start_menu(n, x));
        w.on_bar_context(move |x| bar_menu(n, x));
        w.on_show_desktop(taskwin::show_desktop);
        w.on_peek(move |on| {
            let desk = crate::desktop::windows().get(n).or(crate::desktop::windows().first()).map(|d| crate::window_hwnd(d.window())).unwrap_or(0);
            let bar = BARS.with(|b| b.borrow().get(n).map(|b| crate::window_hwnd(b.w.window()))).unwrap_or(0);
            taskwin::peek(on, desk, bar);
        });
        w.on_activate(move |i| {
            NO_THUMBS.with(|c| c.set(true));
            close_thumbs();
            if let Some(g) = group_at(n, i as usize) {
                click(&g);
                // Stan (aktywne / zminimalizowane) od razu, bez czekania na zegar.
                slint::Timer::single_shot(Duration::from_millis(150), refresh);
            }
        });
        w.on_reorder(move |i, x, each| reorder(n, i as usize, x, each));
        w.on_context(move |i, x, y| {
            NO_THUMBS.with(|c| c.set(true));
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
        w.on_usb_open(move |x| usb_menu(n, x));
        w.on_calendar_open(move |x| open_calendar(n, x));
        w.on_notices_open(move |x| open_notices(n, x));
        w.on_vd_menu(move |x| vd_menu(n, x));
        w.on_vd_wheel(|d| {
            std::thread::spawn(move || powerdesk_core::vdesk::step(d));
            slint::Timer::single_shot(Duration::from_millis(400), vd_status);
        });
        w.set_notice_count(crate::notices::unread());
        let ww = w.as_weak();
        w.on_tip(move |t, x, y| {
            if let Some(w) = ww.upgrade() {
                tip(w.window(), &t, x, y);
            }
        });
        w.on_tray_app_click(|key, b| tray_click(&key, b));
        w.on_tray_app_drop(|id, dx, to_arrow| {
            TRAY_DRAG.with(|d| d.set(false));
            tray_drop_on_bar(&id, dx, to_arrow)
        });
        w.on_tray_drag_state(|on| {
            let was = TRAY_DRAG.with(|d| d.replace(on));
            // Koniec przeciągania bez upuszczenia (np. Esc) – zaległe zmiany ikon.
            if was && !on {
                tray_apps();
            }
        });
        w.on_tray_overflow(move |x| open_tray_overflow(n, x));
        w.set_main_bar((m.x, m.y) == (0, 0));
        let _ = w.show();
        let _ = powerdesk_core::dnd::register(crate::window_hwnd(w.window()), Box::new(BarDrop { bar: n }));
        bars.push(Bar { w, origin: (m.x, m.y), mon: (m.width, m.height), pos: (m.x, m.y + m.height), placed: false, blurred: None, wins: Vec::new(), groups: Vec::new(), pins: Vec::new(), fg: 0 });
    }
    BARS.with(|b| *b.borrow_mut() = bars);
    restyle();
    // Przy starcie programu pętla zdarzeń jeszcze nie ruszyła i okno systemowe dymka nie istnieje –
    // nie dałoby się go schować i zostałby pusty prostokąt nad wszystkimi oknami.
    slint::Timer::single_shot(Duration::from_millis(200), ensure_tip);
    // Zasobnik z ikonami innych programów (własny Shell_TrayWnd, przed paskiem Eksploratora).
    // `start` działa raz na proces – po przebudowie pasków (zmiana monitorów) tylko odświeżenie.
    powerdesk_core::trayhost::on_balloon(|title, text| {
        let _ = slint::invoke_from_event_loop(move || crate::notices::add("Program w zasobniku", &title, &text));
    });
    powerdesk_core::trayhost::start(|| {
        let _ = slint::invoke_from_event_loop(tray_apps);
    });
    tray_apps();
    // Pierwsze uruchomienie: programy dodają ikony przez kilka sekund po starcie – potem
    // pierwsze 6 trafia na pasek, reszta pod strzałkę (później decyduje użytkownik).
    slint::Timer::single_shot(Duration::from_secs(10), tray_first_run);
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
            // Win+S – samo wyszukiwanie; otwarty Start się zamyka (i odwrotnie).
            b'S' => ui(|| open_start(bar_under_cursor(), true)),
            // Win+A – szybkie ustawienia (sieć), Win+N – powiadomienia, Win+X – menu Startu.
            b'A' => ui(|| {
                let bar = bar_under_cursor();
                let x = BARS.with(|b| b.borrow().get(bar).map(|b| b.mon.0 as f32 / b.w.window().scale_factor())).unwrap_or(0.0);
                open_network(bar, x - 300.0);
            }),
            b'N' => ui(crate::taskbar::open_notices_here),
            b'X' => ui(|| {
                crate::start::close();
                start_state();
                start_menu(bar_under_cursor(), 0.0)
            }),
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
        let _ = slint::invoke_from_event_loop(|| open_start(bar_under_cursor(), false));
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
        it("Same ikony (bez nazw okien)", "grid", |_| edit_bar(|p| p.taskbar_labels = !p.taskbar_labels)).checked(!bar_pref(|p| p.taskbar_labels)),
        it("Przyciski na środku", "grid", |_| edit_bar(|p| p.taskbar_center = !p.taskbar_center)).checked(bar_pref(|p| p.taskbar_center)),
        it("Wszystkie ikony zasobnika na pasku", "grid", |_| edit_tray(|p| p.tray_all = !p.tray_all)).checked(bar_pref(|p| p.tray_all)),
        sep(),
    ];
    items.extend(crate::start::power_items());
    show_menu(bar, items, x);
}

/// Prawy przycisk na Start: PowerDesk i narzędzia systemowe.
fn start_menu(bar: usize, x: f32) {
    let run = |cmd: &'static str| {
        move |_: &Shared| {
            let _ = std::process::Command::new(cmd).spawn();
        }
    };
    let items = vec![
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
        it("Wyszukaj", "search", move |_| open_start(bar, true)).kbd("Win+S"),
    ];
    show_menu(bar, items, x);
}

/// Wartość z ustawień (do znaczników w menu paska).
fn bar_pref<T>(f: impl FnOnce(&crate::model::Prefs) -> T) -> T
where
    T: Default,
{
    let mut out = T::default();
    crate::with_state(|s| out = f(&s.borrow().prefs));
    out
}

/// Zmiana ustawień paska z jego menu – zapis i nowy wygląd na wszystkich paskach.
fn edit_bar(f: impl FnOnce(&mut crate::model::Prefs)) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        f(&mut b.prefs);
        b.prefs.save();
    });
    restyle();
}

/// Obraz przycisku Start z pliku: obraz, liczba klatek (jedna pod drugą) i proporcje klatki.
/// 0 klatek w ustawieniach – z proporcji: kwadratowe klatki w pionowym pasku (np. 3 dla 54×162).
fn start_image(path: &str, frames: u8) -> Option<(slint::Image, i32, f32)> {
    if path.is_empty() {
        return None;
    }
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = (img.width().max(1), img.height().max(1));
    let n = if frames > 0 { frames as u32 } else { ((h as f32 / w as f32).round() as u32).clamp(1, 4) };
    let fh = (h / n).max(1);
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(img.as_raw(), w, h);
    Some((slint::Image::from_rgba8(buf), n as i32, w as f32 / fh as f32))
}

/// Wygląd pasków z ustawień: wysokość, same ikony, środek, data, przycisk Start.
/// Zmiana wysokości układa paski od nowa (położenie i obszar roboczy).
pub fn restyle() {
    let mut p = None;
    crate::with_state(|s| {
        let b = s.borrow();
        p = Some((b.prefs.taskbar_height, b.prefs.taskbar_labels, b.prefs.taskbar_center, b.prefs.taskbar_date, b.prefs.start_image.clone(), b.prefs.start_frames, b.prefs.taskbar_icon));
    });
    let Some((h, labels, center, date, img, frames, icon)) = p else { return };
    let resized = (BAR_H.with(|c| c.replace(h)) - h).abs() > 0.5;
    // Inny rozmiar ikon – obrazy programów pobierane od nowa w nowej wielkości.
    let reicon = (ICON.with(|c| c.replace(icon)) - icon).abs() > 0.5;
    let start = start_image(&img, frames);
    BARS.with(|b| {
        for bar in b.borrow_mut().iter_mut() {
            bar.w.set_labels(labels);
            bar.w.set_centered(center);
            bar.w.set_show_date(date);
            bar.w.set_bar_h(h);
            bar.w.set_app_icon(icon);
            // Przyciski od nowa (inny rozmiar ikon, grupowanie, nazwy) przy najbliższym odświeżeniu.
            let _ = reicon;
            bar.wins.clear();
            match &start {
                Some((i, n, aspect)) => {
                    bar.w.set_start_img(i.clone());
                    bar.w.set_start_frames(*n);
                    bar.w.set_start_aspect(*aspect);
                    bar.w.set_has_start_img(true);
                }
                None => bar.w.set_has_start_img(false),
            }
            if resized {
                bar.placed = false;
            }
        }
    });
    tray_apps();
    refresh();
}

/// Przycisk Start podświetlony, gdy Start albo wyszukiwanie jest otwarte.
pub fn start_state() {
    let open = crate::start::is_open();
    BARS.with(|b| b.borrow().iter().for_each(|bar| bar.w.set_start_open(open)));
}

/// Menu Start nad lewym końcem paska `bar`; `search` – samo wyszukiwanie (Win+S).
fn open_start(bar: usize, search: bool) {
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
    crate::start::toggle(x, y, scale, light, accent, alpha, search);
    start_state();
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

/// Przycisk paska pod punktem `x` okna paska (piksele okna) – ten sam układ co w taskbar.slint.
fn button_at(bar: usize, x: i32) -> Option<usize> {
    BARS.with(|b| {
        let b = b.borrow();
        let bar = b.get(bar)?;
        let scale = bar.w.window().scale_factor();
        let (mut left, x, each) = (bar.w.get_lane_x(), x as f32 / scale, bar.w.get_lane_each());
        for (i, g) in bar.groups.iter().enumerate() {
            let w = if g.wins.is_empty() { pin_w() } else { each };
            if x >= left && x < left + w {
                return Some(i);
            }
            left += w + 4.0;
        }
        None
    })
}

thread_local! {
    /// Przeciąganie plików nad przyciskiem: pasek, przycisk, od kiedy i czy okno już wysunięte.
    static DRAG_OVER: Cell<Option<(usize, usize, std::time::Instant, bool)>> = const { Cell::new(None) };
}

/// Pliki przeciągane na pasek – jak w Windows: przytrzymanie nad programem wysuwa jego okno,
/// upuszczenie na program bez okien (przypięty) otwiera w nim pliki.
struct BarDrop {
    bar: usize,
}

impl powerdesk_core::dnd::DropHandler for BarDrop {
    fn over(&self, x: i32, _y: i32, _keys: powerdesk_core::dnd::Keys, _paths: &[String]) -> powerdesk_core::dnd::Effect {
        use powerdesk_core::dnd::Effect;
        let Some(i) = button_at(self.bar, x) else {
            DRAG_OVER.with(|d| d.set(None));
            return Effect::None;
        };
        let now = std::time::Instant::now();
        let (since, shown) = match DRAG_OVER.with(|d| d.get()) {
            Some((b, j, t, s)) if b == self.bar && j == i => (t, s),
            _ => (now, false),
        };
        let g = group_at(self.bar, i);
        let mut shown = shown;
        if !shown && now.duration_since(since) >= Duration::from_millis(600) {
            if let Some(top) = g.as_ref().and_then(|g| g.wins.first()) {
                taskwin::activate(top.hwnd);
            }
            shown = true;
        }
        DRAG_OVER.with(|d| d.set(Some((self.bar, i, since, shown))));
        match g {
            Some(g) if g.wins.is_empty() && !g.exe.is_empty() => Effect::Copy,
            _ => Effect::None,
        }
    }

    fn leave(&self) {
        DRAG_OVER.with(|d| d.set(None));
    }

    fn dropped(&self, x: i32, _y: i32, _keys: powerdesk_core::dnd::Keys, paths: &[String]) -> powerdesk_core::dnd::Effect {
        use powerdesk_core::dnd::Effect;
        DRAG_OVER.with(|d| d.set(None));
        let Some(g) = button_at(self.bar, x).and_then(|i| group_at(self.bar, i)) else { return Effect::None };
        if !g.wins.is_empty() || g.exe.is_empty() {
            return Effect::None;
        }
        let _ = std::process::Command::new(&g.exe).args(paths).spawn();
        Effect::Copy
    }
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
        let w = if g.wins.is_empty() { pin_w() } else { each };
        if x < left + w + 2.0 {
            to = i;
            break;
        }
        left += w + 4.0;
    }
    if from == to {
        return;
    }
    let mut npins = 0;
    crate::with_state(|s| npins = s.borrow().prefs.taskbar_pins.len());
    if from < npins {
        // Przypięty – nowe miejsce wśród przypiętych (zapamiętane w ustawieniach).
        crate::with_state(|s| {
            let mut b = s.borrow_mut();
            let pins = &mut b.prefs.taskbar_pins;
            let to = to.min(pins.len() - 1);
            let p = pins.remove(from);
            pins.insert(to, p);
            b.prefs.save();
        });
    } else {
        // Program bez przypięcia – jego okna przechodzą w kolejności paska przed (albo za)
        // okna programu, na który go upuszczono; wśród przypiętych staje na początku reszty.
        let to = to.max(npins);
        let (Some(moved), Some(target)) = (groups.get(from).cloned(), groups.get(to).cloned()) else { return };
        BARS.with(|b| {
            let mut b = b.borrow_mut();
            let Some(bar) = b.get_mut(bar) else { return };
            let mine: Vec<isize> = moved.wins.iter().map(|w| w.hwnd).collect();
            let picked: Vec<Win> = bar.wins.iter().filter(|w| mine.contains(&w.hwnd)).cloned().collect();
            bar.wins.retain(|w| !mine.contains(&w.hwnd));
            let anchor = if to > from { target.wins.last() } else { target.wins.first() };
            let at = anchor.and_then(|a| bar.wins.iter().position(|w| w.hwnd == a.hwnd)).map_or(bar.wins.len(), |i| if to > from { i + 1 } else { i });
            for (k, w) in picked.into_iter().enumerate() {
                bar.wins.insert((at + k).min(bar.wins.len()), w);
            }
            // Przyciski od nowa przy najbliższym odświeżeniu.
            bar.fg = -1;
        });
    }
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
/// Okno programu, które jest „aktywne” z punktu widzenia paska: bieżące, a gdy fokus ma sam
/// pasek (albo inne okienko PowerDesk) – ostatnie aktywne okno programu.
fn app_foreground() -> isize {
    let fg = taskwin::foreground();
    let known = BARS.with(|b| b.borrow().iter().any(|b| b.groups.iter().any(|g| g.wins.iter().any(|w| w.hwnd == fg))));
    if known {
        fg
    } else {
        LAST_APP.with(|l| l.get())
    }
}

fn click_group(group: &[Win]) {
    group.iter().for_each(|w| powerdesk_core::shellhook::clear(w.hwnd));
    match group {
        [] => {}
        // Jak w Windows: klik w aktywne okno je minimalizuje (widać to, co było pod nim),
        // w inne – przywraca i wysuwa na wierzch.
        [one] => {
            if one.hwnd == app_foreground() && !taskwin::is_minimized(one.hwnd) {
                taskwin::minimize(one.hwnd);
                LAST_APP.with(|l| l.set(0));
            } else {
                taskwin::activate(one.hwnd);
            }
        }
        _ => {
            let fg = app_foreground();
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
/// Lista szybkiego dostępu: ostatnie pliki programu (jak na pasku Windows), otwierane tym programem.
fn jump_items(hwnd: Option<isize>, exe: &str) -> Vec<Item> {
    if exe.is_empty() {
        return Vec::new();
    }
    let mut items = Vec::new();
    // Zadania i własne kategorie programu (np. „Nowe okno InPrivate”), w kolejności z jego listy.
    let mut cat: Option<String> = None;
    for t in powerdesk_core::jumplist::tasks(exe) {
        if cat.as_deref() != Some(t.category.as_str()) {
            items.push(crate::menus::header(if t.category.is_empty() { "Zadania".to_string() } else { t.category.clone() }));
            cat = Some(t.category.clone());
        }
        let (path, args) = (t.path.clone(), t.args.clone());
        items.push(it(t.title, "open", move |_| {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new(&path).raw_arg(&args).spawn();
        }));
    }
    let files = powerdesk_core::jumplist::recent(hwnd, exe, 10);
    if !files.is_empty() {
        items.push(crate::menus::header("Ostatnie"));
    }
    for f in files {
        let name = std::path::Path::new(&f).file_name().map_or_else(|| f.clone(), |n| n.to_string_lossy().to_string());
        let icon = if std::path::Path::new(&f).is_dir() { "folder" } else { "file" };
        let exe = exe.to_string();
        items.push(it(name, icon, move |_| {
            let _ = std::process::Command::new(&exe).arg(&f).spawn();
        }));
    }
    if !items.is_empty() {
        items.push(sep());
    }
    items
}

fn context(bar: usize, i: usize, x: f32, _y: f32) {
    let Some(g) = group_at(bar, i) else { return };
    let mut items = jump_items(g.wins.first().map(|w| w.hwnd), &g.exe);
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
    close_popup();
    let Some((pos, _, _, scale)) = spot(bar) else { return };
    // Tuż nad paskiem: okienko menu samo przesuwa się w górę, gdy nie mieści się nad nim.
    menus::show_at(items, pos.0 + (x * scale) as i32, pos.1 - (6.0 * scale) as i32);
}

/// Pasek po wyborze z menu (przypięcie, odpięcie…) – stan od razu, nie po sekundzie.
pub fn refresh_soon() {
    slint::Timer::single_shot(Duration::from_millis(300), refresh);
}

/// Monitor z punktem (piksele ekranu): lewy górny róg, rozmiar i skala paska na nim.
pub fn monitor_at(x: i32, y: i32) -> ((i32, i32), (i32, i32), f32) {
    BARS.with(|b| {
        let b = b.borrow();
        let bar = b.iter().find(|bar| x >= bar.origin.0 && x < bar.origin.0 + bar.mon.0 && y >= bar.origin.1 && y < bar.origin.1 + bar.mon.1).or(b.first());
        bar.map(|bar| (bar.origin, bar.mon, bar.w.window().scale_factor())).unwrap_or(((0, 0), (1920, 1080), 1.0))
    })
}

/// Nowy wygląd (motyw, krycie, rozmycie) na paskach i otwartych okienkach.
pub fn relook() {
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            crate::settings::popup_theme(&bar.w.global::<Theme>());
        }
    });
    TIP.with(|t| {
        if let Some(w) = t.borrow().as_ref() {
            crate::settings::popup_theme(&w.global::<Theme>());
            popup_look(crate::window_hwnd(w.window()));
        }
    });
    close_popup();
    menus::close_pop();
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
    // Otwarte okienko (np. rozwijana lista urządzeń) – położenie i rozmiar jednym ruchem:
    // osobno okno najpierw podskakiwało w górę, a dopiero potem rosło.
    if win.is_visible() {
        taskwin::set_rect(crate::window_hwnd(win), sx, sy, pw, ph);
    } else {
        win.set_position(PhysicalPosition::new(sx, sy));
    }
    win.set_size(PhysicalSize::new(pw as u32, ph as u32));
    win.request_redraw();
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
/// Jeden wygląd dla wszystkich okienek (menu, Start, okienka paska, podpowiedzi, schowek,
/// powiadomienia): zaokrąglone rogi, obwódka w kolorze linii, tło z krycia ustawień i –
/// jeśli włączone – rozmycie tego, co pod spodem (Acrylic).
pub fn popup_look(hwnd: isize) {
    let line = BARS.with(|b| b.borrow().first().map(|b| b.w.global::<Theme>().get_line()));
    let c = line.unwrap_or(slint::Color::from_rgb_u8(60, 60, 60));
    powerdesk_core::system::style_popup(hwnd, Some((c.red(), c.green(), c.blue())));
    // Akryl przez SetWindowCompositionAttribute – działa na każdym okienku (bez ramki, bez fokusu),
    // tło systemowe Windows 11 na takich oknach się nie pokazywało.
    powerdesk_core::system::set_backdrop(hwnd, 0);
    powerdesk_core::system::see_through(hwnd);
    powerdesk_core::system::blur_behind(hwnd, crate::settings::popup_blur());
}

/// Motyw okienka jak motyw paska (jasny/ciemny, akcent).
macro_rules! same_theme {
    ($bar:expr, $w:expr) => {{
        let _ = $bar;
        crate::settings::popup_theme(&$w.global::<Theme>());
    }};
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
    out.extend(menus::pop_hwnd());
    POPUP.with(|p| {
        out.extend(p.borrow().as_ref().map(|p| match p {
            Popup::Volume(w) => crate::window_hwnd(w.window()),
            Popup::Calendar(w) => crate::window_hwnd(w.window()),
            Popup::Network(w) => crate::window_hwnd(w.window()),
            Popup::Notices(w) => crate::window_hwnd(w.window()),
            Popup::Tray(w, _, _) => crate::window_hwnd(w.window()),
        }))
    });
    THUMBS.with(|t| out.extend(t.borrow().as_ref().map(|t| crate::window_hwnd(t.w.window()))));
    out.extend(crate::start::hwnd());
    out.extend(crate::cliphistory::hwnd());
    out.extend(crate::overlay::hwnd());
    out
}

/// Rodzaj okienka (do rozpoznania drugiego kliku w tę samą ikonę).
fn popup_kind(p: &Popup) -> u8 {
    match p {
        Popup::Volume(_) => 1,
        Popup::Calendar(_) => 2,
        Popup::Network(_) => 3,
        Popup::Notices(_) => 4,
        Popup::Tray(..) => 5,
    }
}

/// Okienko tego rodzaju jest otwarte albo właśnie się zamknęło. Klik w ikonę paska najpierw
/// zabiera fokus okienku (zamyka je), a dopiero potem dociera do paska – bez tego drugi klik
/// otwierałby je od nowa zamiast zwinąć.
fn popup_was(kind: u8) -> bool {
    let open = POPUP.with(|p| p.borrow().as_ref().map(popup_kind)) == Some(kind);
    open || CLOSED.with(|c| c.get()).is_some_and(|(k, at)| k == kind && at.elapsed() < Duration::from_millis(350))
}

fn close_popup() {
    if let Some(k) = POPUP.with(|p| p.borrow().as_ref().map(popup_kind)) {
        CLOSED.with(|c| c.set(Some((k, std::time::Instant::now()))));
    }
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
        Some(Popup::Tray(w, _, _)) => {
            let _ = w.hide();
        }
        None => {}
    }
}

/// Okienko głośności nad ikoną; drugi klik w ikonę je zamyka.
fn open_volume(bar: usize, x: f32) {
    let was = popup_was(1);
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
    let was = popup_was(4);
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
    let was = popup_was(2);
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
    let was = popup_was(3);
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
    w.on_quick(|name| match name.as_str() {
        "bluetooth" => {
            let on = with_network_get(|w| w.get_bt() == 1);
            std::thread::spawn(move || {
                let res = powerdesk_core::quick::set_bluetooth(!on);
                let _ = slint::invoke_from_event_loop(move || quick_done(res));
            });
        }
        "airplane" => {
            let on = with_network_get(|w| w.get_airplane());
            std::thread::spawn(move || {
                let res = powerdesk_core::quick::set_airplane(!on);
                let _ = slint::invoke_from_event_loop(move || quick_done(res));
            });
        }
        page => {
            let page = match page {
                "bluetooth-settings" => "bluetooth",
                "hotspot" => "network-mobilehotspot",
                "nightlight" => "nightlight",
                "batterysaver" => "batterysaver",
                _ => "",
            };
            slint::Timer::single_shot(Duration::ZERO, close_popup);
            sysbar::open_settings(page);
        }
    });
    // Jasność: suwak wysyła wiele wartości – ustawiamy tylko ostatnią, po krótkiej przerwie.
    w.on_set_brightness(|v| {
        BRIGHT.with(|b| b.set(Some((v * 100.0).round() as u32)));
        slint::Timer::single_shot(Duration::from_millis(150), || {
            if let Some(level) = BRIGHT.with(|b| b.take()) {
                std::thread::spawn(move || powerdesk_core::quick::set_brightness(level));
            }
        });
    });
    load_quick();
    // Okienko 340 px wyśrodkowane nad ikoną (32 px).
    above_bar(w.window(), spot, 340.0, 580.0, x + 16.0 - 170.0);
    close_on_blur(w.window());
    show_above(w.window());
    POPUP.with(|p| *p.borrow_mut() = Some(Popup::Network(w)));
    rescan();
}

/// Menu „Bezpieczne usuwanie sprzętu”: dyski wymienne do wysunięcia.
fn usb_menu(bar: usize, x: f32) {
    let roots = powerdesk_core::quick::removable();
    let drives = powerdesk_core::drives::list();
    let mut items: Vec<Item> = Vec::new();
    for root in roots {
        let label = drives.iter().find(|d| d.path.eq_ignore_ascii_case(&root)).map(|d| d.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| "Dysk wymienny".into());
        let name = format!("Wysuń {label} ({})", root.trim_end_matches('\\'));
        items.push(it(name, "eject", move |s| {
            let res = powerdesk_core::drives::eject(0, &root);
            match res {
                Ok(()) => crate::transfers::toast(s, format!("Można bezpiecznie odłączyć {}", root.trim_end_matches('\\')), crate::transfers::Kind::Ok, None),
                Err(e) => crate::transfers::toast(s, format!("Nie udało się wysunąć: {e}"), crate::transfers::Kind::Error, None),
            }
            USB.with(|u| u.set(!powerdesk_core::quick::removable().is_empty()));
            tray_status();
        }));
    }
    if items.is_empty() {
        return;
    }
    show_menu(bar, items, x);
}

thread_local! {
    static USB: Cell<bool> = const { Cell::new(false) };
    /// Jasność czekająca na ustawienie (ostatnia wartość z suwaka).
    static BRIGHT: Cell<Option<u32>> = const { Cell::new(None) };
}

fn with_network_get<T: Default>(f: impl FnOnce(&NetworkWindow) -> T) -> T {
    let mut v = T::default();
    with_network(|w| v = f(w));
    v
}

/// Stan Bluetooth, trybu samolotowego i jasność – w tle (WinRT i WMI bywają wolne).
fn load_quick() {
    std::thread::spawn(|| {
        let (bt, airplane) = powerdesk_core::quick::radio_state();
        let bright = powerdesk_core::quick::brightness();
        let _ = slint::invoke_from_event_loop(move || {
            with_network(|w| {
                w.set_bt(bt.map_or(-1, |on| on as i32));
                w.set_airplane(airplane);
                w.set_brightness(bright.map_or(-1.0, |b| b as f32 / 100.0));
            })
        });
    });
}

fn quick_done(res: Result<(), String>) {
    if let Err(e) = res {
        with_network(|w| w.set_note(e.into()));
    }
    load_quick();
    slint::Timer::single_shot(Duration::from_millis(1500), || {
        with_network(fill_network);
        tray_status();
    });
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
        USB.with(|u| u.set(!powerdesk_core::quick::removable().is_empty()));
    }
    let usb = USB.with(|u| u.get());
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
            w.set_has_usb(usb);
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

/// Otwarte podglądy okien: okienko, grupa i jej okna.
struct Thumbs {
    w: ThumbWindow,
    bar: usize,
    index: usize,
    hwnds: Vec<isize>,
}

/// Wymiary karty podglądu – te same co w ThumbWindow (taskbar.slint).
const CARD_W: f32 = 220.0;
const CARD_H: f32 = 160.0;
const GAP: f32 = 8.0;

/// Kursor nad przyciskiem: gdy podglądy są już otwarte – od razu przełączamy grupę,
/// inaczej po chwili (jak w Windows, żeby przejechanie po pasku ich nie otwierało).
fn hover(bar: usize, i: usize, x: f32) {
    // Ten sam przycisk po kliknięciu – bez podglądów (klik przełączył okno).
    if NO_THUMBS.with(|c| c.get()) && HOVER.with(|h| h.get()) == Some((bar, i)) {
        return;
    }
    NO_THUMBS.with(|c| c.set(false));
    HOVER.with(|h| h.set(Some((bar, i))));
    let open = THUMBS.with(|t| t.borrow().as_ref().map(|t| (t.bar, t.index)));
    if open == Some((bar, i)) {
        return;
    }
    let delay = if open.is_some() { 0 } else { 450 };
    slint::Timer::single_shot(Duration::from_millis(delay), move || {
        // Kursor dalej nad tym samym przyciskiem i nie ma menu.
        if HOVER.with(|h| h.get()) == Some((bar, i)) && !NO_THUMBS.with(|c| c.get()) && !menus::pop_open() {
            show_thumbs(bar, i, x);
        }
    });
}

fn unhover() {
    HOVER.with(|h| h.set(None));
    NO_THUMBS.with(|c| c.set(false));
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
        .map(|win| ThumbCard {
            title: win.title.clone().into(),
            has_img: img.is_some(),
            img: img.clone().unwrap_or_default(),
            active: win.hwnd == fg,
            minimized: win.minimized,
            shot: Default::default(),
            has_shot: false,
        })
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
    THUMB_HOVER.with(|h| h.set(false));
    let gen = THUMB_GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let hwnds: Vec<isize> = g.wins.iter().map(|w| w.hwnd).collect();
    let (sw, sh) = (((CARD_W - 16.0) * spot.3) as u32, ((CARD_H - 42.0) * spot.3) as u32);
    THUMBS.with(|t| *t.borrow_mut() = Some(Thumbs { w, bar, index: i, hwnds: hwnds.clone() }));
    // Obrazy okien w tle, potem co sekundę, póki podglądy są otwarte.
    let shoot = move || {
        let hwnds = hwnds.clone();
        std::thread::spawn(move || {
            let shots: Vec<_> = hwnds.iter().map(|&h| taskwin::snapshot(h, sw, sh)).collect();
            let _ = slint::invoke_from_event_loop(move || put_shots(gen, shots));
        });
    };
    shoot();
    SHOT_TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(1), shoot));
}

/// Obrazy okien do kart – tylko gdy to wciąż te same podglądy.
fn put_shots(gen: u64, shots: Vec<Option<(u32, u32, Vec<u8>)>>) {
    if THUMB_GEN.with(|g| g.get()) != gen {
        return;
    }
    THUMBS.with(|t| {
        let t = t.borrow();
        let Some(t) = t.as_ref() else { return };
        let model = t.w.get_cards();
        for (k, shot) in shots.into_iter().enumerate() {
            let (Some((w, h, px)), Some(mut card)) = (shot, model.row_data(k)) else { continue };
            card.shot = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&px, w, h));
            card.has_shot = true;
            model.set_row_data(k, card);
        }
    });
}

fn close_thumbs() {
    SHOT_TIMER.with(|t| t.stop());
    THUMB_GEN.with(|g| g.set(g.get() + 1));
    if let Some(t) = THUMBS.with(|t| t.borrow_mut().take()) {
        let _ = t.w.hide();
    }
    THUMB_HOVER.with(|h| h.set(false));
}

fn close_menu() {
    menus::close_pop();
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
    let h = (height() * scale).round() as i32;
    bar.pos = (bar.origin.0, bar.origin.1 + bar.mon.1 - h);
    if !bar.placed {
        bar.blurred = None;
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
/// `mode` – grupowanie z ustawień: 0 – zawsze, 1 – gdy przyciski się nie mieszczą (`fits` –
/// ile zmieści się bez grupowania), 2 – nigdy (każde okno osobno; przypięty bierze swoje pierwsze okno).
fn group(pins: &[String], wins: &[Win], mode: u8, fits: usize) -> Vec<Group> {
    let mut out: Vec<Group> = pins.iter().map(|p| Group { exe: p.clone(), pinned: true, wins: Vec::new() }).collect();
    let separate = match mode {
        2 => true,
        1 => {
            // Bez grupowania: przypięte bez okien + każde okno osobno.
            let idle = pins.iter().filter(|p| !wins.iter().any(|w| w.exe.eq_ignore_ascii_case(p))).count();
            idle + wins.len() <= fits
        }
        _ => false,
    };
    for w in wins {
        let same = |g: &&mut Group| !w.exe.is_empty() && g.exe.eq_ignore_ascii_case(&w.exe) && (!separate || (g.pinned && g.wins.is_empty()));
        match out.iter_mut().find(same) {
            Some(g) => g.wins.push(w.clone()),
            None => out.push(Group { exe: w.exe.clone(), pinned: false, wins: vec![w.clone()] }),
        }
    }
    out
}

fn buttons(groups: &[Group], fg: isize, px: u32) -> Vec<TaskButton> {
    let mut idle = 0;
    let flashing = powerdesk_core::shellhook::flashing();
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
                flash: g.wins.iter().any(|w| w.hwnd != fg && flashing.contains(&w.hwnd)),
                count: g.wins.len() as i32,
                pinned: g.pinned,
                idle_before,
            }
        })
        .collect()
}

/// Najmniejsze krycie tła paska (3%) – przy zerze puste miejsca nie łapałyby kliknięć.
const MIN_ALPHA: f32 = 0.03;

fn refresh() {
    // Pasek Windows potrafi wrócić (Start, restart Eksploratora) – chowamy go co obieg.
    powerdesk_core::shellswap::hide_taskbar();
    // Własne okienka (menu, głośność, kalendarz) nie są oknami programów.
    let own = own_popups();
    let all: Vec<Win> = taskwin::list().into_iter().filter(|w| !own.contains(&w.hwnd)).collect();
    let fg = taskwin::foreground();
    if all.iter().any(|w| w.hwnd == fg) {
        LAST_APP.with(|l| l.set(fg));
    }
    let now = Local::now();
    let mut pins = Vec::new();
    let (mut opacity, mut clear, mut seconds, mut blur) = (100.0, false, false, true);
    let (mut grouping, mut labels, mut everywhere) = (0u8, true, false);
    crate::with_state(|s| {
        let b = s.borrow();
        everywhere = b.prefs.taskbar_buttons_all;
        pins = b.prefs.taskbar_pins.clone();
        opacity = b.prefs.taskbar_opacity;
        clear = b.prefs.taskbar_clear_desktop;
        seconds = b.prefs.taskbar_seconds;
        blur = b.prefs.taskbar_blur;
        grouping = b.prefs.taskbar_group;
        labels = b.prefs.taskbar_labels;
    });
    let (time, date) = (now.format(if seconds { "%H:%M:%S" } else { "%H:%M" }).to_string(), now.format("%d.%m.%Y").to_string());
    BARS.with(|b| {
        // Jeden pasek pokazuje wszystkie okna (także z monitorów bez paska).
        let single = b.borrow().len() <= 1;
        for bar in b.borrow_mut().iter_mut() {
            place(bar);
            let mine: Vec<Win> = all.iter().filter(|w| single || everywhere || w.monitor == bar.origin).cloned().collect();
            let wins = keep_order(&bar.wins, mine);
            // Model tylko przy zmianie – inaczej co sekundę migałyby podpowiedzi.
            if wins != bar.wins || fg != bar.fg || pins != bar.pins {
                let px = (icon_size() * bar.w.window().scale_factor()) as u32;
                // Ile przycisków z oknami zmieści się bez grupowania (pas przycisków ≈ monitor bez
                // Startu, zasobnika i zegara).
                let lane = bar.mon.0 as f32 / bar.w.window().scale_factor() - 460.0;
                let each = if labels { 164.0 } else { pin_w() + 4.0 };
                bar.groups = group(&pins, &wins, grouping, (lane / each).max(1.0) as usize);
                bar.pins = pins.clone();
                bar.w.set_buttons(ModelRc::new(VecModel::from(buttons(&bar.groups, fg, px))));
                bar.w.set_idle(bar.groups.iter().filter(|g| g.wins.is_empty()).count() as i32);
                bar.wins = wins;
                bar.fg = fg;
            }
            // Na pulpicie (żadne okno tego monitora nie jest otwarte) – przezroczysty. Najmniej 3%:
            // przy zerowym kryciu Windows przepuszcza kliknięcia przez puste miejsca paska.
            let desktop = clear && bar.wins.iter().all(|w| w.minimized);
            let alpha = if desktop { MIN_ALPHA } else { (opacity / 100.0f32).clamp(MIN_ALPHA, 1.0) };
            bar.w.set_bg_alpha(alpha);
            // Rozmycie pod półprzezroczystym paskiem; przezroczysty na pulpicie – bez rozmycia.
            let want = blur && !desktop && alpha < 0.999;
            if bar.blurred != Some(want) {
                powerdesk_core::system::blur_behind(crate::window_hwnd(bar.w.window()), want);
                bar.blurred = Some(want);
            }
            bar.w.set_time(time.clone().into());
            bar.w.set_date(date.clone().into());
        }
    });
    tray_status();
    vd_status();
}

/// Ikona zasobnika do Slint. Bez własnej podpowiedzi – nazwa programu.
fn tray_app(i: &powerdesk_core::trayhost::Icon) -> TrayApp {
    let mut tip = if i.tip.trim().is_empty() { program_name(&i.exe) } else { i.tip.trim().to_string() };
    if tip.trim().is_empty() {
        tip = "Program w zasobniku".into();
    }
    TrayApp { key: i.key().into(), id: i.id().into(), tip: tip.into(), img: tray_image(i) }
}

thread_local! {
    /// Obraz ikony zasobnika na identyfikator – ten sam obiekt, dopóki piksele się nie zmienią.
    /// Nowy obraz przy każdym odświeżeniu podmieniał wiersz pod kursorem i dymek z nazwą znikał.
    static TRAY_IMG: RefCell<std::collections::HashMap<String, (u64, slint::Image)>> = RefCell::new(std::collections::HashMap::new());
}

fn tray_image(i: &powerdesk_core::trayhost::Icon) -> slint::Image {
    use std::hash::{Hash, Hasher};
    let Some((w, h, px)) = i.image.as_ref() else { return slint::Image::default() };
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    (w, h, px).hash(&mut hs);
    let sum = hs.finish();
    TRAY_IMG.with(|c| {
        let mut c = c.borrow_mut();
        match c.get(&i.id()) {
            Some((s, img)) if *s == sum => img.clone(),
            _ => {
                let img = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(px, *w, *h));
                c.insert(i.id(), (sum, img.clone()));
                img
            }
        }
    })
}

/// Ikony programów: przypięte (w kolejności z ustawień) na pasku głównego monitora, reszta
/// w okienku nad strzałką; „wszystkie na pasku” – bez okienka.
fn tray_apps() {
    if TRAY_DRAG.with(|d| d.get()) {
        return;
    }
    let icons = powerdesk_core::trayhost::icons();
    let (mut pinned, mut all) = (Vec::new(), false);
    crate::with_state(|s| {
        let b = s.borrow();
        pinned = b.prefs.tray_bar.clone();
        all = b.prefs.tray_all;
    });
    let rank = |i: &powerdesk_core::trayhost::Icon| pinned.iter().position(|p| *p == i.id());
    let mut on_bar: Vec<&powerdesk_core::trayhost::Icon> = icons.iter().filter(|i| all || rank(i).is_some()).collect();
    on_bar.sort_by_key(|i| rank(i).unwrap_or(usize::MAX));
    let shown: Vec<TrayApp> = on_bar.iter().map(|i| tray_app(i)).collect();
    let hidden: Vec<TrayApp> = icons.iter().filter(|i| !all && rank(i).is_none()).map(tray_app).collect();
    // Ikony programów tylko na pasku głównego monitora (jak w Windows).
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            let main = bar.origin == (0, 0);
            crate::sync_model(bar.w.get_tray_apps(), if main { shown.clone() } else { Vec::new() }, |m| bar.w.set_tray_apps(m));
            bar.w.set_tray_hidden(if main { hidden.len() as i32 } else { 0 });
            bar.w.set_tray_all(main && all);
        }
    });
    // Otwarte okienko ukrytych – nowa zawartość i wysokość.
    let open = POPUP.with(|p| match &*p.borrow() {
        Some(Popup::Tray(w, bar, x)) => {
            crate::sync_model(w.get_apps(), hidden.clone(), |m| w.set_apps(m));
            w.set_show_all(all);
            Some((*bar, *x, hidden.len()))
        }
        _ => None,
    });
    if let Some((bar, x, n)) = open {
        place_tray_overflow(bar, x, n);
    }
}

fn tray_click(key: &str, b: i32) {
    use powerdesk_core::trayhost::{self, Click};
    let (x, y) = taskwin::cursor();
    let button = match b {
        1 => Click::Double,
        2 => Click::Right,
        _ => Click::Left,
    };
    trayhost::click(key, button, x, y);
}

/// Zmiana listy ikon na pasku (zapis i odświeżenie).
fn edit_tray(f: impl FnOnce(&mut crate::model::Prefs)) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        f(&mut b.prefs);
        b.prefs.save();
    });
    tray_apps();
}

/// Kursor nad którymś paskiem (puszczenie ikony na pasku).
fn cursor_on_bar() -> bool {
    let (cx, cy) = taskwin::cursor();
    BARS.with(|b| {
        b.borrow().iter().any(|bar| {
            let h = (height() * bar.w.window().scale_factor()) as i32;
            cx >= bar.origin.0 && cx < bar.origin.0 + bar.mon.0 && cy >= bar.pos.1 && cy < bar.pos.1 + h
        })
    })
}

/// Ikona z paska puszczona: na strzałce albo nad paskiem – do okienka ukrytych, na pasku –
/// nowe miejsce.
fn tray_drop_on_bar(id: &str, dx: f32, to_arrow: bool) {
    let id = id.to_string();
    let icons: Vec<String> = powerdesk_core::trayhost::icons().iter().map(|i| i.id()).collect();
    let off = to_arrow || !cursor_on_bar();
    edit_tray(move |p| {
        // „Wszystkie na pasku”: zaczynamy od pełnej listy w obecnej kolejności.
        if p.tray_all {
            let mut full: Vec<String> = p.tray_bar.iter().filter(|x| icons.contains(x)).cloned().collect();
            let rest: Vec<String> = icons.iter().filter(|x| !full.contains(x)).cloned().collect();
            full.extend(rest);
            p.tray_bar = full;
            p.tray_all = false;
        }
        let Some(from) = p.tray_bar.iter().position(|x| *x == id) else { return };
        let item = p.tray_bar.remove(from);
        if off {
            return;
        }
        // Ikona 24 px, bez odstępu (taskbar.slint).
        let to = (from as i32 + (dx / 24.0).round() as i32).clamp(0, p.tray_bar.len() as i32) as usize;
        p.tray_bar.insert(to, item);
    });
}

/// Ikona z okienka puszczona na pasku – zostaje na nim (na końcu).
fn tray_drop_from_overflow(id: &str) {
    if !cursor_on_bar() {
        return;
    }
    let id = id.to_string();
    edit_tray(move |p| {
        if !p.tray_bar.contains(&id) {
            p.tray_bar.push(id);
        }
    });
}

fn tray_first_run() {
    let mut done = true;
    crate::with_state(|s| done = s.borrow().prefs.tray_init);
    if done {
        return;
    }
    let first: Vec<String> = powerdesk_core::trayhost::icons().iter().take(6).map(|i| i.id()).collect();
    edit_tray(move |p| {
        p.tray_init = true;
        for id in first {
            if !p.tray_bar.contains(&id) {
                p.tray_bar.push(id);
            }
        }
    });
}

/// Okno dymka – tworzone raz przy otwarciu pasków (pokazanie okna Slint zabiera fokus;
/// potem już tylko natywne pokaż/schowaj bez aktywacji).
fn ensure_tip() {
    if TIP.with(|t| t.borrow().is_some()) {
        return;
    }
    let Ok(w) = TipWindow::new() else { return };
    same_theme!(0, w);
    let fg = taskwin::foreground();
    w.window().set_size(PhysicalSize::new(1, 1));
    let _ = w.show();
    let hwnd = crate::window_hwnd(w.window());
    if hwnd == 0 {
        // Okno systemowe jeszcze nie powstało – schowane, kolejna próba za chwilę.
        let _ = w.hide();
        slint::Timer::single_shot(Duration::from_millis(200), ensure_tip);
        return;
    }
    taskwin::make_passive(hwnd);
    popup_look(hwnd);
    taskwin::hide_native(hwnd);
    if fg != 0 {
        taskwin::activate(fg);
    }
    TIP.with(|t| *t.borrow_mut() = Some(w));
}

/// Podpowiedź z okna paska (albo jego okienka): po 0,6 s bez zmian dymek nad oknem, przy
/// kursorze; pusty tekst chowa go od razu.
fn tip(win: &slint::Window, text: &str, x: f32, _y: f32) {
    let seq = TIP_SEQ.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    if text.is_empty() {
        TIP.with(|t| {
            if let Some(w) = t.borrow().as_ref() {
                taskwin::hide_native(crate::window_hwnd(w.window()));
            }
        });
        return;
    }
    let (pos, scale) = (win.position(), win.scale_factor());
    let (ax, top) = (pos.x + (x * scale) as i32, pos.y);
    let text = text.to_string();
    // Okno dymka mogło nie powstać przy starcie (pętla zdarzeń jeszcze nie ruszyła).
    ensure_tip();
    slint::Timer::single_shot(Duration::from_millis(500), move || {
        if TIP_SEQ.with(|s| s.get()) != seq {
            return;
        }
        TIP.with(|t| {
            let t = t.borrow();
            let Some(w) = t.as_ref() else { return };
            w.set_text(text.clone().into());
            let s = w.window().scale_factor();
            // Miara tekstu bywa zerowa w schowanym oknie (nie było jeszcze klatki) – wtedy dymek miał
            // kilka pikseli i nie było go widać. Zapasowo szerokość z liczby znaków.
            let guess_w = (text.chars().count().min(64) as f32 * 6.6 + 20.0).min(440.0);
            let (lw, lh) = (w.get_want_w(), w.get_want_h());
            let lw = if lw < 30.0 { guess_w } else { lw };
            let lh = if lh < 20.0 { 26.0 } else { lh };
            let (tw, th) = ((lw * s) as i32, (lh * s) as i32);
            let (mx, mw) = monitor_under_cursor().map(|(o, m, _)| (o.0, m.0)).unwrap_or((0, 1920));
            let tx = (ax - tw / 2).clamp(mx + 4, (mx + mw - tw - 4).max(mx + 4));
            let ty = top - th - (6.0 * s) as i32;
            w.window().set_size(PhysicalSize::new(tw as u32, th as u32));
            taskwin::show_passive(crate::window_hwnd(w.window()), tx, ty, tw, th);
            // Okno schowane natywnie nie dostawało nowej klatki – rysujemy jawnie po pokazaniu.
            w.window().request_redraw();
        });
        slint::Timer::single_shot(Duration::from_millis(40), || {
            TIP.with(|t| {
                if let Some(w) = t.borrow().as_ref() {
                    w.window().request_redraw();
                }
            })
        });
    });
}

/// Wysokość okienka ukrytych: te same wymiary co w TrayOverflowWindow (7 ikon w rzędzie).
fn overflow_size(n: usize) -> (f32, f32) {
    let rows = n.div_ceil(7).max(1) as f32;
    (248.0, 12.0 + 20.0 + 8.0 + rows * 40.0 + (rows - 1.0) * 8.0 + 8.0 + 14.0 + 8.0 + 30.0 + 12.0)
}

fn place_tray_overflow(bar: usize, x: f32, n: usize) {
    let Some(spot) = spot(bar) else { return };
    let (w, h) = overflow_size(n);
    POPUP.with(|p| {
        if let Some(Popup::Tray(win, _, _)) = &*p.borrow() {
            above_bar(win.window(), spot, w, h, x + 16.0 - w / 2.0);
        }
    });
}

/// Strzałka zasobnika: okienko z ukrytymi ikonami nad nią (drugi klik – zamknięcie).
fn open_tray_overflow(bar: usize, x: f32) {
    let was = popup_was(5);
    close_menu();
    close_popup();
    if was {
        return;
    }
    let Ok(w) = TrayOverflowWindow::new() else { return };
    same_theme!(bar, w);
    w.on_app_click(|key, b| tray_click(&key, b));
    w.on_app_drop(|id| tray_drop_from_overflow(&id));
    let ww = w.as_weak();
    w.on_tip(move |t, x, y| {
        if let Some(w) = ww.upgrade() {
            tip(w.window(), &t, x, y);
        }
    });
    w.on_set_show_all(|on| {
        edit_tray(move |p| p.tray_all = on);
        // Wszystkie na pasku – okienko nie ma już czego pokazywać.
        if on {
            slint::Timer::single_shot(Duration::ZERO, close_popup);
        }
    });
    close_on_blur(w.window());
    POPUP.with(|p| *p.borrow_mut() = Some(Popup::Tray(w, bar, x)));
    tray_apps();
    POPUP.with(|p| {
        if let Some(Popup::Tray(w, _, _)) = &*p.borrow() {
            show_above(w.window());
        }
    });
}

/// Ikony programów dochodzą w tle – przerysowujemy paski, na których są.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            if bar.groups.iter().any(|g| paths.contains(&g.exe)) {
                let px = (icon_size() * bar.w.window().scale_factor()) as u32;
                bar.w.set_buttons(ModelRc::new(VecModel::from(buttons(&bar.groups, bar.fg, px))));
            }
        }
    });
}
