#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! PowerDesk – natywny interfejs (Slint) nad backendem `powerdesk-core`.

mod fmt;
mod menus;
mod model;
mod query;
mod thumbs;
mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use model::{Col, GroupKey, KindFilter, Prefs, Search, SortKey, Tab, View, HOME};
use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::{drives, fsx, shell};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

slint::include_modules!();

pub struct State {
    tabs: Vec<Tab>,
    active: usize,
    prefs: Prefs,
    ui: Weak<MainWindow>,
    /// Okno widoku: przesunięcie, szerokość, wysokość (piksele logiczne).
    viewport: (f32, f32, f32),
    /// Metadane zdjęć i filmów: proporcje w galerii, kolumny Wymiary / Data wykonania / Czas trwania.
    media: HashMap<String, shell::MediaInfo>,
    media_pending: HashSet<String>,
    /// Rozmiary folderów policzone w tle (klucz: ścieżka małymi literami).
    dir_sizes: HashMap<String, u64>,
    dir_pending: HashSet<String>,
    menu: Option<menus::Open>,
    /// Dyski – odświeżane po zmianie (podłączenie pendrive'a), nie przy każdym rysowaniu.
    drives: Vec<drives::Drive>,
    /// Zaznaczenie sprzed ramki (z Ctrl/Shift ramka je rozszerza); Some, dopóki trwa przeciąganie.
    marquee: Option<HashSet<String>>,
    /// Komunikat (np. błąd zmiany nazwy) pokazywany chwilowo w pasku stanu.
    notice: Option<String>,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    /// Stan żyje w wątku UI; zdarzenia z backendu trafiają tu przez `invoke_from_event_loop`.
    static STATE: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

pub fn with_state(f: impl FnOnce(&Shared)) {
    let s = STATE.with(|s| s.borrow().clone());
    if let Some(s) = s {
        f(&s)
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    apply_theme(&ui);

    let start = dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "C:\\".into());
    let st: Shared = Rc::new(RefCell::new(State {
        tabs: vec![Tab::new(&start)],
        active: 0,
        prefs: Prefs::load(),
        ui: ui.as_weak(),
        viewport: (0.0, 900.0, 600.0),
        media: HashMap::new(),
        media_pending: HashSet::new(),
        dir_sizes: HashMap::new(),
        dir_pending: HashSet::new(),
        menu: None,
        marquee: None,
        notice: None,
        drives: drives::list(),
    }));
    STATE.with(|s| *s.borrow_mut() = Some(st.clone()));

    powerdesk_core::events::set_sink(|name, payload| {
        let name = name.to_string();
        let _ = slint::invoke_from_event_loop(move || with_state(|s| on_event(s, &name, payload)));
    });
    fsx::init_watcher(Core);
    drives::init_watch(Core);

    wire(&ui, &st);
    load(&st, false);
    ui.run()
}

fn wire(ui: &MainWindow, st: &Shared) {
    ui.on_drag({
        let w = ui.as_weak();
        move || {
            if let Some(ui) = w.upgrade() {
                ui.window().with_winit_window(|win| {
                    let _ = win.drag_window();
                });
            }
        }
    });
    ui.on_minimize({
        let w = ui.as_weak();
        move || {
            if let Some(ui) = w.upgrade() {
                ui.window().set_minimized(true);
            }
        }
    });
    ui.on_toggle_maximize({
        let w = ui.as_weak();
        move || {
            if let Some(ui) = w.upgrade() {
                let m = !ui.window().is_maximized();
                ui.window().set_maximized(m);
                ui.set_is_maximized(m);
            }
        }
    });
    ui.on_close_window(|| {
        let _ = slint::quit_event_loop();
    });

    let s = st.clone();
    ui.on_navigate(move |p| navigate(&s, p.as_str(), true));
    let s = st.clone();
    ui.on_back(move || back(&s));
    let s = st.clone();
    ui.on_forward(move || forward(&s));
    let s = st.clone();
    ui.on_up(move || up(&s));
    let s = st.clone();
    ui.on_refresh(move || load(&s, true));
    let s = st.clone();
    ui.on_open(move |i| open(&s, i as usize));
    let s = st.clone();
    ui.on_select(move |i, ctrl, shift| select(&s, i as usize, ctrl, shift));
    let s = st.clone();
    ui.on_marquee_begin(move |additive| {
        tab_mut(&s, |t| {
            if !additive {
                t.selected.clear();
            }
        });
        let base = tab_mut(&s, |t| t.selected.clone());
        s.borrow_mut().marquee = Some(base);
        render_items(&s);
    });
    let s = st.clone();
    ui.on_marquee_update(move |x0, y0, x1, y1| {
        {
            let mut b = s.borrow_mut();
            let b = &mut *b;
            let Some(base) = &b.marquee else { return };
            let t = &mut b.tabs[b.active];
            let mut sel = base.clone();
            for i in t.in_rect(x0, y0, x1, y1) {
                sel.insert(t.items[i].path.clone());
            }
            if sel == t.selected {
                return;
            }
            t.selected = sel;
        }
        render_items(&s);
    });
    let s = st.clone();
    ui.on_marquee_end(move || s.borrow_mut().marquee = None);
    let s = st.clone();
    ui.on_viewport_changed(move |top, w, h| {
        let relayout = {
            let mut b = s.borrow_mut();
            let changed = (b.viewport.1 - w).abs() > 0.5;
            b.viewport = (top, w, h);
            changed
        };
        if relayout {
            layout(&s);
            render_chrome(&s);
        }
        render_items(&s);
    });
    let s = st.clone();
    ui.on_context(move |i, x, y, shift| context(&s, i as usize, x, y, shift));
    let s = st.clone();
    ui.on_background_context(move |x, y, shift| {
        tab_mut(&s, |t| t.selected.clear());
        render_items(&s);
        let path = tab_mut(&s, |t| t.path.clone());
        if path.starts_with("ofp:") {
            return;
        }
        if shift {
            native_menu(&s, &[path], x, y);
        } else {
            menus::background_menu(&s, x, y);
        }
    });

    // Filtr na żywo i wyszukiwanie w podfolderach.
    let s = st.clone();
    ui.on_filter_changed(move |q| {
        tab_mut(&s, |t| t.filter = q.to_string());
        refresh(&s);
    });
    let s = st.clone();
    ui.on_set_kind(move |k| {
        let f = KindFilter::ALL.get(k as usize).copied().unwrap_or_default();
        tab_mut(&s, |t| t.kind_filter = if t.kind_filter == f { KindFilter::All } else { f });
        set_scroll(&s, 0.0);
        refresh(&s);
    });
    let s = st.clone();
    ui.on_search(move |q| start_search(&s, q.as_str()));
    let s = st.clone();
    ui.on_clear_search(move || clear_search(&s));

    // Sortowanie, widoki, wielkość miniatur.
    let s = st.clone();
    ui.on_sort_menu(move |x, y| menus::sort_menu(&s, x, y));
    let s = st.clone();
    ui.on_header_sort(move |id| {
        let k = match Col::ALL.get(id as usize) {
            Some(c) => match c.sort() {
                Some(k) => k,
                None => return,
            },
            None => SortKey::Name,
        };
        set_sort(&s, k);
    });

    // Kolumny listy: kolejność, szerokości, wybór.
    let s = st.clone();
    ui.on_col_move(move |id, dx| {
        let Some(&c) = Col::ALL.get(id as usize) else { return };
        {
            let mut b = s.borrow_mut();
            b.prefs.move_col(c, dx);
            b.prefs.save();
        }
        render_all(&s);
    });
    let s = st.clone();
    ui.on_col_resize(move |id, w| {
        let Some(&c) = Col::ALL.get(id as usize) else { return };
        s.borrow_mut().prefs.list_col_w.insert(c, w.clamp(48.0, 600.0).round());
        render_all(&s);
    });
    let s = st.clone();
    ui.on_col_resize_end(move || s.borrow().prefs.save());
    let s = st.clone();
    ui.on_col_reset(move |id| {
        let Some(&c) = Col::ALL.get(id as usize) else { return };
        {
            let mut b = s.borrow_mut();
            b.prefs.list_col_w.remove(&c);
            b.prefs.save();
        }
        render_all(&s);
    });
    let s = st.clone();
    ui.on_col_menu(move |x, y| menus::column_menu(&s, x, y));

    // Zmiana nazwy w miejscu.
    let s = st.clone();
    ui.on_rename_commit(move |t| {
        rename_commit(&s, t.as_str());
    });
    let s = st.clone();
    ui.on_rename_cancel(move || {
        tab_mut(&s, |t| t.renaming = None);
        render_items(&s);
    });
    let s = st.clone();
    ui.on_toggle_group(move |i| toggle_group(&s, i as usize));
    let s = st.clone();
    ui.on_select_group(move |i, ctrl| select_group(&s, i as usize, ctrl));
    let s = st.clone();
    ui.on_rename_next(move |t, back| rename_next(&s, t.as_str(), back));
    let s = st.clone();
    ui.on_set_view(move |v| set_view(&s, View::from_index(v)));
    let s = st.clone();
    ui.on_zoom_changed(move |z| set_zoom(&s, z));
    let s = st.clone();
    ui.on_zoom_step(move |d| {
        let (view, g, r) = {
            let b = s.borrow();
            (b.tabs[b.active].view, b.prefs.grid_size, b.prefs.gallery_row)
        };
        match view {
            View::List => {}
            View::Grid => set_zoom(&s, g + 12.0 * d as f32),
            View::Gallery => set_zoom(&s, r + 16.0 * d as f32),
        }
    });
    let s = st.clone();
    ui.on_crumb_menu(move |p, x, y| menus::crumb_menu(&s, p.as_str(), x, y));
    let s = st.clone();
    ui.on_new_menu(move |x, y| menus::new_menu(&s, x, y));
    let s = st.clone();
    ui.on_menu_chosen(move |i| menus::chosen(&s, i as usize));
    let s = st.clone();
    ui.on_command(move |c| command(&s, c.as_str()));
    let s = st.clone();
    ui.on_key(move |k, ctrl, shift, alt| key(&s, k.as_str(), ctrl, shift, alt));

    // Karty.
    let s = st.clone();
    ui.on_new_tab(move || new_tab(&s));
    let s = st.clone();
    ui.on_activate_tab(move |i| {
        save_scroll(&s);
        s.borrow_mut().active = i as usize;
        show_tab(&s);
    });
    let s = st.clone();
    ui.on_close_tab(move |i| close_tab(&s, i as usize));
}

// ---------- Karty i nawigacja ----------

fn tab_mut<R>(s: &Shared, f: impl FnOnce(&mut Tab) -> R) -> R {
    let mut b = s.borrow_mut();
    let a = b.active;
    f(&mut b.tabs[a])
}

fn new_tab(s: &Shared) {
    let p = tab_mut(s, |t| t.path.clone());
    new_tab_at(s, &p, true);
}

/// Nowa karta obok bieżącej; w tle (`activate == false`) jak „Otwórz w nowej karcie”.
fn new_tab_at(s: &Shared, path: &str, activate: bool) {
    save_scroll(s);
    let prev = s.borrow().active;
    {
        let mut b = s.borrow_mut();
        let at = b.active + 1;
        b.tabs.insert(at, Tab::new(path));
        b.active = at;
    }
    apply_prefs(s);
    load(s, false);
    if !activate {
        s.borrow_mut().active = prev;
        show_tab(s);
    }
}

fn close_tab(s: &Shared, i: usize) {
    {
        let mut b = s.borrow_mut();
        if b.tabs.len() == 1 {
            drop(b);
            let _ = slint::quit_event_loop();
            return;
        }
        b.tabs.remove(i);
        if b.active >= b.tabs.len() || b.active > i {
            b.active = b.active.saturating_sub(1);
        }
    }
    show_tab(s);
}

fn save_scroll(s: &Shared) {
    let top = s.borrow().viewport.0;
    tab_mut(s, |t| t.scroll = top);
}

/// Przełączenie karty: zawartość jest już wczytana, tylko odświeżamy widok.
fn show_tab(s: &Shared) {
    let (scroll, filter) = tab_mut(s, |t| (t.scroll, t.filter.clone()));
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_query(filter.into());
    }
    layout(s);
    render_all(s);
    set_scroll(s, scroll);
    watch(s);
}

