//! Okno „Otwórz” / „Zapisz jako” / „Wybierz folder” dla innych programów: `powerdesk.exe --pick-file`.
//! Uruchamia je biblioteka `powerdesk-dialog` (skrzynka `dialog/`) w procesie programu – opis okna
//! przychodzi jednym wierszem JSON na stdin, wynik wraca na stdout (protokół w dialog/src/lib.rs).
//! Koniec procesu bez odpowiedzi (awaria) to dla biblioteki znak, żeby pokazać okno Windows.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use powerdesk_core::filedlg;
use powerdesk_core::fsx::{self, Entry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, VecModel, Weak};

use crate::model::{natural, Prefs};
use crate::{fmt, icons, thumbs, FilePickerWindow, PickLine, PickPlace, PickRow, Theme};

pub const FLAG: &str = "--pick-file";

/// Opis okna od biblioteki (ustawienia, które program przekazał oknu Windows).
#[derive(Deserialize, Default)]
#[serde(default)]
struct Req {
    save: bool,
    title: Option<String>,
    ok: Option<String>,
    cancel: Option<String>,
    name_label: Option<String>,
    name: Option<String>,
    /// Folder narzucony przez program (SetFolder).
    folder: Option<String>,
    /// Folder, który pokazałby Windows (ostatnio używany w tym programie albo domyślny).
    suggested: Option<String>,
    /// Rozszerzenie dopisywane do nazwy bez rozszerzenia (SetDefaultExtension).
    ext: Option<String>,
    /// (nazwa, wzorce „*.txt;*.log”).
    filters: Vec<(String, String)>,
    #[serde(rename = "type")]
    ty: u32,
    multi: bool,
    folders: bool,
    must_exist: bool,
    path_must_exist: bool,
    overwrite: bool,
    no_readonly: bool,
    hidden: bool,
    places: Vec<String>,
    owner: i64,
    app: String,
}

/// Ostatni folder w każdym programie i wybrany widok – `%LOCALAPPDATA%\PowerDesk\pick-folders.json`.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Memory {
    folders: HashMap<String, String>,
    grid: bool,
}

impl Memory {
    fn file() -> PathBuf {
        powerdesk_core::log::dir().join("pick-folders.json")
    }

    fn load() -> Memory {
        std::fs::read_to_string(Self::file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(s) = serde_json::to_string(self) {
            let _ = std::fs::create_dir_all(powerdesk_core::log::dir());
            let _ = std::fs::write(Self::file(), s);
        }
    }
}

struct Place {
    name: String,
    path: String,
    icon: &'static str,
    tip: String,
    header: bool,
}

struct Picker {
    req: Req,
    ui: Weak<FilePickerWindow>,
    dir: String,
    back: Vec<String>,
    fwd: Vec<String>,
    all: Vec<Entry>,
    /// Numery z `all` po filtrze, wyszukiwaniu i sortowaniu – to widzi lista.
    shown: Vec<usize>,
    /// Zaznaczone pozycje (numery w `shown`).
    selected: HashSet<usize>,
    anchor: Option<usize>,
    focus: Option<usize>,
    filter: usize,
    /// Wzorzec wpisany w polu nazwy (np. *.txt) zamiast filtra z listy.
    pattern: Option<String>,
    search: String,
    grid: bool,
    cols: usize,
    sort: (i32, bool),
    hidden: bool,
    scale: f32,
    epoch: u64,
    loading: bool,
    error: Option<String>,
    /// Plik do zastąpienia – czeka na „Tak”.
    pending: Option<String>,
    /// Do zaznaczenia po wczytaniu (nowy folder).
    select_after: Option<String>,
    places: Vec<Place>,
    drives: Vec<Place>,
    done: bool,
}

thread_local! {
    static P: RefCell<Option<Picker>> = const { RefCell::new(None) };
    static MODELS: RefCell<Option<(Rc<Rows>, Rc<Lines>)>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Picker) -> R) -> Option<R> {
    P.with(|p| p.try_borrow_mut().ok().and_then(|mut b| b.as_mut().map(f)))
}

fn peek<R>(f: impl FnOnce(&Picker) -> R) -> Option<R> {
    P.with(|p| p.try_borrow().ok().and_then(|b| b.as_ref().map(f)))
}

fn ui() -> Option<FilePickerWindow> {
    peek(|p| p.ui.upgrade()).flatten()
}

