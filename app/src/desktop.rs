//! Pulpit PowerDesk zamiast pulpitu Windows: na każdym monitorze okno bez ramki z tapetą
//! z ustawień Windows (dziecko `Progman` – Win+D go nie chowa), a na głównym ikony z Pulpitu
//! użytkownika i publicznego. Ikony Windows ukryte; przywraca je core::shellswap.
//! Ikony leżą w siatce: ręcznie przesunięte trzymają swoje miejsce (prefs), reszta układa się
//! kolumnami od lewego górnego rogu. Zaznaczanie kliknięciem, Ctrl, Shift i ramką; menu
//! i zmiana nazwy w stylu PowerDesk.

use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use powerdesk_core::desktop::{self as data, Fit, Monitor};
use powerdesk_core::fsx::Entry;
use powerdesk_core::{clip, ops};
use slint::{Color, ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::menus::{self, it, sep, Item};
use crate::{fmt, transfers, DeskIcon, DesktopWindow, Shared, Theme, WidgetItem};

const MARGIN: f32 = 8.0;
/// „Ten komputer” na pulpicie – wirtualna ikona (otwiera widok Komputer PowerDesk).
pub const COMPUTER: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";

/// Pole siatki i rozmiar ikony (piksele logiczne) dla rozmiaru z ustawień: małe, średnie, duże.
fn dims(size: u8) -> (f32, f32, f32) {
    match size {
        0 => (76.0, 80.0, 32.0),
        2 => (104.0, 120.0, 64.0),
        _ => (84.0, 96.0, 48.0),
    }
}

#[derive(Default)]
struct Desk {
    /// Okna pulpitu (po jednym na monitor); pierwsze – monitor główny z ikonami.
    windows: Vec<DesktopWindow>,
    /// Monitor każdego okna (x, y, szerokość, wysokość – piksele ekranu).
    rects: Vec<(i32, i32, i32, i32)>,
    /// Tapeta Windows każdego okna (do powrotu z pokazu slajdów i własnych obrazów).
    defaults: Vec<String>,
    /// Uchwyty okien po wpięciu w Progman – po restarcie Eksploratora przestają istnieć.
    hwnds: Vec<isize>,
    items: Vec<Entry>,
    /// Miejsce każdej ikony w siatce (kolumna, wiersz) – w kolejności `items`.
    cells: Vec<(i32, i32)>,
    /// Lewy górny róg każdej ikony w oknie (z siatki albo dowolny) – w kolejności `items`.
    spots: Vec<(f32, f32)>,
    /// Pole siatki (szerokość, wysokość) i ikona – z ustawień przy ostatnim rysowaniu.
    cell: (f32, f32, f32),
    /// Ikony ustawiane dowolnie (bez wyrównania do siatki).
    free: bool,
    sel: HashSet<String>,
    /// Ostatnio kliknięta ikona – początek zakresu z Shift.
    anchor: Option<usize>,
    renaming: Option<String>,
    /// Pozycje otwartego menu i miejsce kliknięcia (do menu Windows z „Więcej opcji”).
    menu: Vec<Item>,
    menu_at: (f32, f32),
}

thread_local! {
    static D: RefCell<Desk> = RefCell::new(Desk::default());
    /// Pilnowanie: ikony Windows ukryte, okna wpięte (restart Eksploratora je niszczy).
    static WATCH: slint::Timer = slint::Timer::default();
    /// Podpis ekranu (monitory, DPI, tapeta) z chwili budowy pulpitu.
    static SCREEN: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn is_open() -> bool {
    D.with(|d| !d.borrow().windows.is_empty())
}

/// Folder, do którego trafia to, co tworzymy na pulpicie (Pulpit użytkownika).
fn desktop_dir() -> String {
    data::folders().into_iter().next().unwrap_or_default()
}

/// Tapety dekodujemy w tle – duży obraz na wątku UI zamroziłby okno.
fn decode(path: &str) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::open(path).ok()?.to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

pub fn open(_s: &Shared) {
    if is_open() {
        return;
    }
    SCREEN.with(|c| *c.borrow_mut() = data::screen_signature());
    let cfg = crate::wallpapers::config();
    std::thread::spawn(move || {
        let mons = data::monitors();
        // Numer okna jak w build(): główny monitor pierwszy.
        let mut order: Vec<usize> = (0..mons.len()).collect();
        order.sort_by_key(|&i| !mons[i].primary);
        let images = cfg.images();
        let bing = if cfg.is_bing() { powerdesk_core::bing::today() } else { None };
        let mut walls: Vec<(Option<SharedPixelBuffer<Rgba8Pixel>>, bool)> = vec![(None, false); mons.len()];
        for (n, &i) in order.iter().enumerate() {
            let (path, own) = cfg.choose(n, &mons[i].wallpaper, &images, bing.as_ref());
            walls[i] = (if path.is_empty() { None } else { decode(&path) }, own);
        }
        let items = data::items();
        let _ = slint::invoke_from_event_loop(move || crate::with_state(|s| build(s, mons, walls, items)));
    });
}

fn build(s: &Shared, mons: Vec<Monitor>, mut walls: Vec<(Option<SharedPixelBuffer<Rgba8Pixel>>, bool)>, items: Vec<Entry>) {
    if mons.is_empty() {
        transfers::toast(s, "Nie udało się odczytać monitorów i tapet", transfers::Kind::Error, None);
        return;
    }
    let look = s.borrow().ui.upgrade().map(|u| crate::settings::look(&u));
    // Główny monitor pierwszy – to on dostaje ikony.
    let mut order: Vec<usize> = (0..mons.len()).collect();
    order.sort_by_key(|&i| !mons[i].primary);
    let mut out = Vec::new();
    let mut rects = Vec::new();
    let mut defaults = Vec::new();
    for (n, i) in order.into_iter().enumerate() {
        let m = &mons[i];
        let Ok(w) = DesktopWindow::new() else { continue };
        // Globalne Slint są osobne dla każdego okna – motyw menu i pola nazwy z głównego.
        if let Some(l) = &look {
            let t = w.global::<Theme>();
            t.set_light(l.light);
            t.set_accent_override(l.accent);
            t.set_popup_alpha(l.popup_alpha);
        }
        w.set_bg(Color::from_rgb_u8(m.color.0, m.color.1, m.color.2));
        defaults.push(m.wallpaper.clone());
        let own = walls[i].1;
        if let Some(buf) = walls[i].0.take() {
            w.set_wallpaper(slint::Image::from_rgba8(buf));
            w.set_has_wallpaper(true);
        }
        w.set_fit(match m.fit {
            Fit::Fill | Fit::Span => 0,
            Fit::Fit => 1,
            Fit::Stretch => 2,
            Fit::Center => 3,
            Fit::Tile => 4,
        });
        w.window().set_position(PhysicalPosition::new(m.x, m.y));
        w.window().set_size(PhysicalSize::new(m.width as u32, m.height as u32));
        if n == 0 {
            wire_icons(s, &w);
        } else {
            // Pozostałe monitory: sama tapeta, ale menu pulpitu też działa.
            let s2 = s.clone();
            w.on_context(move |_, x, y| background_menu(&s2, n, x, y));
            let s2 = s.clone();
            w.on_menu_chosen(move |i| menu_chosen(&s2, i as usize));
        }
        w.on_refresh(refresh);
        wire_widgets(&w, n);
        crate::widgetcfg::wire(&w.global::<crate::WidgetEdit>());
        crate::widgets::apply(&w.global::<crate::WidgetData>());
        if own {
            w.set_fit(0);
        }
        let _ = w.show();
        out.push(w);
        rects.push((m.x, m.y, m.width, m.height));
    }
    D.with(|d| {
        let mut d = d.borrow_mut();
        d.windows = out;
        d.rects = rects;
        d.defaults = defaults;
        d.items = with_specials(items, s.borrow().prefs.desktop_computer);
    });
    render(s);
    render_widgets(s);
    crate::widgets::start();
    crate::wallpapers::start_timer();
    // Okno systemowe powstaje dopiero w pętli zdarzeń – styl (bez paska zadań i Alt+Tab)
    // i wpięcie w Progman chwilę później.
    slint::Timer::single_shot(Duration::from_millis(100), attach);
    WATCH.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(2), watch));
}

