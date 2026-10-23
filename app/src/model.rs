//! Karta: zawartość folderu, sortowanie, filtr, wyszukiwanie i układ widoków (FileView.svelte).

use std::collections::HashMap;
use std::path::Path;

use crate::fmt::{kind_of, Kind};
use powerdesk_core::fsx::Entry;
use powerdesk_core::shell::MediaInfo;
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
    Taken,
    Duration,
    Dims,
}

impl SortKey {
    pub const ALL: [SortKey; 8] = [SortKey::Name, SortKey::Modified, SortKey::Created, SortKey::Size, SortKey::Type, SortKey::Taken, SortKey::Duration, SortKey::Dims];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Nazwa",
            SortKey::Modified => "Data modyfikacji",
            SortKey::Created => "Data utworzenia",
            SortKey::Size => "Rozmiar",
            SortKey::Type => "Typ",
            SortKey::Taken => "Data wykonania",
            SortKey::Duration => "Czas trwania",
            SortKey::Dims => "Wymiary",
        }
    }
    /// Klucze, które potrzebują metadanych zdjęć i filmów.
    pub fn media(self) -> bool {
        matches!(self, SortKey::Taken | SortKey::Duration | SortKey::Dims)
    }
}

/// Grupowanie listy (GroupKey w state.svelte.ts).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum GroupKey {
    #[default]
    None,
    Name,
    Modified,
    Created,
    Taken,
    Type,
    Kind,
    Size,
}

impl GroupKey {
    pub const ALL: [GroupKey; 8] = [GroupKey::None, GroupKey::Name, GroupKey::Modified, GroupKey::Created, GroupKey::Taken, GroupKey::Type, GroupKey::Kind, GroupKey::Size];
    pub fn label(self) -> &'static str {
        match self {
            GroupKey::None => "Brak",
            GroupKey::Name => "Nazwa",
            GroupKey::Modified => "Data modyfikacji",
            GroupKey::Created => "Data utworzenia",
            GroupKey::Taken => "Data wykonania",
            GroupKey::Type => "Typ",
            GroupKey::Kind => "Rodzaj",
            GroupKey::Size => "Rozmiar",
        }
    }
}

/// Grupa w bieżącej liście: elementy `start..end`.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub label: String,
    pub start: usize,
    pub end: usize,
}

/// Ranga grupy – ustala kolejność grup (rosnąco = „naturalna”).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    N(i64),
    S(String),
}

const DAY: i64 = 86_400_000;
const HEAD_H: f32 = 38.0;
const MONTHS: [&str; 12] = ["Styczeń", "Luty", "Marzec", "Kwiecień", "Maj", "Czerwiec", "Lipiec", "Sierpień", "Wrzesień", "Październik", "Listopad", "Grudzień"];

/// Przedziały dat jak w Eksploratorze (dateGroup); ranga – im mniejsza, tym nowsze.
fn date_group(t: i64, now: chrono::DateTime<chrono::Local>) -> (Rank, String) {
    use chrono::{Datelike, TimeZone};
    if t <= 0 {
        return (Rank::N(99999), "Brak daty".into());
    }
    let ms = |y: i32, m: u32| {
        let (y, m) = if m == 0 { (y - 1, 12) } else { (y, m) };
        chrono::Local.with_ymd_and_hms(y, m, 1, 0, 0, 0).earliest().map(|d| d.timestamp_millis()).unwrap_or(0)
    };
    let today = chrono::Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .earliest()
        .map(|d| d.timestamp_millis())
        .unwrap_or(0);
    let week_start = today - now.weekday().num_days_from_monday() as i64 * DAY;
    let month_start = ms(now.year(), now.month());
    let last_month = ms(now.year(), now.month() - 1);
    let year_start = ms(now.year(), 1);
    let r = |n: i64, l: &str| (Rank::N(n), l.to_string());
    if t >= today + DAY {
        return r(0, "W przyszłości");
    }
    if t >= today {
        return r(1, "Dziś");
    }
    if t >= today - DAY {
        return r(2, "Wczoraj");
    }
    if t >= week_start {
        return r(3, "Wcześniej w tym tygodniu");
    }
    if t >= week_start - 7 * DAY {
        return r(4, "W zeszłym tygodniu");
    }
    if t >= month_start {
        return r(5, "Wcześniej w tym miesiącu");
    }
    if t >= last_month {
        return r(6, "W zeszłym miesiącu");
    }
    let d = chrono::Local.timestamp_millis_opt(t).earliest().unwrap_or(now);
    if t >= year_start {
        return (Rank::N(10 + (11 - d.month0() as i64)), format!("{} {}", MONTHS[d.month0() as usize], d.year()));
    }
    (Rank::N(100 + (now.year() - d.year()) as i64), d.year().to_string())
}

