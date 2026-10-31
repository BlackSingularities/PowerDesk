//! Zajętość i porządki (SpaceView.svelte) na karcie `ofp:space|<folder>`: mapa kafelkowa
//! z listą największych elementów i bezpieczne porządki (puste foldery, śmieci, TEMP) do Kosza.
//! Skanowanie w `powerdesk_core::space`, układ kafelków (squarified treemap) liczony tutaj.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;

use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::ops;
use powerdesk_core::space::{self, Junk, Node};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::fmt::{self, Kind};
use crate::menus::{it, sep};
use crate::{Shared, SpaceJunk, SpaceRow, SpaceState, SpaceTile};

/// Przedrostek ścieżki karty – po „|” jest analizowany folder.
pub const PREFIX: &str = "ofp:space|";

#[derive(Default)]
struct St {
    root: String,
    tree: Option<Node>,
    /// Droga od korzenia do oglądanego folderu – indeksy w `children`.
    stack: Vec<usize>,
    /// Numer bieżącego skanowania – wyniki starszych są pomijane.
    gen: u64,
    junk: Option<Vec<Junk>>,
    picked: HashSet<String>,
    w: f32,
    h: f32,
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
}

fn with_ui(s: &Shared, f: impl FnOnce(&SpaceState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<SpaceState>());
    }
}

/// Otwiera analizę folderu w bieżącej karcie (openSpace).
pub fn open(s: &Shared, path: &str) {
    if path.is_empty() || path.starts_with("ofp:") {
        return;
    }
    crate::navigate(s, &format!("{PREFIX}{path}"), true);
}

/// Analiza folderu otwartego w bieżącej karcie.
pub fn open_here(s: &Shared) {
    let path = crate::tab_mut(s, |t| t.path.clone());
    if path.starts_with("ofp:") || crate::archives::browsing(&path) {
        crate::transfers::toast(s, crate::tr("Otwórz zwykły folder, aby sprawdzić jego zajętość"), crate::transfers::Kind::Info, None);
        return;
    }
    open(s, &path);
}

pub fn title(path: &str) -> Option<String> {
    let root = path.strip_prefix(PREFIX)?;
    let name = Path::new(root).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| root.to_string());
    Some(powerdesk_core::trf!("Zajętość: {name}"))
}

pub fn sync_tab(s: &Shared) {
    let root = crate::tab_mut(s, |t| if t.viewer.is_none() { t.path.strip_prefix(PREFIX).map(str::to_string) } else { None });
    with_ui(s, |g| g.set_open(root.is_some()));
    let Some(root) = root else { return };
    if ST.with(|st| st.borrow().root == root) {
        return;
    }
    ST.with(|st| {
        let mut st = st.borrow_mut();
        st.root = root.clone();
        st.tree = None;
        st.stack.clear();
        st.junk = None;
        st.picked.clear();
    });
    with_ui(s, |g| {
        g.set_root(root.into());
        g.set_mode(0);
        g.set_error("".into());
    });
    scan(s);
}

// ---------- Zajętość ----------

fn scan(s: &Shared) {
    let (root, gen) = ST.with(|st| {
        let mut st = st.borrow_mut();
        st.gen += 1;
        st.tree = None;
        st.stack.clear();
        (st.root.clone(), st.gen)
    });
    with_ui(s, |g| {
        g.set_busy(true);
        g.set_error("".into());
        g.set_progress(crate::tr("Skanuję…").into());
    });
    render_map(s);
    std::thread::spawn(move || {
        let res = space::scan(&Core, &root);
        let _ = slint::invoke_from_event_loop(move || {
            crate::with_state(|s| {
                if ST.with(|st| st.borrow().gen) != gen {
                    return;
                }
                with_ui(s, |g| g.set_busy(false));
                match res {
                    Ok(tree) => ST.with(|st| st.borrow_mut().tree = Some(tree)),
                    Err(e) => with_ui(s, |g| g.set_error(e.into())),
                }
                render_map(s);
            })
        });
    });
}

/// Zdarzenie „space-progress” – liczba plików i bajtów policzonych do tej pory.
pub fn on_progress(s: &Shared, p: serde_json::Value) {
    let files = p["files"].as_u64().unwrap_or(0);
    let bytes = p["bytes"].as_u64().unwrap_or(0);
    let current = p["current"].as_str().unwrap_or_default().to_string();
    with_ui(s, |g| {
        if g.get_busy() && files > 0 {
            g.set_progress(powerdesk_core::trf!("{} plików · {}", fmt::thousands(files), fmt::size(bytes)).into());
            g.set_current(current.into());
        }
    });
}