/// Okna pulpitu jako dzieci Progman, na swoich monitorach; ikony Windows ukryte.
fn attach() {
    D.with(|d| {
        let mut d = d.borrow_mut();
        let mut hwnds = Vec::new();
        for (w, &(x, y, wd, ht)) in d.windows.iter().zip(d.rects.iter()) {
            let hwnd = crate::window_hwnd(w.window());
            if hwnd == 0 {
                continue;
            }
            data::hide_from_taskbar(hwnd);
            powerdesk_core::system::no_frame_border(hwnd);
            // Zwykłe okna pod wszystkimi programami – z Eksploratorem tuż nad jego pulpitem (Progman),
            // bez niego na samym dnie. (Warstwa tapety Windows – WorkerW – nie rysuje naszych okien.)
            powerdesk_core::shellmode::keep_at_bottom(hwnd);
            powerdesk_core::shellmode::place_desktop(hwnd, x, y, wd, ht);
            // Pierwsze okno (monitor główny) – okno powłoki systemu, gdy PowerDesk nią jest.
            if hwnds.is_empty() && powerdesk_core::shellmode::active() {
                powerdesk_core::shellmode::register_shell_window(hwnd);
            }
            hwnds.push(hwnd);
        }
        // Bez Progman (Eksplorator jeszcze wstaje) – kolejna próba przy pilnowaniu.
        d.hwnds = if hwnds.len() == d.windows.len() { hwnds } else { Vec::new() };
    });
    powerdesk_core::shellswap::hide_desktop_icons();
}

/// Restart Eksploratora niszczy Progman razem z naszymi oknami – budujemy pulpit od nowa.
/// Ikony Windows wracają też po zmianie tapety z animacją – chowamy je ponownie.
fn watch() {
    // Inny układ monitorów, skala albo tapeta – pulpit i pasek od nowa.
    if SCREEN.with(|c| *c.borrow() != data::screen_signature()) {
        close();
        crate::taskbar::close();
        crate::with_state(|s| {
            open(s);
            crate::taskbar::open(s);
        });
        return;
    }
    let dead = D.with(|d| d.borrow().hwnds.iter().any(|&h| !powerdesk_core::shellswap::alive(h)));
    if dead {
        close();
        crate::with_state(|s| open(s));
        return;
    }
    if D.with(|d| d.borrow().hwnds.is_empty() && !d.borrow().windows.is_empty()) {
        attach();
    }
    if powerdesk_core::shellmode::active() {
        return;
    }
    // Z Eksploratorem: jego pulpit potrafi wyjść nad nasze okna (np. po zmianie tapety) – wracamy
    // tuż nad niego; ikony Windows zostają ukryte.
    D.with(|d| d.borrow().hwnds.iter().for_each(|&h| powerdesk_core::shellmode::lower(h)));
    powerdesk_core::shellswap::hide_desktop_icons();
}