fn navigate(s: &Shared, path: &str, push: bool) {
    let mut path = path.trim().trim_matches('"').to_string();
    if path.is_empty() {
        return;
    }
    if path.len() == 2 && path.ends_with(':') {
        path.push('\\');
    }
    if !path.starts_with("ofp:") && !Path::new(&path).is_dir() {
        // Ścieżka do pliku: otwieramy jego folder.
        match Path::new(&path).parent().filter(|p| p.is_dir()) {
            Some(parent) => path = parent.to_string_lossy().to_string(),
            None => return,
        }
    }
    let prev = tab_mut(s, |t| {
        if t.search.as_ref().map(|x| x.running).unwrap_or(false) {
            fsx::cancel_search();
        }
        if push {
            t.history.truncate(t.h_index + 1);
            t.history.push(path.clone());
            t.h_index = t.history.len() - 1;
        }
        t.remember_selection();
        t.renaming = None;
        let prev = std::mem::replace(&mut t.path, path.clone());
        t.search = None;
        t.filter.clear();
        t.kind_filter = KindFilter::All;
        t.selected.clear();
        t.focus = None;
        t.anchor = None;
        prev
    });
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_query(SharedString::new());
    }
    apply_prefs(s);
    load(s, false);
    // Po wejściu „w górę” zaznaczamy folder, z którego przyszliśmy – jak w Eksploratorze.
    let came_up = Path::new(&prev).parent().map(|p| same(&p.to_string_lossy(), &path)).unwrap_or(false);
    if came_up {
        let i = tab_mut(s, |t| {
            let i = t.items.iter().position(|e| same(&e.path, &prev));
            if let Some(i) = i {
                t.selected.insert(t.items[i].path.clone());
                t.focus = Some(i);
                t.anchor = Some(i);
            }
            i
        });
        if let Some(i) = i {
            reveal(s, i);
        }
        render_items(s);
    } else if let Some(i) = tab_mut(s, |t| t.restore_selection()) {
        // Powrót do folderu przywraca poprzednie zaznaczenie.
        reveal(s, i);
        render_items(s);
    }
}

