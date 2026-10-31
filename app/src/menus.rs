//! Menu wyskakujące (ContextMenu.svelte) i ich zawartość: menu elementu, tła, sortowania, okruszków.

use std::path::Path;
use std::rc::Rc;

use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::{archive, clip, drives, fsx, ops, shell};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::model::{Col, GroupKey, SortKey, View};
use crate::{icons, MenuEntry, Shared};

pub type Act = Rc<dyn Fn(&Shared)>;

#[derive(Clone)]
pub struct Item {
    label: String,
    icon: &'static str,
    kbd: &'static str,
    checked: bool,
    danger: bool,
    disabled: bool,
    sep: bool,
    act: Option<Act>,
    sub: Vec<Item>,
}

pub fn it(label: impl Into<String>, icon: &'static str, act: impl Fn(&Shared) + 'static) -> Item {
    Item { label: label.into(), icon, kbd: "", checked: false, danger: false, disabled: false, sep: false, act: Some(Rc::new(act)), sub: Vec::new() }
}

pub fn sep() -> Item {
    Item { label: String::new(), icon: "", kbd: "", checked: false, danger: false, disabled: false, sep: true, act: None, sub: Vec::new() }
}

/// Nagłówek grupy w menu (nieczynny napis, np. „Ostatnie” na liście szybkiego dostępu).
pub fn header(label: impl Into<String>) -> Item {
    Item { disabled: true, act: None, ..it(label, "", |_| {}) }
}

pub fn sub(label: &str, icon: &'static str, items: Vec<Item>) -> Item {
    Item { sub: items, act: None, ..it(label, icon, |_| {}) }
}

impl Item {
    pub fn kbd(mut self, k: &'static str) -> Item {
        self.kbd = k;
        self
    }
    pub fn checked(mut self, on: bool) -> Item {
        self.checked = on;
        self
    }
    pub fn danger(mut self) -> Item {
        self.danger = true;
        self
    }
    pub fn is_sep(&self) -> bool {
        self.sep
    }
    /// Pozycja nieczynna w tym miejscu (np. zamknięcie jedynej karty).
    fn disabled(mut self) -> Item {
        self.disabled = true;
        self
    }
    /// Pozycja widoczna jak w wersji webowej, ale jeszcze nieprzeniesiona.
    fn later(mut self) -> Item {
        self.disabled = true;
        self
    }
}

/// Otwarte menu: pozycje i miejsce (do otwierania podmenu obok).
pub struct Open {
    items: Vec<Item>,
    x: f32,
    y: f32,
    width: f32,
}

/// Pozycje dla `MenuPopup`, szerokość i miejsce dopasowane do okna o rozmiarze `size` (logicznym).
/// Wspólne dla głównego okna i innych okien z własnym menu (pulpit).
pub fn layout(items: &[Item], x: f32, y: f32, size: slint::LogicalSize) -> (Vec<MenuEntry>, f32, f32, f32) {
    // Szerokość jak w CSS: min 220, max 380 – według najdłuższej etykiety.
    let longest = items.iter().map(|i| i.label.chars().count() + i.kbd.chars().count()).max().unwrap_or(0);
    let width = (longest as f32 * 7.0 + 96.0).clamp(220.0, 380.0);
    let h: f32 = items.iter().map(|e| if e.sep { 11.0 } else { 32.0 }).sum::<f32>() + 14.0;
    let mx = if x + width > size.width - 8.0 { (size.width - width - 8.0).max(8.0) } else { x };
    let my = if y + h > size.height - 8.0 { (size.height - h - 8.0).max(8.0) } else { y };
    let entries: Vec<MenuEntry> = items
        .iter()
        .map(|i| MenuEntry {
            label: i.label.clone().into(),
            icon: icons::icon(i.icon).into(),
            kbd: i.kbd.into(),
            checked: i.checked,
            sep: i.sep,
            disabled: i.disabled,
            danger: i.danger,
            has_sub: !i.sub.is_empty(),
        })
        .collect();
    (entries, width, mx, my)
}

/// Wykonuje pozycję menu (bez podmenu – te obsługuje tylko główne okno).
pub fn run(s: &Shared, item: &Item) {
    if let Some(act) = &item.act {
        act(s);
    }
}

/// Menu w głównym oknie (x, y – współrzędne logiczne okna) – jako osobne okienko.
pub fn show(s: &Shared, items: Vec<Item>, x: f32, y: f32) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    show_in(ui.window(), items, x, y);
}

// ---------- Menu jako osobne okienko ----------
//
// Wszystkie menu podręczne (okno folderów, pulpit, Start, pasek) to to samo okienko
// `BarMenuWindow` – ten sam wygląd, krycie i rozmycie z ustawień. Podmenu to kolejne okienko
// obok pozycji; menu nadrzędne zostaje widoczne (stos poziomów).

thread_local! {
    static POP: std::cell::RefCell<Vec<Pop>> = const { std::cell::RefCell::new(Vec::new()) };
}