/// Tapety Windows okien pulpitu w ich kolejności.
pub fn wallpaper_defaults() -> Vec<String> {
    D.with(|d| d.borrow().defaults.clone())
}

/// Nowa tapeta okna `n`; `own` – nie z Windows (zawsze „wypełnij”).
pub fn set_wallpaper(n: usize, buf: Option<SharedPixelBuffer<Rgba8Pixel>>, own: bool) {
    D.with(|d| {
        let d = d.borrow();
        let Some(w) = d.windows.get(n) else { return };
        match buf {
            Some(b) => {
                w.set_wallpaper(slint::Image::from_rgba8(b));
                w.set_has_wallpaper(true);
            }
            None => w.set_has_wallpaper(false),
        }
        if own {
            w.set_fit(0);
        }
    });
}

/// Okna pulpitu (widżety odświeżają ich dane).
pub fn each_window(f: impl Fn(&DesktopWindow)) {
    D.with(|d| d.borrow().windows.iter().for_each(&f));
}

/// Widżety każdego monitora z ustawień; numer w modelu to miejsce na liście w ustawieniach.
fn render_widgets(s: &Shared) {
    let all = s.borrow().prefs.desktop_widgets.clone();
    D.with(|d| {
        let d = d.borrow();
        for (n, w) in d.windows.iter().enumerate() {
            let items: Vec<WidgetItem> = all
                .iter()
                .filter(|x| x.monitor == n || (n == 0 && x.monitor >= d.windows.len()))
                .map(|x| WidgetItem { kind: x.kind.clone().into(), note: x.note.clone().into(), x: x.x, y: x.y, w: x.w, h: x.h, cfg: crate::widgetcfg::to_slint(&x.kind, &x.cfg) })
                .collect();
            w.set_widgets(ModelRc::new(VecModel::from(items)));
        }
    });
}

/// Przerysowanie widżetów wszystkich monitorów (po zmianie ustawień widżetu).
pub fn refresh_widgets() {
    crate::with_state(render_widgets);
}

/// Okno pulpitu monitora `n`, jeśli jest.
pub fn with_window(n: usize, f: impl FnOnce(&DesktopWindow)) {
    D.with(|d| {
        if let Some(w) = d.borrow().windows.get(n) {
            f(w);
        }
    });
}

/// Rozmiar okna monitora `n` w pikselach logicznych.
fn logical_size(n: usize) -> (f32, f32) {
    let mut out = (1920.0, 1080.0);
    with_window(n, |w| {
        let s = w.window().size().to_logical(w.window().scale_factor());
        out = (s.width, s.height);
    });
    out
}

/// Numer widżetu w ustawieniach z numeru w modelu okna `n`.
fn widget_index(s: &Shared, n: usize, i: usize) -> Option<usize> {
    let count = D.with(|d| d.borrow().windows.len());
    let b = s.borrow();
    b.prefs.desktop_widgets.iter().enumerate().filter(|(_, x)| x.monitor == n || (n == 0 && x.monitor >= count)).nth(i).map(|(k, _)| k)
}

/// Zmiana widżetu `i` okna `n` w ustawieniach i zapis; `redraw` – przerysowanie (nie przy pisaniu).
fn edit_widget(s: &Shared, n: usize, i: usize, redraw: bool, f: impl FnOnce(&mut crate::model::DeskWidget)) {
    let Some(k) = widget_index(s, n, i) else { return };
    {
        let mut b = s.borrow_mut();
        if let Some(x) = b.prefs.desktop_widgets.get_mut(k) {
            f(x);
        }
        b.prefs.save();
    }
    if redraw {
        render_widgets(s);
    }
}

/// Wyrównanie do siatki 8 px – widżety łatwo ustawić równo.
fn snap(v: f32) -> f32 {
    (v / 8.0).round() * 8.0
}

fn wire_widgets(w: &DesktopWindow, n: usize) {
    w.on_widget_moved(move |i, x, y| crate::with_state(|s| edit_widget(s, n, i as usize, true, |d| {
        d.x = snap(x.max(0.0));
        d.y = snap(y.max(0.0));
    })));
    w.on_widget_resized(move |i, wd, ht| crate::with_state(|s| edit_widget(s, n, i as usize, true, |d| {
        d.w = snap(wd);
        d.h = snap(ht);
    })));
    w.on_widget_note(move |i, text| crate::with_state(|s| edit_widget(s, n, i as usize, false, |d| d.note = text.to_string())));
    w.on_widget_context(move |i, x, y| {
        let i = i as usize;
        let kind = widget_kind(n, i);
        let mut items = vec![it("Dostosuj…", "settings", move |s| {
            if let Some(k) = widget_index(s, n, i) {
                crate::widgetcfg::open(crate::widgetcfg::Target::Desk { win: n, index: k }, x, y, logical_size(n));
            }
        })];
        items.extend(crate::widgetfeeds::config_items(&kind));
        items.push(sep());
        items.extend([
            it("Usuń widżet", "trash", move |s| {
                if let Some(k) = widget_index(s, n, i) {
                    {
                        let mut b = s.borrow_mut();
                        b.prefs.desktop_widgets.remove(k);
                        b.prefs.save();
                    }
                    render_widgets(s);
                }
            })
            .danger(),
        ]);
        show_menu(n, items, x, y);
    });
}

/// Rodzaj widżetu `i` okna pulpitu `n`.
pub fn widget_kind(n: usize, i: usize) -> String {
    let mut k = String::new();
    crate::with_state(|s| {
        if let Some(x) = widget_index(s, n, i) {
            k = s.borrow().prefs.desktop_widgets[x].kind.clone();
        }
    });
    k
}