fn same(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

/// „C:” → „C:\” (sama litera dysku to w Windows bieżący folder na tym dysku).
fn normalize(p: &str) -> String {
    let p = p.trim().trim_matches('"');
    if p.len() == 2 && p.ends_with(':') {
        format!("{p}\\")
    } else {
        p.to_string()
    }
}

// ---------- Modele listy i siatki ----------

/// Wiersze powstają dopiero, gdy lista ich potrzebuje – ikony tylko dla widocznych.
struct Rows(ModelNotify);

impl Model for Rows {
    type Data = PickRow;
    fn row_count(&self) -> usize {
        peek(|p| p.shown.len()).unwrap_or(0)
    }
    fn row_data(&self, i: usize) -> Option<PickRow> {
        peek(|p| p.row(i, false)).flatten()
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.0
    }
}

struct Lines(ModelNotify);

impl Model for Lines {
    type Data = PickLine;
    fn row_count(&self) -> usize {
        peek(|p| if p.grid { p.shown.len().div_ceil(p.cols.max(1)) } else { 0 }).unwrap_or(0)
    }
    fn row_data(&self, l: usize) -> Option<PickLine> {
        peek(|p| {
            let c = p.cols.max(1);
            let a = l * c;
            if a >= p.shown.len() {
                return None;
            }
            let b = (a + c).min(p.shown.len());
            let items: Vec<PickRow> = (a..b).filter_map(|i| p.row(i, true)).collect();
            Some(PickLine { first: a as i32, items: ModelRc::new(VecModel::from(items)) })
        })
        .flatten()
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.0
    }
}

fn reset_models() {
    MODELS.with(|m| {
        if let Some((r, l)) = &*m.borrow() {
            r.0.reset();
            l.0.reset();
        }
    });
}

/// Odświeża tylko te pozycje (zaznaczenie, miniatury).
fn changed(rows: &[usize]) {
    let cols = peek(|p| p.cols.max(1)).unwrap_or(1);
    MODELS.with(|m| {
        if let Some((r, l)) = &*m.borrow() {
            let mut lines = HashSet::new();
            for &i in rows {
                r.0.row_changed(i);
                lines.insert(i / cols);
            }
            if peek(|p| p.grid).unwrap_or(false) {
                for line in lines {
                    l.0.row_changed(line);
                }
            }
        }
    });
}

impl Picker {
    fn row(&self, i: usize, tile: bool) -> Option<PickRow> {
        let e = self.all.get(*self.shown.get(i)?)?;
        let icon = match (tile, self.grid) {
            (true, true) => thumbs::get(&e.path, (72.0 * self.scale) as u32, e.dir),
            (false, false) => thumbs::get(&e.path, (18.0 * self.scale) as u32, true),
            _ => None,
        };
        let kind = fmt::type_label(e);
        let size = if e.dir { String::new() } else { fmt::size(e.size) };
        let date = fmt::date(e.modified);
        let tip = if e.dir { format!("{}\nZmieniony: {date}", e.name) } else { format!("{}\n{kind} · {size}\nZmieniony: {date}", e.name) };
        Some(PickRow {
            name: e.name.clone().into(),
            path: e.path.clone().into(),
            dir: e.dir,
            hidden: e.hidden,
            date: date.into(),
            kind: kind.into(),
            size: size.into(),
            tip: tip.into(),
            has_icon: icon.is_some(),
            icon: icon.unwrap_or_default(),
            selected: self.selected.contains(&i),
            focused: self.focus == Some(i),
        })
    }

    fn entry(&self, i: usize) -> Option<&Entry> {
        self.all.get(*self.shown.get(i)?)
    }

    /// Zaznaczone w kolejności listy.
    fn chosen(&self) -> Vec<&Entry> {
        let mut v: Vec<usize> = self.selected.iter().copied().collect();
        v.sort_unstable();
        v.into_iter().filter_map(|i| self.entry(i)).collect()
    }

    /// Wzorce bieżącego filtra (małymi literami); pusta lista – wszystko.
    fn patterns(&self) -> Vec<String> {
        let spec = match &self.pattern {
            Some(p) => p.clone(),
            None => self.req.filters.get(self.filter).map(|f| f.1.clone()).unwrap_or_default(),
        };
        spec.split(';').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
    }

    /// Rozszerzenie bieżącego filtra, gdy jest jedno konkretne („*.txt” → „txt”).
    fn filter_ext(&self) -> Option<String> {
        let first = self.req.filters.get(self.filter)?.1.split(';').next()?.trim().to_string();
        let ext = first.strip_prefix("*.")?;
        (!ext.is_empty() && !ext.contains(['*', '?'])).then(|| ext.to_string())
    }

    fn visible(&self, e: &Entry, pats: &[String], search: &str) -> bool {
        if !self.hidden && (e.hidden || e.system) {
            return false;
        }
        if !search.is_empty() && !e.name.to_lowercase().contains(search) {
            return false;
        }
        if e.dir {
            return true;
        }
        if self.req.folders {
            return false;
        }
        let name = e.name.to_lowercase();
        pats.is_empty() || pats.iter().any(|p| p == "*.*" || p == "*" || glob(p, &name))
    }

    /// Filtr, wyszukiwanie i sortowanie od nowa; zaznaczenie zostaje (po ścieżkach).
    fn refilter(&mut self) {
        let keep: HashSet<String> = self.chosen().iter().map(|e| e.path.clone()).collect();
        let focus = self.focus.and_then(|i| self.entry(i)).map(|e| e.path.clone());
        let pats = self.patterns();
        let search = self.search.to_lowercase();
        let mut shown: Vec<usize> = (0..self.all.len()).filter(|&i| self.visible(&self.all[i], &pats, &search)).collect();
        let (col, desc) = self.sort;
        let all = &self.all;
        shown.sort_by(|&a, &b| {
            let (x, y) = (&all[a], &all[b]);
            let ord = match col {
                1 => x.modified.cmp(&y.modified),
                2 => fmt::type_label(x).cmp(&fmt::type_label(y)),
                3 => x.size.cmp(&y.size),
                _ => std::cmp::Ordering::Equal,
            }
            .then_with(|| natural(&x.name.to_lowercase(), &y.name.to_lowercase()));
            // Foldery zawsze na górze.
            y.dir.cmp(&x.dir).then(if desc { ord.reverse() } else { ord })
        });
        self.shown = shown;
        self.selected = (0..self.shown.len()).filter(|&i| self.entry(i).is_some_and(|e| keep.contains(&e.path))).collect();
        self.focus = focus.and_then(|f| (0..self.shown.len()).find(|&i| self.entry(i).is_some_and(|e| e.path == f)));
        self.anchor = self.focus;
    }

    /// Nazwa w polu po zmianie zaznaczenia (`None` – bez zmiany).
    fn name_for_selection(&self) -> Option<String> {
        let sel = self.chosen();
        if self.req.folders {
            let dirs: Vec<&&Entry> = sel.iter().filter(|e| e.dir).collect();
            return (dirs.len() == 1).then(|| dirs[0].name.clone());
        }
        let files: Vec<&&Entry> = sel.iter().filter(|e| !e.dir).collect();
        match files.len() {
            0 => None,
            1 => Some(files[0].name.clone()),
            _ => Some(files.iter().map(|e| format!("\"{}\"", e.name)).collect::<Vec<_>>().join(" ")),
        }
    }
}

/// Wzorzec z * i ? (bez rozróżniania wielkości liter – oba argumenty małymi literami).
fn glob(pat: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pat.chars().collect(), name.chars().collect());
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

// ---------- Widok ----------

fn render() {
    let Some(ui) = ui() else { return };
    peek(|p| {
        ui.set_path_text(p.dir.clone().into());
        ui.set_can_back(!p.back.is_empty());
        ui.set_can_forward(!p.fwd.is_empty());
        ui.set_can_up(Path::new(&p.dir).parent().is_some());
        ui.set_grid(p.grid);
        ui.set_sort_col(p.sort.0);
        ui.set_sort_desc(p.sort.1);
        let empty = if p.loading {
            "Wczytywanie…".to_string()
        } else if let Some(e) = &p.error {
            format!("Nie można otworzyć tego folderu: {e}")
        } else if !p.search.is_empty() {
            "Nic nie pasuje do wyszukiwania".to_string()
        } else {
            "Ten folder jest pusty".to_string()
        };
        ui.set_empty_text(empty.into());
        let places: Vec<PickPlace> = p
            .places
            .iter()
            .chain(p.drives.iter())
            .map(|pl| PickPlace { name: pl.name.clone().into(), path: pl.path.clone().into(), icon: pl.icon.into(), tip: pl.tip.clone().into(), header: pl.header, current: !pl.header && same(&pl.path, &p.dir) })
            .collect();
        ui.set_places(ModelRc::new(VecModel::from(places)));
    });
}

fn navigate(path: &str, history: bool) {
    let path = normalize(path);
    let epoch = with(|p| {
        if history && !p.dir.is_empty() && !same(&p.dir, &path) {
            p.back.push(std::mem::take(&mut p.dir));
            p.fwd.clear();
        }
        p.dir = path.clone();
        p.all.clear();
        p.shown.clear();
        p.selected.clear();
        p.anchor = None;
        p.focus = None;
        p.search.clear();
        p.pattern = None;
        p.loading = true;
        p.error = None;
        p.epoch += 1;
        p.epoch
    });
    let Some(epoch) = epoch else { return };
    if let Some(ui) = ui() {
        ui.set_search("".into());
        ui.invoke_scroll_top();
    }
    render();
    reset_models();
    std::thread::spawn(move || {
        let r = fsx::list_dir(&path);
        let _ = slint::invoke_from_event_loop(move || listed(epoch, r));
    });
}

fn listed(epoch: u64, r: Result<Vec<Entry>, String>) {
    let select = with(|p| {
        if p.epoch != epoch {
            return None;
        }
        p.loading = false;
        match r {
            Ok(v) => p.all = v,
            Err(e) => p.error = Some(e),
        }
        p.refilter();
        let target = p.select_after.take()?;
        let i = (0..p.shown.len()).find(|&i| p.entry(i).is_some_and(|e| same(&e.path, &target)))?;
        p.selected = HashSet::from([i]);
        p.focus = Some(i);
        p.anchor = Some(i);
        Some(i)
    })
    .flatten();
    render();
    reset_models();
    if let (Some(i), Some(ui)) = (select, ui()) {
        ui.invoke_reveal(i as i32);
    }
}

fn refilter() {
    with(|p| p.refilter());
    render();
    reset_models();
}

fn click(i: i32, ctrl: bool, shift: bool) {
    if i < 0 {
        return;
    }
    let i = i as usize;
    let res = with(|p| {
        if i >= p.shown.len() {
            return None;
        }
        let before: Vec<usize> = p.selected.iter().copied().chain(p.focus).collect();
        if p.req.multi && ctrl {
            if !p.selected.remove(&i) {
                p.selected.insert(i);
            }
            p.anchor = Some(i);
        } else if p.req.multi && shift {
            let a = p.anchor.unwrap_or(i);
            p.selected = (a.min(i)..=a.max(i)).collect();
        } else {
            p.selected = HashSet::from([i]);
            p.anchor = Some(i);
        }
        p.focus = Some(i);
        let mut rows = before;
        rows.extend(p.selected.iter().copied());
        rows.push(i);
        Some((rows, p.name_for_selection()))
    })
    .flatten();
    let Some((rows, name)) = res else { return };
    changed(&rows);
    if let (Some(name), Some(ui)) = (name, ui()) {
        ui.set_file_name(name.into());
    }
}

thread_local! {
    /// Wpisywany początek nazwy i chwila ostatniej litery.
    static TYPED: RefCell<(String, Option<std::time::Instant>)> = const { RefCell::new((String::new(), None)) };
}

/// Pisanie na liście przeskakuje do pasującej nazwy (kolejne litery w ciągu 0,8 s wydłużają
/// początek, ta sama litera chodzi po kolejnych) – jak w oknach Windows.
fn type_ahead(k: slint::SharedString) {
    // Klawisze specjalne Slinta leżą w obszarze prywatnym U+F700–U+F8FF.
    if k.chars().count() != 1 || k.chars().any(|c| c.is_control() || (0xF700..=0xF8FF).contains(&(c as u32))) {
        return;
    }
    let typed = TYPED.with(|t| {
        let mut t = t.borrow_mut();
        if t.1.is_none_or(|at| at.elapsed() > std::time::Duration::from_millis(800)) {
            t.0.clear();
        }
        t.0.push_str(&k.to_lowercase());
        t.1 = Some(std::time::Instant::now());
        t.0.clone()
    });
    let found = peek(|p| {
        let n = p.shown.len();
        if n == 0 {
            return None;
        }
        let f = p.focus.unwrap_or(n - 1);
        let start = if typed.chars().count() == 1 { f + 1 } else { f };
        (0..n).map(|k| (start + k) % n).find(|&i| p.entry(i).is_some_and(|e| e.name.to_lowercase().starts_with(&typed)))
    })
    .flatten();
    if let Some(i) = found {
        click(i as i32, false, false);
        if let Some(ui) = ui() {
            ui.invoke_reveal(i as i32);
        }
    }
}

fn key_nav(delta: i32, shift: bool) {
    let Some((to, n)) = peek(|p| (p.focus.map(|f| f as i32 + delta).unwrap_or(if delta > 0 { 0 } else { p.shown.len() as i32 - 1 }), p.shown.len())) else { return };
    if n == 0 {
        return;
    }
    let to = to.clamp(0, n as i32 - 1);
    click(to, false, shift);
    if let Some(ui) = ui() {
        ui.invoke_reveal(to);
    }
}

fn open_row(i: i32) {
    let i = if i < 0 { peek(|p| p.focus).flatten() } else { Some(i as usize) };
    let Some(i) = i else { return accept() };
    let Some((dir, path)) = peek(|p| p.entry(i).map(|e| (e.dir, e.path.clone()))).flatten() else { return accept() };
    if dir {
        navigate(&path, true);
    } else {
        // Plik: zaznacz tylko jego i zatwierdź (przy zapisie – pytanie o zastąpienie).
        with(|p| {
            p.selected = HashSet::from([i]);
            p.focus = Some(i);
        });
        if let Some(name) = peek(|p| p.name_for_selection()).flatten() {
            if let Some(ui) = ui() {
                ui.set_file_name(name.into());
            }
        }
        accept();
    }
}

// ---------- Zatwierdzenie ----------

enum Act {
    Nothing,
    Navigate(String),
    Pattern(String),
    Notice(String),
    Confirm(String, String),
    Done(Vec<String>),
}

/// Nazwy z pola: kilka w cudzysłowach („"a.txt" "b.txt"”) albo jedna.
fn split_names(s: &str) -> Vec<String> {
    if !s.contains('"') {
        return vec![s.trim().to_string()];
    }
    s.split('"').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, n)| n.trim().to_string()).filter(|n| !n.is_empty()).collect()
}