fn back(s: &Shared) {
    if tab_mut(s, |t| t.search.is_some()) {
        return clear_search(s);
    }
    let i = tab_mut(s, |t| t.h_index);
    if i > 0 {
        jump(s, i - 1);
    }
}

fn forward(s: &Shared) {
    let (i, n) = tab_mut(s, |t| (t.h_index, t.history.len()));
    if i + 1 < n {
        jump(s, i + 1);
    }
}

fn up(s: &Shared) {
    let p = tab_mut(s, |t| t.path.clone());
    if let Some(parent) = Path::new(&p).parent() {
        navigate(s, &parent.to_string_lossy(), true);
    }
}

fn jump(s: &Shared, idx: usize) {
    let p = tab_mut(s, |t| {
        t.h_index = idx;
        t.history[idx].clone()
    });
    navigate(s, &p, false);
}

fn same(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

/// Widok i sortowanie zapamiętane dla folderu.
fn apply_prefs(s: &Shared) {
    let mut b = s.borrow_mut();
    let a = b.active;
    let p = b.prefs.folders.get(&b.tabs[a].path.to_lowercase()).copied().unwrap_or_default();
    let t = &mut b.tabs[a];
    t.view = p.view;
    t.sort = p.sort;
    t.desc = p.desc;
    t.group = p.group;
    t.collapsed.clear();
}

fn save_prefs(s: &Shared) {
    let mut b = s.borrow_mut();
    let a = b.active;
    let t = &b.tabs[a];
    if t.special() || t.search.is_some() {
        return;
    }
    let (key, fp) = (t.path.to_lowercase(), model::FolderPrefs { view: t.view, sort: t.sort, desc: t.desc, group: t.group });
    b.prefs.folders.insert(key, fp);
    b.prefs.save();
}

fn load(s: &Shared, keep_selection: bool) {
    let path = tab_mut(s, |t| {
        t.generation += 1;
        t.path.clone()
    });
    let res = fsx::list_dir(&path);
    let auto = {
        let b = s.borrow();
        b.prefs.auto_gallery && !b.prefs.folders.contains_key(&path.to_lowercase())
    };
    tab_mut(s, |t| {
        match res {
            Ok(list) => {
                t.entries = list;
                t.error.clear();
            }
            Err(e) => {
                t.entries.clear();
                t.error = e;
            }
        }
        if !keep_selection {
            t.selected.clear();
            t.focus = None;
            t.anchor = None;
            // Folder pełen zdjęć/filmów od razu otwiera się w galerii (o ile nie ustawiono inaczej).
            if auto && t.entries.len() >= 6 {
                let files: Vec<_> = t.entries.iter().filter(|e| !e.dir).collect();
                let media = files.iter().filter(|e| fmt::is_media(e)).count();
                if !files.is_empty() && media as f32 / files.len() as f32 >= 0.7 {
                    t.view = View::Gallery;
                }
            }
        }
    });
    refresh(s);
    if !keep_selection {
        set_scroll(s, 0.0);
    }
    watch(s);
}

/// Obserwujemy foldery wszystkich otwartych kart.
fn watch(s: &Shared) {
    let b = s.borrow();
    let mut dirs: Vec<String> = b.tabs.iter().filter(|t| !t.special()).map(|t| t.path.clone()).collect();
    dirs.sort();
    dirs.dedup();
    fsx::watch_dirs(dirs);
}

/// Filtr/sortowanie → układ → widok.
fn refresh(s: &Shared) {
    {
        let mut b = s.borrow_mut();
        let b = &mut *b;
        let (hidden, ff) = (b.prefs.show_hidden, b.prefs.folders_first);
        let (sizes, media) = (&b.dir_sizes, &b.media);
        b.tabs[b.active].rebuild(hidden, ff, |e| size_of(sizes, e), |e| media.get(&e.path).copied());
    }
    request_media(s);
    request_dir_sizes(s);
    layout(s);
    render_all(s);
}

fn layout(s: &Shared) {
    let mut b = s.borrow_mut();
    let b = &mut *b;
    let (w, g, r) = (b.viewport.1, b.prefs.grid_size, b.prefs.gallery_row);
    let media = &b.media;
    let t = &mut b.tabs[b.active];
    t.layout(w, g, r, |e| ratio_of(media, e));
}

/// Jak `ratioOf` w FileView.svelte.
fn ratio_of(media: &HashMap<String, shell::MediaInfo>, e: &fsx::Entry) -> f32 {
    if e.dir || !fmt::is_media(e) {
        return 1.0;
    }
    match media.get(&e.path) {
        None => 1.5,
        Some(m) if m.w == 0 || m.h == 0 => 1.0,
        Some(m) => (m.w as f32 / m.h as f32).clamp(0.33, 3.2),
    }
}

/// Rozmiar do sortowania – dla folderu policzony w tle albo -1 (jak `sizeOf` w state.svelte.ts).
fn size_of(sizes: &HashMap<String, u64>, e: &fsx::Entry) -> i64 {
    if e.dir {
        sizes.get(&e.path.to_lowercase()).map(|n| *n as i64).unwrap_or(-1)
    } else {
        e.size as i64
    }
}

/// Metadane zdjęć i filmów – partiami w tle (galeria albo kolumny z metadanymi), potem ponowny układ.
fn request_media(s: &Shared) {
    let todo: Vec<(String, i64)> = {
        let mut b = s.borrow_mut();
        let b = &mut *b;
        let t = &b.tabs[b.active];
        let cols = t.view == View::List && b.prefs.list_cols.iter().any(|c| c.media());
        if t.view != View::Gallery && !cols && !t.sort.media() && t.group != GroupKey::Taken {
            return;
        }
        let v: Vec<(String, i64)> = t
            .items
            .iter()
            .filter(|e| fmt::is_media(e) && !b.media.contains_key(&e.path) && !b.media_pending.contains(&e.path))
            .map(|e| (e.path.clone(), e.modified))
            .collect();
        for (p, _) in &v {
            b.media_pending.insert(p.clone());
        }
        v
    };
    if todo.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for chunk in todo.chunks(200) {
            let info = shell::media_info(chunk.to_vec());
            let res: Vec<(String, shell::MediaInfo)> = chunk.iter().zip(info).map(|((p, _), m)| (p.clone(), m)).collect();
            let _ = slint::invoke_from_event_loop(move || {
                with_state(|s| {
                    {
                        let mut b = s.borrow_mut();
                        for (p, m) in res {
                            b.media_pending.remove(&p);
                            b.media.insert(p, m);
                        }
                    }
                    // Sortowanie po metadanych – kolejność zmienia się wraz z kolejnymi partiami.
                    if tab_mut(s, |t| t.sort.media() || t.group == GroupKey::Taken) {
                        refresh(s);
                    } else {
                        layout(s);
                        render_all(s);
                    }
                })
            });
        }
    });
}