const SIZE_GROUPS: [(u64, &str); 7] = [
    (0, "Puste"),
    (16 << 10, "Malutkie (do 16 KB)"),
    (1 << 20, "Małe (16 KB – 1 MB)"),
    (128 << 20, "Średnie (1 – 128 MB)"),
    (1 << 30, "Duże (128 MB – 1 GB)"),
    (4 << 30, "Ogromne (1 – 4 GB)"),
    (u64::MAX, "Gigantyczne (ponad 4 GB)"),
];

fn group_of(e: &Entry, g: GroupKey, taken: i64, now: chrono::DateTime<chrono::Local>) -> (Rank, String) {
    let r = |n: i64, l: &str| (Rank::N(n), l.to_string());
    match g {
        GroupKey::None => r(0, ""),
        GroupKey::Name => match e.name.chars().next().map(|c| c.to_uppercase().next().unwrap_or(c)) {
            Some(c @ 'A'..='H') if c.is_ascii() => r(1, "A – H"),
            Some(c @ 'I'..='P') if c.is_ascii() => r(2, "I – P"),
            Some(c @ 'Q'..='Z') if c.is_ascii() => r(3, "Q – Z"),
            Some('0'..='9') => r(0, "0 – 9"),
            _ => r(4, "Inne"),
        },
        GroupKey::Modified => date_group(e.modified, now),
        GroupKey::Created => date_group(e.created, now),
        GroupKey::Taken if e.dir => r(-1, "Foldery"),
        GroupKey::Taken => date_group(taken, now),
        GroupKey::Type => {
            let l = crate::fmt::type_label(e);
            (Rank::S(if e.dir { String::new() } else { l.to_lowercase() }), l)
        }
        GroupKey::Kind => {
            const KINDS: [(&[Kind], &str); 8] = [
                (&[Kind::Folder], "Foldery"),
                (&[Kind::Image, Kind::Raw], "Zdjęcia"),
                (&[Kind::Video], "Wideo"),
                (&[Kind::Audio], "Muzyka"),
                (&[Kind::Pdf, Kind::Doc, Kind::Sheet, Kind::Slides, Kind::Text], "Dokumenty"),
                (&[Kind::Archive], "Archiwa"),
                (&[Kind::Code], "Kod"),
                (&[Kind::App], "Programy"),
            ];
            let k = kind_of(e);
            match KINDS.iter().position(|(ks, _)| ks.contains(&k)) {
                Some(i) => r(i as i64, KINDS[i].1),
                None => r(50, "Inne"),
            }
        }
        GroupKey::Size if e.dir => r(-1, "Foldery"),
        GroupKey::Size => {
            let i = SIZE_GROUPS.iter().position(|(max, _)| if *max == 0 { e.size == 0 } else { e.size < *max }).unwrap_or(6);
            r(i as i64, SIZE_GROUPS[i].1)
        }
    }
}