fn display(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

fn decide(p: &Picker, name: &str) -> Act {
    let dir = Path::new(&p.dir);
    let resolve = |n: &str| -> PathBuf {
        let n = n.trim().trim_matches('"');
        let q = Path::new(n);
        if q.is_absolute() || n.starts_with("\\\\") {
            q.to_path_buf()
        } else {
            dir.join(q)
        }
    };
    let sel = p.chosen();
    let one_dir = || (sel.len() == 1 && sel[0].dir).then(|| sel[0].path.clone());
    if p.req.folders {
        if name.is_empty() {
            let dirs: Vec<String> = sel.iter().filter(|e| e.dir).map(|e| e.path.clone()).collect();
            return match dirs.len() {
                0 => Act::Done(vec![p.dir.clone()]),
                _ if p.req.multi => Act::Done(dirs),
                _ => Act::Done(vec![dirs[0].clone()]),
            };
        }
        let names = split_names(name);
        // Pole „Folder” podpowiada nazwę bieżącego folderu (często wyświetlaną po polsku, np.
        // „Muzyka” dla Music) – dołączona do ścieżki dawała nieistniejące „Music\Muzyka”.
        // Jedna nazwa bez ukośników, której nie ma w tym folderze: zaznaczony folder albo bieżący.
        if let [one] = names.as_slice() {
            let plain = !one.contains(['\\', '/']);
            if plain && !resolve(one).is_dir() {
                return Act::Done(vec![one_dir().unwrap_or_else(|| p.dir.clone())]);
            }
        }
        let paths: Vec<PathBuf> = names.iter().map(|n| resolve(n)).collect();
        if let Some(miss) = paths.iter().find(|q| !q.is_dir()) {
            return Act::Notice(format!("Nie znaleziono folderu „{}”.", display(miss)));
        }
        if paths.len() > 1 && !p.req.multi {
            return Act::Notice("Wybierz jeden folder.".into());
        }
        return Act::Done(paths.iter().map(|q| display(q)).collect());
    }
    if !p.req.save {
        if name.is_empty() {
            let files: Vec<String> = sel.iter().filter(|e| !e.dir).map(|e| e.path.clone()).collect();
            if !files.is_empty() {
                return Act::Done(if p.req.multi { files } else { vec![files[0].clone()] });
            }
            return one_dir().map(Act::Navigate).unwrap_or(Act::Nothing);
        }
        let names = split_names(name);
        if names.len() > 1 {
            if !p.req.multi {
                return Act::Notice("Ten program przyjmuje jeden plik naraz.".into());
            }
            let paths: Vec<PathBuf> = names.iter().map(|n| resolve(n)).collect();
            if let Some(miss) = paths.iter().find(|q| !q.is_file()) {
                if p.req.must_exist || miss.is_dir() {
                    return Act::Notice(format!("Nie znaleziono pliku „{}”.", display(miss)));
                }
            }
            return Act::Done(paths.iter().map(|q| display(q)).collect());
        }
        let Some(n) = names.first().filter(|n| !n.is_empty()) else { return Act::Nothing };
        if n.contains(['*', '?']) {
            return Act::Pattern(n.clone());
        }
        let t = resolve(n);
        if t.is_dir() {
            return Act::Navigate(display(&t));
        }
        if t.exists() {
            return Act::Done(vec![display(&t)]);
        }
        if p.req.must_exist {
            return Act::Notice(format!("Nie znaleziono pliku „{n}”.\nSprawdź nazwę i spróbuj ponownie."));
        }
        if p.req.path_must_exist && !t.parent().is_some_and(|d| d.is_dir()) {
            return Act::Notice(format!("Nie znaleziono folderu dla „{n}”."));
        }
        return Act::Done(vec![display(&t)]);
    }
    // Zapis.
    let n = name.trim().trim_matches('"');
    if n.is_empty() {
        return one_dir().map(Act::Navigate).unwrap_or(Act::Nothing);
    }
    if n.contains(['*', '?']) {
        return Act::Pattern(n.to_string());
    }
    if n.contains(['<', '>', '|', '"']) {
        return Act::Notice("Nazwa pliku nie może zawierać znaków: < > | \" * ?".into());
    }
    let t = resolve(n);
    if t.is_dir() {
        return Act::Navigate(display(&t));
    }
    let t = with_ext(p, t);
    let parent = t.parent().map(Path::to_path_buf).unwrap_or_default();
    if !parent.is_dir() {
        return Act::Notice(format!("Folder „{}” nie istnieje.", display(&parent)));
    }
    let file = t.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    if let Ok(md) = std::fs::metadata(&t) {
        if p.req.no_readonly && md.permissions().readonly() {
            return Act::Notice(format!("Plik „{file}” jest tylko do odczytu. Wybierz inną nazwę."));
        }
        if p.req.overwrite {
            return Act::Confirm(format!("Plik „{file}” już istnieje.\nCzy chcesz go zastąpić?"), display(&t));
        }
    }
    Act::Done(vec![display(&t)])
}

/// Rozszerzenie dopisujemy tylko wtedy, gdy program ustawił domyślne (jak okno Windows):
/// z wybranego filtra, a gdy filtr ma kilka albo żadnego – domyślne.
fn with_ext(p: &Picker, t: PathBuf) -> PathBuf {
    let Some(def) = p.req.ext.as_deref().map(|e| e.trim_start_matches('.')).filter(|e| !e.is_empty()) else { return t };
    if t.extension().is_some() {
        return t;
    }
    let ext = p.filter_ext().unwrap_or_else(|| def.to_string());
    let mut s = t.into_os_string();
    s.push(".");
    s.push(ext);
    s.into()
}

fn accept() {
    let Some(ui) = ui() else { return };
    let name = ui.get_file_name().trim().to_string();
    let Some(act) = peek(|p| decide(p, &name)) else { return };
    match act {
        Act::Nothing => {}
        Act::Navigate(d) => {
            ui.set_file_name("".into());
            navigate(&d, true);
        }
        Act::Pattern(pat) => {
            with(|p| p.pattern = Some(pat));
            refilter();
        }
        Act::Notice(t) => ui.set_notice(t.into()),
        Act::Confirm(t, path) => {
            with(|p| p.pending = Some(path));
            ui.set_confirm(t.into());
        }
        Act::Done(paths) => finish(paths),
    }
}

fn finish(paths: Vec<String>) {
    let Some((ty, app, dir, grid)) = peek(|p| (if p.req.filters.is_empty() { 0 } else { p.filter as u32 + 1 }, p.req.app.clone(), p.dir.clone(), p.grid)) else { return };
    if !app.is_empty() {
        let mut m = Memory::load();
        m.folders.insert(app.to_lowercase(), dir);
        m.grid = grid;
        m.save();
    }
    answer(json!({ "result": "ok", "paths": paths, "type": ty }));
}

/// Jedna odpowiedź na stdout i koniec okna.
fn answer(v: Value) {
    let Some((first, owner)) = with(|p| (!std::mem::replace(&mut p.done, true), p.req.owner as isize)) else { return };
    if !first {
        return;
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
    filedlg::return_focus(owner);
    if let Some(ui) = ui() {
        let _ = ui.hide();
    }
    let _ = slint::quit_event_loop();
}

fn cancel() {
    answer(json!({ "result": "cancel" }));
}

// ---------- Miejsca ----------

fn quick_places(req: &Req, pinned: &[String]) -> Vec<Place> {
    let mut v = vec![Place { name: "Szybki dostęp".into(), path: String::new(), icon: "", tip: String::new(), header: true }];
    let known = [
        ("Pulpit", dirs::desktop_dir(), "desktop"),
        ("Dokumenty", dirs::document_dir(), "doc"),
        ("Pobrane", dirs::download_dir(), "download"),
        ("Obrazy", dirs::picture_dir(), "image"),
        ("Muzyka", dirs::audio_dir(), "music"),
        ("Wideo", dirs::video_dir(), "video"),
        ("Folder osobisty", dirs::home_dir(), "home"),
    ];
    for (name, dir, icon) in known {
        if let Some(d) = dir.filter(|d| d.is_dir()) {
            let path = display(&d);
            v.push(Place { name: name.into(), tip: path.clone(), path, icon: icons::icon(icon), header: false });
        }
    }
    // Przypięte w PowerDesk (panel boczny → Przypięte) – te same co w oknie folderów.
    let pinned: Vec<&String> = pinned.iter().filter(|p| Path::new(p.as_str()).is_dir()).collect();
    if !pinned.is_empty() {
        v.push(Place { name: "Przypięte".into(), path: String::new(), icon: "", tip: String::new(), header: true });
        for p in pinned {
            let name = Path::new(p.as_str()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
            v.push(Place { name, path: p.clone(), icon: icons::icon("pin"), tip: p.clone(), header: false });
        }
    }
    let own: Vec<&String> = req.places.iter().filter(|p| Path::new(p).is_dir()).collect();
    if !own.is_empty() {
        v.push(Place { name: "Ten program".into(), path: String::new(), icon: "", tip: String::new(), header: true });
        for p in own {
            let name = Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
            v.push(Place { name, path: p.clone(), icon: icons::icon("star"), tip: format!("{p}\nMiejsce dodane przez program"), header: false });
        }
    }
    v
}

/// Dyski w tle – sprawdzanie napędów sieciowych bywa wolne.
fn load_drives() {
    std::thread::spawn(|| {
        let list = powerdesk_core::drives::list();
        let _ = slint::invoke_from_event_loop(move || {
            let mut v = vec![Place { name: "Ten komputer".into(), path: String::new(), icon: "", tip: String::new(), header: true }];
            for d in list {
                let (fallback, icon) = match d.kind.as_str() {
                    "removable" => ("Dysk wymienny", "usb"),
                    "network" => ("Dysk sieciowy", "network"),
                    "cdrom" => ("Napęd optyczny", "disc"),
                    _ => ("Dysk lokalny", "drive"),
                };
                let label = if d.label.is_empty() { fallback.to_string() } else { d.label.clone() };
                let free = if d.total > 0 { format!("\nWolne {} z {}", fmt::size(d.free), fmt::size(d.total)) } else { String::new() };
                v.push(Place { name: format!("{label} ({})", d.letter), tip: format!("{}{free}", d.path), path: d.path.clone(), icon: icons::icon(icon), header: false });
            }
            with(|p| p.drives = v);
            render();
        });
    });
}

// ---------- Start ----------

/// `powerdesk.exe --pick-file`: okno wyboru zamiast zwykłego startu.
pub fn run() {
    let mut line = String::new();
    let req: Option<Req> = std::io::stdin().read_line(&mut line).ok().and_then(|_| serde_json::from_str(&line).ok());
    let Some(req) = req else {
        println!("{}", json!({ "result": "fallback" }));
        return;
    };
    let Ok(ui) = FilePickerWindow::new() else {
        println!("{}", json!({ "result": "fallback" }));
        return;
    };
    let prefs = Prefs::load();
    let memory = Memory::load();
    let (light, accent) = crate::settings::light_accent(&prefs);
    let t = ui.global::<Theme>();
    t.set_light(light);
    t.set_accent_override(accent);

    // Etykiety z programu bywają z „&” (skrót klawiszowy w oknie Windows).
    let label = |s: &Option<String>, def: &str| s.as_deref().map(|t| t.replace('&', "")).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| def.to_string());
    let (title, ok) = if req.folders {
        ("Wybierz folder", "Wybierz folder")
    } else if req.save {
        ("Zapisz jako", "Zapisz")
    } else {
        ("Otwórz", "Otwórz")
    };
    ui.set_window_title(label(&req.title, title).into());
    ui.set_ok_label(label(&req.ok, ok).into());
    ui.set_cancel_label(label(&req.cancel, "Anuluj").into());
    ui.set_name_label(label(&req.name_label, if req.folders { "Folder:" } else { "Nazwa pliku:" }).into());
    ui.set_save(req.save);
    ui.set_folders_mode(req.folders);
    ui.set_multi(req.multi);
    let filters: Vec<slint::SharedString> = req.filters.iter().map(|(n, s)| if n.trim().is_empty() { s.clone() } else { n.clone() }.into()).collect();
    let filter = (req.ty as usize).saturating_sub(1).min(filters.len().saturating_sub(1));
    ui.set_filters(ModelRc::new(VecModel::from(filters)));
    ui.set_filter_index(filter as i32);
    let name = req.name.clone().unwrap_or_default();
    ui.set_file_name(name.clone().into());

    // Folder startowy: narzucony przez program, ostatni z tego programu, podpowiedź Windows, Dokumenty.
    let remembered = memory.folders.get(&req.app.to_lowercase()).cloned();
    let start = [req.folder.clone(), remembered, req.suggested.clone(), dirs::document_dir().map(|d| display(&d)), dirs::home_dir().map(|d| display(&d))]
        .into_iter()
        .flatten()
        .find(|d| Path::new(d).is_dir())
        .unwrap_or_else(|| "C:\\".into());
    let owner = req.owner as isize;

    let rows = Rc::new(Rows(ModelNotify::default()));
    let lines = Rc::new(Lines(ModelNotify::default()));
    ui.set_rows(ModelRc::from(rows.clone()));
    ui.set_lines(ModelRc::from(lines.clone()));
    MODELS.with(|m| *m.borrow_mut() = Some((rows, lines)));
    let places = quick_places(&req, &prefs.favorites);
    P.with(|p| {
        *p.borrow_mut() = Some(Picker {
            hidden: prefs.show_hidden || req.hidden,
            req,
            ui: ui.as_weak(),
            dir: String::new(),
            back: Vec::new(),
            fwd: Vec::new(),
            all: Vec::new(),
            shown: Vec::new(),
            selected: HashSet::new(),
            anchor: None,
            focus: None,
            filter,
            pattern: None,
            search: String::new(),
            grid: memory.grid,
            cols: 1,
            sort: (0, false),
            scale: 1.0,
            epoch: 0,
            loading: true,
            error: None,
            pending: None,
            select_after: None,
            places,
            drives: Vec::new(),
            done: false,
        })
    });
    wire(&ui);

    if let Some((x, y, w, h, scale)) = filedlg::place_over(owner, 980.0, 620.0) {
        ui.window().set_position(slint::PhysicalPosition::new(x, y));
        ui.window().set_size(slint::PhysicalSize::new(w, h));
        with(|p| p.scale = scale);
    }
    ui.window().on_close_requested(|| {
        cancel();
        slint::CloseRequestResponse::HideWindow
    });
    if ui.show().is_err() {
        println!("{}", json!({ "result": "fallback" }));
        return;
    }
    navigate(&start, false);
    load_drives();
    // Po pierwszej klatce: okno należy do okna programu, a biblioteka dostaje „ready”.
    let w = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
        let Some(ui) = w.upgrade() else { return };
        let scale = ui.window().scale_factor();
        with(|p| p.scale = scale);
        reset_models();
        filedlg::attach(crate::window_hwnd(ui.window()), owner, !light);
        // Nazwa zaznaczona bez rozszerzenia – od razu można pisać nową.
        let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().len()).unwrap_or(name.len());
        ui.invoke_focus_name(0, stem as i32);
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "ready");
        let _ = out.flush();
    });
    let _ = slint::run_event_loop();
    // Okno zamknięte inaczej niż przyciskiem – rezygnacja.
    cancel();
}