// ---------- Renderowanie ----------

/// Kolejka rozmiarów folderów: nowsze żądania (bieżący folder) wyprzedzają stare, liczą dwa wątki naraz.
static DS_QUEUE: LazyLock<Mutex<std::collections::VecDeque<String>>> = LazyLock::new(Default::default);
static DS_RUNNING: AtomicUsize = AtomicUsize::new(0);

/// Rozmiary folderów – tylko gdy widać kolumnę Rozmiar albo sortujemy po rozmiarze; bez dysków sieciowych.
fn request_dir_sizes(s: &Shared) {
    let todo: Vec<String> = {
        let mut b = s.borrow_mut();
        let b = &mut *b;
        let t = &b.tabs[b.active];
        let shown = (t.view == View::List && b.prefs.list_cols.contains(&Col::Size)) || t.sort == SortKey::Size;
        if !b.prefs.folder_sizes || t.search.is_some() || !shown {
            return;
        }
        let net: Vec<String> = b.drives.iter().filter(|d| d.kind == "network").map(|d| d.path[..2.min(d.path.len())].to_lowercase()).collect();
        let v: Vec<String> = t
            .items
            .iter()
            .filter(|e| e.dir && !e.path.starts_with("\\\\"))
            .map(|e| e.path.to_lowercase())
            .filter(|p| !b.dir_sizes.contains_key(p) && !b.dir_pending.contains(p) && !net.iter().any(|n| p.starts_with(n.as_str())))
            .collect();
        b.dir_pending.extend(v.iter().cloned());
        v
    };
    if todo.is_empty() {
        return;
    }
    {
        let mut q = DS_QUEUE.lock().unwrap();
        for p in todo.into_iter().rev() {
            q.push_front(p);
        }
    }
    while DS_RUNNING.load(Ordering::SeqCst) < 2 {
        DS_RUNNING.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(|| {
            loop {
                let next = DS_QUEUE.lock().unwrap().pop_front();
                let Some(p) = next else { break };
                let size = fsx::dir_size(&p).size;
                let _ = slint::invoke_from_event_loop(move || {
                    with_state(|s| {
                        let by_size = {
                            let mut b = s.borrow_mut();
                            b.dir_pending.remove(&p);
                            b.dir_sizes.insert(p, size);
                            b.tabs[b.active].sort == SortKey::Size
                        };
                        if by_size {
                            refresh(s);
                        } else {
                            render_items(s);
                        }
                    })
                });
            }
            DS_RUNNING.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

fn render_all(s: &Shared) {
    render_chrome(s);
    render_items(s);
}

/// Wszystko poza samymi elementami: karty, pasek narzędzi, panel boczny, stan.
fn render_chrome(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.tabs[b.active];
    ui.set_path(t.path.clone().into());
    ui.set_special(t.special());
    ui.set_can_back(t.h_index > 0 || t.search.is_some());
    ui.set_can_forward(t.h_index + 1 < t.history.len());
    ui.set_crumbs(ModelRc::new(VecModel::from(crumbs(&t.path, &b.drives))));
    ui.set_view(t.view.index());
    ui.set_sort_name(t.sort == SortKey::Name);
    ui.set_desc(t.desc);
    let has_type = b.prefs.list_cols.contains(&Col::Type);
    ui.set_columns(ModelRc::new(VecModel::from(
        b.prefs
            .list_cols
            .iter()
            .map(|&c| Column {
                id: Col::ALL.iter().position(|x| *x == c).unwrap_or(0) as i32,
                label: c.label().into(),
                w: b.prefs.col_w(c),
                right: c.right(),
                sorted: c.sort() == Some(t.sort) && (c != Col::Ext || !has_type),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_sort_label(
        if t.group == GroupKey::None { t.sort.label().to_string() } else { format!("{} · {}", t.sort.label(), t.group.label()) }.into(),
    );
    ui.set_filters(ModelRc::new(VecModel::from(
        t.shown_filters(b.prefs.show_hidden)
            .into_iter()
            .map(|f| FilterChip {
                id: KindFilter::ALL.iter().position(|x| *x == f).unwrap_or(0) as i32,
                icon: ic(match f {
                    KindFilter::All => "filter",
                    KindFilter::Media => "gallery",
                    KindFilter::Image => "image",
                    KindFilter::Video => "video",
                    KindFilter::Doc => "doc",
                    KindFilter::Audio => "music",
                    KindFilter::Archive => "archive",
                }),
                on: t.kind_filter == f,
            })
            .collect::<Vec<_>>(),
    )));
    let (zmin, zmax, z) = match t.view {
        View::Gallery => (120.0, 420.0, b.prefs.gallery_row),
        _ => (84.0, 280.0, b.prefs.grid_size),
    };
    ui.set_zoom_min(zmin);
    ui.set_zoom_max(zmax);
    ui.set_zoom(z);
    ui.set_searching(t.search.as_ref().map(|x| x.running).unwrap_or(false));
    ui.set_search_active(t.search.is_some());
    ui.set_total_height(t.total_h);
    ui.set_empty_text(
        if !t.error.is_empty() {
            t.error.clone()
        } else if t.items.is_empty() {
            match &t.search {
                Some(x) if x.running => "Szukam…".into(),
                Some(_) => "Nic nie znaleziono".into(),
                None if !t.filter.is_empty() || t.kind_filter != KindFilter::All => "Brak elementów pasujących do filtra".into(),
                None => "Ten folder jest pusty".into(),
            }
        } else {
            String::new()
        }
        .into(),
    );
    ui.set_tabs(ModelRc::new(VecModel::from(
        b.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabData {
                title: t.title().into(),
                icon: tab_icon(t),
                active: i == b.active,
                pinned: false,
                loading: t.search.as_ref().map(|x| x.running).unwrap_or(false),
            })
            .collect::<Vec<_>>(),
    )));
    drop(b);
    render_side(s);
    render_status(s);
}

fn render_status(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.tabs[b.active];
    if let Some(n) = &b.notice {
        ui.set_status(n.clone().into());
        return;
    }
    let mut text = fmt::items(t.items.len());
    if let Some(x) = &t.search {
        text = format!("{text} · przeszukano {}", x.scanned);
        let chips = query::parse(&x.query).chips;
        if !chips.is_empty() {
            text = format!("{text} · {}", chips.join(" · "));
        }
    }
    let sel: Vec<_> = t.items.iter().filter(|e| t.selected.contains(&e.path)).collect();
    if !sel.is_empty() {
        let bytes: u64 = sel.iter().filter(|e| !e.dir).map(|e| e.size).sum();
        text = format!("{text} · zaznaczono {}", sel.len());
        if bytes > 0 {
            text = format!("{text} ({})", fmt::size(bytes));
        }
    }
    ui.set_status(text.into());
    ui.set_has_selection(!sel.is_empty());
}

/// Elementy w oknie widoku – reszta nie istnieje w drzewie Slint (wirtualne przewijanie).
pub fn render_items(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.tabs[b.active];
    let (top, _, h) = b.viewport;
    let range = t.visible(top, h);
    let scale = ui.window().scale_factor();
    let mut v = Vec::with_capacity(range.len());
    for i in range {
        let (Some(e), Some(bx)) = (t.items.get(i), t.boxes.get(i)) else { continue };
        if bx.w <= 0.0 {
            continue;
        }
        let media = fmt::is_media(e);
        let (thumb, icon) = match t.view {
            View::List => (None, thumbs::get(&e.path, (18.0 * scale) as u32, true)),
            View::Grid => (thumbs::get(&e.path, ((bx.w - 12.0) * scale) as u32, e.dir), None),
            View::Gallery => (thumbs::get(&e.path, (bx.w.max(bx.h) * scale) as u32, e.dir || !media), None),
        };
        v.push(VisItem {
            index: i as i32,
            x: bx.x,
            y: bx.y,
            w: bx.w,
            h: bx.h,
            name: e.name.clone().into(),
            cells: if t.view == View::List { ModelRc::new(VecModel::from(cells(&b, e))) } else { ModelRc::default() },
            dir: e.dir,
            hidden: e.hidden,
            media,
            selected: t.selected.contains(&e.path),
            focused: t.focus == Some(i),
            renaming: t.renaming.as_deref() == Some(e.path.as_str()),
            has_icon: icon.is_some(),
            icon: icon.unwrap_or_default(),
            has_thumb: thumb.is_some(),
            thumb: thumb.unwrap_or_default(),
        });
    }
    let pad = if t.view == View::List { 6.0 } else { 12.0 };
    let heads: Vec<GroupHead> = match &t.groups {
        Some(groups) => t
            .heads
            .iter()
            .filter(|hd| hd.y + 38.0 >= top - h * 0.5 && hd.y <= top + h * 1.5)
            .map(|hd| GroupHead {
                index: hd.group as i32,
                x: pad,
                y: hd.y,
                w: (b.viewport.1 - pad * 2.0).max(100.0),
                label: groups[hd.group].label.clone().into(),
                count: (groups[hd.group].end - groups[hd.group].start) as i32,
                shut: hd.shut,
            })
            .collect(),
        None => Vec::new(),
    };
    ui.set_heads(ModelRc::new(VecModel::from(heads)));
    // Ten sam zakres (zmiana zaznaczenia) albo trwająca ramka: wiersze aktualizowane w miejscu,
    // żeby elementy Slint nie były tworzone od nowa – inaczej przeciąganie ramki z wiersza by się urwało.
    let model = ui.get_items();
    let vm = model.as_any().downcast_ref::<VecModel<VisItem>>();
    match vm {
        Some(vm) if b.marquee.is_some() || (vm.row_count() == v.len() && vm.row_data(0).map(|r| r.index) == v.first().map(|r| r.index)) => {
            let n = v.len();
            for (i, it) in v.into_iter().enumerate() {
                if i >= vm.row_count() {
                    vm.push(it);
                } else if vm.row_data(i).as_ref() != Some(&it) {
                    vm.set_row_data(i, it);
                }
            }
            while vm.row_count() > n {
                vm.remove(vm.row_count() - 1);
            }
        }
        _ => ui.set_items(ModelRc::new(VecModel::from(v))),
    }
    drop(b);
    render_status(s);
}

/// Teksty kolumn listy (`cell` w FileView.svelte).
fn cells(b: &State, e: &fsx::Entry) -> Vec<SharedString> {
    let m = b.media.get(&e.path);
    b.prefs
        .list_cols
        .iter()
        .map(|c| match c {
            Col::Modified => fmt::date(e.modified),
            Col::Created => fmt::date(e.created),
            Col::Type => fmt::type_label(e),
            Col::Ext => if e.dir { String::new() } else { e.ext.to_uppercase() },
            Col::Size if e.dir => match b.dir_sizes.get(&e.path.to_lowercase()) {
                Some(n) => fmt::size(*n),
                None if b.prefs.folder_sizes => "…".into(),
                None => String::new(),
            },
            Col::Size => fmt::size(e.size),
            Col::Dims => m.filter(|m| m.w > 0).map(|m| format!("{} × {}", m.w, m.h)).unwrap_or_default(),
            Col::Taken => m.filter(|m| m.taken > 0).map(|m| fmt::date(m.taken)).unwrap_or_default(),
            Col::Duration => m.filter(|m| m.dur > 0).map(|m| fmt::duration(m.dur)).unwrap_or_default(),
            Col::Attrs => [(e.readonly, "R"), (e.hidden, "H"), (e.system, "S"), (e.link, "L"), (e.cloud, "C")]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, l)| *l)
                .collect::<Vec<_>>()
                .join(" "),
        }
        .into())
        .collect()
}

fn set_scroll(s: &Shared, top: f32) {
    let ui = s.borrow().ui.upgrade();
    if let Some(ui) = ui {
        s.borrow_mut().viewport.0 = top;
        ui.set_scroll_y(-top);
        render_items(s);
    }
}

/// Przewija tak, żeby element był widoczny.
fn reveal(s: &Shared, i: usize) {
    let (top, h, bx) = {
        let b = s.borrow();
        (b.viewport.0, b.viewport.2, b.tabs[b.active].boxes.get(i).copied())
    };
    let Some(bx) = bx else { return };
    if bx.y < top {
        set_scroll(s, (bx.y - 6.0).max(0.0));
    } else if bx.y + bx.h > top + h {
        set_scroll(s, bx.y + bx.h - h + 6.0);
    }
}

// ---------- Zaznaczanie i otwieranie ----------

fn select(s: &Shared, i: usize, ctrl: bool, shift: bool) {
    tab_mut(s, |t| {
        let Some(e) = t.items.get(i) else { return };
        let p = e.path.clone();
        if shift {
            let from = t.anchor.unwrap_or(i);
            let (lo, hi) = (from.min(i), from.max(i));
            if !ctrl {
                t.selected.clear();
            }
            for e in &t.items[lo..=hi] {
                t.selected.insert(e.path.clone());
            }
        } else if ctrl {
            if !t.selected.remove(&p) {
                t.selected.insert(p);
            }
            t.anchor = Some(i);
        } else {
            t.selected.clear();
            t.selected.insert(p);
            t.anchor = Some(i);
        }
        t.focus = Some(i);
    });
    render_items(s);
}

fn open(s: &Shared, i: usize) {
    let e = tab_mut(s, |t| t.items.get(i).cloned());
    let Some(e) = e else { return };
    if e.dir {
        navigate(s, &e.path, true);
    } else {
        let _ = open::that_detached(&e.path);
    }
}

fn hwnd(s: &Shared) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Some(ui) = s.borrow().ui.upgrade() else { return 0 };
    ui.window()
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => 0,
        })
        .unwrap_or(0)
}

/// Prawy przycisk na elemencie: własne menu, z Shift – od razu menu Eksploratora.
fn context(s: &Shared, i: usize, x: f32, y: f32, shift: bool) {
    let paths = tab_mut(s, |t| {
        let Some(e) = t.items.get(i) else { return Vec::new() };
        if !t.selected.contains(&e.path) {
            t.selected.clear();
            t.selected.insert(e.path.clone());
            t.focus = Some(i);
            t.anchor = Some(i);
        }
        t.items.iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.path.clone()).collect::<Vec<_>>()
    });
    render_items(s);
    if paths.is_empty() {
        return;
    }
    if shift {
        native_menu(s, &paths, x, y);
    } else {
        menus::item_menu(s, x, y);
    }
}

/// Pełne menu Eksploratora (IContextMenu) w miejscu kliknięcia.
fn native_menu(s: &Shared, paths: &[String], x: f32, y: f32) {
    let scale = s.borrow().ui.upgrade().map(|u| u.window().scale_factor()).unwrap_or(1.0);
    let h = hwnd(s);
    if let Ok(true) = shell::native_menu(h, paths, (x * scale) as i32, (y * scale) as i32, false) {
        load(s, true);
    }
}

fn selected_paths(s: &Shared) -> Vec<String> {
    tab_mut(s, |t| t.items.iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.path.clone()).collect())
}

fn command(s: &Shared, c: &str) {
    let sel = selected_paths(s);
    match c {
        "cut" => menus::copy(&sel, true),
        "copy" => menus::copy(&sel, false),
        "paste" => menus::paste(s, None),
        "trash" => menus::trash(&sel),
        "props" => {
            let p = sel.first().cloned().unwrap_or_else(|| tab_mut(s, |t| t.path.clone()));
            shell::show_properties(hwnd(s), &p);
        }
        "rename" if sel.len() == 1 => start_rename(s),
        // Masowa zmiana nazw – etap 2.
        _ => {}
    }
}

fn select_all(s: &Shared) {
    tab_mut(s, |t| t.selected = t.items.iter().map(|e| e.path.clone()).collect());
    render_items(s);
}

/// Zaznacza element o podanej ścieżce (np. cel skrótu) i przewija do niego.
fn select_path(s: &Shared, path: &str) {
    let i = tab_mut(s, |t| {
        let i = t.items.iter().position(|e| same(&e.path, path));
        if let Some(i) = i {
            t.selected.clear();
            t.selected.insert(t.items[i].path.clone());
            t.focus = Some(i);
            t.anchor = Some(i);
        }
        i
    });
    if let Some(i) = i {
        reveal(s, i);
    }
    render_items(s);
}

fn create(s: &Shared, folder: bool) {
    let dir = tab_mut(s, |t| t.path.clone());
    if dir.starts_with("ofp:") {
        return;
    }
    let base = Path::new(&dir).join(if folder { "Nowy folder" } else { "Nowy dokument tekstowy.txt" });
    let p = powerdesk_core::ops::unique_path(&base);
    let ok = if folder { std::fs::create_dir(&p).is_ok() } else { std::fs::File::create(&p).is_ok() };
    if !ok {
        return;
    }
    load(s, true);
    let p = p.to_string_lossy().to_string();
    let i = tab_mut(s, |t| {
        let i = t.items.iter().position(|e| e.path == p);
        if let Some(i) = i {
            t.selected.clear();
            t.selected.insert(p.clone());
            t.focus = Some(i);
            t.anchor = Some(i);
        }
        i
    });
    if let Some(i) = i {
        reveal(s, i);
    }
    // Jak w Eksploratorze: nowy element od razu czeka na nazwę.
    start_rename(s);
}

// ---------- Zmiana nazwy w miejscu ----------

/// Pole nazwy nad zaznaczonym elementem; zaznaczona jest nazwa bez rozszerzenia.
pub fn start_rename(s: &Shared) {
    let sel = tab_mut(s, |t| {
        let e = t.items.iter().find(|e| t.selected.contains(&e.path))?.clone();
        t.renaming = Some(e.path.clone());
        let i = t.items.iter().position(|x| x.path == e.path);
        t.focus = i;
        Some((e, i))
    });
    let Some((e, i)) = sel else { return };
    let dot = if e.dir { None } else { e.name.rfind('.').filter(|d| *d > 0) };
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_rename_sel(dot.unwrap_or(e.name.len()) as i32);
    }
    if let Some(i) = i {
        reveal(s, i);
    }
    render_items(s);
}

