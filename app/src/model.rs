//! Karta: zawartość folderu, sortowanie, filtr, wyszukiwanie i układ widoków (FileView.svelte).

use std::collections::HashMap;
use std::path::Path;

use crate::fmt::{kind_of, Kind};
use powerdesk_core::fsx::Entry;
use serde::{Deserialize, Serialize};

pub const HOME: &str = "ofp:home";

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum View {
    #[default]
    List,
    Grid,
    Gallery,
}

impl View {
    pub fn index(self) -> i32 {
        self as i32
    }
    pub fn from_index(i: i32) -> View {
        match i {
            1 => View::Grid,
            2 => View::Gallery,
            _ => View::List,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    #[default]
    Name,
    Modified,
    Created,
    Size,
    Type,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [SortKey::Name, SortKey::Modified, SortKey::Created, SortKey::Size, SortKey::Type];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Nazwa",
            SortKey::Modified => "Data modyfikacji",
            SortKey::Created => "Data utworzenia",
            SortKey::Size => "Rozmiar",
            SortKey::Type => "Typ",
        }
    }
}

/// Kolumna widoku listy (LIST_COLS w state.svelte.ts). Nazwa jest zawsze pierwsza i nie jest tu wymieniana.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Col {
    Modified,
    Created,
    Type,
    Ext,
    Size,
    Dims,
    Taken,
    Duration,
    Attrs,
}

impl Col {
    pub const ALL: [Col; 9] = [Col::Modified, Col::Created, Col::Type, Col::Ext, Col::Size, Col::Dims, Col::Taken, Col::Duration, Col::Attrs];
    pub const DEFAULT: [Col; 3] = [Col::Modified, Col::Type, Col::Size];
    pub fn label(self) -> &'static str {
        match self {
            Col::Modified => "Zmodyfikowano",
            Col::Created => "Utworzono",
            Col::Type => "Typ",
            Col::Ext => "Rozszerzenie",
            Col::Size => "Rozmiar",
            Col::Dims => "Wymiary",
            Col::Taken => "Data wykonania",
            Col::Duration => "Czas trwania",
            Col::Attrs => "Atrybuty",
        }
    }
    pub fn width(self) -> f32 {
        match self {
            Col::Modified | Col::Created | Col::Taken => 140.0,
            Col::Type => 130.0,
            Col::Ext | Col::Duration => 90.0,
            Col::Size => 86.0,
            Col::Dims => 100.0,
            Col::Attrs => 74.0,
        }
    }
    /// Klucz sortowania po kliknięciu nagłówka (kolumny z metadanych mediów jeszcze bez sortowania).
    pub fn sort(self) -> Option<SortKey> {
        match self {
            Col::Modified => Some(SortKey::Modified),
            Col::Created => Some(SortKey::Created),
            Col::Type | Col::Ext => Some(SortKey::Type),
            Col::Size => Some(SortKey::Size),
            _ => None,
        }
    }
    pub fn right(self) -> bool {
        matches!(self, Col::Size | Col::Dims | Col::Duration)
    }
    /// Kolumny, które potrzebują metadanych zdjęć i filmów.
    pub fn media(self) -> bool {
        matches!(self, Col::Dims | Col::Taken | Col::Duration)
    }
}

/// Filtr kategorii plików (przyciski w pasku poleceń).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KindFilter {
    #[default]
    All,
    Media,
    Image,
    Video,
    Doc,
    Audio,
    Archive,
}

impl KindFilter {
    pub const ALL: [KindFilter; 7] = [KindFilter::All, KindFilter::Media, KindFilter::Image, KindFilter::Video, KindFilter::Doc, KindFilter::Audio, KindFilter::Archive];
    /// Czy kategoria pliku należy do grupy (KIND_GROUPS w state.svelte.ts).
    pub fn has(self, k: Kind) -> bool {
        match self {
            KindFilter::All => true,
            KindFilter::Media => matches!(k, Kind::Image | Kind::Raw | Kind::Video),
            KindFilter::Image => matches!(k, Kind::Image | Kind::Raw),
            KindFilter::Video => k == Kind::Video,
            KindFilter::Doc => matches!(k, Kind::Pdf | Kind::Doc | Kind::Sheet | Kind::Slides | Kind::Text),
            KindFilter::Audio => k == Kind::Audio,
            KindFilter::Archive => k == Kind::Archive,
        }
    }
}

/// Wyniki wyszukiwania w podfolderach.
pub struct Search {
    pub query: String,
    pub id: u64,
    pub running: bool,
    pub scanned: u64,
    pub results: Vec<Entry>,
}