fn wire(w: &FilePickerWindow) {
    w.on_back(|| {
        if let Some(d) = with(|p| {
            let d = p.back.pop()?;
            p.fwd.push(p.dir.clone());
            Some(d)
        })
        .flatten()
        {
            navigate(&d, false);
        }
    });
    w.on_forward(|| {
        if let Some(d) = with(|p| {
            let d = p.fwd.pop()?;
            p.back.push(p.dir.clone());
            Some(d)
        })
        .flatten()
        {
            navigate(&d, false);
        }
    });
    w.on_up(|| {
        if let Some(d) = peek(|p| Path::new(&p.dir).parent().map(display)).flatten() {
            navigate(&d, true);
        }
    });
    w.on_refresh(|| {
        if let Some(d) = peek(|p| p.dir.clone()) {
            navigate(&d, false);
        }
    });
    w.on_go(|text| {
        let t = normalize(&std::env::var("USERPROFILE").map(|h| text.replace('~', &h)).unwrap_or_else(|_| text.to_string()));
        let path = Path::new(&t);
        if path.is_dir() {
            navigate(&t, true);
        } else if let (true, Some(parent)) = (path.is_file(), path.parent()) {
            navigate(&display(parent), true);
            if let Some(ui) = ui() {
                ui.set_file_name(path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default().into());
            }
        } else if let Some(ui) = ui() {
            ui.set_notice(format!("Nie znaleziono „{t}”.").into());
            render();
        }
    });
    w.on_open_place(|p| navigate(&p, true));
    w.on_row_click(click);
    w.on_row_open(open_row);
    w.on_key_nav(key_nav);
    w.on_type_ahead(type_ahead);
    w.on_submit(accept);
    w.on_cancel(cancel);
    w.on_system_dialog(|| answer(json!({ "result": "fallback" })));
    w.on_filter_picked(|i| {
        let ext = with(|p| {
            p.filter = i.max(0) as usize;
            p.pattern = None;
            if p.req.save && p.req.ext.is_some() {
                p.filter_ext()
            } else {
                None
            }
        })
        .flatten();
        // Przy zapisie zmiana rodzaju zmienia rozszerzenie wpisanej nazwy (jak w Windows).
        if let (Some(ext), Some(ui)) = (ext, ui()) {
            let name = ui.get_file_name().to_string();
            let p = Path::new(&name);
            if !name.is_empty() && p.extension().is_some() {
                ui.set_file_name(p.with_extension(ext).to_string_lossy().to_string().into());
            }
        }
        refilter();
    });
    w.on_search_edited(|t| {
        with(|p| p.search = t.to_string());
        refilter();
    });
    w.on_toggle_grid(|| {
        with(|p| p.grid = !p.grid);
        render();
        reset_models();
    });
    w.on_sort(|c| {
        with(|p| p.sort = if p.sort.0 == c { (c, !p.sort.1) } else { (c, false) });
        refilter();
    });
    w.on_grid_cols(|c| {
        with(|p| p.cols = c.max(1) as usize);
        reset_models();
    });
    w.on_confirm_yes(|| {
        if let Some(path) = with(|p| p.pending.take()).flatten() {
            finish(vec![path]);
        }
    });
    w.on_make_folder(|name| {
        let name = name.trim().to_string();
        let Some(dir) = peek(|p| p.dir.clone()) else { return };
        if name.is_empty() || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            if let Some(ui) = ui() {
                ui.set_notice("Nazwa folderu nie może być pusta ani zawierać znaków: \\ / : * ? \" < > |".into());
            }
            return;
        }
        let target = Path::new(&dir).join(&name);
        match std::fs::create_dir(&target) {
            Ok(()) => {
                with(|p| p.select_after = Some(display(&target)));
                navigate(&dir, false);
            }
            Err(e) => {
                if let Some(ui) = ui() {
                    ui.set_notice(format!("Nie można utworzyć folderu „{name}”: {e}").into());
                }
            }
        }
    });
}

/// Miniatury dotarły – odświeżenie tylko tych pozycji (wołane z thumbs.rs).
pub fn thumbs_arrived(paths: &HashSet<String>) {
    let Some(rows) = peek(|p| (0..p.shown.len()).filter(|&i| p.entry(i).is_some_and(|e| paths.contains(&e.path))).collect::<Vec<_>>()) else { return };
    if !rows.is_empty() {
        changed(&rows);
    }
}
