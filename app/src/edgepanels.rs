//! Panele folderów przypięte do krawędzi pulpitu (lewa, prawa, górna) – osobno na każdym
//! monitorze. Zwinięty panel to wąski pasek; po najechaniu wysuwa się z zawartością folderu,
//! klik w podfolder wchodzi w głąb w miejscu, w plik – otwiera go. Po schowaniu panel wraca do
//! folderu głównego. Kilka paneli na jednej krawędzi dzieli ją po równo. Rysuje je okno
//! pulpitu (desktop.slint), ustawienia trzyma `Prefs::edge_panels`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use powerdesk_core::fsx::{self, Entry};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::menus::{self, it, sep};
use crate::model::EdgePanelPref;
use crate::{fmt, DesktopWindow, EdgeItem, EdgePanel, Shared};

/// Odstęp od rogów ekranu i między panelami na jednej krawędzi.
const GAP: f32 = 8.0;
/// Rozmiar ikon w wierszach panelu (piksele logiczne).
const ICON: f32 = 20.0;
/// Najwięcej elementów w panelu – resztę pokazuje okno PowerDesk (przycisk pełnego ekranu).
const MAX_ITEMS: usize = 500;
pub const EDGES: [&str; 3] = ["Lewa krawędź", "Prawa krawędź", "Górna krawędź"];

#[derive(Default)]
struct Panels {
    /// Model paneli każdego okna pulpitu i numery paneli w ustawieniach w kolejności modelu.
    models: Vec<(Rc<VecModel<EdgePanel>>, Vec<usize>)>,
    /// Bieżący folder panelu (numer w ustawieniach); brak – folder główny.
    cur: HashMap<usize, String>,
    /// Zawartość wczytanych folderów (po ścieżce).
    lists: HashMap<String, Vec<Entry>>,
    /// Foldery wczytywane w tle.
    loading: HashSet<String>,
}

thread_local! {
    static P: RefCell<Panels> = RefCell::new(Panels::default());
}

/// Obsługa paneli okna pulpitu `n` – wołane przy budowie okna.
pub fn wire(w: &DesktopWindow, n: usize) {
    w.on_edge_open(move |i, k| with_panel(n, i, |s, p| open_item(s, p, k as usize)));
    w.on_edge_up(move |i| with_panel(n, i, up));
    w.on_edge_root(move |i| with_panel(n, i, |s, p| go(s, p, None)));
    w.on_edge_collapsed(move |i| with_panel(n, i, |s, p| go(s, p, None)));
    w.on_edge_full(move |i| with_panel(n, i, |s, p| crate::open_args(s, vec![current(s, p)])));
    w.on_edge_resized(move |i, wd| with_panel(n, i, |s, p| edit(s, p, |e| e.width = wd.clamp(160.0, 900.0).round())));
    w.on_edge_context(move |i, x, y| with_panel(n, i, |s, p| context(s, n, p, x, y)));
}

/// Numer panelu w ustawieniach z wiersza `i` modelu okna `n`.
fn with_panel(n: usize, i: i32, f: impl FnOnce(&Shared, usize)) {
    let Some(p) = P.with(|x| x.borrow().models.get(n).and_then(|m| m.1.get(i as usize).copied())) else { return };
    crate::with_state(|s| f(s, p));
}

/// Zamknięcie pulpitu – modele i wczytane foldery od nowa przy następnym otwarciu.
pub fn clear() {
    P.with(|x| *x.borrow_mut() = Panels::default());
}

fn pref(s: &Shared, p: usize) -> Option<EdgePanelPref> {
    s.borrow().prefs.edge_panels.get(p).cloned()
}

/// Bieżący folder panelu `p` (bez wejścia w głąb – folder główny).
fn current(s: &Shared, p: usize) -> String {
    let root = pref(s, p).map(|e| e.folder).unwrap_or_default();
    P.with(|x| x.borrow().cur.get(&p).cloned()).unwrap_or(root)
}

/// Nazwa folderu do nagłówka (dysk – sama litera z dwukropkiem).
fn folder_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.trim_end_matches('\\').to_string())
}