/// Nowy widżet w miejscu kliknięcia (x, y w oknie monitora `n`).
fn add_widget(s: &Shared, n: usize, kind: &str, x: f32, y: f32) {
    let (w, h) = crate::widgets::KINDS.iter().find(|k| k.0 == kind).map_or((240.0, 160.0), |k| k.4);
    {
        let mut b = s.borrow_mut();
        b.prefs.desktop_widgets.push(crate::model::DeskWidget { kind: kind.to_string(), note: String::new(), monitor: n, x: snap(x), y: snap(y), w, h, cfg: Default::default() });
        b.prefs.save();
    }
    render_widgets(s);
}

fn wire_icons(s: &Shared, w: &DesktopWindow) {
    let s2 = s.clone();
    w.on_open(move |i| open_item(&s2, i as usize));
    let s2 = s.clone();
    w.on_select(move |i, ctrl, shift| select(&s2, i, ctrl, shift));
    let s2 = s.clone();
    w.on_band_done(move |x, y, bw, bh, ctrl| band(&s2, (x, y, bw, bh), ctrl));
    let s2 = s.clone();
    w.on_dropped(move |dx, dy| dropped(&s2, dx, dy));
    let s2 = s.clone();
    w.on_context(move |i, x, y| {
        if i < 0 {
            background_menu(&s2, 0, x, y);
        } else {
            icon_menu(&s2, x, y);
        }
    });
    let s2 = s.clone();
    w.on_key(move |k, ctrl, shift| key(&s2, k.as_str(), ctrl, shift));
    let s2 = s.clone();
    w.on_rename_done(move |i, name| rename_done(&s2, i as usize, name.as_str()));
    let s2 = s.clone();
    w.on_rename_cancel(move || {
        D.with(|d| d.borrow_mut().renaming = None);
        render(&s2);
    });
    let s2 = s.clone();
    w.on_menu_chosen(move |i| menu_chosen(&s2, i as usize));
}

pub fn close() {
    WATCH.with(|t| t.stop());
    let ws = D.with(|d| {
        let mut d = d.borrow_mut();
        d.sel.clear();
        d.renaming = None;
        d.hwnds.clear();
        d.rects.clear();
        std::mem::take(&mut d.windows)
    });
    for w in ws {
        let _ = w.hide();
    }
}

// ---------- Siatka ----------

/// Liczba wierszy i kolumn siatki na monitorze głównym.
fn grid(w: &DesktopWindow, (cw, ch, _): (f32, f32, f32)) -> (i32, i32) {
    let size = w.window().size().to_logical(w.window().scale_factor());
    let rows = (((size.height - 2.0 * MARGIN) / ch).floor() as i32).max(1);
    let cols = (((size.width - 2.0 * MARGIN) / cw).floor() as i32).max(1);
    (rows, cols)
}

/// Elementy pulpitu z dysku i ikony wirtualne („Ten komputer”) na początku.
fn with_specials(mut items: Vec<Entry>, computer: bool) -> Vec<Entry> {
    if computer {
        items.insert(0, Entry { name: "Ten komputer".into(), path: COMPUTER.into(), dir: true, size: 0, modified: 0, created: 0, ext: String::new(), hidden: false, system: true, readonly: true, link: false, cloud: false });
    }
    items
}

fn virtual_item(path: &str) -> bool {
    path.starts_with("::")
}

fn key_of(path: &str) -> String {
    path.to_lowercase()
}

/// Ręcznie ustawione ikony na swoich miejscach, pozostałe – w wolne pola kolumnami.
fn layout(items: &[Entry], saved: &std::collections::HashMap<String, (i32, i32)>, rows: i32, cols: i32) -> Vec<(i32, i32)> {
    let mut taken: HashSet<(i32, i32)> = HashSet::new();
    let mut cells: Vec<Option<(i32, i32)>> = items
        .iter()
        .map(|e| {
            let c = saved.get(&key_of(&e.path)).copied().filter(|&(c, r)| c >= 0 && r >= 0 && c < cols && r < rows)?;
            taken.insert(c).then_some(c)
        })
        .collect();
    let mut next = 0i32;
    for c in cells.iter_mut().filter(|c| c.is_none()) {
        while taken.contains(&(next / rows, next % rows)) {
            next += 1;
        }
        let cell = (next / rows, next % rows);
        taken.insert(cell);
        *c = Some(cell);
    }
    cells.into_iter().map(|c| c.unwrap_or((0, 0))).collect()
}

/// Nazwa jak na pulpicie Windows: skróty bez .lnk / .url.
fn label(e: &Entry) -> String {
    let low = e.name.to_lowercase();
    if !e.dir && (low.ends_with(".lnk") || low.ends_with(".url")) {
        e.name[..e.name.len() - 4].to_string()
    } else {
        e.name.clone()
    }
}

