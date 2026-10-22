#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! PowerDesk – natywny interfejs (Slint) nad backendem `powerdesk-core`.

mod fmt;
mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use powerdesk_core::{drives, fsx, shell};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel, Weak};

slint::include_modules!();

const HOME: &str = "ofp:home";

struct Tab {
    path: String,
    history: Vec<String>,
    h_index: usize,
    entries: Vec<fsx::Entry>,
    selected: Vec<bool>,
    anchor: Option<usize>,
    /// Rośnie przy każdym wczytaniu – spóźnione ikony starego folderu są odrzucane.
    generation: u64,
}

impl Tab {
    fn new(path: &str) -> Tab {
        Tab { path: path.into(), history: vec![path.into()], h_index: 0, entries: Vec::new(), selected: Vec::new(), anchor: None, generation: 0 }
    }
    fn title(&self) -> String {
        if self.path == HOME {
            return "Ten komputer".into();
        }
        let p = Path::new(&self.path);
        p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.path.trim_end_matches('\\').to_string())
    }
}

struct State {
    tabs: Vec<Tab>,
    active: usize,
    rows: Rc<VecModel<FileRow>>,
    ui: Weak<MainWindow>,
}

type Shared = Rc<RefCell<State>>;

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    apply_theme(&ui);

    let rows = Rc::new(VecModel::<FileRow>::default());
    ui.set_rows(ModelRc::from(rows.clone()));
    let start = dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "C:\\".into());
    let st: Shared = Rc::new(RefCell::new(State { tabs: vec![Tab::new(&start)], active: 0, rows, ui: ui.as_weak() }));
    load(&st);

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

    ui.on_navigate(cb(&st, |s, p: SharedString| navigate(s, p.as_str(), true)));
    ui.on_back(cb0(&st, |s| {
        let t = s.borrow().tabs[s.borrow().active].h_index;
        if t > 0 {
            jump(s, t - 1);
        }
    }));
    ui.on_forward(cb0(&st, |s| {
        let (i, n) = {
            let b = s.borrow();
            let t = &b.tabs[b.active];
            (t.h_index, t.history.len())
        };
        if i + 1 < n {
            jump(s, i + 1);
        }
    }));
    ui.on_up(cb0(&st, |s| {
        let p = { s.borrow().tabs[s.borrow().active].path.clone() };
        if let Some(parent) = Path::new(&p).parent() {
            navigate(s, &parent.to_string_lossy(), true);
        }
    }));
    ui.on_refresh(cb0(&st, |s| load(s)));
    ui.on_open(cb(&st, |s, i: i32| open(s, i as usize)));
    ui.on_select({
        let st = st.clone();
        move |i, ctrl, shift| select(&st, i as usize, ctrl, shift)
    });
    ui.on_context(|_i, _x, _y| {});
    ui.on_new_tab(cb0(&st, |s| {
        let p = { s.borrow().tabs[s.borrow().active].path.clone() };
        {
            let mut b = s.borrow_mut();
            b.tabs.push(Tab::new(&p));
            b.active = b.tabs.len() - 1;
        }
        load(s);
    }));
    ui.on_activate_tab(cb(&st, |s, i: i32| {
        s.borrow_mut().active = i as usize;
        load(s);
    }));
    ui.on_close_tab(cb(&st, |s, i: i32| {
        {
            let mut b = s.borrow_mut();
            if b.tabs.len() == 1 {
                drop(b);
                let _ = slint::quit_event_loop();
                return;
            }
            b.tabs.remove(i as usize);
            if b.active >= b.tabs.len() || b.active > i as usize {
                b.active = b.active.saturating_sub(1);
            }
        }
        load(s);
    }));

    ui.run()
}