fn cur<'a>(tree: &'a Node, stack: &[usize]) -> &'a Node {
    stack.iter().fold(tree, |n, &i| &n.children[i])
}

fn cur_mut<'a>(tree: &'a mut Node, stack: &[usize]) -> &'a mut Node {
    stack.iter().fold(tree, |n, &i| &mut n.children[i])
}

fn hsl(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let f = |v: f32| ((v + m) * 255.0).round() as u8;
    Color::from_rgb_u8(f(r), f(g), f(b))
}

/// Kolor kafelka: foldery w odcieniach po kolei, pliki według rodzaju (jak colorOf w SpaceView).
fn color_of(n: &Node, i: usize) -> Color {
    if n.dir {
        return hsl((i * 47 % 360) as f32, 0.32, (38 - i.min(8) as i32) as f32 / 100.0);
    }
    let ext = n.name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let hex: u32 = match fmt::kind_ext(ext) {
        Kind::Image => 0xe8a33d,
        Kind::Raw => 0xd48a2a,
        Kind::Video => 0xd9534f,
        Kind::Audio => 0x9b59b6,
        Kind::Archive => 0x7f8c8d,
        Kind::App => 0x3498db,
        Kind::Doc => 0x2e86de,
        Kind::Pdf => 0xc0392b,
        Kind::Sheet => 0x27ae60,
        Kind::Code => 0x16a085,
        _ => 0x55606e,
    };
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Squarified treemap – ten sam algorytm co w SpaceView.svelte. Zwraca (indeks, x, y, w, h).
fn squarify(nodes: &[Node], w: f32, h: f32) -> Vec<(usize, f32, f32, f32, f32)> {
    let items: Vec<usize> = (0..nodes.len()).filter(|&i| nodes[i].size > 0).collect();
    let total: f64 = items.iter().map(|&i| nodes[i].size as f64).sum();
    let mut out = Vec::new();
    if total <= 0.0 || w <= 0.0 || h <= 0.0 {
        return out;
    }
    let scale = (w as f64 * h as f64) / total;
    let area = |i: usize| nodes[i].size as f64 * scale;
    let (mut rx, mut ry, mut rw, mut rh) = (0f64, 0f64, w as f64, h as f64);
    let worst = |r: &[usize], side: f64| {
        let s: f64 = r.iter().map(|&i| area(i)).sum();
        let mx = r.iter().map(|&i| area(i)).fold(0.0, f64::max);
        let mn = r.iter().map(|&i| area(i)).fold(f64::MAX, f64::min);
        f64::max(side * side * mx / (s * s), s * s / (side * side * mn))
    };
    let mut layout = |r: &[usize], rx: &mut f64, ry: &mut f64, rw: &mut f64, rh: &mut f64| {
        let s: f64 = r.iter().map(|&i| area(i)).sum();
        if *rw >= *rh {
            let cw = s / *rh;
            let mut yy = *ry;
            for &i in r {
                let hh = area(i) / cw;
                out.push((i, *rx as f32, yy as f32, cw as f32, hh as f32));
                yy += hh;
            }
            *rx += cw;
            *rw -= cw;
        } else {
            let ch = s / *rw;
            let mut xx = *rx;
            for &i in r {
                let ww = area(i) / ch;
                out.push((i, xx as f32, *ry as f32, ww as f32, ch as f32));
                xx += ww;
            }
            *ry += ch;
            *rh -= ch;
        }
    };
    let mut row: Vec<usize> = Vec::new();
    for &i in &items {
        let side = rw.min(rh);
        let mut with = row.clone();
        with.push(i);
        if row.is_empty() || worst(&with, side) <= worst(&row, side) {
            row = with;
        } else {
            layout(&row, &mut rx, &mut ry, &mut rw, &mut rh);
            row = vec![i];
        }
    }
    if !row.is_empty() {
        layout(&row, &mut rx, &mut ry, &mut rw, &mut rh);
    }
    out
}

fn render_map(s: &Shared) {
    ST.with(|st| {
        let st = st.borrow();
        let Some(tree) = &st.tree else {
            with_ui(s, |g| {
                g.set_ready(false);
                g.set_tiles(ModelRc::default());
                g.set_rows(ModelRc::default());
                g.set_crumbs(ModelRc::default());
            });
            return;
        };
        let c = cur(tree, &st.stack);
        let mut crumbs: Vec<SharedString> = vec![Path::new(&tree.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| tree.path.clone()).into()];
        let mut n = tree;
        for &i in &st.stack {
            n = &n.children[i];
            crumbs.push(n.name.clone().into());
        }
        let tiles: Vec<SpaceTile> = squarify(&c.children, st.w, st.h)
            .into_iter()
            .map(|(i, x, y, w, h)| {
                let n = &c.children[i];
                SpaceTile {
                    index: i as i32,
                    x,
                    y,
                    w,
                    h,
                    name: n.name.clone().into(),
                    size: fmt::size(n.size).into(),
                    tip: if n.dir { powerdesk_core::trf!("{}\n{} · {} plików", n.name, fmt::size(n.size), fmt::thousands(n.files)) } else { format!("{}\n{}", n.name, fmt::size(n.size)) }.into(),
                    color: color_of(n, i),
                    dir: n.dir,
                }
            })
            .collect();
        let total = c.size.max(1) as f32;
        let rows: Vec<SpaceRow> = c
            .children
            .iter()
            .take(40)
            .enumerate()
            .map(|(i, n)| SpaceRow { index: i as i32, name: n.name.clone().into(), size: fmt::size(n.size).into(), color: color_of(n, i), frac: n.size as f32 / total })
            .collect();
        let summary = powerdesk_core::trf!("{} · {} plików", fmt::size(c.size), fmt::thousands(c.files));
        with_ui(s, |g| {
            g.set_ready(true);
            g.set_crumbs(ModelRc::new(VecModel::from(crumbs)));
            g.set_summary(summary.into());
            g.set_tiles(ModelRc::new(VecModel::from(tiles)));
            g.set_rows(ModelRc::new(VecModel::from(rows)));
        });
    });
}

/// Kliknięcie kafelka: folder ze szczegółami – w głąb mapy, bez szczegółów – otwarcie folderu.
fn open_tile(s: &Shared, i: usize) {
    let go = ST.with(|st| {
        let mut st = st.borrow_mut();
        let St { tree, stack, .. } = &mut *st;
        let n = cur(tree.as_ref()?, stack).children.get(i)?;
        if !n.dir {
            return None;
        }
        if n.children.is_empty() {
            return Some(n.path.clone());
        }
        stack.push(i);
        None
    });
    match go {
        Some(p) => crate::navigate(s, &p, true),
        None => render_map(s),
    }
}

fn tile_path(i: usize) -> Option<(String, String, u64, bool, bool)> {
    ST.with(|st| {
        let st = st.borrow();
        let n = cur(st.tree.as_ref()?, &st.stack).children.get(i)?;
        (!n.path.is_empty()).then(|| (n.path.clone(), n.name.clone(), n.size, n.dir, !n.children.is_empty()))
    })
}

fn parent(p: &str) -> String {
    Path::new(p).parent().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string())
}

