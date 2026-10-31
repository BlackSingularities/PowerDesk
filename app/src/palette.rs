//! Dwie palety w jednym oknie: poleceń (Ctrl+Shift+P) i szybkiego przejścia (Ctrl+K).
//! Lista poleceń powstaje za każdym otwarciem, więc widać tylko to, co w danym miejscu ma sens
//! (np. „Wklej” dopiero wtedy, gdy w schowku są pliki). Lista miejsc przelicza się przy każdym
//! wpisanym znaku, bo zależy od tekstu (podfoldery, wpisana ścieżka). Dopasowanie szuka wszystkich
//! wpisanych cząstek w nazwie, bez względu na wielkość liter i polskie znaki.

use std::rc::Rc;

use powerdesk_core::clip;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::model::{GroupKey, SortKey, View};
use crate::{Palette, PaletteRow, Shared};

type Act = Rc<dyn Fn(&Shared)>;

pub struct Cmd {
    label: String,
    /// Grupa, w której pozycja się pokazuje („Widok”, „Karty”, „Dyski”…).
    group: String,
    icon: &'static str,
    kbd: &'static str,
    act: Act,
}

fn cmd(label: impl Into<String>, group: &str, icon: &'static str, kbd: &'static str, act: impl Fn(&Shared) + 'static) -> Cmd {
    Cmd { label: label.into(), group: group.to_string(), icon, kbd, act: Rc::new(act) }
}

/// Co pokazuje paleta.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Czynności programu (Ctrl+Shift+P).
    Commands,
    /// Foldery, dyski i wpisana ścieżka (Ctrl+K).
    Places,
}

thread_local! {
    /// Pozycje tego otwarcia palety (w kolejności pokazanej na liście).
    static SHOWN: std::cell::RefCell<Vec<Cmd>> = const { std::cell::RefCell::new(Vec::new()) };
    static MODE: std::cell::Cell<Mode> = const { std::cell::Cell::new(Mode::Commands) };
}

