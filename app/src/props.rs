//! Okno właściwości (Dialogs.svelte → props, MetaView.svelte): rozmiar z zawartością folderów,
//! lokalizacja, daty, atrybuty, SHA-256 na żądanie i metadane systemu pogrupowane w sekcje.
//! Rozmiary folderów, metadane i skrót liczą się w tle; spóźnione wyniki starego okna są pomijane.

use std::cell::Cell;
use std::path::Path;

use chrono::{Local, TimeZone};
use powerdesk_core::fsx::{self, Entry};
use powerdesk_core::{clip, maps, shell};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::transfers::{self, Kind};
use crate::{Gauge, PropRow, PropSection, Props, Shared};

thread_local! {
    /// Numer otwarcia okna – wynik z tła pasuje tylko do okna, dla którego go liczono.
    static GEN: Cell<u64> = const { Cell::new(0) };
    /// Współrzędne pokazanego zdjęcia – do otwarcia w przeglądarce.
    static COORDS: Cell<Option<(f64, f64)>> = const { Cell::new(None) };
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
    // Techniczne identyfikatory i duble dat (Windows dokłada je do zwykłych obrazów).
    "System.ItemPathDisplayNarrow", "System.Media.ClassPrimaryID", "System.Media.ClassSecondaryID", "System.Media.CollectionGroupID",
    "System.Media.CollectionID", "System.Media.ContentID", "System.Media.UniqueFileIdentifier", "System.Document.DateCreated",
    "System.Document.DateSaved", "System.ItemDate", "System.DateImported", "System.DateAcquired",
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

pub fn row(label: &str, value: impl Into<SharedString>) -> PropRow {
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

pub fn plural(n: u64, one: &str, few: &str, many: &str) -> String {
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
        let fl35 = num("System.Photo.FocalLengthInFilm").map(|x| powerdesk_core::trf!(" ({x} mm pełnej klatki)")).unwrap_or_default();
        parts.push(format!("{} mm{fl35}", (fl * 10.0).round() / 10.0));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Skale ekspozycji: przysłona, czas, ISO i ogniskowa na torach w typowych zakresach.
/// Położenie liczone logarytmicznie, bo tak działa każda z tych skal.
pub fn gauges(props: &[shell::Prop]) -> Vec<Gauge> {
    let num = |k: &str| props.iter().find(|p| p.key == k).and_then(|p| p.num).filter(|n| *n > 0.0);
    let pos = |v: f64, lo: f64, hi: f64| ((v.ln() - lo.ln()) / (hi.ln() - lo.ln())).clamp(0.0, 1.0) as f32;
    let mut v = Vec::new();
    if let Some(f) = num("System.Photo.FNumber") {
        v.push(Gauge {
            label: crate::tr("Przysłona").into(),
            value: format!("f/{}", (f * 10.0).round() / 10.0).into(),
            // Większa liczba f to mniejszy otwór – skala rośnie w prawo wraz z liczbą.
            pos: pos(f, 1.0, 32.0),
            lo: "f/1".into(),
            hi: "f/32".into(),
        });
    }
    if let Some(t) = num("System.Photo.ExposureTime") {
        v.push(Gauge {
            label: crate::tr("Czas").into(),
            value: if t >= 1.0 { format!("{} s", (t * 10.0).round() / 10.0) } else { format!("1/{} s", (1.0 / t).round()) }.into(),
            pos: pos(t, 1.0 / 8000.0, 30.0),
            lo: "1/8000 s".into(),
            hi: crate::tr("30 s").into(),
        });
    }
    if let Some(iso) = num("System.Photo.ISOSpeed") {
        v.push(Gauge { label: crate::tr("Czułość").into(), value: format!("ISO {iso}").into(), pos: pos(iso, 50.0, 25600.0), lo: "ISO 50".into(), hi: "ISO 25600".into() });
    }
    if let Some(fl) = num("System.Photo.FocalLength") {
        v.push(Gauge {
            label: crate::tr("Ogniskowa").into(),
            value: format!("{} mm", (fl * 10.0).round() / 10.0).into(),
            pos: pos(fl, 8.0, 600.0),
            lo: crate::tr("8 mm").into(),
            hi: crate::tr("600 mm").into(),
        });
    }
    v
}

/// Współrzędne zdjęcia: najpierw gotowe wartości dziesiętne, inaczej stopnie, minuty i sekundy.
pub fn gps(props: &[shell::Prop]) -> Option<(f64, f64)> {
    let find = |k: &str| props.iter().find(|p| p.key == k);
    let dec = |k: &str| find(k).and_then(|p| p.num.or_else(|| p.value.replace(',', ".").trim().parse().ok()));
    let sign = |k: &str, neg: char| if find(k).map(|p| p.value.to_uppercase().starts_with(neg)).unwrap_or(false) { -1.0 } else { 1.0 };
    // „52; 13; 45,6” albo „52° 13' 45,6"” – bierzemy po kolei liczby.
    let dms = |k: &str| {
        let p = find(k)?;
        let nums: Vec<f64> = p
            .value
            .split(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.'))
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.replace(',', ".").parse::<f64>().ok())
            .collect();
        match nums.as_slice() {
            [d] => Some(*d),
            [d, m] => Some(d + m / 60.0),
            [d, m, sec, ..] => Some(d + m / 60.0 + sec / 3600.0),
            _ => None,
        }
    };
    let lat = dec("System.GPS.LatitudeDecimal").or_else(|| dms("System.GPS.Latitude").map(|v| v * sign("System.GPS.LatitudeRef", 'S')))?;
    let lon = dec("System.GPS.LongitudeDecimal").or_else(|| dms("System.GPS.Longitude").map(|v| v * sign("System.GPS.LongitudeRef", 'W')))?;
    (lat.abs() <= 90.0 && lon.abs() <= 180.0 && (lat != 0.0 || lon != 0.0)).then_some((lat, lon))
}

/// PNG mapy na bufor pikseli dla Slint (dekodowanie w tle, poza wątkiem UI).
pub fn decode_png(png: &[u8]) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
    Some(slint::SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

/// Otwiera ostatnio pokazany punkt w przeglądarce (OpenStreetMap).
pub fn open_map() {
    if let Some((lat, lon)) = COORDS.with(|c| c.get()) {
        let _ = open::that(maps::web_url(lat, lon));
    }
}

/// Wymiary obrazu albo kadru wideo z liczbą megapikseli.
fn dims(props: &[shell::Prop]) -> Option<String> {
    let num = |a: &str, b: &str| props.iter().find(|p| p.key == a || p.key == b).and_then(|p| p.num).filter(|n| *n > 0.0);
    let (w, h) = (num("System.Image.HorizontalSize", "System.Video.FrameWidth")?, num("System.Image.VerticalSize", "System.Video.FrameHeight")?);
    Some(powerdesk_core::trf!("{w} × {h} ({:.1} Mpx)", w * h / 1e6))
}

/// Metadane w sekcjach; pierwsze wiersze to wyliczona ekspozycja i wymiary.
pub fn sections(props: &[shell::Prop]) -> Vec<PropSection> {
    let mut groups: Vec<Vec<PropRow>> = vec![Vec::new(); SECTIONS.len()];
    if let Some(e) = exposure(props) {
        groups[0].push(row(crate::tr("Ekspozycja"), e));
    }
    if let Some(d) = dims(props) {
        let video = props.iter().any(|p| p.key.starts_with("System.Video."));
        groups[if video { 3 } else { 1 }].push(row(crate::tr("Wymiary"), d));
    }
    for p in props.iter().filter(|p| !SKIP.contains(&p.key.as_str())) {
        let i = SECTIONS.iter().position(|(_, _, m)| m(&p.key)).unwrap_or(SECTIONS.len() - 1);
        groups[i].push(row(&p.name, p.value.clone()));
    }
    SECTIONS
        .iter()
        .zip(groups)
        .filter(|(_, rows)| !rows.is_empty())
        .map(|((title, icon, _), rows)| PropSection { title: crate::tr(*title).into(), icon: crate::icons::icon(icon).into(), rows: ModelRc::new(VecModel::from(rows)) })
        .collect()
}

fn attributes(e: &Entry) -> String {
    let a: Vec<&str> = [(e.readonly, crate::tr("tylko do odczytu")), (e.hidden, "ukryty"), (e.system, "systemowy"), (e.link, crate::tr("dowiązanie")), (e.cloud, crate::tr("tylko w chmurze"))]
        .into_iter()
        .filter_map(|(on, l)| on.then_some(l))
        .collect();
    if a.is_empty() { "brak".into() } else { a.join(", ") }
}

/// Wiersze podstawowe; `totals` – (rozmiar, pliki, foldery), gdy już policzone.
fn basic(entries: &[Entry], totals: Option<(u64, u64, u64)>) -> Vec<PropRow> {
    let mut v = vec![row(crate::tr("Rozmiar"), totals.map(|t| size_text(t.0)).unwrap_or_else(|| crate::tr("liczę…").into()))];
    if let Some((_, files, dirs)) = totals.filter(|t| t.2 > 0 || entries.len() > 1) {
        v.push(row(crate::tr("Zawiera"), format!("{}, {}", plural(files, "plik", "pliki", crate::tr("plików")), plural(dirs, "folder", "foldery", crate::tr("folderów")))));
    }
    if let [e] = entries {
        v.push(row(crate::tr("Lokalizacja"), Path::new(&e.path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| e.path.clone())));
        v.push(row(crate::tr("Utworzono"), date_full(e.created)));
        v.push(row(crate::tr("Zmodyfikowano"), date_full(e.modified)));
        v.push(row(crate::tr("Atrybuty"), attributes(e)));
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
    let e = if crate::archives::inside(&dir) { powerdesk_core::archive::stat(&dir).ok_or_else(|| crate::tr("Nie znaleziono elementu archiwum").to_string()) } else { fsx::stat(&dir) };
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
                g.set_title(plural(entries.len() as u64, "element", "elementy", crate::tr("elementów")).into());
                g.set_subtitle(crate::tr("Wiele elementów").into());
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
        g.set_gauges(ModelRc::default());
        g.set_has_map(false);
        g.set_coords("".into());
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
            let coords = gps(&props);
            deliver(gen, move |_, g| {
                g.set_gauges(ModelRc::new(VecModel::from(gauges(&props))));
                g.set_sections(ModelRc::new(VecModel::from(sections(&props))));
            });
            // Mapa dochodzi osobno – kafelki trzeba pobrać z sieci.
            let Some((lat, lon)) = coords else { return };
            deliver(gen, move |_, g| {
                COORDS.with(|c| c.set(Some((lat, lon))));
                g.set_coords(format!("{lat:.5}, {lon:.5}").into());
            });
            let Some(buf) = maps::map_png(lat, lon, 15, 1024, 360).and_then(|png| decode_png(&png)) else { return };
            deliver(gen, move |_, g| {
                g.set_map(slint::Image::from_rgba8(buf));
                g.set_has_map(true);
            });
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
        transfers::toast(s, crate::tr("Skopiowano"), Kind::Ok, None);
    }
}

pub fn system(s: &Shared) {
    let mut path = String::new();
    with_ui(s, |g| path = g.get_path().into());
    if !path.is_empty() {
        shell::show_properties(crate::hwnd(s), &path);
    }
}