/// Prostokąt elementu w układzie widoku (piksele logiczne).
#[derive(Clone, Copy, Default)]
pub struct BoxR {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub struct Tab {
    pub path: String,
    pub history: Vec<String>,
    pub h_index: usize,
    pub entries: Vec<Entry>,
    pub error: String,
    pub view: View,
    pub sort: SortKey,
    pub desc: bool,
    pub filter: String,
    pub kind_filter: KindFilter,
    pub search: Option<Search>,
    /// Elementy po filtrze i sortowaniu – to, co pokazuje widok.
    pub items: Vec<Entry>,
    pub selected: std::collections::HashSet<String>,
    pub anchor: Option<usize>,
    pub focus: Option<usize>,
    pub scroll: f32,
    pub boxes: Vec<BoxR>,
    pub total_h: f32,
    /// Rośnie przy każdym wczytaniu – spóźnione wyniki starego folderu są odrzucane.
    pub generation: u64,
}

impl Tab {
    pub fn new(path: &str) -> Tab {
        Tab {
            path: path.into(),
            history: vec![path.into()],
            h_index: 0,
            entries: Vec::new(),
            error: String::new(),
            view: View::List,
            sort: SortKey::Name,
            desc: false,
            filter: String::new(),
            kind_filter: KindFilter::All,
            search: None,
            items: Vec::new(),
            selected: Default::default(),
            anchor: None,
            focus: None,
            scroll: 0.0,
            boxes: Vec::new(),
            total_h: 0.0,
            generation: 0,
        }
    }

    pub fn special(&self) -> bool {
        self.path.starts_with("ofp:")
    }

    pub fn title(&self) -> String {
        if let Some(s) = &self.search {
            return format!("„{}”", s.query);
        }
        if self.path == HOME {
            return "Ten komputer".into();
        }
        Path::new(&self.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.path.trim_end_matches('\\').to_string())
    }

    /// Filtr i sortowanie jak w `Tab.grouped` (state.svelte.ts).
    /// `size_of` zwraca rozmiar elementu – dla folderów policzony w tle albo -1, dopóki go nie ma.
    pub fn rebuild(&mut self, show_hidden: bool, folders_first: bool, size_of: impl Fn(&Entry) -> i64) {
        let source = match &self.search {
            Some(s) => &s.results,
            None => &self.entries,
        };
        let f = self.filter.trim().to_lowercase();
        let wild = f.contains('*') || f.contains('?');
        let mut v: Vec<Entry> = source
            .iter()
            .filter(|e| show_hidden || !(e.hidden || e.system))
            .filter(|e| self.kind_filter == KindFilter::All || (!e.dir && self.kind_filter.has(kind_of(e))))
            .filter(|e| f.is_empty() || if wild { wildcard(&f, &e.name.to_lowercase()) } else { e.name.to_lowercase().contains(&f) })
            .cloned()
            .collect();
        let key = self.sort;
        let desc = self.desc;
        v.sort_by(|a, b| {
            if folders_first && a.dir != b.dir {
                return b.dir.cmp(&a.dir);
            }
            let r = match key {
                SortKey::Modified => a.modified.cmp(&b.modified),
                SortKey::Created => a.created.cmp(&b.created),
                SortKey::Size => size_of(a).cmp(&size_of(b)),
                SortKey::Type => a.ext.to_lowercase().cmp(&b.ext.to_lowercase()),
                SortKey::Name => std::cmp::Ordering::Equal,
            };
            let r = r.then_with(|| natural(&a.name, &b.name));
            if desc {
                r.reverse()
            } else {
                r
            }
        });
        self.items = v;
        let alive: std::collections::HashSet<&str> = self.items.iter().map(|e| e.path.as_str()).collect();
        self.selected.retain(|p| alive.contains(p.as_str()));
    }

    /// Kategorie, które mają sens w tym folderze – „Wszystko” zawsze, „Media” tylko gdy są i zdjęcia, i filmy.
    pub fn shown_filters(&self, show_hidden: bool) -> Vec<KindFilter> {
        let source = match &self.search {
            Some(s) => &s.results,
            None => &self.entries,
        };
        let mut counts = [0usize; 7];
        for e in source.iter().filter(|e| !e.dir && (show_hidden || !(e.hidden || e.system))) {
            let k = kind_of(e);
            for (i, f) in KindFilter::ALL.iter().enumerate().skip(1) {
                if f.has(k) {
                    counts[i] += 1;
                }
            }
        }
        KindFilter::ALL
            .iter()
            .zip(counts)
            .filter(|(f, n)| **f == KindFilter::All || (*n > 0 && !(**f == KindFilter::Media && (counts[2] == 0 || counts[3] == 0))))
            .map(|(f, _)| *f)
            .collect()
    }