fn tile_menu(s: &Shared, i: usize, x: f32, y: f32) {
    let Some((path, name, size, dir, deeper)) = tile_path(i) else { return };
    let folder = if dir { path.clone() } else { parent(&path) };
    let mut v = Vec::new();
    if dir && deeper {
        v.push(it(crate::tr("Pokaż szczegóły"), "grid", move |s| open_tile(s, i)));
    }
    let f = folder.clone();
    v.push(it(if dir { crate::tr("Otwórz folder") } else { crate::tr("Pokaż w folderze") }, "folder", move |s| crate::navigate(s, &f, true)));
    v.push(it(crate::tr("Otwórz w nowej karcie"), "plus", move |s| crate::new_tab_at(s, &folder, false)));
    v.push(sep());
    v.push(
        it(crate::tr("Do Kosza"), "trash", move |s| {
            let path = path.clone();
            trash_paths(s, vec![path.clone()], &format!("„{name}” ({})", fmt::size(size)), Box::new(move |s| drop_tile(s, &path)));
        })
        .danger(),
    );
    crate::menus::show(s, v, x, y);
}

/// Usunięty element znika z mapy, a jego rozmiar z folderów nad nim.
fn drop_tile(s: &Shared, path: &str) {
    ST.with(|st| {
        let mut st = st.borrow_mut();
        let St { tree, stack, .. } = &mut *st;
        let Some(tree) = tree.as_mut() else { return };
        let Some(i) = cur(tree, stack).children.iter().position(|n| n.path == path) else { return };
        let (size, files) = {
            let n = &cur(tree, stack).children[i];
            (n.size, n.files)
        };
        let mut n = &mut *tree;
        for &k in stack.iter() {
            n.size -= size;
            n.files -= files;
            n = &mut n.children[k];
        }
        n.size -= size;
        n.files -= files;
        cur_mut(tree, stack).children.remove(i);
    });
    render_map(s);
}

