//! Menu wyskakujące (ContextMenu.svelte) i ich zawartość: menu elementu, tła, sortowania, okruszków.

use std::path::Path;
use std::rc::Rc;

use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::{archive, clip, fsx, ops, shell};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::model::{Col, SortKey, View};
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

fn sub(label: &str, icon: &'static str, items: Vec<Item>) -> Item {
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
    fn danger(mut self) -> Item {
        self.danger = true;
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

pub fn show(s: &Shared, items: Vec<Item>, x: f32, y: f32) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    // Szerokość jak w CSS: min 220, max 380 – według najdłuższej etykiety.
    let longest = items.iter().map(|i| i.label.chars().count() + i.kbd.chars().count()).max().unwrap_or(0);
    let width = (longest as f32 * 7.0 + 96.0).clamp(220.0, 380.0);
    let size = ui.window().size().to_logical(ui.window().scale_factor());
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
    ui.set_menu_x(mx);
    ui.set_menu_y(my);
    ui.set_menu_width(width);
    ui.set_menu_entries(ModelRc::new(VecModel::from(entries)));
    s.borrow_mut().menu = Some(Open { items, x: mx, y: my, width });
    ui.invoke_show_menu();
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
pub fn item_menu(s: &Shared, x: f32, y: f32) {
    let sel = selection(s);
    if sel.is_empty() {
        return;
    }
    let one = (sel.len() == 1).then(|| sel[0].clone());
    let searching = s.borrow().tabs[s.borrow().active].search.is_some();
    let mut v = Vec::new();
    match &one {
        Some(e) if e.dir => {
            let p = e.path.clone();
            v.push(it("Otwórz", "folder", move |s| crate::navigate(s, &p, true)).kbd("Enter"));
            let p = e.path.clone();
            v.push(it("Otwórz w nowej karcie", "plus", move |s| crate::new_tab_at(s, &p, false)).kbd("Ctrl+Enter"));
        }
        Some(e) => {
            let p = e.path.clone();
            v.push(it("Otwórz", "open", move |_| {
                let _ = open::that_detached(&p);
            })
            .kbd("Enter"));
            v.push(it("Podgląd", "eye", |_| {}).kbd("Spacja").later());
            let p = e.path.clone();
            v.push(it("Otwórz za pomocą…", "app", move |s| {
                shell::open_with(crate::hwnd(s), &p);
            }));
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
        v.push(it(format!("Zmień nazwy ({})…", sel.len()), "edit", |_| {}).kbd("F2").later());
    }
    let p2 = ps.clone();
    v.push(it("Spakuj do ZIP…", "archive", move |s| zip(s, &p2)));
    if sel.len() > 1 {
        v.push(it("Przenieś do nowego folderu…", "folderPlus", |_| {}).later());
    }
    let p2 = ps.clone();
    v.push(it("Do Kosza", "trash", move |_| trash(&p2)).kbd("Del"));
    v.push(it("Usuń trwale", "trash", |_| {}).kbd("Shift+Del").danger().later());
    v.push(sep());
    if let Some(e) = one.as_ref().filter(|e| e.dir) {
        v.push(it("Przypnij w panelu bocznym", "pin", |_| {}).later());
        v.push(it("Pokaż na ekranie Komputer", "grid", |_| {}).later());
        let p = e.path.clone();
        v.push(it("Terminal tutaj", "terminal", move |_| terminal(&p)));
    }
    v.push(it("Dodaj do schowka plików", "archive", |_| {}).kbd("Ctrl+Shift+S").later());
    let first = ps[0].clone();
    v.push(it("Pokaż w Eksploratorze", "open", move |_| reveal(&first)));
    v.push(sep());
    let first = ps[0].clone();
    v.push(
        it("Właściwości", "info", move |s| {
            shell::show_properties(crate::hwnd(s), &first);
        })
        .kbd("Alt+Enter"),
    );
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
    v.push(it("Odśwież", "refresh", |s| crate::load(s, true)).kbd("F5"));
    v.push(it("Zaznacz wszystko", "check", crate::select_all).kbd("Ctrl+A"));
    v.push(sep());
    v.push(it("Przypnij ten folder", "pin", |_| {}).later());
    let p = path.clone();
    v.push(it("Terminal tutaj", "terminal", move |_| terminal(&p)));
    v.push(it("Zajętość i porządki", "grid", |_| {}).later());
    let p = path.clone();
    v.push(it("Otwórz w Eksploratorze", "open", move |_| {
        let _ = open::that_detached(&p);
    }));
    v.push(sep());
    let p = path.clone();
    v.push(it("Właściwości folderu", "info", move |s| {
        shell::show_properties(crate::hwnd(s), &p);
    }));
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
    show(s, v, x, y);
}

pub fn new_menu(s: &Shared, x: f32, y: f32) {
    show(
        s,
        vec![it("Folder", "folderPlus", |s| crate::create(s, true)).kbd("Ctrl+Shift+N"), it("Dokument tekstowy", "filePlus", |s| crate::create(s, false))],
        x,
        y,
    );
}

/// Podfoldery okruszka.
pub fn crumb_menu(s: &Shared, path: &str, x: f32, y: f32) {
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
    ops::start(Core, if cut { "move" } else { "copy" }.into(), files, dest, ops::Conflict::Rename);
}

pub fn trash(paths: &[String]) {
    if !paths.is_empty() {
        ops::start(Core, "trash".into(), paths.to_vec(), String::new(), ops::Conflict::Skip);
    }
}

fn zip(s: &Shared, sources: &[String]) {
    let dir = s.borrow().tabs[s.borrow().active].path.clone();
    let name = if sources.len() == 1 {
        Path::new(&sources[0]).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Archiwum".into())
    } else {
        Path::new(&dir).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Archiwum".into())
    };
    let dest = Path::new(&dir).join(format!("{name}.zip")).to_string_lossy().to_string();
    let sources = sources.to_vec();
    std::thread::spawn(move || {
        let _ = archive::create_zip(&sources, &dest);
    });
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
