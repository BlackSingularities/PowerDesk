//! Karta: zawartość folderu, sortowanie, filtr, wyszukiwanie i układ widoków (FileView.svelte).

use std::collections::HashMap;
use std::path::Path;

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
    pub fn rebuild(&mut self, show_hidden: bool, folders_first: bool) {
        let source = match &self.search {
            Some(s) => &s.results,
            None => &self.entries,
        };
        let f = self.filter.trim().to_lowercase();
        let wild = f.contains('*') || f.contains('?');
        let mut v: Vec<Entry> = source
            .iter()
            .filter(|e| show_hidden || !(e.hidden || e.system))
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
                SortKey::Size => a.size.cmp(&b.size),
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
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { folders: HashMap::new(), grid_size: 132.0, gallery_row: 210.0, show_hidden: false, folders_first: true, auto_gallery: true }
    }
}

impl Prefs {
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

    #[test]
    fn wildcards() {
        assert!(wildcard("*.jpg", "zdjęcie.jpg"));
        assert!(wildcard("img_??.png", "img_01.png"));
        assert!(!wildcard("*.jpg", "plik.png"));
    }
}
