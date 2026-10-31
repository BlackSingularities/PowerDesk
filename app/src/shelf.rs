//! Półka – schowek plików (Shelf.svelte): pliki zbierane z różnych folderów, potem kopiowane albo
//! przenoszone naraz do otwartego folderu. Niezależna od schowka Windows; zawartość pamiętana w Prefs.

use std::collections::HashSet;
use std::path::Path;

use powerdesk_core::fsx::{self, Entry};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::transfers::{self, Kind};
use crate::{Shared, ShelfItem, ShelfState};

#[derive(Default)]
pub struct Shelf {
    pub items: Vec<Entry>,
    pub picked: HashSet<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

/// Półka z zapisanych ścieżek – bez tych, których już nie ma.
pub fn restore(s: &Shared) {
    let paths = s.borrow().prefs.shelf.clone();
    s.borrow_mut().shelf.items = paths.iter().filter_map(|p| fsx::stat(p).ok()).collect();
}

fn save(s: &Shared) {
    let mut b = s.borrow_mut();
    b.prefs.shelf = b.shelf.items.iter().map(|e| e.path.clone()).collect();
    b.prefs.save();
}

/// Dodaje elementy (bez powtórek, bez ekranów specjalnych i wnętrza archiwów) i pokazuje półkę.
pub fn add(s: &Shared, paths: &[String]) {
    let added: Vec<Entry> = {
        let b = s.borrow();
        let have: HashSet<String> = b.shelf.items.iter().map(|e| e.path.to_lowercase()).collect();
        let mut seen = HashSet::new();
        paths
            .iter()
            .filter(|p| !p.starts_with("ofp:") && !crate::archives::inside(p) && !have.contains(&p.to_lowercase()) && seen.insert(p.to_lowercase()))
            .filter_map(|p| fsx::stat(p).ok())
            .collect()
    };
    if added.is_empty() {
        return set_open(s, true);
    }
    let what = if added.len() == 1 { format!("„{}”", added[0].name) } else { format!("{} elem.", added.len()) };
    s.borrow_mut().shelf.items.extend(added);
    save(s);
    set_open(s, true);
    transfers::toast(s, powerdesk_core::trf!("Dodano do półki: {what}"), Kind::Ok, None);
}

/// Ctrl+Shift+S: zaznaczone na półkę, a bez zaznaczenia – pokaż albo schowaj półkę.
pub fn shortcut(s: &Shared) {
    let sel = crate::selected_paths(s);
    if sel.is_empty() {
        let open = s.borrow().prefs.shelf_open;
        set_open(s, !open);
    } else {
        add(s, &sel);
    }
}

pub fn set_open(s: &Shared, open: bool) {
    {
        let mut b = s.borrow_mut();
        b.prefs.shelf_open = open;
        b.prefs.save();
    }
    render(s);
}

pub fn toggle(s: &Shared) {
    let open = s.borrow().prefs.shelf_open;
    set_open(s, !open);
}

/// Klik w pozycję: sama, z Ctrl – dołącza albo odłącza.
pub fn pick(s: &Shared, i: usize, ctrl: bool) {
    {
        let mut b = s.borrow_mut();
        let Some(p) = b.shelf.items.get(i).map(|e| e.path.clone()) else { return };
        let picked = &mut b.shelf.picked;
        let was = picked.contains(&p);
        if !ctrl {
            let only = was && picked.len() == 1;
            picked.clear();
            if !only {
                picked.insert(p);
            }
        } else if was {
            picked.remove(&p);
        } else {
            picked.insert(p);
        }
    }
    render(s);
}

pub fn remove(s: &Shared, i: usize) {
    {
        let mut b = s.borrow_mut();
        if i >= b.shelf.items.len() {
            return;
        }
        let e = b.shelf.items.remove(i);
        b.shelf.picked.remove(&e.path);
    }
    save(s);
    render(s);
}

pub fn clear(s: &Shared) {
    {
        let mut b = s.borrow_mut();
        b.shelf.items.clear();
        b.shelf.picked.clear();
    }
    save(s);
    render(s);
}

/// Wybrane pozycje, a bez wyboru – cała półka.
fn chosen(s: &Shared) -> Vec<String> {
    let b = s.borrow();
    let all = b.shelf.items.iter().map(|e| e.path.clone());
    if b.shelf.picked.is_empty() {
        all.collect()
    } else {
        all.filter(|p| b.shelf.picked.contains(p)).collect()
    }
}

/// Kopiuj albo przenieś do otwartego folderu; przeniesione znikają z półki.
pub fn put(s: &Shared, mv: bool) {
    let dest = crate::tab_mut(s, |t| t.path.clone());
    if dest.starts_with("ofp:") {
        return transfers::toast(s, crate::tr("Otwórz folder, do którego mają trafić pliki"), Kind::Info, None);
    }
    let items = chosen(s);
    if items.is_empty() {
        return;
    }
    transfers::transfer(s, items.clone(), &dest, if mv { "move" } else { "copy" });
    {
        let mut b = s.borrow_mut();
        if mv {
            b.shelf.items.retain(|e| !items.contains(&e.path));
        }
        b.shelf.picked.clear();
    }
    save(s);
    render(s);
}

/// Przeciąganie z półki (OLE) – wybrane albo sama wskazana pozycja.
pub fn drag(s: &Shared, i: usize) {
    let paths = {
        let b = s.borrow();
        let Some(e) = b.shelf.items.get(i) else { return };
        if b.shelf.picked.contains(&e.path) {
            drop(b);
            chosen(s)
        } else {
            vec![e.path.clone()]
        }
    };
    crate::dragdrop::start_paths(s, paths, |s| prune(s));
}

/// Po przeciągnięciu z półki: pozycje przeniesione gdzie indziej znikają.
fn prune(s: &Shared) {
    let gone = {
        let mut b = s.borrow_mut();
        let before = b.shelf.items.len();
        b.shelf.items.retain(|e| Path::new(&e.path).exists());
        before != b.shelf.items.len()
    };
    if gone {
        save(s);
        render(s);
    }
}

/// Czy dotarła ikona którejś pozycji (do odświeżenia po wczytaniu ikon).
pub fn shows_any(s: &Shared, paths: &HashSet<String>) -> bool {
    let b = s.borrow();
    b.prefs.shelf_open && b.shelf.items.iter().any(|e| paths.contains(&e.path))
}

pub fn render(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let g = ui.global::<ShelfState>();
    let t = &b.tabs[b.active];
    let scale = ui.window().scale_factor();
    let items: Vec<ShelfItem> = b
        .shelf
        .items
        .iter()
        .map(|e| {
            let img = crate::thumbs::get(&e.path, (32.0 * scale) as u32, e.dir);
            ShelfItem {
                name: e.name.clone().into(),
                place: Path::new(&e.path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default().into(),
                path: e.path.clone().into(),
                picked: b.shelf.picked.contains(&e.path),
                has_img: img.is_some(),
                img: img.unwrap_or_default(),
                dir: e.dir,
            }
        })
        .collect();
    let total: u64 = b.shelf.items.iter().filter(|e| !e.dir).map(|e| e.size).sum();
    let n = b.shelf.items.len();
    g.set_count(n as i32);
    g.set_summary(if n > 0 { format!("{n} · {}", crate::fmt::size(total)).into() } else { "".into() });
    let special = t.special();
    let picked = b.shelf.picked.len();
    let target = if special { crate::tr("Otwórz folder docelowy").to_string() } else { format!("Do: {}", t.title()) };
    g.set_target(if picked > 0 { powerdesk_core::trf!("{target} · wybrane: {picked}") } else { target }.into());
    g.set_can_put(!special && n > 0);
    // Folderu nie da się przenieść do niego samego ani do jego wnętrza.
    let low = t.path.to_lowercase();
    g.set_can_move(!special && n > 0 && !b.shelf.items.iter().any(|e| same(&e.path, &t.path) || low.starts_with(&format!("{}\\", e.path.to_lowercase().trim_end_matches('\\')))));
    g.set_items(ModelRc::new(VecModel::from(items)));
    g.set_open(b.prefs.shelf_open);
}