/// Zawartość folderu w tle; po wczytaniu panele przerysowują się same.
fn load(path: &str) {
    let fresh = P.with(|x| x.borrow_mut().loading.insert(path.to_string()));
    if !fresh {
        return;
    }
    let mut hidden = false;
    crate::with_state(|s| hidden = s.borrow().prefs.show_hidden);
    let path = path.to_string();
    std::thread::spawn(move || {
        let mut list: Vec<Entry> = fsx::list_dir(&path).unwrap_or_default().into_iter().filter(|e| hidden || !(e.hidden || e.system)).collect();
        list.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
        let _ = slint::invoke_from_event_loop(move || {
            P.with(|x| {
                let mut x = x.borrow_mut();
                x.loading.remove(&path);
                x.lists.insert(path, list);
            });
            crate::with_state(update);
        });
    });
}

/// Przejście panelu `p` do folderu (`None` – folder główny); zawartość czytana od nowa.
fn go(s: &Shared, p: usize, to: Option<String>) {
    let changed = P.with(|x| {
        let mut x = x.borrow_mut();
        match to {
            Some(t) => x.cur.insert(p, t.clone()).as_deref() != Some(t.as_str()),
            None => x.cur.remove(&p).is_some(),
        }
    });
    // Świeża lista przy każdym wejściu (folder mógł się zmienić) – do tego czasu poprzednia.
    load(&current(s, p));
    if changed {
        update(s);
    }
}

fn open_item(s: &Shared, p: usize, k: usize) {
    let cur = current(s, p);
    let Some(e) = P.with(|x| x.borrow().lists.get(&cur).and_then(|l| l.get(k).cloned())) else { return };
    if e.dir {
        go(s, p, Some(e.path));
    } else {
        let _ = open::that_detached(&e.path);
    }
}

/// Folder wyżej – nie ponad folder główny panelu.
fn up(s: &Shared, p: usize) {
    let Some(root) = pref(s, p).map(|e| e.folder) else { return };
    let cur = current(s, p);
    if cur.eq_ignore_ascii_case(&root) {
        return;
    }
    let parent = Path::new(&cur).parent().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
    let inside = parent.to_lowercase().starts_with(&root.to_lowercase()) && !parent.eq_ignore_ascii_case(&root);
    go(s, p, inside.then_some(parent));
}

/// Zmiana ustawień panelu `p` (menu panelu, Ustawienia), zapis i ułożenie paneli od nowa.
pub fn edit(s: &Shared, p: usize, f: impl FnOnce(&mut EdgePanelPref)) {
    {
        let mut b = s.borrow_mut();
        if let Some(e) = b.prefs.edge_panels.get_mut(p) {
            f(e);
        }
        b.prefs.save();
    }
    relayout(s);
}

/// Nowy panel z folderem `folder` przy krawędzi `edge` monitora `monitor`.
pub fn add(s: &Shared, folder: String, monitor: usize, edge: u8) {
    if folder.is_empty() {
        return;
    }
    {
        let mut b = s.borrow_mut();
        b.prefs.edge_panels.push(EdgePanelPref { folder, monitor, edge: edge.min(2), ..Default::default() });
        b.prefs.save();
    }
    relayout(s);
}

/// Wybór folderu i nowy panel przy krawędzi `edge` monitora `monitor`.
pub fn pick_and_add(s: &Shared, monitor: usize, edge: u8) {
    if let Some(f) = powerdesk_core::shell::pick_folder(crate::hwnd(s), crate::tr("Folder przy krawędzi pulpitu"), None) {
        add(s, f, monitor, edge);
    }
}

pub fn remove(s: &Shared, p: usize) {
    {
        let mut b = s.borrow_mut();
        if p < b.prefs.edge_panels.len() {
            b.prefs.edge_panels.remove(p);
        }
        b.prefs.save();
    }
    // Numery dalszych paneli się przesuwają – wszystkie wracają do folderów głównych.
    P.with(|x| x.borrow_mut().cur.clear());
    relayout(s);
}