// Pomocnicze opakowania callbacków – stan jest współdzielony przez Rc.
fn cb<A: 'static>(st: &Shared, f: impl Fn(&Shared, A) + 'static) -> impl Fn(A) + 'static {
    let st = st.clone();
    move |a| f(&st, a)
}
fn cb0(st: &Shared, f: impl Fn(&Shared) + 'static) -> impl Fn() + 'static {
    let st = st.clone();
    move || f(&st)
}

fn navigate(s: &Shared, path: &str, push: bool) {
    let mut path = path.trim().to_string();
    if path.len() == 2 && path.ends_with(':') {
        path.push('\\');
    }
    {
        let mut b = s.borrow_mut();
        let a = b.active;
        let t = &mut b.tabs[a];
        if push {
            t.history.truncate(t.h_index + 1);
            t.history.push(path.clone());
            t.h_index = t.history.len() - 1;
        }
        t.path = path;
    }
    load(s);
}

fn jump(s: &Shared, idx: usize) {
    let p = {
        let mut b = s.borrow_mut();
        let a = b.active;
        let t = &mut b.tabs[a];
        t.h_index = idx;
        t.history[idx].clone()
    };
    navigate(s, &p, false);
}

fn load(s: &Shared) {
    let (path, generation) = {
        let mut b = s.borrow_mut();
        let a = b.active;
        let t = &mut b.tabs[a];
        t.generation += 1;
        (t.path.clone(), t.generation)
    };
    let mut list = fsx::list_dir(&path).unwrap_or_default();
    list.retain(|e| !(e.hidden || e.system));
    list.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    {
        let mut b = s.borrow_mut();
        let a = b.active;
        let t = &mut b.tabs[a];
        t.selected = vec![false; list.len()];
        t.anchor = None;
        t.entries = list;
    }
    render(s);
    request_icons(s, generation);
}

fn render(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.tabs[b.active];
    let rows: Vec<FileRow> = t
        .entries
        .iter()
        .zip(&t.selected)
        .map(|(e, sel)| FileRow {
            name: e.name.clone().into(),
            path: e.path.clone().into(),
            dir: e.dir,
            modified: fmt::date(e.modified).into(),
            kind: fmt::type_label(e).into(),
            size: if e.dir { SharedString::new() } else { fmt::size(e.size).into() },
            icon: Image::default(),
            has_icon: false,
            selected: *sel,
            hidden: e.hidden,
        })
        .collect();
    b.rows.set_vec(rows);
    ui.set_path(t.path.clone().into());
    ui.set_can_back(t.h_index > 0);
    ui.set_can_forward(t.h_index + 1 < t.history.len());
    ui.set_status(fmt::items(t.entries.len()).into());
    ui.set_tabs(ModelRc::new(VecModel::from(
        b.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabData { title: t.title().into(), icon: tab_icon(&t.path), active: i == b.active, pinned: false, loading: false })
            .collect::<Vec<_>>(),
    )));
    ui.set_side(ModelRc::new(VecModel::from(side_items(&t.path))));
}

fn refresh_selection(s: &Shared) {
    let b = s.borrow();
    let t = &b.tabs[b.active];
    for (i, sel) in t.selected.iter().enumerate() {
        if let Some(mut r) = b.rows.row_data(i) {
            if r.selected != *sel {
                r.selected = *sel;
                b.rows.set_row_data(i, r);
            }
        }
    }
    if let Some(ui) = b.ui.upgrade() {
        let n = t.selected.iter().filter(|x| **x).count();
        let base = fmt::items(t.entries.len());
        ui.set_status(if n > 0 { format!("{base} · zaznaczono {n}") } else { base }.into());
    }
}

fn select(s: &Shared, i: usize, ctrl: bool, shift: bool) {
    {
        let mut b = s.borrow_mut();
        let a = b.active;
        let t = &mut b.tabs[a];
        if i >= t.selected.len() {
            return;
        }
        if shift {
            let from = t.anchor.unwrap_or(i);
            let (lo, hi) = (from.min(i), from.max(i));
            if !ctrl {
                t.selected.iter_mut().for_each(|x| *x = false);
            }
            t.selected[lo..=hi].iter_mut().for_each(|x| *x = true);
        } else if ctrl {
            t.selected[i] = !t.selected[i];
            t.anchor = Some(i);
        } else {
            t.selected.iter_mut().for_each(|x| *x = false);
            t.selected[i] = true;
            t.anchor = Some(i);
        }
    }
    refresh_selection(s);
}