/// Zatwierdza nazwę; zwraca nową ścieżkę (albo starą, gdy nic się nie zmieniło).
fn rename_commit(s: &Shared, name: &str) -> Option<String> {
    let path = tab_mut(s, |t| t.renaming.take())?;
    let old = Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if name.trim().is_empty() || name == old {
        render_items(s);
        return Some(path);
    }
    match powerdesk_core::ops::rename_one(&path, name) {
        Ok(p) => {
            load(s, true);
            select_path(s, &p);
            Some(p)
        }
        Err(e) => {
            notice(s, e);
            render_items(s);
            None
        }
    }
}

/// Tab / Shift+Tab: zatwierdza i przechodzi do zmiany nazwy sąsiada.
fn rename_next(s: &Shared, name: &str, back: bool) {
    let Some(p) = rename_commit(s, name) else { return };
    let next = tab_mut(s, |t| {
        let i = t.items.iter().position(|e| e.path == p)?;
        let j = if back { i.checked_sub(1)? } else { i + 1 };
        t.items.get(j).map(|e| e.path.clone())
    });
    if let Some(n) = next {
        select_path(s, &n);
        start_rename(s);
    }
}

/// Komunikat w pasku stanu na kilka sekund (do czasu własnych toastów).
fn notice(s: &Shared, text: String) {
    s.borrow_mut().notice = Some(text);
    render_status(s);
    let s2 = s.clone();
    slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
        s2.borrow_mut().notice = None;
        render_status(&s2);
    });
}