struct Pop {
    w: crate::BarMenuWindow,
    items: Vec<Item>,
    /// Lewy górny róg i szerokość (piksele ekranu) oraz skala.
    at: (i32, i32),
    pw: i32,
    scale: f32,
}

/// Menu w oknie `win` w punkcie (x, y) – współrzędne logiczne tego okna.
pub fn show_in(win: &slint::Window, items: Vec<Item>, x: f32, y: f32) {
    let (p, sc) = (win.position(), win.scale_factor());
    show_at(items, p.x + (x * sc) as i32, p.y + (y * sc) as i32);
}

/// Menu w punkcie ekranu (piksele): w prawo i w dół, przy krawędzi monitora (albo pasku) – w drugą stronę.
pub fn show_at(items: Vec<Item>, sx: i32, sy: i32) {
    open_level(items, sx, sy, 0, None);
}

/// Zamyka poziomy od `level` w górę (0 – wszystkie).
fn truncate(level: usize) {
    let gone: Vec<Pop> = POP.with(|p| {
        let mut p = p.borrow_mut();
        let n = p.len();
        if level < n {
            p.drain(level..).collect()
        } else {
            Vec::new()
        }
    });
    for g in gone {
        let _ = g.w.hide();
    }
}

/// Okienko menu na poziomie `level` (0 – główne). `beside` – lewa krawędź menu nadrzędnego:
/// podmenu, które nie mieści się z prawej, staje po lewej stronie rodzica.
fn open_level(items: Vec<Item>, sx: i32, sy: i32, level: usize, beside: Option<i32>) {
    use slint::winit_030::WinitWindowAccessor;
    truncate(level);
    if items.is_empty() {
        return;
    }
    let Ok(w) = crate::BarMenuWindow::new() else { return };
    crate::settings::popup_theme(&w.global::<crate::Theme>());
    let (entries, width, _, _) = layout(&items, 0.0, 0.0, slint::LogicalSize::new(f32::MAX, f32::MAX));
    let height: f32 = items.iter().map(|i| if i.sep { 11.0 } else { 32.0 }).sum::<f32>() + 14.0;
    let ((mx, my), (mw, mh), scale) = crate::taskbar::monitor_at(sx, sy);
    let (pw, ph) = ((width * scale) as i32, (height * scale) as i32);
    let bottom = my + mh - (crate::taskbar::height() * scale) as i32;
    let x = if sx + pw > mx + mw - 4 {
        match beside {
            Some(left) => (left - pw + (4.0 * scale) as i32).max(mx + 4),
            None => (sx - pw).max(mx + 4),
        }
    } else {
        sx
    };
    let y = if sy + ph > bottom - 4 { if beside.is_some() { (bottom - 4 - ph).max(my + 4) } else { (sy - ph).max(my + 4) } } else { sy };
    w.set_entries(ModelRc::new(VecModel::from(entries)));
    w.set_menu_width(width);
    w.window().set_position(slint::PhysicalPosition::new(x, y));
    w.window().set_size(slint::PhysicalSize::new(pw as u32, ph as u32));
    w.on_chosen(move |i| pop_chosen(level, i as usize));
    // Escape: ten poziom (i głębsze); w głównym – całe menu.
    w.on_dismiss(move || {
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            if level == 0 {
                close_pop();
                crate::start::menu_closed(false);
            } else {
                truncate(level);
                if let Some(h) = POP.with(|p| p.borrow().last().map(|p| crate::window_hwnd(p.w.window()))) {
                    powerdesk_core::taskwin::activate(h);
                }
            }
        })
    });
    // Fokus poza wszystkimi poziomami menu (klik obok) – zamknięcie całości.
    w.window().on_winit_window_event(|_, ev| {
        if let slint::winit_030::winit::event::WindowEvent::Focused(false) = ev {
            slint::Timer::single_shot(std::time::Duration::from_millis(30), || {
                let fg = powerdesk_core::taskwin::foreground();
                if pop_open() && !pop_hwnd().contains(&fg) {
                    close_pop();
                    crate::start::menu_closed(false);
                }
            });
        }
        slint::winit_030::EventResult::Propagate
    });
    let _ = w.show();
    let hwnd = crate::window_hwnd(w.window());
    powerdesk_core::desktop::hide_from_taskbar(hwnd);
    crate::taskbar::popup_look(hwnd);
    powerdesk_core::taskwin::activate(hwnd);
    POP.with(|p| p.borrow_mut().push(Pop { w, items, at: (x, y), pw, scale }));
}

fn pop_chosen(level: usize, i: usize) {
    let Some((item, sub_at, left)) = POP.with(|p| {
        let p = p.borrow();
        let p = p.get(level)?;
        let item = p.items.get(i)?.clone();
        // Podmenu obok pozycji, która je otwiera.
        let off: f32 = p.items[..i].iter().map(|e| if e.sep { 11.0 } else { 32.0 }).sum();
        Some((item, (p.at.0 + p.pw - (4.0 * p.scale) as i32, p.at.1 + (off * p.scale) as i32), p.at.0))
    }) else {
        return;
    };
    if !item.sub.is_empty() {
        open_level(item.sub.clone(), sub_at.0, sub_at.1, level + 1, Some(left));
        return;
    }
    // Najpierw zamykamy menu – polecenie może przenieść fokus na inne okno.
    close_pop();
    crate::with_state(|s| run(s, &item));
    crate::start::menu_closed(true);
    crate::taskbar::refresh_soon();
}