fn render(s: &Shared) {
    let (saved, xy, size, free, computer) = {
        let b = s.borrow();
        let p = &b.prefs;
        (p.desktop_pos.clone(), p.desktop_xy.clone(), p.desktop_icon_size, p.desktop_free, p.desktop_computer)
    };
    D.with(|d| {
        let mut d = d.borrow_mut();
        let Some(w) = d.windows.first().map(|w| w.clone_strong()) else { return };
        // „Ten komputer” włączany i wyłączany w ustawieniach – bez czytania pulpitu od nowa.
        let has = d.items.first().is_some_and(|e| e.path == COMPUTER);
        if has != computer {
            let items: Vec<Entry> = std::mem::take(&mut d.items).into_iter().filter(|e| !virtual_item(&e.path)).collect();
            d.items = with_specials(items, computer);
        }
        let cell = dims(size);
        d.cell = cell;
        d.free = free;
        w.set_cell_w(cell.0);
        w.set_cell_h(cell.1);
        w.set_icon_size(cell.2);
        let (rows, cols) = grid(&w, cell);
        d.cells = layout(&d.items, &saved, rows, cols);
        // Ułożenie dowolne: zapamiętane miejsce w pikselach, a bez niego – pole siatki.
        let area = w.window().size().to_logical(w.window().scale_factor());
        d.spots = d
            .items
            .iter()
            .zip(d.cells.iter())
            .map(|(e, &(c, r))| {
                let grid_at = (MARGIN + c as f32 * cell.0, MARGIN + r as f32 * cell.1);
                if !free {
                    return grid_at;
                }
                xy.get(&key_of(&e.path)).map_or(grid_at, |&(x, y)| (x.clamp(0.0, (area.width - cell.0).max(0.0)), y.clamp(0.0, (area.height - cell.1).max(0.0))))
            })
            .collect();
        let px = (cell.2 * w.window().scale_factor()) as u32;
        let icons: Vec<DeskIcon> = d
            .items
            .iter()
            .zip(d.cells.iter())
            .zip(d.spots.iter())
            .map(|((e, &(col, row)), &(x, y))| {
                // Zdjęcia i filmy z miniaturą, reszta (skróty, dokumenty) – ikona systemowa.
                let img = if virtual_item(&e.path) { None } else { crate::thumbs::get(&e.path, px, !fmt::is_media(e)) };
                DeskIcon {
                    name: if d.renaming.as_deref() == Some(e.path.as_str()) { e.name.clone() } else { label(e) }.into(),
                    path: e.path.clone().into(),
                    has_img: img.is_some(),
                    img: img.unwrap_or_default(),
                    dir: e.dir,
                    col,
                    row,
                    x,
                    y,
                    sel: d.sel.contains(&e.path),
                    renaming: d.renaming.as_deref() == Some(e.path.as_str()),
                }
            })
            .collect();
        w.set_icons(ModelRc::new(VecModel::from(icons)));
    });
}

/// Miniatury i ikony dochodzą w tle – przerysowujemy, gdy trafiły ikony z pulpitu.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    if is_open() && D.with(|d| d.borrow().items.iter().any(|e| paths.contains(&e.path))) {
        crate::with_state(render);
    }
}

/// Lista elementów od nowa (po zmianach na dysku); zaznaczenie zostaje dla tego, co nadal jest.
fn refresh() {
    std::thread::spawn(|| {
        let items = data::items();
        let _ = slint::invoke_from_event_loop(move || {
            D.with(|d| {
                let mut d = d.borrow_mut();
                let have: HashSet<String> = items.iter().map(|e| e.path.clone()).collect();
                d.sel.retain(|p| have.contains(p) || virtual_item(p));
                let computer = d.items.first().is_some_and(|e| e.path == COMPUTER);
                d.items = with_specials(items, computer);
            });
            crate::with_state(render);
        });
    });
}

/// Odświeżenie po operacji w tle (kopiowanie, Kosz) – zanim się skończy, mija chwila.
fn refresh_soon() {
    for ms in [400, 1500, 4000] {
        slint::Timer::single_shot(Duration::from_millis(ms), refresh);
    }
}

// ---------- Zaznaczanie i przesuwanie ----------

/// Zaznaczone pliki i foldery (bez ikon wirtualnych – tych nie kopiujemy ani nie usuwamy).
fn selected(d: &Desk) -> Vec<String> {
    d.items.iter().filter(|e| d.sel.contains(&e.path) && !virtual_item(&e.path)).map(|e| e.path.clone()).collect()
}

fn select(s: &Shared, i: i32, ctrl: bool, shift: bool) {
    D.with(|d| {
        let mut d = d.borrow_mut();
        let d = &mut *d;
        if i < 0 {
            d.sel.clear();
            d.anchor = None;
            return;
        }
        let i = i as usize;
        let Some(path) = d.items.get(i).map(|e| e.path.clone()) else { return };
        match (ctrl, shift, d.anchor) {
            // Zakres w kolejności siatki (kolumnami, jak się czyta pulpit).
            (_, true, Some(a)) => {
                let mut order: Vec<usize> = (0..d.items.len()).collect();
                order.sort_by_key(|&k| d.cells.get(k).copied().unwrap_or_default());
                let (pa, pi) = (order.iter().position(|&k| k == a), order.iter().position(|&k| k == i));
                if let (Some(pa), Some(pi)) = (pa, pi) {
                    if !ctrl {
                        d.sel.clear();
                    }
                    for &k in &order[pa.min(pi)..=pa.max(pi)] {
                        d.sel.insert(d.items[k].path.clone());
                    }
                }
                return;
            }
            (true, _, _) => {
                if !d.sel.remove(&path) {
                    d.sel.insert(path);
                }
            }
            _ => {
                d.sel.clear();
                d.sel.insert(path);
            }
        }
        d.anchor = Some(i);
    });
    render(s);
}

/// Ramka: zaznacza ikony, których pola na nią zachodzą (z Ctrl – dodaje do zaznaczenia).
fn band(s: &Shared, (x, y, w, h): (f32, f32, f32, f32), ctrl: bool) {
    D.with(|d| {
        let mut d = d.borrow_mut();
        let d = &mut *d;
        if !ctrl {
            d.sel.clear();
        }
        let (cw, ch, _) = d.cell;
        for (e, &(ix, iy)) in d.items.iter().zip(d.spots.iter()) {
            if ix < x + w && ix + cw > x && iy < y + h && iy + ch > y {
                d.sel.insert(e.path.clone());
            }
        }
    });
    render(s);
}