/// Zamiana miejscami z sąsiednim panelem na tej samej krawędzi tego samego monitora.
fn shift(s: &Shared, p: usize, later: bool) {
    {
        let mut b = s.borrow_mut();
        let v = &mut b.prefs.edge_panels;
        let Some(me) = v.get(p).cloned() else { return };
        let same = |e: &EdgePanelPref| e.monitor == me.monitor && e.edge == me.edge;
        let other = if later { (p + 1..v.len()).find(|&k| same(&v[k])) } else { (0..p).rev().find(|&k| same(&v[k])) };
        let Some(o) = other else { return };
        v.swap(p, o);
        b.prefs.save();
    }
    P.with(|x| x.borrow_mut().cur.clear());
    relayout(s);
}

/// Panele po zmianie ustawień: te same panele na tych samych monitorach zmieniają się w miejscu
/// (wysunięty zostaje wysunięty), inny układ – nowe modele.
pub fn relayout(s: &Shared) {
    render(s);
    crate::settings::refresh_edges(s);
}

/// Odcinek krawędzi każdego panelu i modele okien pulpitu.
pub fn render(s: &Shared) {
    let wins = crate::desktop::windows();
    let (all, tb_all, tb_h) = {
        let b = s.borrow();
        (b.prefs.edge_panels.clone(), b.prefs.taskbar_all_monitors, b.prefs.taskbar_height)
    };
    let count = wins.len();
    let old = P.with(|x| std::mem::take(&mut x.borrow_mut().models));
    let mut models = Vec::new();
    for (n, w) in wins.iter().enumerate() {
        let size = w.window().size().to_logical(w.window().scale_factor());
        // Pasek zadań na dole monitora – panele boczne kończą się nad nim.
        let bottom = if crate::taskbar::is_open() && (n == 0 || tb_all) { tb_h } else { 0.0 };
        let mine: Vec<usize> = (0..all.len()).filter(|&k| all[k].monitor == n || (n == 0 && all[k].monitor >= count)).collect();
        let mut rows = Vec::new();
        for &k in &mine {
            let e = &all[k];
            let edge = e.edge.min(2);
            let same: Vec<usize> = mine.iter().copied().filter(|&j| all[j].edge.min(2) == edge).collect();
            let (m, at) = (same.len() as f32, same.iter().position(|&j| j == k).unwrap_or(0) as f32);
            let avail = if edge == 2 { size.width - 2.0 * GAP } else { size.height - bottom - 2.0 * GAP };
            let len = ((avail - GAP * (m - 1.0)) / m).max(40.0);
            let start = GAP + at * (len + GAP);
            let (x, y) = if edge == 2 { (start, 0.0) } else { (0.0, start) };
            rows.push(EdgePanel { edge: edge as i32, x, y, len, size: e.width, title: Default::default(), path: Default::default(), root: true, items: ModelRc::default() });
        }
        match old.get(n).filter(|(m, idx)| *idx == mine && m.row_count() == rows.len()) {
            Some((model, _)) => {
                for (i, r) in rows.into_iter().enumerate() {
                    let Some(d) = model.row_data(i) else { continue };
                    model.set_row_data(i, EdgePanel { title: d.title, path: d.path, root: d.root, items: d.items, ..r });
                }
                models.push((model.clone(), mine));
            }
            None => {
                let model = Rc::new(VecModel::from(rows));
                w.set_edge_panels(ModelRc::from(model.clone()));
                models.push((model, mine));
            }
        }
    }
    P.with(|x| x.borrow_mut().models = models);
    update(s);
}