/// Zamyka otwarte menu ze wszystkimi podmenu (bez wyboru).
pub fn close_pop() {
    truncate(0);
}

pub fn pop_open() -> bool {
    POP.with(|p| !p.borrow().is_empty())
}

/// Okna otwartego menu i podmenu (żeby pasek nie brał ich za okna programów).
pub fn pop_hwnd() -> Vec<isize> {
    POP.with(|p| p.borrow().iter().map(|p| crate::window_hwnd(p.w.window())).collect())
}

pub fn chosen(s: &Shared, i: usize) {
    let Some(open) = s.borrow_mut().menu.take() else { return };
    let Some(item) = open.items.get(i).cloned() else { return };
    if !item.sub.is_empty() {
        // Podmenu obok pozycji, która je otwiera.
        let y = open.y + 6.0 + open.items[..i].iter().map(|e| if e.sep { 11.0 } else { 32.0 }).sum::<f32>() - 6.0;
        let s2 = s.clone();
        // Poprzednie okienko musi się zamknąć, zanim pokażemy następne.
        slint::Timer::single_shot(std::time::Duration::from_millis(1), move || show(&s2, item.sub.clone(), open.x + open.width - 4.0, y));
        return;
    }
    if let Some(act) = item.act {
        act(s);
    }
}

// ---------- Zawartość menu ----------

fn selection(s: &Shared) -> Vec<fsx::Entry> {
    let b = s.borrow();
    let t = &b.tabs[b.active];
    t.items.iter().filter(|e| t.selected.contains(&e.path)).cloned().collect()
}

fn paths(v: &[fsx::Entry]) -> Vec<String> {
    v.iter().map(|e| e.path.clone()).collect()
}