/// Nagłówek grupy w układzie.
#[derive(Clone, Copy, Default)]
pub struct Head {
    pub group: usize,
    pub y: f32,
    pub shut: bool,
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
    /// Klucz sortowania po kliknięciu nagłówka.
    pub fn sort(self) -> Option<SortKey> {
        match self {
            Col::Modified => Some(SortKey::Modified),
            Col::Created => Some(SortKey::Created),
            Col::Type | Col::Ext => Some(SortKey::Type),
            Col::Size => Some(SortKey::Size),
            Col::Taken => Some(SortKey::Taken),
            Col::Duration => Some(SortKey::Duration),
            Col::Dims => Some(SortKey::Dims),
            Col::Attrs => None,
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
    pub fn label(self) -> &'static str {
        match self {
            KindFilter::All => "Wszystko",
            KindFilter::Media => "Media (zdjęcia i wideo)",
            KindFilter::Image => "Zdjęcia",
            KindFilter::Video => "Wideo",
            KindFilter::Doc => "Dokumenty",
            KindFilter::Audio => "Audio",
            KindFilter::Archive => "Archiwa",
        }
    }
    /// Krótka nazwa do panelu filtrów i paska adresu.
    pub fn short(self) -> &'static str {
        match self {
            KindFilter::Media => "Media",
            f => f.label(),
        }
    }
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
    /// Wybrane filtry dopasowane do zawartości (id z facets.rs) i ręczne otwarcie ich paska.
    pub facets: std::collections::BTreeSet<String>,
    pub facets_open: Option<bool>,
    pub group: GroupKey,
    /// Grupy bieżącej listy (None – bez grupowania) i zwinięte grupy (etykiety).
    pub groups: Option<Vec<Group>>,
    pub collapsed: std::collections::HashSet<String>,
    pub heads: Vec<Head>,
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
    /// Zaznaczenie zapamiętane dla każdego folderu (ścieżki i element z fokusem) – wraca po powrocie.
    pub sel_memory: HashMap<String, (Vec<String>, Option<String>)>,
    /// Element, któremu właśnie zmieniamy nazwę.
    /// Karta przypięta: wąska, bez krzyżyka, zawsze po lewej, wraca po uruchomieniu.
    pub pinned: bool,
    /// Zawartość już wczytana – przypięte karty z poprzedniego uruchomienia czekają na pierwsze wejście.
    pub loaded: bool,
    /// Przeglądarka zdjęć tej karty (karta `ofp:viewer`).
    pub viewer: Option<crate::viewer::Viewer>,
    pub renaming: Option<String>,
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
            facets: Default::default(),
            facets_open: None,
            group: GroupKey::None,
            groups: None,
            collapsed: Default::default(),
            heads: Vec::new(),
            search: None,
            items: Vec::new(),
            selected: Default::default(),
            anchor: None,
            focus: None,
            scroll: 0.0,
            boxes: Vec::new(),
            total_h: 0.0,
            generation: 0,
            sel_memory: HashMap::new(),
            renaming: None,
            pinned: false,
            loaded: false,
            viewer: None,
        }
    }

    pub fn special(&self) -> bool {
        // Biblioteka to zwykła lista plików, tylko z kilku folderów naraz.
        self.path.starts_with("ofp:") && !self.path.starts_with(crate::LIB)
    }

    pub fn title(&self) -> String {
        if let Some(v) = &self.viewer {
            return v.list[v.index].name.clone();
        }
        if let Some(s) = &self.search {
            return format!("„{}”", s.query);
        }
        if self.path == HOME {
            return "Ten komputer".into();
        }
        if self.path == "ofp:settings" {
            return "Ustawienia".into();
        }
        if self.path == "ofp:sync" {
            return "Synchronizacja".into();
        }
        if let Some(n) = crate::library_name(&self.path) {
            return n;
        }
        if let Some(t) = crate::spaceview::title(&self.path) {
            return t;
        }
        Path::new(&self.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.path.trim_end_matches('\\').to_string())
    }

    /// Filtr i sortowanie jak w `Tab.grouped` (state.svelte.ts).
    /// `size_of` zwraca rozmiar elementu – dla folderów policzony w tle albo -1, dopóki go nie ma;
    /// `media` – metadane zdjęć i filmów (do sortowania po dacie wykonania, czasie i wymiarach).
    pub fn rebuild(&mut self, show_hidden: bool, folders_first: bool, size_of: impl Fn(&Entry) -> i64, media: impl Fn(&Entry) -> Option<MediaInfo>) {
        let source = match &self.search {
            Some(s) => &s.results,
            None => &self.entries,
        };
        let pq = crate::query::parse(&self.filter);
        let f = pq.text.trim().to_lowercase();
        let wild = f.contains('*') || f.contains('?');
        // Filtry z zapytania wyszukiwania (typ:, rozmiar:…) sprawdzamy przy odbiorze wyników.
        let sq = self.search.as_ref().and_then(|s| crate::query::parse(&s.query).test);
        let mut v: Vec<Entry> = source
            .iter()
            .filter(|e| show_hidden || !(e.hidden || e.system))
            .filter(|e| self.kind_filter == KindFilter::All || (!e.dir && self.kind_filter.has(kind_of(e))))
            .filter(|e| f.is_empty() || if wild { wildcard(&f, &e.name.to_lowercase()) } else { e.name.to_lowercase().contains(&f) })
            .filter(|e| pq.test.as_ref().map_or(true, |t| t(e)) && sq.as_ref().map_or(true, |t| t(e)))
            .filter(|e| self.facets.is_empty() || crate::facets::matches(self.facets.iter(), e, media(e)))
            .cloned()
            .collect();
        let key = self.sort;
        let desc = self.desc;
        let base = |a: &Entry, b: &Entry| {
            if folders_first && a.dir != b.dir {
                return b.dir.cmp(&a.dir);
            }
            let r = match key {
                SortKey::Modified => a.modified.cmp(&b.modified),
                SortKey::Created => a.created.cmp(&b.created),
                SortKey::Size => size_of(a).cmp(&size_of(b)),
                SortKey::Type => a.ext.to_lowercase().cmp(&b.ext.to_lowercase()),
                // Jak takenOf: bez daty wykonania liczy się data modyfikacji.
                SortKey::Taken => {
                    let t = |e: &Entry| media(e).map(|m| m.taken).filter(|t| *t > 0).unwrap_or(e.modified);
                    t(a).cmp(&t(b))
                }
                SortKey::Duration => {
                    let d = |e: &Entry| media(e).map(|m| m.dur).unwrap_or(0);
                    d(a).cmp(&d(b))
                }
                SortKey::Dims => {
                    let px = |e: &Entry| media(e).map(|m| m.w as u64 * m.h as u64).unwrap_or(0);
                    px(a).cmp(&px(b))
                }
                SortKey::Name => std::cmp::Ordering::Equal,
            };
            let r = r.then_with(|| natural(&a.name, &b.name));
            if desc {
                r.reverse()
            } else {
                r
            }
        };
        let g = self.group;
        if g == GroupKey::None {
            v.sort_by(base);
            self.groups = None;
        } else {
            // Grupy w kolejności rang, w środku zwykłe sortowanie. Sortowanie po tym samym kluczu odwraca grupy.
            let now = chrono::Local::now();
            let mut keyed: Vec<((Rank, String), Entry)> = v
                .into_iter()
                .map(|e| {
                    let taken = media(&e).map(|m| m.taken).filter(|t| *t > 0).unwrap_or(e.modified);
                    (group_of(&e, g, taken, now), e)
                })
                .collect();
            let same_key = matches!(
                (g, key),
                (GroupKey::Name, SortKey::Name) | (GroupKey::Modified, SortKey::Modified) | (GroupKey::Created, SortKey::Created) | (GroupKey::Taken, SortKey::Taken) | (GroupKey::Type, SortKey::Type) | (GroupKey::Size, SortKey::Size)
            );
            let gdesc = (g == GroupKey::Size) != (same_key && desc);
            keyed.sort_by(|x, y| {
                let r = x.0 .0.cmp(&y.0 .0);
                let r = if gdesc { r.reverse() } else { r };
                r.then_with(|| base(&x.1, &y.1))
            });
            let mut groups: Vec<Group> = Vec::new();
            for (i, ((_, label), _)) in keyed.iter().enumerate() {
                match groups.last_mut() {
                    Some(last) if last.label == *label => last.end = i + 1,
                    _ => groups.push(Group { label: label.clone(), start: i, end: i + 1 }),
                }
            }
            v = keyed.into_iter().map(|(_, e)| e).collect();
            self.groups = Some(groups);
        }
        self.items = v;
        let alive: std::collections::HashSet<&str> = self.items.iter().map(|e| e.path.as_str()).collect();
        self.selected.retain(|p| alive.contains(p.as_str()));
    }

    /// Fasety dla plików bieżącej kategorii (bez ukrytych).
    pub fn facet_list(&self, show_hidden: bool, media: impl Fn(&Entry) -> Option<MediaInfo>) -> Vec<crate::facets::Facet> {
        let source = match &self.search {
            Some(s) => &s.results,
            None => &self.entries,
        };
        let base: Vec<Entry> = source
            .iter()
            .filter(|e| !e.dir && (show_hidden || !(e.hidden || e.system)) && (self.kind_filter == KindFilter::All || self.kind_filter.has(kind_of(e))))
            .cloned()
            .collect();
        crate::facets::facets_for(&base, media, &self.facets)
    }

    /// Panel faset otwiera się tylko na żądanie (wysuwa się nad listą, nie rozpycha interfejsu).
    pub fn facets_shown(&self, any: bool) -> bool {
        any && self.facets_open.unwrap_or(false)
    }

    /// Zapamiętuje zaznaczenie bieżącego folderu przed wyjściem z niego.
    pub fn remember_selection(&mut self) {
        if self.search.is_some() || self.special() {
            return;
        }
        let focus = self.focus.and_then(|i| self.items.get(i)).filter(|e| self.selected.contains(&e.path)).map(|e| e.path.clone());
        self.sel_memory.insert(self.path.to_lowercase(), (self.selected.iter().cloned().collect(), focus));
    }

    /// Przywraca zapamiętane zaznaczenie (po wczytaniu folderu); zwraca indeks elementu z fokusem.
    pub fn restore_selection(&mut self) -> Option<usize> {
        let (paths, focus) = self.sel_memory.get(&self.path.to_lowercase())?.clone();
        let alive: std::collections::HashSet<&str> = self.items.iter().map(|e| e.path.as_str()).collect();
        self.selected = paths.into_iter().filter(|p| alive.contains(p.as_str())).collect();
        let i = focus.and_then(|f| self.items.iter().position(|e| e.path == f));
        self.focus = i;
        self.anchor = i;
        i
    }

    /// Kategorie, które mają sens w tym folderze – „Wszystko” zawsze, „Media” tylko gdy są i zdjęcia, i filmy.
    pub fn shown_filters(&self, show_hidden: bool) -> Vec<(KindFilter, usize)> {
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
            .map(|(f, n)| (*f, n))
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
            if b.w > 0.0 && b.y + b.h >= y0 && b.x < x1 && b.x + bw > x0 {
                out.push(i);
            }
        }
        out
    }

    /// Układ jak w FileView.svelte: lista 32 px, siatka kafelków rozciągniętych do szerokości,
    /// galeria – wiersze o równej wysokości wypełnione co do piksela.
    pub fn layout(&mut self, width: f32, grid_size: f32, gallery_row: f32, ratio: impl Fn(&Entry) -> f32) {
        let pad = if self.view == View::List { 6.0 } else { 12.0 };
        let top0 = if self.view == View::List { 4.0 } else { pad };
        let avail = (width - pad * 2.0).max(100.0);
        let n = self.items.len();
        let mut boxes = vec![BoxR::default(); n];
        let mut y = top0;
        let mut heads = Vec::new();
        let all = [Group { label: String::new(), start: 0, end: n }];
        let groups: &[Group] = self.groups.as_deref().unwrap_or(&all);
        for (gi, grp) in groups.iter().enumerate() {
            if self.groups.is_some() {
                if !heads.is_empty() {
                    y += if self.view == View::List { 6.0 } else { 10.0 };
                }
                let shut = self.collapsed.contains(&grp.label);
                heads.push(Head { group: gi, y, shut });
                y += HEAD_H;
                if shut {
                    // Zwinięte: elementy bez miejsca (szerokość 0) – widok je pomija.
                    for b in boxes[grp.start..grp.end].iter_mut() {
                        *b = BoxR { x: pad, y, w: 0.0, h: 0.0 };
                    }
                    continue;
                }
            }
            self.layout_group(grp.start..grp.end, &mut boxes, &mut y, pad, avail, grid_size, gallery_row, &ratio);
        }
        self.heads = heads;
        self.boxes = boxes;
        self.total_h = y + pad;
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_group(&self, range: std::ops::Range<usize>, boxes: &mut [BoxR], y: &mut f32, pad: f32, avail: f32, grid_size: f32, gallery_row: f32, ratio: &impl Fn(&Entry) -> f32) {
        const LIST_H: f32 = 32.0;
        const GAP: f32 = 6.0;
        let (lo, n) = (range.start, range.end);
        if lo >= n {
            return;
        }
        match self.view {
            View::List => {
                for b in boxes[lo..n].iter_mut() {
                    *b = BoxR { x: pad, y: *y, w: avail, h: LIST_H };
                    *y += LIST_H;
                }
            }
            View::Grid => {
                let cols = ((avail + GAP) / (grid_size + GAP)).floor().max(1.0) as usize;
                let tile_w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
                let tile_h = tile_w + 40.0;
                for (k, b) in boxes[lo..n].iter_mut().enumerate() {
                    let c = k % cols;
                    if k > 0 && c == 0 {
                        *y += tile_h + GAP;
                    }
                    *b = BoxR { x: pad + c as f32 * (tile_w + GAP), y: *y, w: tile_w, h: tile_h };
                }
                *y += tile_h + GAP;
            }
            View::Gallery => {
                let h0 = gallery_row;
                let mut start = lo;
                let mut sum = 0.0f32;
                let flush = |start: usize, end: usize, sum: f32, last: bool, y: &mut f32, boxes: &mut [BoxR]| {
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
                for i in lo..n {
                    sum += ratio(&self.items[i]);
                    if sum * h0 + GAP * (i - start) as f32 >= avail {
                        flush(start, i + 1, sum, false, y, boxes);
                        start = i + 1;
                        sum = 0.0;
                    }
                }
                if start < n {
                    flush(start, n, sum, true, y, boxes);
                }
            }
        }
    }

    /// Indeksy elementów, których prostokąty przecinają pas treści [lo, hi].
    pub fn visible_in(&self, lo: f32, hi: f32) -> std::ops::Range<usize> {
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
#[serde(default)]
pub struct FolderPrefs {
    pub view: View,
    pub sort: SortKey,
    pub desc: bool,
    pub group: GroupKey,
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
    pub saved_searches: Vec<SavedSearch>,
    /// Zwinięte sekcje panelu bocznego (nazwy nagłówków).
    pub side_shut: Vec<String>,
    /// Panel boczny zwinięty do paska ikon.
    pub side_mini: bool,
    /// Ścieżki na półce i czy jest otwarta.
    pub shelf: Vec<String>,
    pub shelf_open: bool,
    /// Panel podglądu: widoczny i szerokość.
    pub preview_open: bool,
    pub preview_width: f32,
    /// Panel informacji (EXIF) w przeglądarce.
    pub viewer_info: bool,
    /// Przypięte karty – wracają przy uruchomieniu.
    pub pinned_tabs: Vec<String>,
    /// Motyw: „system”, „dark” albo „light”; pusty – jeszcze z ustawień wersji webowej.
    pub theme: String,
    /// Akcent: „mono” (biały / czarny), „system” (z Windows) albo kolor #rrggbb.
    pub accent: String,
    /// Po uruchomieniu: „home” (folder domowy), „computer” (Ten komputer) albo „folder” (`start_folder`).
    pub start: String,
    pub start_folder: String,
    /// Widok folderu, dla którego nic nie zapamiętano.
    pub default_view: View,
    pub confirm_trash: bool,
    /// Wbudowany terminal: widoczny i wysokość panelu.
    pub terminal_open: bool,
    pub terminal_height: f32,
    /// Tło okna: „none”, „mica” albo „acrylic”.
    pub window_effect: String,
    /// Krycie wyskakujących okienek w procentach (60–100).
    pub popup_opacity: f32,
    /// Foldery przypięte w panelu bocznym (sekcja „Przypięte”).
    pub favorites: Vec<String>,
    /// Szerokość panelu bocznego.
    pub side_width: f32,
    /// Lokalizacje sieciowe dodane w panelu bocznym (ścieżki UNC).
    pub net_places: Vec<String>,
    /// Kolejność sekcji ekranu „Ten komputer” (klucze z home.rs) i sekcje zwinięte.
    pub home_order: Vec<String>,
    pub home_shut: Vec<String>,
    /// Ikony pulpitu ustawione ręcznie: ścieżka (małymi literami) → kolumna i wiersz siatki.
    pub desktop_pos: HashMap<String, (i32, i32)>,
    /// Programy przypięte do paska zadań (pełne ścieżki .exe) w kolejności przycisków.
    pub taskbar_pins: Vec<String>,
    /// Kafelki menu Start (ścieżki shell:AppsFolder…) w kolejności przypinania – stary zapis,
    /// przenoszony do `start_groups` przy pierwszym użyciu.
    pub start_pins: Vec<String>,
    /// Grupy kafelków menu Start (jak w Windows 10): nazwa i kafelki, także foldery.
    pub start_groups: Vec<StartGroup>,
    /// Aplikacje ukryte w menu Start (te same ścieżki).
    pub start_hidden: Vec<String>,
    /// Rozmiar menu Start (piksele logiczne) – zmieniany przeciąganiem górnej i prawej krawędzi.
    pub start_size: (f32, f32),
    /// Start: duże ikony w siatce zamiast listy z nazwami.
    pub start_grid: bool,
    /// Widżety na pulpicie.
    pub desktop_widgets: Vec<DeskWidget>,
    /// Ustawienia z OpenFilePower już przeniesione (migrate.rs).
    pub migrated_ofp: bool,
    /// Nakładka z wydajnością (Alt+R): włączona, róg (0 – lewy górny, 1 – prawy górny,
    /// 2 – lewy dolny, 3 – prawy dolny) i pełna (z wykresem, RAM i VRAM) albo mała.
    pub overlay_on: bool,
    pub overlay_corner: u8,
    pub overlay_full: bool,
    /// Ustawienia widżetów: strefy zegarów świata (puste – domyślne), miejscowość pogody,
    /// folder zdjęć (pusty – Obrazy) i przypomnienia.
    pub world_zones: Vec<String>,
    pub weather_city: String,
    pub photo_folder: String,
    pub reminders: Vec<Reminder>,
    /// Jedno kliknięcie otwiera plik albo folder (zaznacza Ctrl, Shift i ramka).
    pub single_click: bool,
    /// Sekcje panelu bocznego wyłączone w Ustawieniach (nazwy nagłówków).
    pub side_hidden: Vec<String>,
    /// Krycie tła okna z Mica/Acrylic w procentach (0 – domyślne dla efektu).
    pub window_opacity: f32,
    /// Krzyżyk chowa okno do zasobnika zamiast kończyć PowerDesk.
    pub close_to_tray: bool,
    /// Przy starcie z Windows pokazuje też okno folderów (inaczej tylko pulpit, pasek i zasobnik).
    pub autostart_window: bool,
    /// Krycie tła paska zadań w procentach (0 – całkiem przezroczysty).
    pub taskbar_opacity: f32,
    /// Pasek całkiem przezroczysty, gdy na jego monitorze nie ma otwartych okien.
    pub taskbar_clear_desktop: bool,
}

/// Grupa kafelków menu Start.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct StartGroup {
    pub name: String,
    pub tiles: Vec<StartPin>,
}

/// Kafelek menu Start: aplikacja albo folder (niepuste `items`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct StartPin {
    /// Ścieżka aplikacji (shell:AppsFolder…); pusta w folderze.
    pub path: String,
    /// Rozmiar: 0 – średni, 1 – mały, 2 – szeroki, 3 – duży.
    pub size: u8,
    /// Nazwa folderu (aplikacja ma nazwę z listy aplikacji).
    pub name: String,
    pub items: Vec<StartPin>,
    /// Widżet zamiast aplikacji (rodzaj z widgets.rs) i treść notatki.
    pub widget: String,
    pub note: String,
}

/// Przypomnienie: czas (sekundy od 1970, czas lokalny przeliczony) i treść.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Reminder {
    pub at: i64,
    pub text: String,
}