// ---------- Sortowanie, widoki, wyszukiwanie ----------

pub fn set_group(s: &Shared, g: GroupKey) {
    tab_mut(s, |t| {
        t.group = g;
        t.collapsed.clear();
    });
    save_prefs(s);
    refresh(s);
}

fn toggle_group(s: &Shared, gi: usize) {
    tab_mut(s, |t| {
        let Some(label) = t.groups.as_ref().and_then(|g| g.get(gi)).map(|g| g.label.clone()) else { return };
        if !t.collapsed.remove(&label) {
            t.collapsed.insert(label);
        }
    });
    layout(s);
    render_all(s);
}

/// Klik w etykietę grupy zaznacza jej elementy (Ctrl – dokłada).
fn select_group(s: &Shared, gi: usize, ctrl: bool) {
    tab_mut(s, |t| {
        let Some(g) = t.groups.as_ref().and_then(|g| g.get(gi)).cloned() else { return };
        if !ctrl {
            t.selected.clear();
        }
        for e in &t.items[g.start..g.end] {
            t.selected.insert(e.path.clone());
        }
        t.focus = Some(g.start);
        t.anchor = Some(g.start);
    });
    render_items(s);
}

fn set_sort(s: &Shared, k: SortKey) {
    tab_mut(s, |t| {
        if t.sort == k {
            t.desc = !t.desc;
        } else {
            t.sort = k;
            t.desc = matches!(k, SortKey::Modified | SortKey::Created | SortKey::Size);
        }
    });
    save_prefs(s);
    refresh(s);
}

