//! Okno właściwości (Dialogs.svelte → props, MetaView.svelte): rozmiar z zawartością folderów,
//! lokalizacja, daty, atrybuty, SHA-256 na żądanie i metadane systemu pogrupowane w sekcje.
//! Rozmiary folderów, metadane i skrót liczą się w tle; spóźnione wyniki starego okna są pomijane.

use std::cell::Cell;
use std::path::Path;

use chrono::{Local, TimeZone};
use powerdesk_core::fsx::{self, Entry};
use powerdesk_core::{clip, shell};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::transfers::{self, Kind};
use crate::{PropRow, PropSection, Props, Shared};

thread_local! {
    /// Numer otwarcia okna – wynik z tła pasuje tylko do okna, dla którego go liczono.
    static GEN: Cell<u64> = const { Cell::new(0) };
}

/// Metadane pokazywane osobno (nazwa, rozmiar, daty pliku) albo nic niemówiące.
const SKIP: &[&str] = &[
    "System.ItemNameDisplay", "System.ItemName", "System.FileName", "System.ParsingName", "System.ItemPathDisplay", "System.ItemFolderPathDisplay",
    "System.ItemFolderNameDisplay", "System.ItemFolderPathDisplayNarrow", "System.Size", "System.ItemTypeText", "System.ItemType", "System.FileExtension",
    "System.DateModified", "System.DateCreated", "System.DateAccessed", "System.FileAttributes", "System.FileOwner", "System.ComputerName",
    "System.ContentType", "System.PerceivedType", "System.Kind", "System.KindText", "System.SharingStatus", "System.ZoneIdentifier", "System.IsShared",
    "System.SFGAOFlags", "System.Link.TargetParsingPath", "System.ItemNameSortOverride", "System.ThumbnailCacheId", "System.StorageProviderUIStatus",
    "System.OfflineAvailability", "System.OfflineStatus", "System.Image.Dimensions", "System.Video.FrameWidth", "System.Video.FrameHeight",
    "System.Image.HorizontalSize", "System.Image.VerticalSize", "System.Photo.FNumber", "System.Photo.ExposureTime", "System.Photo.ISOSpeed",
    "System.Photo.FocalLength", "System.Photo.Aperture", "System.Photo.ShutterSpeed", "System.Photo.ExposureTimeNumerator",
    "System.Photo.ExposureTimeDenominator", "System.Photo.FNumberNumerator", "System.Photo.FNumberDenominator", "System.Photo.ApertureNumerator",
    "System.Photo.ApertureDenominator",
];

/// Sekcje jak w MetaView.svelte: (tytuł, ikona, czy klucz należy do sekcji).
const SECTIONS: &[(&str, &str, fn(&str) -> bool)] = &[
    ("Aparat", "image", |k| k.starts_with("System.Photo.")),
    ("Obraz", "gallery", |k| k.starts_with("System.Image.")),
    ("Lokalizacja", "pin", |k| k.starts_with("System.GPS.")),
    ("Wideo", "video", |k| k.starts_with("System.Video.")),
    ("Dźwięk", "music", |k| k.starts_with("System.Audio.")),
    ("Muzyka", "music", |k| k.starts_with("System.Music.") || k.starts_with("System.Media.")),
    (
        "Dokument",
        "doc",
        |k| {
            k.starts_with("System.Document.")
                || matches!(k, "System.Title" | "System.Author" | "System.Company" | "System.Subject" | "System.Keywords" | "System.Comment" | "System.Category" | "System.ApplicationName")
        },
    ),
    ("Program", "app", |k| k.starts_with("System.Software.") || matches!(k, "System.FileVersion" | "System.FileDescription" | "System.Copyright")),
    ("Inne", "info", |_| true),
];

fn row(label: &str, value: impl Into<SharedString>) -> PropRow {
    PropRow { label: label.into(), value: value.into() }
}

fn date_full(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).earliest().map(|d| d.format("%d.%m.%Y, %H:%M:%S").to_string()).unwrap_or_default()
}

fn size_text(n: u64) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('\u{a0}');
        }
        grouped.push(c);
    }
    format!("{} ({grouped} B)", crate::fmt::size(n))
}

fn plural(n: u64, one: &str, few: &str, many: &str) -> String {
    let w = if n == 1 {
        one
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        few
    } else {
        many
    };
    format!("{n} {w}")
}