/// Wszystkie polecenia, które w tym miejscu mają sens.
fn all(s: &Shared) -> Vec<Cmd> {
    let (special, searching, split, has_sel, pinned, closed, view, hidden, sizes, side_mini) = {
        let b = s.borrow();
        let t = &b.tabs[b.active];
        (
            t.special(),
            t.search.is_some(),
            b.panes.len() == 2,
            !t.selected.is_empty(),
            t.pinned,
            !b.closed.is_empty(),
            t.view,
            b.prefs.show_hidden,
            b.prefs.folder_sizes,
            b.prefs.side_mini,
        )
    };
    let files = !special;
    let mut v = vec![
        cmd(crate::tr("Nowa karta"), crate::tr("Karty"), "plus", "Ctrl+T", crate::new_tab),
        cmd(crate::tr("Zamknij kartę"), crate::tr("Karty"), "close", "Ctrl+W", |s| {
            let i = s.borrow().active;
            crate::close_tab(s, i);
        }),
        cmd(if pinned { crate::tr("Odepnij kartę") } else { crate::tr("Przypnij kartę") }, crate::tr("Karty"), "pin", "", |s| {
            let i = s.borrow().active;
            crate::pin_tab(s, i);
        }),
        cmd(if split { "Jeden panel" } else { crate::tr("Podziel na dwa panele") }, crate::tr("Karty"), "split", "Ctrl+\\", crate::toggle_split),
    ];
    if closed {
        v.push(cmd(crate::tr("Przywróć zamkniętą kartę"), crate::tr("Karty"), "refresh", "Ctrl+Shift+T", crate::reopen_closed));
    }
    v.push(cmd(crate::tr("Lista"), crate::tr("Widok"), "list", "Ctrl+1", |s| crate::set_view(s, View::List)));
    v.push(cmd(crate::tr("Siatka"), crate::tr("Widok"), "grid", "Ctrl+2", |s| crate::set_view(s, View::Grid)));
    v.push(cmd(crate::tr("Galeria"), crate::tr("Widok"), "gallery", "Ctrl+3", |s| crate::set_view(s, View::Gallery)));
    let _ = view;
    v.push(cmd(if hidden { crate::tr("Ukryj pliki ukryte") } else { crate::tr("Pokaż pliki ukryte") }, crate::tr("Widok"), "eye", "Ctrl+H", |s| {
        {
            let mut b = s.borrow_mut();
            b.prefs.show_hidden = !b.prefs.show_hidden;
            b.prefs.save();
        }
        crate::refresh(s);
    }));
    v.push(cmd(if side_mini { crate::tr("Rozwiń panel boczny") } else { crate::tr("Zwiń panel boczny") }, crate::tr("Widok"), "sidebar", "Ctrl+B", crate::toggle_side_mini));
    v.push(cmd(crate::tr("Panel podglądu"), crate::tr("Widok"), "eye", "Alt+P", crate::preview::toggle));
    v.push(cmd(crate::tr("Półka"), crate::tr("Widok"), "archive", "", crate::shelf::toggle));
    v.push(cmd(if s.borrow().prefs.terminal_open { crate::tr("Ukryj terminal") } else { crate::tr("Pokaż terminal") }, crate::tr("Widok"), "terminal", "Ctrl+`", crate::termview::toggle));
    v.push(cmd(crate::tr("Ustawienia"), crate::tr("Program"), "settings", "Ctrl+,", crate::settings::open));
    v.push(cmd(crate::tr("Synchronizacja"), crate::tr("Program"), "sync", "", crate::syncview::open));
    v.push(cmd(if sizes { crate::tr("Nie licz rozmiarów folderów") } else { crate::tr("Licz rozmiary folderów") }, crate::tr("Widok"), "drive", "", |s| {
        {
            let mut b = s.borrow_mut();
            b.prefs.folder_sizes = !b.prefs.folder_sizes;
            b.prefs.save();
        }
        crate::refresh(s);
    }));
    for (label, key) in [
        (crate::tr("Nazwa"), SortKey::Name),
        (crate::tr("Rozmiar"), SortKey::Size),
        (crate::tr("Data modyfikacji"), SortKey::Modified),
        (crate::tr("Typ"), SortKey::Type),
    ] {
        v.push(cmd(powerdesk_core::trf!("Sortuj po: {label}"), crate::tr("Sortowanie"), "sort", "", move |s| crate::set_sort(s, key)));
    }
    for (label, key) in [(crate::tr("bez grup"), GroupKey::None), (crate::tr("po nazwie"), GroupKey::Name), (crate::tr("po dacie"), GroupKey::Modified), (crate::tr("po typie"), GroupKey::Kind)] {
        v.push(cmd(format!("Grupuj: {label}"), crate::tr("Sortowanie"), "grid", "", move |s| crate::set_group(s, key)));
    }
    if files {
        v.push(cmd(crate::tr("Nowy folder"), crate::tr("Pliki"), "folderPlus", "Ctrl+Shift+N", |s| crate::create(s, true)));
        v.push(cmd(crate::tr("Nowy plik tekstowy"), crate::tr("Pliki"), "filePlus", "", |s| crate::create(s, false)));
        if clip::has_files() {
            v.push(cmd(crate::tr("Wklej"), crate::tr("Pliki"), "paste", "Ctrl+V", |s| crate::menus::paste(s, None)));
        }
        v.push(cmd(crate::tr("Odśwież"), crate::tr("Pliki"), "refresh", "F5", |s| crate::load(s, true)));
        v.push(cmd(crate::tr("Zajętość i porządki w tym folderze"), crate::tr("Pliki"), "grid", "", crate::spaceview::open_here));
        v.push(cmd(crate::tr("Zaznacz wszystko"), crate::tr("Pliki"), "check", "Ctrl+A", crate::select_all));
        v.push(cmd(crate::tr("Właściwości"), crate::tr("Pliki"), "info", "Alt+Enter", |s| crate::command(s, "props")));
        v.push(cmd(crate::tr("Folder wyżej"), crate::tr("Nawigacja"), "up", "Alt+↑", crate::up));
    }
    if has_sel {
        v.push(cmd(crate::tr("Kopiuj"), crate::tr("Zaznaczenie"), "copy", "Ctrl+C", |s| crate::command(s, "copy")));
        v.push(cmd(crate::tr("Wytnij"), crate::tr("Zaznaczenie"), "cut", "Ctrl+X", |s| crate::command(s, "cut")));
        v.push(cmd(crate::tr("Zmień nazwę"), crate::tr("Zaznaczenie"), "edit", "F2", crate::start_rename));
        v.push(cmd(crate::tr("Masowa zmiana nazw"), crate::tr("Zaznaczenie"), "edit", "", crate::bulk::open));
        v.push(cmd(crate::tr("Do Kosza"), crate::tr("Zaznaczenie"), "trash", crate::tr("Delete"), |s| crate::command(s, "trash")));
        v.push(cmd(crate::tr("Dodaj na półkę"), crate::tr("Zaznaczenie"), "archive", "Ctrl+Shift+S", crate::shelf::shortcut));
        v.push(cmd(crate::tr("Skopiuj ścieżkę"), crate::tr("Zaznaczenie"), "link", "Ctrl+Shift+C", |s| {
            let sel = crate::selected_paths(s);
            let _ = clip::set_text(&sel.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("
"));
        }));
    }
    if searching {
        v.push(cmd(crate::tr("Zakończ wyszukiwanie"), crate::tr("Nawigacja"), "close", crate::tr("Esc"), crate::clear_search));
    }
    v
}

/// Miejsca, do których można przejść: podfoldery tego folderu, szybki dostęp, dyski,
/// historia karty i – gdy tekst wygląda na ścieżkę – ona sama.
fn places(s: &Shared, query: &str) -> Vec<Cmd> {
    let mut v = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut add = |v: &mut Vec<Cmd>, name: String, group: &str, icon: &'static str, path: String| {
        if path.is_empty() || !seen.insert(path.to_lowercase()) {
            return;
        }
        let p = path.clone();
        v.push(Cmd { label: name, group: group.to_string(), icon, kbd: "", act: Rc::new(move |s: &Shared| crate::navigate(s, &p, true)) });
    };
    // Wpisana ścieżka – pierwsza na liście, gdy istnieje.
    let typed = query.trim().trim_matches('"');
    if (typed.contains(':') || typed.starts_with("\\\\")) && std::path::Path::new(typed).is_dir() {
        add(&mut v, typed.to_string(), crate::tr("Wpisana ścieżka"), "fwd", typed.to_string());
    }
    // Podfoldery otwartego folderu.
    {
        let b = s.borrow();
        let t = &b.tabs[b.active];
        for e in t.entries.iter().filter(|e| e.dir).take(400) {
            add(&mut v, e.name.clone(), crate::tr("W tym folderze"), "folder", e.path.clone());
        }
    }
    let known: [(&str, Option<std::path::PathBuf>, &str); 7] = [
        (crate::tr("Katalog domowy"), dirs::home_dir(), "home"),
        (crate::tr("Pulpit"), dirs::desktop_dir(), "desktop"),
        (crate::tr("Pobrane"), dirs::download_dir(), "download"),
        (crate::tr("Dokumenty"), dirs::document_dir(), "doc"),
        (crate::tr("Obrazy"), dirs::picture_dir(), "image"),
        (crate::tr("Wideo"), dirs::video_dir(), "video"),
        (crate::tr("Muzyka"), dirs::audio_dir(), "music"),
    ];
    for (name, p, icon) in known {
        let Some(p) = p.filter(|p| p.is_dir()) else { continue };
        add(&mut v, name.to_string(), crate::tr("Szybki dostęp"), icon, p.to_string_lossy().to_string());
    }
    {
        let b = s.borrow();
        for d in b.drives.iter().filter(|d| d.online) {
            let name = if d.label.is_empty() { powerdesk_core::trf!("Dysk ({})", d.letter) } else { format!("{} ({})", d.label, d.letter) };
            let icon = match d.kind.as_str() {
                "removable" => "usb",
                "network" => "network",
                _ => "drive",
            };
            add(&mut v, name, crate::tr("Dyski"), icon, d.path.clone());
        }
        // Historia karty – od ostatnio odwiedzonych.
        let t = &b.tabs[b.active];
        for p in t.history.iter().rev() {
            if p.starts_with("ofp:") {
                continue;
            }
            let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
            add(&mut v, name, crate::tr("Historia"), "back", p.clone());
        }
        // Zapisane wyszukiwania są w panelu bocznym – tu dajemy ich korzenie.
        for q in b.prefs.saved_searches.iter() {
            add(&mut v, q.name.clone(), crate::tr("Zapisane"), "search", q.root.clone());
        }
    }
    v
}

/// Otwiera paletę poleceń (Ctrl+Shift+P).
pub fn open(s: &Shared) {
    show(s, Mode::Commands);
}

/// Otwiera paletę szybkiego przejścia (Ctrl+K).
pub fn open_places(s: &Shared) {
    show(s, Mode::Places);
}

fn show(s: &Shared, mode: Mode) {
    MODE.with(|m| m.set(mode));
    let list = match mode {
        Mode::Commands => all(s),
        Mode::Places => places(s, ""),
    };
    SHOWN.with(|c| *c.borrow_mut() = list);
    if let Some(ui) = s.borrow().ui.upgrade() {
        let g = ui.global::<Palette>();
        g.set_query("".into());
        g.set_sel(0);
        g.set_hint(if mode == Mode::Places { crate::tr("Wpisz nazwę folderu albo ścieżkę…").into() } else { crate::tr("Wpisz polecenie…").into() });
        g.set_open(true);
    }
    render(s, "");
}

/// Skrót naciśnięty przy otwartej palecie: ta sama ją zamyka, druga przełącza tryb.
pub fn toggle(s: &Shared, places: bool) {
    let want = if places { Mode::Places } else { Mode::Commands };
    if MODE.with(|m| m.get()) == want {
        close(s);
    } else {
        show(s, want);
    }
}

pub fn close(s: &Shared) {
    SHOWN.with(|c| c.borrow_mut().clear());
    if let Some(ui) = s.borrow().ui.upgrade() {
        let g = ui.global::<Palette>();
        g.set_open(false);
        g.set_rows(ModelRc::default());
    }
}

/// Czy paleta jest otwarta – klawisze idą wtedy do niej, nie do listy plików.
pub fn is_open(s: &Shared) -> bool {
    s.borrow().ui.upgrade().is_some_and(|ui| ui.global::<Palette>().get_open())
}

/// Polskie znaki na łacińskie i małe litery – „Zmień” znajdzie się po wpisaniu „zmien”.
pub(crate) fn fold(t: &str) -> String {
    t.chars()
        .map(|c| match c.to_lowercase().next().unwrap_or(c) {
            'ą' => 'a',
            'ć' => 'c',
            'ę' => 'e',
            'ł' => 'l',
            'ń' => 'n',
            'ó' => 'o',
            'ś' => 's',
            'ź' | 'ż' => 'z',
            x => x,
        })
        .collect()
}

/// Numery poleceń pasujących do zapytania: każda cząstka musi być w nazwie albo w grupie.
fn matching(query: &str) -> Vec<usize> {
    let q = fold(query);
    let parts: Vec<&str> = q.split_whitespace().collect();
    SHOWN.with(|c| {
        c.borrow()
            .iter()
            .enumerate()
            .filter(|(_, cmd)| {
                let hay = fold(&format!("{} {}", cmd.label, cmd.group));
                parts.iter().all(|p| hay.contains(p))
            })
            .map(|(i, _)| i)
            .collect()
    })
}

fn render(s: &Shared, query: &str) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let g = ui.global::<Palette>();
    let rows: Vec<PaletteRow> = SHOWN.with(|c| {
        let list = c.borrow();
        matching(query)
            .into_iter()
            .map(|i| {
                let cmd = &list[i];
                PaletteRow {
                    index: i as i32,
                    label: cmd.label.clone().into(),
                    group: cmd.group.clone().into(),
                    icon: crate::icons::icon(cmd.icon).into(),
                    kbd: cmd.kbd.into(),
                }
            })
            .collect()
    });
    let sel = g.get_sel().clamp(0, (rows.len() as i32 - 1).max(0));
    g.set_sel(sel);
    g.set_rows(ModelRc::new(VecModel::from(rows)));
}