fn set_view(s: &Shared, v: View) {
    tab_mut(s, |t| t.view = v);
    save_prefs(s);
    refresh(s);
    if let Some(f) = tab_mut(s, |t| t.focus) {
        reveal(s, f);
    }
}

fn set_zoom(s: &Shared, z: f32) {
    {
        let mut b = s.borrow_mut();
        let a = b.active;
        match b.tabs[a].view {
            View::Gallery => b.prefs.gallery_row = z.clamp(120.0, 420.0),
            _ => b.prefs.grid_size = z.clamp(84.0, 280.0),
        }
        b.prefs.save();
    }
    layout(s);
    render_all(s);
}

fn start_search(s: &Shared, q: &str) {
    let (root, hidden) = {
        let b = s.borrow();
        (b.tabs[b.active].path.clone(), b.prefs.show_hidden)
    };
    if root.starts_with("ofp:") {
        return;
    }
    // Słowa idą do wyszukiwarki, filtry (typ:, rozmiar:, data:…) sprawdzamy przy odbiorze wyników.
    let id = fsx::search(Core, root, query::parse(q).text, hidden);
    tab_mut(s, |t| {
        t.filter.clear();
        t.selected.clear();
        t.focus = None;
        t.search = Some(Search { query: q.to_string(), id, running: true, scanned: 0, results: Vec::new() });
    });
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_query(SharedString::new());
    }
    set_scroll(s, 0.0);
    refresh(s);
}

fn clear_search(s: &Shared) {
    let had = tab_mut(s, |t| {
        t.filter.clear();
        if t.search.as_ref().map(|x| x.running).unwrap_or(false) {
            fsx::cancel_search();
        }
        t.search.take().is_some()
    });
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_query(SharedString::new());
    }
    refresh(s);
    if had {
        set_scroll(s, 0.0);
    }
}

#[derive(serde::Deserialize)]
struct SearchBatch {
    id: u64,
    items: Vec<fsx::Entry>,
    done: bool,
    scanned: u64,
}

fn on_event(s: &Shared, name: &str, payload: serde_json::Value) {
    match name {
        "search" => {
            let Ok(batch) = serde_json::from_value::<SearchBatch>(payload) else { return };
            let hit = {
                let mut b = s.borrow_mut();
                let mut hit = false;
                for t in b.tabs.iter_mut() {
                    if let Some(x) = t.search.as_mut().filter(|x| x.id == batch.id) {
                        x.results.extend(batch.items.iter().cloned());
                        x.running = !batch.done;
                        x.scanned = batch.scanned;
                        hit = true;
                    }
                }
                hit
            };
            if hit {
                refresh(s);
            }
        }
        "drives-changed" => {
            s.borrow_mut().drives = drives::list();
            render_side(s);
        }
        "fs-changed" => {
            let Ok(dirs) = serde_json::from_value::<Vec<String>>(payload) else { return };
            {
                let mut b = s.borrow_mut();
                let low: Vec<String> = dirs.iter().map(|d| d.to_lowercase()).collect();
                b.dir_sizes.retain(|k, _| !low.iter().any(|d| d.starts_with(k.as_str())));
            }
            let (path, searching) = tab_mut(s, |t| (t.path.clone(), t.search.is_some()));
            if !searching && dirs.iter().any(|d| same(d, &path)) {
                load(s, true);
            }
        }
        _ => {}
    }
}

// ---------- Klawiatura ----------