fn trash_paths(s: &Shared, paths: Vec<String>, what: &str, done: Box<dyn Fn(&Shared)>) {
    let done = std::rc::Rc::new(done);
    crate::transfers::confirm(
        s,
        crate::tr("Przenieść do Kosza?"),
        &powerdesk_core::trf!("{what} trafi do Kosza – można je stamtąd przywrócić."),
        crate::tr("Do Kosza"),
        false,
        std::rc::Rc::new(move |s| {
            ops::start(Core, "trash".into(), paths.clone(), String::new(), ops::Conflict::Skip);
            crate::transfers::toast(s, crate::tr("Przeniesiono do Kosza"), crate::transfers::Kind::Ok, None);
            done(s);
        }),
    );
}

// ---------- Porządki ----------

fn clean_scan(s: &Shared) {
    let temp = s.borrow().ui.upgrade().is_some_and(|u| u.global::<SpaceState>().get_with_temp());
    let (root, gen) = ST.with(|st| {
        let mut st = st.borrow_mut();
        st.gen += 1;
        st.junk = None;
        st.picked.clear();
        (st.root.clone(), st.gen)
    });
    with_ui(s, |g| {
        g.set_busy(true);
        g.set_error("".into());
        g.set_progress("Szukam…".into());
        g.set_current("".into());
    });
    render_junk(s);
    std::thread::spawn(move || {
        let junk = space::cleanup_scan(&root, temp);
        let _ = slint::invoke_from_event_loop(move || {
            crate::with_state(|s| {
                if ST.with(|st| st.borrow().gen) != gen {
                    return;
                }
                ST.with(|st| {
                    let mut st = st.borrow_mut();
                    st.picked = junk.iter().map(|j| j.path.clone()).collect();
                    st.junk = Some(junk);
                });
                with_ui(s, |g| g.set_busy(false));
                render_junk(s);
            })
        });
    });
}

const GROUPS: [(&str, &str, &str); 3] = [
    ("empty", "Puste foldery", "Nie zawierają żadnych plików, także w podfolderach"),
    ("junk", "Śmieci", "Pamięć miniatur, pozostałości po macOS i Office, stare pliki tymczasowe i nieukończone pobierania"),
    ("temp", "Folder TEMP", "Pliki tymczasowe programów nieużywane od tygodnia"),
];

fn render_junk(s: &Shared) {
    ST.with(|st| {
        let st = st.borrow();
        let Some(junk) = &st.junk else {
            with_ui(s, |g| {
                g.set_scanned(false);
                g.set_junk(ModelRc::default());
            });
            return;
        };
        let mut rows = Vec::new();
        for (gi, (kind, title, hint)) in GROUPS.iter().enumerate() {
            let (title, hint) = (&crate::tr(*title), &crate::tr(*hint));
            let list: Vec<&Junk> = junk.iter().filter(|j| j.kind == *kind).collect();
            if list.is_empty() {
                continue;
            }
            let size: u64 = list.iter().map(|j| j.size).sum();
            rows.push(SpaceJunk {
                header: true,
                group: gi as i32,
                title: (*title).into(),
                hint: (*hint).into(),
                path: "".into(),
                reason: format!("{} · {}", list.len(), fmt::size(size)).into(),
                size: "".into(),
                when: "".into(),
                on: list.iter().all(|j| st.picked.contains(&j.path)),
            });
            for j in list.into_iter().take(500) {
                rows.push(SpaceJunk {
                    header: false,
                    group: gi as i32,
                    title: "".into(),
                    hint: "".into(),
                    path: j.path.clone().into(),
                    reason: j.reason.clone().into(),
                    size: if j.size > 0 { fmt::size(j.size) } else { String::new() }.into(),
                    when: fmt::date(j.modified).into(),
                    on: st.picked.contains(&j.path),
                });
            }
        }
        let size: u64 = junk.iter().filter(|j| st.picked.contains(&j.path)).map(|j| j.size).sum();
        let count = st.picked.len();
        with_ui(s, |g| {
            g.set_scanned(true);
            g.set_junk(ModelRc::new(VecModel::from(rows)));
            g.set_picked_count(count as i32);
            g.set_picked_text(powerdesk_core::trf!("Zaznaczono {count} · {}", fmt::size(size)).into());
        });
    });
}