/// Upuszczenie przeciąganych ikon: przesunięcie o tyle pól, ile wyszło z ruchu myszy.
/// Zajęte pole – najbliższe wolne dalej w kolejności kolumn. Nowe miejsca trafiają do prefs.
fn dropped(s: &Shared, dx: f32, dy: f32) {
    if D.with(|d| d.borrow().free) {
        dropped_free(s, dx, dy);
        return;
    }
    let cell = D.with(|d| d.borrow().cell);
    let (dc, dr) = ((dx / cell.0).round() as i32, (dy / cell.1).round() as i32);
    if dc == 0 && dr == 0 {
        return;
    }
    let moved: Vec<(String, (i32, i32))> = D.with(|d| {
        let d = d.borrow();
        let Some(w) = d.windows.first() else { return Vec::new() };
        let (rows, cols) = grid(w, cell);
        let moving: Vec<usize> = (0..d.items.len()).filter(|&k| d.sel.contains(&d.items[k].path)).collect();
        let mut taken: HashSet<(i32, i32)> = (0..d.items.len()).filter(|k| !moving.contains(k)).map(|k| d.cells[k]).collect();
        let mut out = Vec::new();
        for k in moving {
            let (c, r) = d.cells[k];
            let mut cell = ((c + dc).clamp(0, cols - 1), (r + dr).clamp(0, rows - 1));
            let mut n = cell.0 * rows + cell.1;
            while taken.contains(&cell) && n < rows * cols {
                n += 1;
                cell = (n / rows, n % rows);
            }
            taken.insert(cell);
            out.push((key_of(&d.items[k].path), cell));
        }
        out
    });
    {
        let mut b = s.borrow_mut();
        for (k, c) in moved {
            b.prefs.desktop_pos.insert(k, c);
        }
        b.prefs.save();
    }
    render(s);
}

/// Upuszczenie bez siatki: ikony dokładnie tam, gdzie je puszczono (w granicach ekranu).
fn dropped_free(s: &Shared, dx: f32, dy: f32) {
    if dx.abs() < 2.0 && dy.abs() < 2.0 {
        return;
    }
    let moved: Vec<(String, (f32, f32))> = D.with(|d| {
        let d = d.borrow();
        d.items.iter().zip(d.spots.iter()).filter(|(e, _)| d.sel.contains(&e.path)).map(|(e, &(x, y))| (key_of(&e.path), ((x + dx).max(0.0).round(), (y + dy).max(0.0).round()))).collect()
    });
    {
        let mut b = s.borrow_mut();
        for (k, at) in moved {
            b.prefs.desktop_xy.insert(k, at);
        }
        b.prefs.save();
    }
    render(s);
}

/// „Uporządkuj ikony” – zapomina ręczne miejsca, wszystko kolumnami od lewego górnego rogu.
fn arrange(s: &Shared) {
    {
        let mut b = s.borrow_mut();
        b.prefs.desktop_pos.clear();
        b.prefs.desktop_xy.clear();
        b.prefs.save();
    }
    render(s);
}

/// Ustawienia pulpitu z menu: wyrównanie do siatki, rozmiar ikon, „Ten komputer”.
fn set_desk(s: &Shared, f: impl FnOnce(&mut crate::model::Prefs)) {
    {
        let mut b = s.borrow_mut();
        f(&mut b.prefs);
        b.prefs.save();
    }
    render(s);
}

/// Pulpit po zmianie ustawień w oknie Ustawień.
pub fn relayout() {
    if is_open() {
        crate::with_state(render);
    }
}

// ---------- Polecenia ----------

/// Dwuklik / Enter: folder w oknie PowerDesk, plik i skrót – w swoim programie.
fn open_item(s: &Shared, i: usize) {
    let Some(e) = D.with(|d| d.borrow().items.get(i).cloned()) else { return };
    open_path(s, &e);
}

fn open_path(s: &Shared, e: &Entry) {
    if e.path == COMPUTER {
        crate::open_computer(s);
        crate::show_window(s);
    } else if e.dir {
        crate::open_args(s, vec![e.path.clone()]);
    } else {
        let _ = open::that_detached(&e.path);
    }
}

fn open_selected(s: &Shared) {
    let items: Vec<Entry> = D.with(|d| {
        let d = d.borrow();
        d.items.iter().filter(|e| d.sel.contains(&e.path)).cloned().collect()
    });
    for e in &items {
        open_path(s, e);
    }
}

fn start_rename(s: &Shared) {
    D.with(|d| {
        let mut d = d.borrow_mut();
        let sel = selected(&d);
        if sel.len() == 1 && d.sel.len() == 1 {
            d.renaming = sel.into_iter().next();
        }
    });
    render(s);
}

fn rename_done(s: &Shared, i: usize, name: &str) {
    let Some(e) = D.with(|d| {
        let mut d = d.borrow_mut();
        d.renaming = None;
        d.items.get(i).cloned()
    }) else {
        return;
    };
    if name.trim() != e.name && !name.trim().is_empty() {
        match ops::rename_one(&e.path, name) {
            Ok(new) => {
                // Nowa ścieżka zachowuje miejsce i zaznaczenie starej.
                let mut b = s.borrow_mut();
                if let Some(c) = b.prefs.desktop_pos.remove(&key_of(&e.path)) {
                    b.prefs.desktop_pos.insert(key_of(&new), c);
                }
                if let Some(c) = b.prefs.desktop_xy.remove(&key_of(&e.path)) {
                    b.prefs.desktop_xy.insert(key_of(&new), c);
                }
                b.prefs.save();
                drop(b);
                D.with(|d| {
                    let mut d = d.borrow_mut();
                    d.sel.clear();
                    d.sel.insert(new);
                });
            }
            Err(err) => transfers::toast(s, err, transfers::Kind::Error, None),
        }
    }
    refresh();
}