    /// Elementy, których prostokąt przecina ramkę zaznaczenia (na liście liczy się tylko pierwsze 420 px).
    pub fn in_rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<usize> {
        let first = self.boxes.partition_point(|b| b.y + b.h < y0);
        let mut out = Vec::new();
        for (i, b) in self.boxes.iter().enumerate().skip(first) {
            if b.y > y1 {
                break;
            }
            let bw = if self.view == View::List { b.w.min(420.0) } else { b.w };
            if b.y + b.h >= y0 && b.x < x1 && b.x + bw > x0 {
                out.push(i);
            }
        }
        out
    }

    /// Układ jak w FileView.svelte: lista 32 px, siatka kafelków rozciągniętych do szerokości,
    /// galeria – wiersze o równej wysokości wypełnione co do piksela.
    pub fn layout(&mut self, width: f32, grid_size: f32, gallery_row: f32, ratio: impl Fn(&Entry) -> f32) {
        const LIST_H: f32 = 32.0;
        const GAP: f32 = 6.0;
        let pad = if self.view == View::List { 6.0 } else { 12.0 };
        let top0 = if self.view == View::List { 4.0 } else { pad };
        let avail = (width - pad * 2.0).max(100.0);
        let n = self.items.len();
        let mut boxes = vec![BoxR::default(); n];
        let mut y = top0;
        match self.view {
            View::List => {
                for b in boxes.iter_mut() {
                    *b = BoxR { x: pad, y, w: avail, h: LIST_H };
                    y += LIST_H;
                }
            }
            View::Grid => {
                let cols = ((avail + GAP) / (grid_size + GAP)).floor().max(1.0) as usize;
                let tile_w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
                let tile_h = tile_w + 40.0;
                for (i, b) in boxes.iter_mut().enumerate() {
                    let c = i % cols;
                    if i > 0 && c == 0 {
                        y += tile_h + GAP;
                    }
                    *b = BoxR { x: pad + c as f32 * (tile_w + GAP), y, w: tile_w, h: tile_h };
                }
                if n > 0 {
                    y += tile_h + GAP;
                }
            }
            View::Gallery => {
                let h0 = gallery_row;
                let mut start = 0;
                let mut sum = 0.0f32;
                let flush = |start: usize, end: usize, sum: f32, last: bool, y: &mut f32, boxes: &mut Vec<BoxR>| {
                    let mut h = (avail - GAP * (end - start - 1) as f32) / sum;
                    if last && h > h0 {
                        h = h0;
                    }
                    let h = h.round();
                    let mut x = pad;
                    for i in start..end {
                        // Ostatni w pełnym wierszu domyka krawędź co do piksela.
                        let w = if i == end - 1 && !last { pad + avail - x } else { (ratio(&self.items[i]) * h).round() };
                        boxes[i] = BoxR { x, y: *y, w, h };
                        x += w + GAP;
                    }
                    *y += h + GAP;
                };
                for i in 0..n {
                    sum += ratio(&self.items[i]);
                    if sum * h0 + GAP * (i - start) as f32 >= avail {
                        flush(start, i + 1, sum, false, &mut y, &mut boxes);
                        start = i + 1;
                        sum = 0.0;
                    }
                }
                if start < n {
                    flush(start, n, sum, true, &mut y, &mut boxes);
                }
            }
        }
        self.boxes = boxes;
        self.total_h = y + pad;
    }