/// Ekspozycja w jednym wierszu: „f/2.8 · 1/250 s · ISO 200 · 35 mm”.
fn exposure(props: &[shell::Prop]) -> Option<String> {
    let num = |k: &str| props.iter().find(|p| p.key == k).and_then(|p| p.num).filter(|n| *n > 0.0);
    let mut parts = Vec::new();
    if let Some(f) = num("System.Photo.FNumber") {
        parts.push(format!("f/{}", (f * 10.0).round() / 10.0));
    }
    if let Some(t) = num("System.Photo.ExposureTime") {
        parts.push(if t >= 1.0 { format!("{} s", (t * 10.0).round() / 10.0) } else { format!("1/{} s", (1.0 / t).round()) });
    }
    if let Some(iso) = num("System.Photo.ISOSpeed") {
        parts.push(format!("ISO {iso}"));
    }
    if let Some(fl) = num("System.Photo.FocalLength") {
        let fl35 = num("System.Photo.FocalLengthInFilm").map(|x| format!(" ({x} mm pełnej klatki)")).unwrap_or_default();
        parts.push(format!("{} mm{fl35}", (fl * 10.0).round() / 10.0));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Wymiary obrazu albo kadru wideo z liczbą megapikseli.
fn dims(props: &[shell::Prop]) -> Option<String> {
    let num = |a: &str, b: &str| props.iter().find(|p| p.key == a || p.key == b).and_then(|p| p.num).filter(|n| *n > 0.0);
    let (w, h) = (num("System.Image.HorizontalSize", "System.Video.FrameWidth")?, num("System.Image.VerticalSize", "System.Video.FrameHeight")?);
    Some(format!("{w} × {h} ({:.1} Mpx)", w * h / 1e6))
}

/// Metadane w sekcjach; pierwsze wiersze to wyliczona ekspozycja i wymiary.
fn sections(props: &[shell::Prop]) -> Vec<PropSection> {
    let mut groups: Vec<Vec<PropRow>> = vec![Vec::new(); SECTIONS.len()];
    if let Some(e) = exposure(props) {
        groups[0].push(row("Ekspozycja", e));
    }
    if let Some(d) = dims(props) {
        let video = props.iter().any(|p| p.key.starts_with("System.Video."));
        groups[if video { 3 } else { 1 }].push(row("Wymiary", d));
    }
    for p in props.iter().filter(|p| !SKIP.contains(&p.key.as_str())) {
        let i = SECTIONS.iter().position(|(_, _, m)| m(&p.key)).unwrap_or(SECTIONS.len() - 1);
        groups[i].push(row(&p.name, p.value.clone()));
    }
    SECTIONS
        .iter()
        .zip(groups)
        .filter(|(_, rows)| !rows.is_empty())
        .map(|((title, icon, _), rows)| PropSection { title: (*title).into(), icon: crate::icons::icon(icon).into(), rows: ModelRc::new(VecModel::from(rows)) })
        .collect()
}

fn attributes(e: &Entry) -> String {
    let a: Vec<&str> = [(e.readonly, "tylko do odczytu"), (e.hidden, "ukryty"), (e.system, "systemowy"), (e.link, "dowiązanie"), (e.cloud, "tylko w chmurze")]
        .into_iter()
        .filter_map(|(on, l)| on.then_some(l))
        .collect();
    if a.is_empty() { "brak".into() } else { a.join(", ") }
}

/// Wiersze podstawowe; `totals` – (rozmiar, pliki, foldery), gdy już policzone.
fn basic(entries: &[Entry], totals: Option<(u64, u64, u64)>) -> Vec<PropRow> {
    let mut v = vec![row("Rozmiar", totals.map(|t| size_text(t.0)).unwrap_or_else(|| "liczę…".into()))];
    if let Some((_, files, dirs)) = totals.filter(|t| t.2 > 0 || entries.len() > 1) {
        v.push(row("Zawiera", format!("{}, {}", plural(files, "plik", "pliki", "plików"), plural(dirs, "folder", "foldery", "folderów"))));
    }
    if let [e] = entries {
        v.push(row("Lokalizacja", Path::new(&e.path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| e.path.clone())));
        v.push(row("Utworzono", date_full(e.created)));
        v.push(row("Zmodyfikowano", date_full(e.modified)));
        v.push(row("Atrybuty", attributes(e)));
    }
    v
}

fn with_ui(s: &Shared, f: impl FnOnce(&Props)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<Props>());
    }
}

/// Wynik z tła – trafia do okna tylko wtedy, gdy to wciąż to samo otwarcie.
fn deliver(gen: u64, f: impl FnOnce(&Shared, &Props) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if GEN.with(|g| g.get()) != gen {
            return;
        }
        crate::with_state(|s| {
            if let Some(ui) = s.borrow().ui.upgrade() {
                f(s, &ui.global::<Props>());
            }
        })
    });
}