/// Nowy folder albo dokument na pulpicie – od razu w polu zmiany nazwy, w miejscu kliknięcia.
fn new_item(s: &Shared, folder: bool, at: (f32, f32)) {
    let name = if folder { "Nowy folder" } else { "Nowy dokument tekstowy.txt" };
    match ops::create(&desktop_dir(), name, folder) {
        Ok(path) => {
            let (cw, ch, _) = D.with(|d| d.borrow().cell);
            let cell = (((at.0 - MARGIN) / cw).floor().max(0.0) as i32, ((at.1 - MARGIN) / ch).floor().max(0.0) as i32);
            let empty = D.with(|d| !d.borrow().cells.contains(&cell));
            let mut b = s.borrow_mut();
            if empty {
                b.prefs.desktop_pos.insert(key_of(&path), cell);
            }
            // Bez siatki – dokładnie w miejscu kliknięcia.
            b.prefs.desktop_xy.insert(key_of(&path), ((at.0 - cw / 2.0).max(0.0), (at.1 - 24.0).max(0.0)));
            b.prefs.save();
            drop(b);
            D.with(|d| {
                let mut d = d.borrow_mut();
                d.sel.clear();
                d.sel.insert(path.clone());
                d.renaming = Some(path);
            });
            refresh();
        }
        Err(e) => transfers::toast(s, e, transfers::Kind::Error, None),
    }
}

fn copy(cut: bool) {
    let paths = D.with(|d| selected(&d.borrow()));
    if !paths.is_empty() {
        let _ = clip::set_files(&paths, cut);
    }
}

fn paste(s: &Shared) {
    menus::paste(s, Some(&desktop_dir()));
    refresh_soon();
}

fn trash(s: &Shared) {
    let paths = D.with(|d| selected(&d.borrow()));
    if paths.is_empty() {
        return;
    }
    // Pytanie przed Koszem (jeśli włączone) pokazuje się w oknie PowerDesk.
    if s.borrow().prefs.confirm_trash {
        crate::show_window(s);
    }
    menus::trash(s, &paths);
    refresh_soon();
}

fn key(s: &Shared, k: &str, ctrl: bool, _shift: bool) {
    use slint::platform::Key;
    let is = |key: Key| slint::SharedString::from(key) == k;
    if is(Key::Return) {
        open_selected(s);
    } else if is(Key::Delete) {
        trash(s);
    } else if is(Key::F2) {
        start_rename(s);
    } else if ctrl && k.eq_ignore_ascii_case("a") {
        D.with(|d| {
            let mut d = d.borrow_mut();
            let all: Vec<String> = d.items.iter().map(|e| e.path.clone()).collect();
            d.sel.extend(all);
        });
        render(s);
    } else if ctrl && k.eq_ignore_ascii_case("c") {
        copy(false);
    } else if ctrl && k.eq_ignore_ascii_case("x") {
        copy(true);
    } else if ctrl && k.eq_ignore_ascii_case("v") {
        paste(s);
    }
}

// ---------- Menu w stylu PowerDesk ----------

fn show_menu(win: usize, items: Vec<Item>, x: f32, y: f32) {
    let Some(w) = D.with(|d| {
        let mut d = d.borrow_mut();
        d.menu_at = (x, y);
        d.windows.get(win).map(|w| w.clone_strong())
    }) else {
        return;
    };
    menus::show_in(w.window(), items, x, y);
}

fn menu_chosen(s: &Shared, i: usize) {
    let item = D.with(|d| d.borrow_mut().menu.get(i).cloned());
    if let Some(item) = item {
        menus::run(s, &item);
    }
}

/// Menu pustego miejsca na monitorze `win` (0 – główny).
fn background_menu(s: &Shared, win: usize, x: f32, y: f32) {
    let at = (x, y);
    let (free, size, computer) = {
        let b = s.borrow();
        (b.prefs.desktop_free, b.prefs.desktop_icon_size, b.prefs.desktop_computer)
    };
    let can_paste = clip::has_files();
    let mut v = vec![
        it("Nowy folder", "folderPlus", move |s| new_item(s, true, at)),
        it("Nowy dokument tekstowy", "filePlus", move |s| new_item(s, false, at)),
        sep(),
    ];
    if can_paste {
        v.push(it("Wklej", "paste", paste).kbd("Ctrl+V"));
    }
    v.extend([
        it("Odśwież", "refresh", |_| refresh()).kbd("F5"),
        menus::sub(
            "Widok",
            "grid",
            vec![
                it("Duże ikony", "grid", |s| set_desk(s, |p| p.desktop_icon_size = 2)).checked(size == 2),
                it("Średnie ikony", "grid", |s| set_desk(s, |p| p.desktop_icon_size = 1)).checked(size == 1),
                it("Małe ikony", "grid", |s| set_desk(s, |p| p.desktop_icon_size = 0)).checked(size == 0),
                sep(),
                it("Wyrównaj ikony do siatki", "grid", |s| set_desk(s, |p| p.desktop_free = !p.desktop_free)).checked(!free),
                it("Pokaż „Ten komputer”", "pc", |s| set_desk(s, |p| p.desktop_computer = !p.desktop_computer)).checked(computer),
            ],
        ),
        it("Uporządkuj ikony", "grid", arrange),
        menus::sub(
            "Dodaj widżet",
            "plus",
            crate::widgets::GROUPS
                .iter()
                .map(|&(group, gicon, kinds)| {
                    let list = kinds.iter().filter_map(|k| crate::widgets::kind(k)).map(|(kind, name, icon, _, _)| it(name, icon, move |s| add_widget(s, win, kind, at.0, at.1))).collect();
                    menus::sub(group, gicon, list)
                })
                .collect(),
        ),
        sep(),
        it("Otwórz folder Pulpit", "folder", |s| crate::open_args(s, vec![desktop_dir()])),
        menus::sub("Tapeta", "image", {
            let mut t = vec![it("Własny obraz na tym monitorze…", "image", move |s| {
                if let Some(p) = powerdesk_core::shell::pick_image(crate::hwnd(s), "Tapeta tego monitora") {
                    crate::wallpapers::set_for_monitor(win, Some(p));
                }
            })];
            if crate::wallpapers::has_own(win) {
                t.push(it("Tapeta jak na pozostałych", "refresh", move |_| crate::wallpapers::set_for_monitor(win, None)));
            }
            if s_mode() == "slideshow" {
                t.push(it("Następna tapeta", "fwd", |_| crate::wallpapers::next()));
            }
            t.push(it("Tapeta i personalizacja Windows", "settings", |_| {
                let _ = open::that_detached("ms-settings:personalization-background");
            }));
            t
        }),
        it("Ustawienia pulpitu", "settings", |s| {
            crate::settings::open_section(s, crate::settings::SECTION_DESKTOP);
            crate::show_window(s);
        }),
    ]);
    show_menu(win, v, x, y);
}