    /// Indeksy elementów widocznych w oknie [top, top+height] z zapasem.
    pub fn visible(&self, top: f32, height: f32) -> std::ops::Range<usize> {
        let lo = top - height * 0.5;
        let hi = top + height * 1.5;
        let first = self.boxes.partition_point(|b| b.y + b.h < lo);
        let last = self.boxes.partition_point(|b| b.y <= hi);
        first..last.max(first)
    }
}

/// Porównanie „naturalne” (plik2 < plik10), bez rozróżniania wielkości liter – jak Intl.Collator({ numeric: true }).
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n1 = String::new();
                while let Some(c) = x.peek().copied().filter(|c| c.is_ascii_digit()) {
                    n1.push(c);
                    x.next();
                }
                let mut n2 = String::new();
                while let Some(c) = y.peek().copied().filter(|c| c.is_ascii_digit()) {
                    n2.push(c);
                    y.next();
                }
                let (t1, t2) = (n1.trim_start_matches('0'), n2.trim_start_matches('0'));
                let r = t1.len().cmp(&t2.len()).then_with(|| t1.cmp(t2));
                if r != std::cmp::Ordering::Equal {
                    return r;
                }
            }
            (Some(c), Some(d)) => {
                let r = c.to_lowercase().cmp(d.to_lowercase());
                if r != std::cmp::Ordering::Equal {
                    return r;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Dopasowanie `*` i `?` (oba ciągi małymi literami).
pub fn wildcard(p: &str, s: &str) -> bool {
    let (p, s): (Vec<char>, Vec<char>) = (p.chars().collect(), s.chars().collect());
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < s.len() {
        if i < p.len() && (p[i] == '?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(st) = star {
            i = st + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

/// Widok i sortowanie zapamiętane per folder.
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct FolderPrefs {
    pub view: View,
    pub sort: SortKey,
    pub desc: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub folders: HashMap<String, FolderPrefs>,
    pub grid_size: f32,
    pub gallery_row: f32,
    pub show_hidden: bool,
    pub folders_first: bool,
    pub auto_gallery: bool,
    pub list_cols: Vec<Col>,
    pub list_col_w: HashMap<Col, f32>,
    /// Rozmiary folderów liczone w tle w kolumnie Rozmiar (bez dysków sieciowych).
    pub folder_sizes: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { folders: HashMap::new(), grid_size: 132.0, gallery_row: 210.0, show_hidden: false, folders_first: true, auto_gallery: true, list_cols: Col::DEFAULT.to_vec(), list_col_w: HashMap::new(), folder_sizes: true }
    }
}

impl Prefs {
    pub fn col_w(&self, c: Col) -> f32 {
        self.list_col_w.get(&c).copied().unwrap_or(c.width())
    }
    /// Nowe miejsce kolumny po przeciągnięciu nagłówka o `dx`: przed pierwszą, której środek jest dalej.
    pub fn move_col(&mut self, c: Col, dx: f32) {
        const GAP: f32 = 12.0;
        let mut x = 0.0;
        let mut center = 0.0;
        let mut centers = Vec::new();
        for &k in &self.list_cols {
            let w = self.col_w(k);
            if k == c {
                center = x + w / 2.0 + dx;
            } else {
                centers.push((k, x + w / 2.0));
            }
            x += w + GAP;
        }
        let idx = centers.iter().position(|(_, m)| center < *m).unwrap_or(centers.len());
        let mut order: Vec<Col> = centers.into_iter().map(|(k, _)| k).collect();
        order.insert(idx, c);
        self.list_cols = order;
    }
    fn file() -> Option<std::path::PathBuf> {
        let d = dirs::data_dir()?.join("PowerDesk");
        let _ = std::fs::create_dir_all(&d);
        Some(d.join("prefs.json"))
    }
    pub fn load() -> Prefs {
        Self::file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if let (Some(p), Ok(s)) = (Self::file(), serde_json::to_string(self)) {
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        assert_eq!(natural("plik2", "plik10"), std::cmp::Ordering::Less);
        assert_eq!(natural("A", "a"), std::cmp::Ordering::Equal);
    }

    fn entry(name: &str, dir: bool) -> Entry {
        let ext = if dir { "" } else { name.rsplit_once('.').map(|x| x.1).unwrap_or("") };
        Entry {
            name: name.into(),
            path: format!("C:/t/{name}"),
            dir,
            size: 0,
            modified: 0,
            created: 0,
            ext: ext.into(),
            hidden: false,
            system: false,
            readonly: false,
            link: false,
            cloud: false,
        }
    }

    #[test]
    fn kind_filters() {
        let mut t = Tab::new("C:/t");
        t.entries = vec![entry("a.jpg", false), entry("b.mp4", false), entry("c.pdf", false), entry("Folder", true)];
        assert_eq!(t.shown_filters(false), vec![KindFilter::All, KindFilter::Media, KindFilter::Image, KindFilter::Video, KindFilter::Doc]);
        t.kind_filter = KindFilter::Media;
        t.rebuild(false, true, |e| e.size as i64);
        assert_eq!(t.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a.jpg", "b.mp4"]);
    }

    #[test]
    fn marquee_hits() {
        let mut t = Tab::new("C:/t");
        t.entries = (0..10).map(|i| entry(&format!("p{i}.txt"), false)).collect();
        t.rebuild(false, true, |e| e.size as i64);
        t.layout(800.0, 132.0, 210.0, |_| 1.0);
        // Wiersze po 32 px od y=4: ramka 40..100 łapie wiersze 1–3; na prawo od 420 px nic.
        assert_eq!(t.in_rect(10.0, 40.0, 50.0, 100.0), vec![1, 2, 3]);
        assert!(t.in_rect(500.0, 40.0, 600.0, 100.0).is_empty());
    }

    #[test]
    fn column_moves() {
        let mut p = Prefs::default();
        // Zmodyfikowano (140) przeciągnięte za Typ (130): środek 70+200 > środek Typu 152+65.
        p.move_col(Col::Modified, 200.0);
        assert_eq!(p.list_cols, [Col::Type, Col::Modified, Col::Size]);
        p.move_col(Col::Size, -1000.0);
        assert_eq!(p.list_cols, [Col::Size, Col::Type, Col::Modified]);
    }

    #[test]
    fn wildcards() {
        assert!(wildcard("*.jpg", "zdjęcie.jpg"));
        assert!(wildcard("img_??.png", "img_01.png"));
        assert!(!wildcard("*.jpg", "plik.png"));
    }
}
