//! Pasek zadań PowerDesk – na razie tryb testowy: na dole każdego monitora okno na wierzchu
//! (nad paskiem Windows, który działa dalej) z przyciskiem Start, oknami z tego monitora
//! i zegarem. Lista okien odświeża się co sekundę; okna jednego programu dzielą przycisk.

use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use chrono::Local;
use powerdesk_core::desktop as data;
use powerdesk_core::taskwin::{self, Win};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, VecModel};

use crate::menus::{self, it, sep, Item};
use crate::{BarMenuWindow, Shared, TaskButton, TaskbarWindow, Theme};

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
    /// Przyciski: okna jednego programu razem, w kolejności `wins`.
    groups: Vec<Vec<Win>>,
    fg: isize,
}

thread_local! {
    static BARS: RefCell<Vec<Bar>> = const { RefCell::new(Vec::new()) };
    static TIMER: slint::Timer = slint::Timer::default();
    /// Otwarte menu paska i jego pozycje.
    static MENU: RefCell<Option<(BarMenuWindow, Vec<Item>)>> = const { RefCell::new(None) };
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
            if let Some(group) = group_at(n, i as usize) {
                click_group(&group);
                // Stan (aktywne / zminimalizowane) od razu, bez czekania na zegar.
                slint::Timer::single_shot(Duration::from_millis(150), refresh);
            }
        });
        w.on_context(move |i, x, y| context(n, i as usize, x, y));
        w.on_close_test(|| slint::Timer::single_shot(Duration::ZERO, close));
        let _ = w.show();
        bars.push(Bar { w, origin: (m.x, m.y), mon: (m.width, m.height), pos: (m.x, m.y + m.height), placed: false, wins: Vec::new(), groups: Vec::new(), fg: 0 });
    }
    BARS.with(|b| *b.borrow_mut() = bars);
    refresh();
    // Okno systemowe powstaje dopiero w pętli zdarzeń – ułożenie zaraz po pierwszym obiegu.
    slint::Timer::single_shot(Duration::from_millis(100), refresh);
    TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(1), refresh));
}

pub fn close() {
    close_menu();
    TIMER.with(|t| t.stop());
    let bars = BARS.with(|b| std::mem::take(&mut *b.borrow_mut()));
    for b in bars {
        let _ = b.w.hide();
    }
}

fn group_at(bar: usize, i: usize) -> Option<Vec<Win>> {
    BARS.with(|b| b.borrow().get(bar).and_then(|b| b.groups.get(i).cloned()))
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
    let Some(group) = group_at(bar, i) else { return };
    let Some(first) = group.first().cloned() else { return };
    let mut items = Vec::new();
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
        items.push(it("Nowe okno programu", "plus", move |_| {
            let _ = std::process::Command::new(&exe).spawn();
        }));
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
    let (pw, ph) = ((width * scale) as i32, (height * scale) as i32);
    let sx = (pos.0 + (x * scale) as i32).clamp(origin.0 + 4, (origin.0 + mon.0 - pw - 4).max(origin.0 + 4));
    let sy = pos.1 - ph - (6.0 * scale) as i32;
    w.set_entries(ModelRc::new(VecModel::from(entries)));
    w.set_menu_width(width);
    w.window().set_position(PhysicalPosition::new(sx, sy));
    w.window().set_size(PhysicalSize::new(pw as u32, ph as u32));
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
    let _ = w.show();
    let hwnd = crate::window_hwnd(w.window());
    data::hide_from_taskbar(hwnd);
    taskwin::activate(hwnd);
    MENU.with(|m| *m.borrow_mut() = Some((w, items)));
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

/// Okna tego samego programu razem – grupa stoi tam, gdzie jej pierwsze okno.
/// Okna bez znanego pliku programu zostają osobno.
fn group(wins: &[Win]) -> Vec<Vec<Win>> {
    let mut out: Vec<Vec<Win>> = Vec::new();
    for w in wins {
        match out.iter_mut().find(|g| !w.exe.is_empty() && g[0].exe.eq_ignore_ascii_case(&w.exe)) {
            Some(g) => g.push(w.clone()),
            None => out.push(vec![w.clone()]),
        }
    }
    out
}

fn buttons(groups: &[Vec<Win>], fg: isize, px: u32) -> Vec<TaskButton> {
    groups
        .iter()
        .map(|g| {
            let w = g.iter().find(|w| w.hwnd == fg).unwrap_or(&g[0]);
            let img = if w.exe.is_empty() { None } else { crate::thumbs::get(&w.exe, px, true) };
            TaskButton {
                title: w.title.clone().into(),
                has_img: img.is_some(),
                img: img.unwrap_or_default(),
                active: w.hwnd == fg,
                minimized: g.iter().all(|w| w.minimized),
                count: g.len() as i32,
            }
        })
        .collect()
}

fn refresh() {
    let all = taskwin::list();
    let fg = taskwin::foreground();
    let now = Local::now();
    let (time, date) = (now.format("%H:%M").to_string(), now.format("%d.%m.%Y").to_string());
    BARS.with(|b| {
        for bar in b.borrow_mut().iter_mut() {
            place(bar);
            let mine: Vec<Win> = all.iter().filter(|w| w.monitor == bar.origin).cloned().collect();
            let wins = keep_order(&bar.wins, mine);
            // Model tylko przy zmianie – inaczej co sekundę migałyby podpowiedzi.
            if wins != bar.wins || fg != bar.fg {
                let px = (20.0 * bar.w.window().scale_factor()) as u32;
                bar.groups = group(&wins);
                bar.w.set_buttons(ModelRc::new(VecModel::from(buttons(&bar.groups, fg, px))));
                bar.wins = wins;
                bar.fg = fg;
            }
            bar.w.set_time(time.clone().into());
            bar.w.set_date(date.clone().into());
        }
    });
}

/// Ikony programów dochodzą w tle – przerysowujemy paski, na których są.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            if bar.wins.iter().any(|w| paths.contains(&w.exe)) {
                let px = (20.0 * bar.w.window().scale_factor()) as u32;
                bar.w.set_buttons(ModelRc::new(VecModel::from(buttons(&bar.groups, bar.fg, px))));
            }
        }
    });
}