/// Menu elementu (FileView.svelte → itemMenu).
/// Pliki, które Windows zwykle umie wydrukować czasownikiem „print”.
const PRINTABLE: [&str; 14] = ["pdf", "txt", "rtf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "jpg", "jpeg", "png", "bmp"];

/// Czasownik powłoki na pliku; błąd – powiadomienie („Nie udało się {co}: …”).
fn verb(s: &Shared, path: &str, name: &str, what: &str) {
    if let Err(e) = shell::invoke_verb(crate::hwnd(s), path, name) {
        crate::transfers::toast(s, format!("Nie udało się {what}: {e}"), crate::transfers::Kind::Error, None);
    }
}

pub fn item_menu(s: &Shared, x: f32, y: f32) {
    let sel = selection(s);
    if sel.is_empty() {
        return;
    }
    let one = (sel.len() == 1).then(|| sel[0].clone());
    let searching = s.borrow().tabs[s.borrow().active].search.is_some();
    let mut v = Vec::new();
    // Dwa foldery – porównanie (różnice w nowej karcie, „Wyrównaj” w powiadomieniu).
    if sel.len() == 2 && sel.iter().all(|e| e.dir) {
        let (a, b) = (sel[0].path.clone(), sel[1].path.clone());
        v.push(it("Porównaj foldery", "split", move |s| crate::comparing::run(s, a.clone(), b.clone())));
        v.push(sep());
    }
    match &one {
        Some(e) if e.dir => {
            let p = e.path.clone();
            v.push(it("Otwórz", "folder", move |s| crate::navigate(s, &p, true)).kbd("Enter"));
            let p = e.path.clone();
            v.push(it("Otwórz w nowej karcie", "plus", move |s| crate::new_tab_at(s, &p, false)).kbd("Ctrl+Enter"));
        }
        Some(e) if crate::archives::browsable(e) => {
            let p = e.path.clone();
            v.push(it("Otwórz jako folder", "archive", move |s| crate::navigate(s, &p, true)).kbd("Enter"));
            let p = e.path.clone();
            v.push(it("Wypakuj tutaj", "download", move |s| crate::archives::extract_here(s, &p)));
            let p = e.path.clone();
            v.push(it("Wypakuj do…", "download", move |s| crate::archives::extract_pick(s, vec![p.clone()])));
            let p = e.path.clone();
            v.push(it("Otwórz w programie", "open", move |_| {
                let _ = open::that_detached(&p);
            }));
        }
        Some(e) => {
            let p = e.path.clone();
            v.push(it("Otwórz", "open", move |s| crate::open_path(s, &p)).kbd("Enter"));
            if crate::fmt::is_media(e) && !crate::archives::inside(&e.path) {
                let p = e.path.clone();
                v.push(it("Podgląd", "eye", move |s| crate::open_path(s, &p)).kbd("Spacja"));
            }
            let p = e.path.clone();
            v.push(it("Otwórz za pomocą…", "app", move |s| {
                shell::open_with(crate::hwnd(s), &p);
            }));
            if ["exe", "bat", "cmd", "msi", "lnk"].iter().any(|x| e.ext.eq_ignore_ascii_case(x)) && !crate::archives::inside(&e.path) {
                let p = e.path.clone();
                v.push(it("Uruchom jako administrator", "shield", move |s| verb(s, &p, "runas", "uruchomić jako administrator")));
            }
            // Obrazy płyt i dysków wirtualnych – podłączenie jako dysk (jak „Zamontuj” w Eksploratorze).
            if ["iso", "img", "vhd", "vhdx"].iter().any(|x| e.ext.eq_ignore_ascii_case(x)) && !crate::archives::inside(&e.path) {
                let p = e.path.clone();
                v.push(it("Zamontuj", "archive", move |s| verb(s, &p, "mount", "zamontować")));
            }
            if PRINTABLE.iter().any(|x| e.ext.eq_ignore_ascii_case(x)) && !crate::archives::inside(&e.path) {
                let p = e.path.clone();
                v.push(it("Drukuj", "open", move |s| verb(s, &p, "print", "wydrukować")));
            }
            if e.ext.eq_ignore_ascii_case("lnk") {
                let p = e.path.clone();
                v.push(it("Otwórz lokalizację pliku", "folder", move |s| shortcut_target(s, &p, false)));
                let p = e.path.clone();
                v.push(it("Otwórz lokalizację w nowej karcie", "plus", move |s| shortcut_target(s, &p, true)));
            }
        }
        None => {}
    }
    if let (true, Some(e)) = (searching, &one) {
        let p = e.path.clone();
        v.push(it("Przejdź do lokalizacji", "folder", move |s| {
            if let Some(parent) = Path::new(&p).parent() {
                crate::navigate(s, &parent.to_string_lossy(), true);
            }
        }));
    }
    // Elementy z archiwum: wypakowanie do folderu, w którym leży archiwum.
    if let Some(dir) = sel.iter().find(|e| crate::archives::inside(&e.path)).and_then(|e| archive::split(&e.path)).and_then(|(arc, _)| arc.parent().map(|d| d.to_string_lossy().to_string())) {
        let ps = paths(&sel);
        v.push(it("Wypakuj obok archiwum", "download", move |s| crate::archives::extract_to(s, ps.clone(), &dir)));
        let ps = paths(&sel);
        v.push(it("Wypakuj do…", "download", move |s| crate::archives::extract_pick(s, ps.clone())));
    }
    v.push(sep());
    let ps = paths(&sel);
    let p2 = ps.clone();
    v.push(it("Wytnij", "cut", move |_| copy(&p2, true)).kbd("Ctrl+X"));
    let p2 = ps.clone();
    v.push(it("Kopiuj", "copy", move |_| copy(&p2, false)).kbd("Ctrl+C"));
    if let Some(e) = one.as_ref().filter(|e| e.dir) {
        let p = e.path.clone();
        v.push(it("Wklej do folderu", "paste", move |s| paste(s, Some(&p))));
    }
    let p2 = ps.clone();
    v.push(
        it("Kopiuj ścieżkę", "link", move |_| {
            let _ = clip::set_text(&p2.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\n"));
        })
        .kbd("Ctrl+Shift+C"),
    );
    v.push(sep());
    if one.is_some() {
        v.push(it("Zmień nazwę", "edit", crate::start_rename).kbd("F2"));
    } else {
        v.push(it(format!("Zmień nazwy ({})…", sel.len()), "edit", crate::bulk::open).kbd("F2"));
    }
    let p2 = ps.clone();
    v.push(it("Spakuj do ZIP…", "archive", move |s| crate::archives::zip(s, &p2)));
    if sel.len() > 1 {
        v.push(it("Przenieś do nowego folderu…", "folderPlus", crate::create_with_selection));
    }
    let p2 = ps.clone();
    v.push(it("Do Kosza", "trash", move |s| trash(s, &p2)).kbd("Del"));
    let p3 = ps.clone();
    v.push(it("Usuń trwale", "trash", move |s| crate::transfers::delete_permanently(s, p3.clone())).kbd("Shift+Del").danger());
    v.push(sep());
    if let Some(e) = one.as_ref().filter(|e| e.dir) {
        let p = e.path.clone();
        let pinned = crate::is_favorite(s, &p);
        v.push(it(if pinned { "Odepnij z panelu bocznego" } else { "Przypnij w panelu bocznym" }, "pin", move |s| crate::toggle_favorite(s, &p)));
        v.push(it("Pokaż na ekranie Komputer", "grid", |_| {}).later());
        let p = e.path.clone();
        v.push(it("Terminal tutaj", "terminal", move |_| terminal(&p)));
    }
    let p2 = ps.clone();
    v.push(it("Dodaj do półki", "archive", move |s| crate::shelf::add(s, &p2)).kbd("Ctrl+Shift+S"));
    let first = ps[0].clone();
    v.push(it("Pokaż w Eksploratorze", "open", move |_| reveal(&first)));
    v.push(sep());
    if let Some(e) = one.as_ref().filter(|e| !crate::archives::inside(&e.path)) {
        let p = e.path.clone();
        v.push(it("Udostępnij…", "link", move |s| verb(s, &p, "Windows.ModernShare", "udostępnić")));
    }
    v.push(it("Właściwości", "info", crate::props::open).kbd("Alt+Enter"));
    if let Some(e) = one.as_ref().filter(|e| !crate::archives::inside(&e.path)) {
        let p = e.path.clone();
        v.push(it("Uprawnienia (Zabezpieczenia)…", "settings", move |s| {
            shell::show_properties_page(crate::hwnd(s), &p, "Zabezpieczenia");
        }));
        // Kopie z punktów przywracania i Historii plików – zakładka Windows (przywracanie i podgląd).
        let p = e.path.clone();
        v.push(it("Przywróć poprzednie wersje…", "history", move |s| {
            shell::show_properties_page(crate::hwnd(s), &p, "Poprzednie wersje");
        }));
    }
    v.push(sep());
    let p2 = ps.clone();
    v.push(it("Pokaż więcej opcji (menu Windows)", "more", move |s| crate::native_menu(s, &p2, x, y)).kbd("Shift+PPM"));
    if let Some(e) = &one {
        let p = e.path.clone();
        v.push(it("Właściwości systemowe", "settings", move |s| {
            shell::show_properties(crate::hwnd(s), &p);
        }));
    }
    show(s, v, x, y);
}

/// Menu tła folderu (FileView.svelte → bgMenu).
pub fn background_menu(s: &Shared, x: f32, y: f32) {
    let path = s.borrow().tabs[s.borrow().active].path.clone();
    let mut v = vec![
        it("Nowy folder", "folderPlus", |s| crate::create(s, true)).kbd("Ctrl+Shift+N"),
        it("Nowy plik tekstowy", "filePlus", |s| crate::create(s, false)),
        it("Wklej", "paste", |s| paste(s, None)).kbd("Ctrl+V"),
        sep(),
        sub(
            "Widok",
            "grid",
            vec![
                it("Lista", "list", |s| crate::set_view(s, View::List)).kbd("Ctrl+1"),
                it("Siatka", "grid", |s| crate::set_view(s, View::Grid)).kbd("Ctrl+2"),
                it("Galeria", "gallery", |s| crate::set_view(s, View::Gallery)).kbd("Ctrl+3"),
            ],
        ),
    ];
    v.push(sort_sub(s));
    v.push(group_sub(s));
    v.push(it("Odśwież", "refresh", |s| crate::load(s, true)).kbd("F5"));
    v.push(it("Zaznacz wszystko", "check", crate::select_all).kbd("Ctrl+A"));
    v.push(sep());
    if !path.starts_with("ofp:") {
        let p = path.clone();
        let pinned = crate::is_favorite(s, &p);
        v.push(it(if pinned { "Odepnij ten folder" } else { "Przypnij ten folder" }, "pin", move |s| crate::toggle_favorite(s, &p)));
    }
    let p = path.clone();
    v.push(it("Terminal tutaj", "terminal", move |_| terminal(&p)));
    v.push(it("Zajętość i porządki", "grid", crate::spaceview::open_here));
    let p = path.clone();
    v.push(it("Otwórz w Eksploratorze", "open", move |_| {
        let _ = open::that_detached(&p);
    }));
    v.push(sep());
    v.push(it("Właściwości folderu", "info", crate::props::open_folder));
    v.push(sep());
    v.push(it("Pokaż więcej opcji (menu Windows)", "more", move |s| crate::native_menu(s, &[path.clone()], x, y)));
    show(s, v, x, y);
}

/// „Sortuj według” – strzałka przy bieżącym kluczu jak w sortMenuItems.
fn sort_sub(s: &Shared) -> Item {
    let (sort, desc) = {
        let b = s.borrow();
        let t = &b.tabs[b.active];
        (t.sort, t.desc)
    };
    let items = SortKey::ALL
        .iter()
        .map(|&k| {
            let mark = if k == sort { if desc { "↓ " } else { "↑ " } } else { "   " };
            it(format!("{mark}{}", k.label()), "", move |s| crate::set_sort(s, k))
        })
        .collect();
    sub("Sortuj według", "sort", items)
}

/// „Grupuj według” – ptaszek przy bieżącym grupowaniu.
fn group_sub(s: &Shared) -> Item {
    let g = crate::tab_mut(s, |t| t.group);
    let items = GroupKey::ALL.iter().map(|&k| it(k.label(), "", move |s| crate::set_group(s, k)).checked(k == g)).collect();
    sub("Grupuj według", "list", items)
}

/// Menu przycisku sortowania na pasku narzędzi.
pub fn sort_menu(s: &Shared, x: f32, y: f32) {
    let (sort, desc, ff) = {
        let b = s.borrow();
        let t = &b.tabs[b.active];
        (t.sort, t.desc, b.prefs.folders_first)
    };
    let mut v: Vec<Item> = SortKey::ALL
        .iter()
        .map(|&k| {
            it(k.label(), "", move |s| {
                crate::tab_mut(s, |t| t.sort = k);
                crate::save_prefs(s);
                crate::refresh(s);
            })
            .checked(k == sort)
        })
        .collect();
    v.push(sep());
    v.push(
        it("Malejąco", "", |s| {
            crate::tab_mut(s, |t| t.desc = !t.desc);
            crate::save_prefs(s);
            crate::refresh(s);
        })
        .checked(desc),
    );
    v.push(
        it("Foldery na górze", "", |s| {
            {
                let mut b = s.borrow_mut();
                b.prefs.folders_first = !b.prefs.folders_first;
                b.prefs.save();
            }
            crate::refresh(s);
        })
        .checked(ff),
    );
    v.push(sep());
    v.push(group_sub(s));
    show(s, v, x, y);
}

pub fn new_menu(s: &Shared, x: f32, y: f32) {
    let file = |label: &str, icon: &'static str, name: &'static str| it(label, icon, move |s| crate::create_named(s, name, false));
    let n = crate::selected_paths(s).len();
    let mut v = vec![
        it("Folder", "folderPlus", |s| crate::create(s, true)).kbd("Ctrl+Shift+N"),
        it("Folder z dzisiejszą datą", "folderPlus", |s| {
            crate::create_named(s, &chrono::Local::now().format("%Y-%m-%d").to_string(), true)
        }),
    ];
    if n > 0 {
        v.push(it(format!("Folder z zaznaczonych ({n})"), "folderPlus", crate::create_with_selection));
    }
    v.push(sep());
    v.push(file("Plik tekstowy", "filePlus", "Nowy dokument tekstowy.txt"));
    v.push(sub(
        "Dokumenty",
        "doc",
        vec![
            file("Dokument Word", "doc", "Nowy dokument.docx"),
            file("Arkusz Excel", "sheet", "Nowy arkusz.xlsx"),
            file("Prezentacja PowerPoint", "doc", "Nowa prezentacja.pptx"),
            file("Notatka Markdown", "doc", "Notatka.md"),
            file("Plik CSV", "sheet", "Dane.csv"),
        ],
    ));
    v.push(sub(
        "Pliki programisty",
        "code",
        vec![
            file("JSON", "code", "plik.json"),
            file("HTML", "code", "index.html"),
            file("Skrypt PowerShell", "terminal", "skrypt.ps1"),
            file("Skrypt wsadowy (.bat)", "terminal", "skrypt.bat"),
            file("Inny plik…", "filePlus", "nowy plik"),
        ],
    ));
    // Typy zarejestrowane przez programy (jak „Nowy” w Eksploratorze).
    let types = crate::shellnew_types();
    if !types.is_empty() {
        v.push(sub(
            "Inne typy",
            "filePlus",
            types.into_iter().map(|t| it(t.name.clone(), "filePlus", move |s| crate::create_typed(s, &t))).collect(),
        ));
    }
    if n > 0 {
        v.push(sep());
        v.push(it(format!("Archiwum ZIP z zaznaczonych ({n})"), "archive", |s| crate::archives::zip(s, &crate::selected_paths(s))));
    }
    show(s, v, x, y);
}

/// Podfoldery okruszka.
/// Strzałka przy ikonie paska adresu – szybkie miejsca jak w Eksploratorze.
pub const PLACES: &str = "ofp:places";

fn places_menu(s: &Shared, x: f32, y: f32) {
    let mut v = vec![it("Komputer", "pc", |s| crate::navigate(s, crate::model::HOME, true))];
    for (name, dir, icon) in [
        ("Pulpit", dirs::desktop_dir(), "desktop"),
        ("Folder osobisty", dirs::home_dir(), "home"),
        ("Dokumenty", dirs::document_dir(), "doc"),
        ("Pobrane", dirs::download_dir(), "download"),
        ("Obrazy", dirs::picture_dir(), "image"),
        ("Muzyka", dirs::audio_dir(), "music"),
        ("Wideo", dirs::video_dir(), "video"),
    ] {
        if let Some(p) = dir.filter(|d| d.is_dir()) {
            let p = p.to_string_lossy().to_string();
            v.push(it(name, icon, move |s| crate::navigate(s, &p, true)));
        }
    }
    let drives: Vec<_> = s.borrow().drives.iter().filter(|d| d.online).map(|d| (if d.label.is_empty() { format!("Dysk ({})", d.letter) } else { format!("{} ({})", d.label, d.letter) }, d.path.clone(), d.kind == "network")).collect();
    if !drives.is_empty() {
        v.push(sep());
    }
    for (name, p, net) in drives {
        v.push(it(name, if net { "network" } else { "drive" }, move |s| crate::navigate(s, &p, true)));
    }
    show(s, v, x, y);
}

pub fn crumb_menu(s: &Shared, path: &str, x: f32, y: f32) {
    if path == PLACES {
        return places_menu(s, x, y);
    }
    let mut dirs: Vec<_> = fsx::list_dir(path).unwrap_or_default().into_iter().filter(|e| e.dir && !(e.hidden || e.system)).collect();
    dirs.sort_by(|a, b| crate::model::natural(&a.name, &b.name));
    dirs.truncate(60);
    if dirs.is_empty() {
        return;
    }
    let v = dirs
        .into_iter()
        .map(|e| {
            let p = e.path.clone();
            it(e.name, "folder", move |s| crate::navigate(s, &p, true))
        })
        .collect();
    show(s, v, x, y);
}

// ---------- Akcje ----------

pub fn copy(paths: &[String], cut: bool) {
    let _ = clip::set_files(paths, cut);
}

/// Wklejenie plików ze schowka (kopiowanie/przenoszenie w tle przez `ops`).
pub fn paste(s: &Shared, into: Option<&str>) {
    let dest = match into {
        Some(p) => p.to_string(),
        None => s.borrow().tabs[s.borrow().active].path.clone(),
    };
    if dest.starts_with("ofp:") {
        return;
    }
    let (files, cut) = clip::get_files();
    if files.is_empty() {
        return;
    }
    crate::transfers::transfer(s, files, &dest, if cut { "move" } else { "copy" });
    // Wycięte pliki po wklejeniu nie zostają w schowku.
    if cut {
        let _ = clip::set_files(&[], false);
    }
}

pub fn trash(s: &Shared, paths: &[String]) {
    if paths.is_empty() || crate::archives::read_only(s, paths) {
        return;
    }
    let paths = paths.to_vec();
    if !s.borrow().prefs.confirm_trash {
        ops::start(Core, "trash".into(), paths, String::new(), ops::Conflict::Skip);
        return;
    }
    let what = if paths.len() == 1 {
        format!("„{}”", Path::new(&paths[0]).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
    } else {
        format!("{} elementów", paths.len())
    };
    crate::transfers::confirm(
        s,
        "Przenieść do Kosza?",
        &format!("{what} trafi do Kosza – można je stamtąd przywrócić."),
        "Do Kosza",
        false,
        std::rc::Rc::new(move |_| {
            ops::start(Core, "trash".into(), paths.clone(), String::new(), ops::Conflict::Skip);
        }),
    );
}

fn terminal(path: &str) {
    use std::os::windows::process::CommandExt;
    let wt = std::process::Command::new("wt").arg("-d").arg(path).creation_flags(0x0800_0000).spawn();
    if wt.is_err() {
        let _ = std::process::Command::new("powershell").arg("-NoExit").current_dir(path).spawn();
    }
}

fn reveal(path: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{path}\"")).spawn();
}

/// Skrót (.lnk): przechodzi do folderu z celem i zaznacza go.
fn shortcut_target(s: &Shared, lnk: &str, new_tab: bool) {
    let target = shell::properties(lnk).into_iter().find(|p| p.key == "System.Link.TargetParsingPath").map(|p| p.value);
    let Some(target) = target.filter(|t| !t.is_empty()) else { return };
    let Some(dir) = Path::new(&target).parent().map(|p| p.to_string_lossy().to_string()) else { return };
    if new_tab {
        crate::new_tab_at(s, &dir, true);
    } else {
        crate::navigate(s, &dir, true);
    }
    crate::select_path(s, &target);
}

/// Wybór kolumn listy (colMenu w FileView.svelte).
pub fn column_menu(s: &Shared, x: f32, y: f32) {
    let (cols, sizes) = {
        let b = s.borrow();
        (b.prefs.list_cols.clone(), b.prefs.folder_sizes)
    };
    let mut v: Vec<Item> = Col::ALL
        .iter()
        .map(|&c| {
            it(c.label(), "", move |s| {
                {
                    let mut b = s.borrow_mut();
                    let cols = &mut b.prefs.list_cols;
                    if let Some(i) = cols.iter().position(|x| *x == c) {
                        cols.remove(i);
                    } else {
                        cols.push(c);
                    }
                    b.prefs.save();
                }
                crate::refresh(s);
            })
            .checked(cols.contains(&c))
        })
        .collect();
    v.push(sep());
    v.push(
        it("Rozmiary folderów", "", |s| {
            {
                let mut b = s.borrow_mut();
                b.prefs.folder_sizes = !b.prefs.folder_sizes;
                b.prefs.save();
            }
            crate::refresh(s);
        })
        .checked(sizes),
    );
    v.push(it("Przywróć domyślne kolumny", "undo", |s| {
        {
            let mut b = s.borrow_mut();
            b.prefs.list_cols = Col::DEFAULT.to_vec();
            b.prefs.list_col_w.clear();
            b.prefs.save();
        }
        crate::refresh(s);
    }));
    show(s, v, x, y);
}

/// Menu zapisanego wyszukiwania w panelu bocznym (savedMenu w Sidebar.svelte).
/// Menu pozycji panelu bocznego (folder, dysk, przypięty folder).
pub fn side_menu(s: &Shared, path: String, x: f32, y: f32) {
    let (a, b, c) = (path.clone(), path.clone(), path.clone());
    let pinned = crate::is_favorite(s, &path);
    let mut v = vec![
        it("Otwórz", "folder", move |s| crate::navigate(s, &a, true)),
        it("Otwórz w nowej karcie", "plus", move |s| crate::new_tab_at(s, &b, false)),
        sep(),
        it(if pinned { "Odepnij" } else { "Przypnij w panelu bocznym" }, "pin", move |s| crate::toggle_favorite(s, &c)),
    ];
    if crate::is_net_place(s, &path) {
        let p = path.clone();
        v.push(it("Usuń lokalizację z panelu", "close", move |s| crate::remove_net_place(s, &p)));
    }
    // Dysk: wysuwanie, odłączanie, oczyszczanie i mapowanie – zależnie od rodzaju.
    let drive = s.borrow().drives.iter().find(|d| d.path.eq_ignore_ascii_case(&path)).cloned();
    if let Some(d) = drive {
        v.push(sep());
        let root = d.path.clone();
        match d.kind.as_str() {
            "removable" | "cdrom" => v.push(it("Wysuń", "eject", move |s| {
                if let Err(e) = drives::eject(crate::hwnd(s), &root) {
                    crate::transfers::toast(s, format!("Nie udało się wysunąć: {e}"), crate::transfers::Kind::Error, None);
                }
            })),
            "network" => v.push(it("Odłącz dysk sieciowy", "unlink", move |s| match drives::unmap(&root, true) {
                Ok(()) => crate::transfers::toast(s, "Dysk sieciowy odłączony", crate::transfers::Kind::Ok, None),
                Err(e) => crate::transfers::toast(s, format!("Nie udało się odłączyć: {e}"), crate::transfers::Kind::Error, None),
            })),
            _ => {}
        }
        let space = d.path.clone();
        v.push(it("Co zajmuje miejsce?", "grid", move |s| crate::navigate(s, &format!("ofp:space|{space}"), true)));
        if d.kind == "fixed" || d.kind == "removable" {
            let l = d.letter.clone();
            v.push(it("Oczyszczanie dysku", "trash", move |_| drives::cleanup(&l)));
            let l = d.letter.clone();
            v.push(it("Formatuj…", "settings", move |s| drives::format_dialog(crate::hwnd(s), &l)).danger());
            v.push(it("Zarządzaj funkcją BitLocker", "settings", |_| drives::bitlocker()));
        }
        let props = d.path.clone();
        v.push(it("Właściwości", "info", move |s| {
            powerdesk_core::shell::show_properties(crate::hwnd(s), &props);
        }));
        v.push(it("Mapuj dysk sieciowy…", "network", |s| drives::map_dialog(crate::hwnd(s))));
    }
    v.push(it("Terminal tutaj", "terminal", move |_| terminal(&path)));
    show(s, v, x, y);
}

pub fn saved_menu(s: &Shared, id: String, x: f32, y: f32) {
    let (a, b, c, d) = (id.clone(), id.clone(), id.clone(), id);
    show(
        s,
        vec![
            it("Uruchom", "search", move |s| crate::run_saved(s, &a, false)),
            it("Uruchom w nowej karcie", "plus", move |s| crate::run_saved(s, &b, true)),
            sep(),
            it("Zmień nazwę", "edit", move |s| crate::rename_saved(s, &d)),
            it("Usuń", "trash", move |s| crate::remove_saved(s, &c)).danger(),
        ],
        x,
        y,
    );
}

/// Menu karty (prawy przycisk na karcie): nowa, duplikat, przypięcie i zamykanie.
pub fn tab_menu(s: &Shared, i: usize, x: f32, y: f32) {
    let (pinned, path, viewer, closable, right) = {
        let b = s.borrow();
        let Some(t) = b.tabs.get(i) else { return };
        (t.pinned, t.path.clone(), t.viewer.is_some(), b.tabs.len() > 1 && !t.pinned, b.tabs.len() > i + 1)
    };
    let others = s.borrow().tabs.iter().enumerate().any(|(j, t)| j != i && !t.pinned);
    let can_reopen = !s.borrow().closed.is_empty();
    let mut v = vec![it("Nowa karta", "plus", |s| crate::new_tab(s)).kbd("Ctrl+T")];
    if !viewer {
        v.push(it("Duplikuj kartę", "copy", move |s| crate::new_tab_at(s, &path, true)));
    }
    v.push(sep());
    if !viewer {
        v.push(it(if pinned { "Odepnij kartę" } else { "Przypnij kartę" }, "pin", move |s| crate::pin_tab(s, i)));
    }
    v.push(it("Przywróć zamkniętą kartę", "refresh", |s| crate::reopen_closed(s)).kbd("Ctrl+Shift+T"));
    if !can_reopen {
        v.pop();
    }
    v.push(sep());
    let mut close = it("Zamknij kartę", "close", move |s| crate::close_tab(s, i)).kbd("Ctrl+W");
    if !closable {
        close = close.disabled();
    }
    v.push(close);
    if others {
        v.push(it("Zamknij pozostałe karty", "close", move |s| crate::close_others(s, i)));
    }
    if right {
        v.push(it("Zamknij karty po prawej", "close", move |s| crate::close_right(s, i)));
    }
    show(s, v, x, y);
}