fn toggle_pick(s: &Shared, path: &str) {
    ST.with(|st| {
        let mut st = st.borrow_mut();
        if !st.picked.remove(path) {
            st.picked.insert(path.to_string());
        }
    });
    render_junk(s);
}

fn toggle_group(s: &Shared, gi: usize) {
    ST.with(|st| {
        let mut st = st.borrow_mut();
        let kind = GROUPS[gi].0;
        let paths: Vec<String> = st.junk.iter().flatten().filter(|j| j.kind == kind).map(|j| j.path.clone()).collect();
        let all = paths.iter().all(|p| st.picked.contains(p));
        for p in paths {
            if all {
                st.picked.remove(&p);
            } else {
                st.picked.insert(p);
            }
        }
    });
    render_junk(s);
}

fn clean(s: &Shared) {
    let (paths, size) = ST.with(|st| {
        let st = st.borrow();
        let picked: Vec<&Junk> = st.junk.iter().flatten().filter(|j| st.picked.contains(&j.path)).collect();
        (picked.iter().map(|j| j.path.clone()).collect::<Vec<_>>(), picked.iter().map(|j| j.size).sum::<u64>())
    });
    if paths.is_empty() {
        return;
    }
    let what = format!("{} ({})", fmt::items(paths.len()), fmt::size(size));
    trash_paths(
        s,
        paths,
        &what,
        Box::new(|s| {
            ST.with(|st| {
                let mut st = st.borrow_mut();
                let St { junk, picked, .. } = &mut *st;
                if let Some(j) = junk {
                    j.retain(|x| !picked.contains(&x.path));
                }
                picked.clear();
            });
            render_junk(s);
        }),
    );
}

fn set_mode(s: &Shared, mode: i32) {
    with_ui(s, |g| g.set_mode(mode));
    let busy = s.borrow().ui.upgrade().is_some_and(|u| u.global::<SpaceState>().get_busy());
    if mode == 1 && ST.with(|st| st.borrow().junk.is_none()) && !busy {
        clean_scan(s);
    } else if mode == 0 && ST.with(|st| st.borrow().tree.is_none()) && !busy {
        scan(s);
    }
}

pub fn bind(ui: &crate::MainWindow, st: &Shared) {
    let g = ui.global::<SpaceState>();
    let s = st.clone();
    g.on_set_mode(move |m| set_mode(&s, m));
    let s = st.clone();
    g.on_rescan(move || {
        let mode = s.borrow().ui.upgrade().map(|u| u.global::<SpaceState>().get_mode()).unwrap_or(0);
        if mode == 1 { clean_scan(&s) } else { scan(&s) }
    });
    let s = st.clone();
    g.on_cancel(move || {
        space::cancel();
        ST.with(|st| st.borrow_mut().gen += 1);
        with_ui(&s, |g| {
            g.set_busy(false);
            g.set_error(crate::tr("Przerwano").into());
        });
    });
    let s = st.clone();
    g.on_resized(move |w, h| {
        ST.with(|st| {
            let mut st = st.borrow_mut();
            st.w = w;
            st.h = h;
        });
        render_map(&s);
    });
    let s = st.clone();
    g.on_open_tile(move |i| open_tile(&s, i as usize));
    let s = st.clone();
    g.on_tile_menu(move |i, x, y| tile_menu(&s, i as usize, x, y));
    let s = st.clone();
    g.on_crumb(move |i| {
        ST.with(|st| st.borrow_mut().stack.truncate(i as usize));
        render_map(&s);
    });
    let s = st.clone();
    g.on_toggle_temp(move || {
        with_ui(&s, |g| g.set_with_temp(!g.get_with_temp()));
        clean_scan(&s);
    });
    let s = st.clone();
    g.on_toggle_pick(move |p| toggle_pick(&s, &p));
    let s = st.clone();
    g.on_toggle_group(move |gi| toggle_group(&s, gi as usize));
    let s = st.clone();
    g.on_clean(move || clean(&s));
}