/// Treść paneli (bieżący folder i jego elementy) bez zmiany modeli – wysunięte zostają wysunięte.
fn update(s: &Shared) {
    let roots: Vec<String> = s.borrow().prefs.edge_panels.iter().map(|e| e.folder.clone()).collect();
    let scale = crate::desktop::windows().first().map_or(1.0, |w| w.window().scale_factor());
    let px = (ICON * scale) as u32;
    let mut want = Vec::new();
    P.with(|x| {
        let x = x.borrow();
        for (model, idx) in &x.models {
            for (row, &p) in idx.iter().enumerate() {
                let Some(root) = roots.get(p) else { continue };
                let cur = x.cur.get(&p).cloned().unwrap_or_else(|| root.clone());
                let items: Vec<EdgeItem> = match x.lists.get(&cur) {
                    Some(list) => list
                        .iter()
                        .take(MAX_ITEMS)
                        .map(|e| {
                            let img = crate::thumbs::get(&e.path, px, !fmt::is_media(e));
                            EdgeItem { name: crate::desktop::label(e).into(), has_img: img.is_some(), img: img.unwrap_or_default(), dir: e.dir }
                        })
                        .collect(),
                    None => {
                        want.push(cur.clone());
                        Vec::new()
                    }
                };
                let Some(mut d) = model.row_data(row) else { continue };
                d.title = folder_name(&cur).into();
                d.path = cur.clone().into();
                d.root = cur.eq_ignore_ascii_case(root);
                d.items = ModelRc::new(VecModel::from(items));
                model.set_row_data(row, d);
            }
        }
    });
    for p in want {
        load(&p);
    }
}

/// Miniatury i ikony dochodzą w tle – przerysowanie, gdy są wśród pokazanych elementów.
pub fn thumbs_arrived(paths: &HashSet<String>) {
    crate::with_state(|s| {
        let roots: Vec<String> = s.borrow().prefs.edge_panels.iter().map(|e| e.folder.clone()).collect();
        let hit = P.with(|x| {
            let x = x.borrow();
            x.models
                .iter()
                .flat_map(|m| m.1.iter())
                .filter_map(|p| x.cur.get(p).or(roots.get(*p)))
                .filter_map(|d| x.lists.get(d))
                .any(|l| l.iter().take(MAX_ITEMS).any(|e| paths.contains(&e.path)))
        });
        if hit {
            update(s);
        }
    });
}

/// Menu panelu: okno PowerDesk, krawędź, kolejność, szerokość, inny folder, usunięcie.
fn context(s: &Shared, n: usize, p: usize, x: f32, y: f32) {
    let Some(e) = pref(s, p) else { return };
    let Some(w) = crate::desktop::windows().into_iter().nth(n) else { return };
    let cur = current(s, p);
    let monitors = crate::desktop::windows().len();
    let mut v = vec![
        it(crate::tr("Otwórz w oknie PowerDesk"), "fullscreen", move |s| crate::open_args(s, vec![cur.clone()])),
        sep(),
        menus::sub(
            crate::tr("Krawędź"),
            "panel",
            (0..3u8).map(|k| it(crate::tr(EDGES[k as usize]), "panel", move |s| edit(s, p, |e| e.edge = k)).checked(e.edge == k)).collect(),
        ),
    ];
    if monitors > 1 {
        v.push(menus::sub(
            crate::tr("Monitor"),
            "desktop",
            (0..monitors).map(|m| it(powerdesk_core::trf!("Monitor {}", m + 1), "desktop", move |s| edit(s, p, |e| e.monitor = m)).checked(e.monitor == m)).collect(),
        ));
    }
    v.extend([
        menus::sub(
            crate::tr("Szerokość"),
            "split",
            [220.0f32, 300.0, 400.0, 520.0, 680.0].into_iter().map(|px| it(format!("{px} px"), "split", move |s| edit(s, p, |e| e.width = px)).checked((e.width - px).abs() < 1.0)).collect(),
        ),
        it(crate::tr("Przesuń wcześniej"), "arrowUp", move |s| shift(s, p, false)),
        it(crate::tr("Przesuń dalej"), "arrowDown", move |s| shift(s, p, true)),
        it(crate::tr("Zmień folder…"), "folder", move |s| {
            let start = pref(s, p).map(|e| e.folder).unwrap_or_default();
            if let Some(f) = powerdesk_core::shell::pick_folder(crate::hwnd(s), crate::tr("Folder przy krawędzi pulpitu"), Some(&start)) {
                P.with(|x| x.borrow_mut().cur.remove(&p));
                edit(s, p, |e| e.folder = f);
            }
        }),
        sep(),
        it(crate::tr("Usuń panel"), "trash", move |s| remove(s, p)).danger(),
    ]);
    menus::show_in(w.window(), v, x, y);
}