/// Zmiana tekstu w polu – lista od nowa, wskazanie na pierwszym wierszu.
pub fn query(s: &Shared, text: &str) {
    if MODE.with(|m| m.get()) == Mode::Places {
        // Wpisana ścieżka zmienia samą listę, nie tylko dopasowanie.
        let list = places(s, text);
        SHOWN.with(|c| *c.borrow_mut() = list);
    }
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.global::<Palette>().set_sel(0);
    }
    render(s, text);
}

/// Strzałki w górę i w dół po liście (z zawinięciem).
pub fn step(s: &Shared, d: i32) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let g = ui.global::<Palette>();
    let n = g.get_rows().row_count() as i32;
    if n > 0 {
        g.set_sel((g.get_sel() + d).rem_euclid(n));
    }
}

/// Wykonanie wskazanego polecenia (Enter) albo wybranego myszą (`row` – numer wiersza listy).
pub fn run(s: &Shared, row: Option<i32>) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let g = ui.global::<Palette>();
    let row = row.unwrap_or_else(|| g.get_sel());
    let rows = g.get_rows();
    let Some(r) = rows.row_data(row.max(0) as usize) else { return };
    let act = SHOWN.with(|c| c.borrow().get(r.index as usize).map(|cmd| cmd.act.clone()));
    close(s);
    if let Some(act) = act {
        act(s);
    }
}