/// Otwiera okno dla zaznaczenia, a bez zaznaczenia – dla otwartego folderu.
pub fn open(s: &Shared) {
    let entries = crate::tab_mut(s, |t| t.items.iter().filter(|e| t.selected.contains(&e.path)).cloned().collect::<Vec<_>>());
    if entries.is_empty() {
        open_folder(s);
    } else {
        show(s, entries);
    }
}

/// Właściwości otwartego folderu (menu tła, przycisk bez zaznaczenia).
pub fn open_folder(s: &Shared) {
    let dir = crate::tab_mut(s, |t| t.path.clone());
    if dir.starts_with("ofp:") {
        return;
    }
    // Korzeń archiwum to plik – jego dane bierzemy z dysku; wnętrze archiwum – z listy archiwum.
    let e = if crate::archives::inside(&dir) { powerdesk_core::archive::stat(&dir).ok_or_else(|| "Nie znaleziono elementu archiwum".to_string()) } else { fsx::stat(&dir) };
    match e {
        Ok(e) => show(s, vec![e]),
        Err(e) => transfers::toast(s, e, Kind::Error, None),
    }
}

fn show(s: &Shared, entries: Vec<Entry>) {
    let gen = GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let one = (entries.len() == 1).then(|| entries[0].clone());
    let scale = s.borrow().ui.upgrade().map(|ui| ui.window().scale_factor()).unwrap_or(1.0);
    let packed = entries.iter().any(|e| crate::archives::inside(&e.path));
    with_ui(s, |g| {
        match &one {
            Some(e) => {
                g.set_title(e.name.clone().into());
                g.set_subtitle(crate::fmt::type_label(e).into());
                let img = crate::thumbs::get(&e.path, (48.0 * scale) as u32, true);
                g.set_has_img(img.is_some());
                g.set_img(img.unwrap_or_default());
            }
            None => {
                g.set_title(plural(entries.len() as u64, "element", "elementy", "elementów").into());
                g.set_subtitle("Wiele elementów".into());
                g.set_has_img(false);
            }
        }
        g.set_path(one.as_ref().map(|e| e.path.clone()).unwrap_or_default().into());
        g.set_can_hash(one.as_ref().is_some_and(|e| !e.dir) && !packed);
        g.set_can_system(one.is_some() && !packed);
        g.set_hash("".into());
        g.set_hashing(false);
        g.set_basic(ModelRc::new(VecModel::from(basic(&entries, None))));
        g.set_sections(ModelRc::default());
        g.set_open(true);
    });
    // Rozmiar z zawartością folderów.
    let list = entries.clone();
    std::thread::spawn(move || {
        let mut t = (0u64, 0u64, list.iter().filter(|e| e.dir).count() as u64);
        for e in &list {
            if e.dir && !crate::archives::inside(&e.path) {
                let d = fsx::dir_size(&e.path);
                t = (t.0 + d.size, t.1 + d.files, t.2 + d.dirs);
            } else if !e.dir {
                t = (t.0 + e.size, t.1 + 1, t.2);
            }
        }
        deliver(gen, move |_, g| g.set_basic(ModelRc::new(VecModel::from(basic(&list, Some(t))))));
    });
    // Metadane systemu – tylko dla jednego elementu spoza archiwum.
    if let Some(e) = one.filter(|_| !packed) {
        std::thread::spawn(move || {
            let props = shell::properties(&e.path);
            deliver(gen, move |_, g| g.set_sections(ModelRc::new(VecModel::from(sections(&props)))));
        });
    }
}

pub fn close(s: &Shared) {
    GEN.with(|g| g.set(g.get() + 1));
    with_ui(s, |g| {
        g.set_open(false);
        g.set_sections(ModelRc::default());
    });
}

pub fn compute_hash(s: &Shared) {
    let mut path = String::new();
    with_ui(s, |g| {
        path = g.get_path().into();
        g.set_hashing(true);
    });
    let gen = GEN.with(|g| g.get());
    std::thread::spawn(move || {
        let res = fsx::sha256(&path);
        deliver(gen, move |s, g| {
            g.set_hashing(false);
            match res {
                Ok(h) => g.set_hash(h.into()),
                Err(e) => transfers::toast(s, e, Kind::Error, None),
            }
        });
    });
}

/// Kopiuje wartość wiersza (ścieżkę, skrót…) do schowka.
pub fn copy(s: &Shared, text: &str) {
    if clip::set_text(text).is_ok() {
        transfers::toast(s, "Skopiowano", Kind::Ok, None);
    }
}

pub fn system(s: &Shared) {
    let mut path = String::new();
    with_ui(s, |g| path = g.get_path().into());
    if !path.is_empty() {
        shell::show_properties(crate::hwnd(s), &path);
    }
}