fn key(s: &Shared, k: &str, ctrl: bool, shift: bool, alt: bool) {
    use slint::platform::Key;
    let is = |key: Key| k == SharedString::from(key).as_str();
    if alt && is(Key::LeftArrow) {
        return back(s);
    }
    if alt && is(Key::RightArrow) {
        return forward(s);
    }
    if alt && is(Key::UpArrow) {
        return up(s);
    }
    if alt && is(Key::Return) {
        return command(s, "props");
    }
    if ctrl && shift && k.eq_ignore_ascii_case("n") {
        return create(s, true);
    }
    if ctrl && k == "t" {
        return new_tab(s);
    }
    if ctrl && k == "w" {
        let a = s.borrow().active;
        return close_tab(s, a);
    }
    if ctrl && is(Key::Tab) {
        save_scroll(s);
        {
            let mut b = s.borrow_mut();
            let n = b.tabs.len();
            b.active = (b.active + if shift { n - 1 } else { 1 }) % n;
        }
        return show_tab(s);
    }
    if ctrl && (k == "1" || k == "2" || k == "3") {
        return set_view(s, View::from_index(k.parse::<i32>().unwrap_or(1) - 1));
    }
    if ctrl && k == "h" {
        {
            let mut b = s.borrow_mut();
            b.prefs.show_hidden = !b.prefs.show_hidden;
            b.prefs.save();
        }
        return refresh(s);
    }
    if ctrl && k == "a" {
        return select_all(s);
    }
    if ctrl && shift && k.eq_ignore_ascii_case("c") {
        let sel = selected_paths(s);
        let _ = powerdesk_core::clip::set_text(&sel.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\n"));
        return;
    }
    if ctrl && k == "c" {
        return command(s, "copy");
    }
    if ctrl && k == "x" {
        return command(s, "cut");
    }
    if ctrl && k == "v" {
        return command(s, "paste");
    }
    if is(Key::Delete) && !shift {
        return command(s, "trash");
    }
    if is(Key::F5) {
        return load(s, true);
    }
    if is(Key::F2) {
        return command(s, "rename");
    }
    if is(Key::Backspace) {
        return if tab_mut(s, |t| t.search.is_some()) { back(s) } else { up(s) };
    }
    if is(Key::Escape) {
        tab_mut(s, |t| t.selected.clear());
        return render_items(s);
    }
    if is(Key::Return) {
        if let Some(f) = tab_mut(s, |t| t.focus) {
            open(s, f);
        }
        return;
    }
    let dir = if is(Key::LeftArrow) {
        Some((-1, 0))
    } else if is(Key::RightArrow) {
        Some((1, 0))
    } else if is(Key::UpArrow) {
        Some((0, -1))
    } else if is(Key::DownArrow) {
        Some((0, 1))
    } else if is(Key::Home) {
        Some((-9, 0))
    } else if is(Key::End) {
        Some((9, 0))
    } else {
        None
    };
    if let Some(d) = dir {
        move_focus(s, d, shift, ctrl);
    }
}

/// Strzałki po geometrii układu: w lewo/prawo – sąsiad, w górę/dół – najbliższy w sąsiednim wierszu.
fn move_focus(s: &Shared, (dx, dy): (i32, i32), shift: bool, ctrl: bool) {
    let to = tab_mut(s, |t| {
        let n = t.items.len();
        if n == 0 {
            return None;
        }
        let Some(f) = t.focus else { return Some(0) };
        let horizontal = t.view != View::List;
        let center = |i: usize| t.boxes[i].x + t.boxes[i].w / 2.0;
        Some(match (dx, dy) {
            (-9, _) => 0,
            (9, _) => n - 1,
            (dx, 0) if horizontal => (f as i32 + dx).clamp(0, n as i32 - 1) as usize,
            (_, 0) => f,
            (_, dy) => {
                let (cy, cx) = (t.boxes[f].y, center(f));
                let mut j = f as i32;
                // Pierwszy element w sąsiednim wierszu…
                loop {
                    j += dy;
                    if j < 0 || j >= n as i32 {
                        return Some(f);
                    }
                    if t.boxes[j as usize].w > 0.0 && (t.boxes[j as usize].y - cy).abs() > 1.0 {
                        break;
                    }
                }
                // …i w nim ten najbliższy w poziomie.
                let row_y = t.boxes[j as usize].y;
                let mut best = j as usize;
                while j >= 0 && j < n as i32 && t.boxes[j as usize].w > 0.0 && (t.boxes[j as usize].y - row_y).abs() < 1.0 {
                    if (center(j as usize) - cx).abs() < (center(best) - cx).abs() {
                        best = j as usize;
                    }
                    j += dy;
                }
                best
            }
        })
        // Elementy zwiniętych grup są pomijane – idziemy dalej w tym samym kierunku.
        .map(|to| {
            let back = (dx < 0 && dx != -9) || dy < 0 || dx == 9;
            let mut k = to as i32;
            while k >= 0 && k < n as i32 && t.boxes[k as usize].w <= 0.0 {
                k += if back { -1 } else { 1 };
            }
            if k >= 0 && k < n as i32 { k as usize } else { f }
        })
    });
    let Some(to) = to else { return };
    if ctrl && !shift {
        tab_mut(s, |t| t.focus = Some(to));
        render_items(s);
    } else {
        select(s, to, false, shift);
    }
    reveal(s, to);
}

// ---------- Pomocnicze ----------

fn ic(name: &str) -> SharedString {
    icons::icon(name).into()
}

fn tab_icon(t: &Tab) -> SharedString {
    let path = &t.path;
    if t.search.is_some() {
        ic("search")
    } else if path == HOME {
        ic("pc")
    } else if path.len() <= 3 && path.contains(':') {
        ic("drive")
    } else if path.starts_with("\\\\") {
        ic("network")
    } else {
        ic("folder")
    }
}

/// Okruszki: Dysk (C:) → Users → redzi.
fn crumbs(path: &str, drives: &[drives::Drive]) -> Vec<Crumb> {
    if path.starts_with("ofp:") {
        return Vec::new();
    }
    let mut v = Vec::new();
    let mut acc = PathBuf::new();
    for c in Path::new(path).components() {
        acc.push(c.as_os_str());
        let name = match c {
            std::path::Component::Prefix(_) => continue,
            std::path::Component::RootDir => {
                let root = acc.to_string_lossy().to_string();
                let letter = root.trim_end_matches('\\').to_string();
                let label = drives.iter().find(|d| same(&d.path, &root)).map(|d| d.label.clone()).unwrap_or_default();
                if label.is_empty() {
                    format!("Dysk ({letter})")
                } else {
                    format!("{label} ({letter})")
                }
            }
            other => other.as_os_str().to_string_lossy().to_string(),
        };
        v.push(Crumb { name: name.into(), path: acc.to_string_lossy().to_string().into() });
    }
    v
}

/// Panel boczny z ikonami systemowymi (te same co w Eksploratorze).
pub fn render_side(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.tabs[b.active];
    let scale = ui.window().scale_factor();
    let px = (18.0 * scale) as u32;
    let items: Vec<SideItem> = side_items(&t.path, &b.drives)
        .into_iter()
        .map(|mut it| {
            if let Some(img) = thumbs::get(it.path.as_str(), px, true) {
                it.img = img;
                it.has_img = true;
            }
            it
        })
        .collect();
    ui.set_side(ModelRc::new(VecModel::from(items)));
    ui.set_folder_icon(thumbs::get(&t.path, (16.0 * scale) as u32, true).unwrap_or_default());
}

fn side_items(current: &str, drives: &[drives::Drive]) -> Vec<SideItem> {
    let mut v = Vec::new();
    let known: [(&str, Option<PathBuf>, &str); 6] = [
        ("Pulpit", dirs::desktop_dir(), "desktop"),
        ("Pobrane", dirs::download_dir(), "download"),
        ("Dokumenty", dirs::document_dir(), "doc"),
        ("Obrazy", dirs::picture_dir(), "image"),
        ("Wideo", dirs::video_dir(), "video"),
        ("Muzyka", dirs::audio_dir(), "music"),
    ];
    let mut first = true;
    for (name, p, icon) in known {
        let Some(p) = p.filter(|p| p.is_dir()) else { continue };
        let p = p.to_string_lossy().to_string();
        v.push(SideItem {
            section: if std::mem::take(&mut first) { "Szybki dostęp".into() } else { SharedString::new() },
            name: name.into(),
            icon: ic(icon),
            active: same(&p, current),
            path: p.into(),
            detail: SharedString::new(),
            usage: 0.0,
            img: Default::default(),
            has_img: false,
        });
    }
    let mut first = true;
    for d in drives.iter().filter(|d| d.online) {
        let name = if d.label.is_empty() { format!("Dysk ({})", d.letter) } else { format!("{} ({})", d.label, d.letter) };
        let used = d.total.saturating_sub(d.free);
        v.push(SideItem {
            section: if std::mem::take(&mut first) { "Dyski".into() } else { SharedString::new() },
            name: name.into(),
            icon: match d.kind.as_str() {
                "removable" => ic("usb"),
                "network" => ic("network"),
                _ => ic("drive"),
            },
            active: same(&d.path, current),
            path: d.path.clone().into(),
            detail: if d.total > 0 { format!("{} wolne z {}", fmt::size(d.free), fmt::size(d.total)).into() } else { SharedString::new() },
            usage: if d.total > 0 { used as f32 / d.total as f32 } else { 0.0 },
            img: Default::default(),
            has_img: false,
        });
    }
    v
}

/// Motyw jak w wersji webowej: ustawienie z state.json, „system” – według Windows.
fn apply_theme(ui: &MainWindow) {
    let state = dirs::data_dir().map(|d| d.join("OpenFilePowerWin").join("state.json"));
    let theme = state
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["settings"]["theme"].as_str().map(String::from))
        .unwrap_or_else(|| "system".into());
    let light = match theme.as_str() {
        "light" => true,
        "dark" => false,
        _ => system_light(),
    };
    ui.global::<Theme>().set_light(light);
}

fn system_light() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "AppsUseLightTheme"])
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("0x1"))
        .unwrap_or(false)
}