fn open(s: &Shared, i: usize) {
    let e = {
        let b = s.borrow();
        b.tabs[b.active].entries.get(i).cloned()
    };
    let Some(e) = e else { return };
    if e.dir {
        navigate(s, &e.path, true);
    } else {
        let _ = open::that_detached(&e.path);
    }
}

/// Ikony z systemowej pamięci podręcznej (IShellItemImageFactory) – dekodowanie poza wątkiem UI.
fn request_icons(s: &Shared, generation: u64) {
    let b = s.borrow();
    let t = &b.tabs[b.active];
    let scale = b.ui.upgrade().map(|u| u.window().scale_factor()).unwrap_or(1.0);
    let px = (18.0 * scale).round() as u32;
    for (i, e) in t.entries.iter().enumerate() {
        let ui = b.ui.clone();
        let path = e.path.clone();
        shell::request(shell::ThumbReq {
            path: path.clone(),
            size: px.max(16),
            icon_only: true,
            reply: Box::new(move |res| {
                let Some((data, _)) = res else { return };
                let Ok(img) = image::load_from_memory(&data) else { return };
                let rgba = img.to_rgba8();
                let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height());
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui.upgrade() else { return };
                    let _ = generation;
                    let rows = ui.get_rows();
                    if let Some(mut r) = rows.row_data(i) {
                        if r.path.as_str() == path {
                            r.icon = Image::from_rgba8(buf);
                            r.has_icon = true;
                            rows.set_row_data(i, r);
                        }
                    }
                });
            }),
        });
    }
}

fn ic(name: &str) -> SharedString {
    icons::icon(name).into()
}

fn tab_icon(path: &str) -> SharedString {
    if path == HOME {
        ic("pc")
    } else if path.len() <= 3 && path.contains(':') {
        ic("drive")
    } else if path.starts_with("\\\\") {
        ic("network")
    } else {
        ic("folder")
    }
}

fn side_items(current: &str) -> Vec<SideItem> {
    let mut v = Vec::new();
    let same = |p: &str| p.trim_end_matches('\\').eq_ignore_ascii_case(current.trim_end_matches('\\'));
    let known: [(&str, Option<PathBuf>, SharedString); 6] = [
        ("Pulpit", dirs::desktop_dir(), ic("desktop")),
        ("Pobrane", dirs::download_dir(), ic("download")),
        ("Dokumenty", dirs::document_dir(), ic("doc")),
        ("Obrazy", dirs::picture_dir(), ic("image")),
        ("Wideo", dirs::video_dir(), ic("video")),
        ("Muzyka", dirs::audio_dir(), ic("music")),
    ];
    let mut first = true;
    for (name, p, icon) in known {
        let Some(p) = p.filter(|p| p.is_dir()) else { continue };
        let p = p.to_string_lossy().to_string();
        v.push(SideItem {
            section: if std::mem::take(&mut first) { "Szybki dostęp".into() } else { SharedString::new() },
            name: name.into(),
            icon,
            active: same(&p),
            path: p.into(),
            detail: SharedString::new(),
            usage: 0.0,
        });
    }
    let mut first = true;
    for d in drives::list().into_iter().filter(|d| d.online) {
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
            active: same(&d.path),
            path: d.path.clone().into(),
            detail: if d.total > 0 { format!("{} wolne z {}", fmt::size(d.free), fmt::size(d.total)).into() } else { SharedString::new() },
            usage: if d.total > 0 { used as f32 / d.total as f32 } else { 0.0 },
        });
    }
    v
}

/// Motyw jak w wersji webowej: ustawienie z state.json, „system” – według Windows.
fn apply_theme(ui: &MainWindow) {
    let state = dirs::data_dir().map(|d| d.join("OpenFilePowerWin").join("state.json"));
    let theme = state
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| {
            let i = s.find("\"theme\":\"")? + 9;
            Some(s[i..].split('"').next()?.to_string())
        })
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