fn s_mode() -> String {
    let mut m = String::new();
    crate::with_state(|s| m = s.borrow().prefs.wallpaper_mode.clone());
    m
}

fn icon_menu(_s: &Shared, x: f32, y: f32) {
    let sel: Vec<Entry> = D.with(|d| {
        let d = d.borrow();
        d.items.iter().filter(|e| d.sel.contains(&e.path)).cloned().collect()
    });
    if sel.is_empty() {
        return;
    }
    if sel.len() == 1 && sel[0].path == COMPUTER {
        let v = vec![
            it("Otwórz", "open", open_selected).kbd("Enter"),
            sep(),
            it("Informacje o systemie", "pc", |_| {
                let _ = open::that_detached("ms-settings:about");
            }),
            it("Zarządzanie komputerem", "settings", |_| {
                let _ = std::process::Command::new("mmc.exe").arg("compmgmt.msc").spawn();
            }),
            it("Ukryj na pulpicie", "eye", |s| set_desk(s, |p| p.desktop_computer = false)),
        ];
        show_menu(0, v, x, y);
        return;
    }
    let sel: Vec<Entry> = sel.into_iter().filter(|e| !virtual_item(&e.path)).collect();
    if sel.is_empty() {
        return;
    }
    let one = (sel.len() == 1).then(|| sel[0].clone());
    let mut v = vec![it("Otwórz", "open", open_selected).kbd("Enter")];
    if let Some(e) = one.as_ref().filter(|e| e.dir) {
        let p = e.path.clone();
        v.push(it("Otwórz w nowej karcie", "plus", move |s| crate::open_args(s, vec![p.clone()])));
    }
    v.extend([
        sep(),
        it("Wytnij", "cut", |_| copy(true)).kbd("Ctrl+X"),
        it("Kopiuj", "copy", |_| copy(false)).kbd("Ctrl+C"),
    ]);
    if one.is_some() {
        v.push(it("Zmień nazwę", "edit", start_rename).kbd("F2"));
    }
    let paths: Vec<String> = sel.iter().map(|e| e.path.clone()).collect();
    let p2 = paths.clone();
    v.extend([
        it("Dodaj na półkę", "pin", move |s| crate::shelf::add(s, &p2)).kbd("Ctrl+Shift+S"),
        sep(),
        it("Do Kosza", "trash", trash).kbd("Del").danger(),
        sep(),
        // Rzadkie polecenia programów (np. „Wyślij do”) – pełne menu Windows.
        it("Więcej opcji…", "more", move |_| windows_menu(&paths)),
    ]);
    show_menu(0, v, x, y);
}

/// Menu Windows w miejscu ostatniego kliknięcia – tylko z „Więcej opcji…”.
fn windows_menu(paths: &[String]) {
    let Some((h, scale, at)) = D.with(|d| {
        let d = d.borrow();
        d.windows.first().map(|w| (crate::window_hwnd(w.window()), w.window().scale_factor(), d.menu_at))
    }) else {
        return;
    };
    if let Ok(true) = powerdesk_core::shell::native_menu(h, paths, (at.0 * scale) as i32, (at.1 * scale) as i32, false) {
        refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(p: &str) -> Entry {
        Entry { name: p.into(), path: p.into(), dir: false, size: 0, modified: 0, created: 0, ext: String::new(), hidden: false, system: false, readonly: false, link: false, cloud: false }
    }

    #[test]
    fn layout_keeps_saved_and_fills_columns() {
        let items = vec![entry("a"), entry("b"), entry("c")];
        let mut saved = std::collections::HashMap::new();
        saved.insert("b".to_string(), (0, 0));
        // b zajmuje pierwsze pole, a i c idą dalej w dół pierwszej kolumny.
        assert_eq!(layout(&items, &saved, 3, 5), vec![(0, 1), (0, 0), (0, 2)]);
        // Miejsce poza siatką (mniejszy monitor) – układ automatyczny.
        saved.insert("b".to_string(), (9, 9));
        assert_eq!(layout(&items, &saved, 3, 5), vec![(0, 0), (0, 1), (0, 2)]);
    }
}