/// Widżet na pulpicie: rodzaj, notatka, monitor (numer okna pulpitu) i miejsce w pikselach logicznych.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct DeskWidget {
    pub kind: String,
    pub note: String,
    pub monitor: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl StartPin {
    pub fn app(path: &str) -> Self {
        StartPin { path: path.to_string(), ..Default::default() }
    }

    pub fn is_folder(&self) -> bool {
        !self.items.is_empty()
    }

    /// Szerokość i wysokość w polach siatki (pole 48 px + 4 px odstępu).
    pub fn cells(&self) -> (usize, usize) {
        match self.size {
            1 => (1, 1),
            2 => (4, 2),
            3 => (4, 4),
            _ => (2, 2),
        }
    }
}

/// Wyszukiwanie zapisane w panelu bocznym.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SavedSearch {
    pub id: String,
    pub name: String,
    pub root: String,
    pub query: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { folders: HashMap::new(), grid_size: 132.0, gallery_row: 210.0, show_hidden: false, folders_first: true, auto_gallery: true, list_cols: Col::DEFAULT.to_vec(), list_col_w: HashMap::new(), folder_sizes: true, saved_searches: Vec::new(), side_shut: Vec::new(), side_mini: false, shelf: Vec::new(), shelf_open: false, preview_open: false, preview_width: 340.0, viewer_info: false, pinned_tabs: Vec::new(), theme: String::new(), accent: "mono".into(), start: "home".into(), start_folder: String::new(), default_view: View::List, confirm_trash: false, terminal_open: false, terminal_height: 260.0, window_effect: "none".into(), popup_opacity: 100.0, favorites: Vec::new(), side_width: 248.0, net_places: Vec::new(), home_order: Vec::new(), home_shut: Vec::new(), desktop_pos: HashMap::new(), taskbar_pins: Vec::new(), start_pins: Vec::new(), start_groups: Vec::new(), start_hidden: Vec::new(), start_size: (660.0, 640.0), start_grid: false, desktop_widgets: Vec::new(), migrated_ofp: false, overlay_on: false, overlay_corner: 1, overlay_full: true, world_zones: Vec::new(), weather_city: String::new(), photo_folder: String::new(), reminders: Vec::new(), single_click: false, side_hidden: Vec::new(), window_opacity: 0.0, close_to_tray: true, autostart_window: false, taskbar_opacity: 100.0, taskbar_clear_desktop: false }
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
        assert_eq!(t.shown_filters(false).into_iter().map(|x| x.0).collect::<Vec<_>>(), vec![KindFilter::All, KindFilter::Media, KindFilter::Image, KindFilter::Video, KindFilter::Doc]);
        t.kind_filter = KindFilter::Media;
        t.rebuild(false, true, |e| e.size as i64, |_| None);
        assert_eq!(t.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a.jpg", "b.mp4"]);
    }

    #[test]
    fn marquee_hits() {
        let mut t = Tab::new("C:/t");
        t.entries = (0..10).map(|i| entry(&format!("p{i}.txt"), false)).collect();
        t.rebuild(false, true, |e| e.size as i64, |_| None);
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
    fn selection_memory() {
        let mut t = Tab::new("C:/t");
        t.entries = vec![entry("a.txt", false), entry("b.txt", false)];
        t.rebuild(false, true, |e| e.size as i64, |_| None);
        t.selected.insert("C:/t/b.txt".into());
        t.focus = Some(1);
        t.remember_selection();
        t.selected.clear();
        t.focus = None;
        assert_eq!(t.restore_selection(), Some(1));
        assert!(t.selected.contains("C:/t/b.txt"));
    }

    #[test]
    fn grouping() {
        let mut t = Tab::new("C:/t");
        t.entries = vec![entry("b.jpg", false), entry("a.pdf", false), entry("Folder", true), entry("c.jpg", false)];
        t.group = GroupKey::Kind;
        t.rebuild(false, true, |e| e.size as i64, |_| None);
        let g = t.groups.clone().unwrap();
        assert_eq!(g.iter().map(|g| (g.label.as_str(), g.end - g.start)).collect::<Vec<_>>(), [("Foldery", 1), ("Zdjęcia", 2), ("Dokumenty", 1)]);
        // Zwinięta grupa zajmuje tylko nagłówek, a jej elementy nie mają miejsca.
        t.collapsed.insert("Zdjęcia".into());
        t.layout(800.0, 132.0, 210.0, |_| 1.0);
        assert_eq!(t.heads.len(), 3);
        assert!(t.boxes[1].w == 0.0 && t.boxes[2].w == 0.0 && t.boxes[3].w > 0.0);
    }

    #[test]
    fn wildcards() {
        assert!(wildcard("*.jpg", "zdjęcie.jpg"));
        assert!(wildcard("img_??.png", "img_01.png"));
        assert!(!wildcard("*.jpg", "plik.png"));
    }
}
