//! Filtry dopasowane do zawartości folderu (facets.ts). Pokazujemy tylko te, które coś zmieniają
//! (pasuje więcej niż zero i mniej niż wszystko), więc zestaw sam się zmienia: w folderze ze zdjęciami –
//! formaty, kadr, daty; z filmami – długość i jakość; z dokumentami – rodzaje plików.
//! Test zależy tylko od identyfikatora („fmt:JPG”, „or:land”…), więc wybrane filtry to zwykłe napisy.

use std::collections::{BTreeSet, HashMap};

use chrono::{Datelike, Local, TimeZone};
use powerdesk_core::fsx::Entry;
use powerdesk_core::shell::MediaInfo;

use crate::fmt::{kind_of, Kind};

#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    pub id: String,
    pub label: String,
    pub group: &'static str,
    pub count: usize,
}

const DAY: i64 = 86_400_000;
const DOC_KINDS: [(Kind, &str, &str); 5] = [(Kind::Pdf, "pdf", "PDF"), (Kind::Doc, "doc", "Teksty"), (Kind::Sheet, "sheet", "Arkusze"), (Kind::Slides, "slides", "Prezentacje"), (Kind::Text, "text", "Notatki")];

/// Format do grupowania: RAW razem, JPG/JPEG/JFIF razem.
fn format(e: &Entry) -> String {
    let ext = e.ext.to_lowercase();
    if kind_of(e) == Kind::Raw {
        "RAW".into()
    } else if ext == "jpeg" || ext == "jfif" {
        "JPG".into()
    } else {
        ext.to_uppercase()
    }
}

fn is_media(k: Kind) -> bool {
    matches!(k, Kind::Image | Kind::Raw | Kind::Video)
}

/// Grupa filtra po identyfikatorze.
fn group_of(id: &str) -> &'static str {
    match id.split_once(':').map(|x| x.0).unwrap_or("") {
        "fmt" => "Format",
        "or" => "Kadr",
        "res" => "Rozdzielczość",
        "dur" => "Długość",
        "vres" => "Jakość",
        "doc" => "Rodzaj",
        "size" => "Rozmiar",
        _ => "Data",
    }
}

thread_local! {
    static BOUNDS: std::cell::Cell<Option<(std::time::Instant, i64, i64)>> = const { std::cell::Cell::new(None) };
}

/// Początek dzisiejszego dnia i bieżącego roku (ms). Strefa czasowa Windows jest droga, a test
/// leci dla każdego pliku i każdego filtra – liczymy najwyżej raz na minutę.
fn day_bounds() -> (i64, i64) {
    BOUNDS.with(|b| {
        if let Some((_, today, year)) = b.get().filter(|(at, _, _)| at.elapsed().as_secs() < 60) {
            return (today, year);
        }
        let now = Local::now();
        let today = now.date_naive().and_hms_opt(0, 0, 0).and_then(|d| Local.from_local_datetime(&d).earliest()).map(|d| d.timestamp_millis()).unwrap_or(0);
        let year = Local.with_ymd_and_hms(now.year(), 1, 1, 0, 0, 0).earliest().map(|d| d.timestamp_millis()).unwrap_or(0);
        b.set(Some((std::time::Instant::now(), today, year)));
        (today, year)
    })
}

/// Test filtra; `m` – metadane zdjęcia/filmu, jeśli już są.
pub fn test(id: &str, e: &Entry, m: Option<MediaInfo>) -> bool {
    let k = kind_of(e);
    let ratio = m.filter(|m| m.w > 0 && m.h > 0).map(|m| m.w as f64 / m.h as f64).unwrap_or(0.0);
    let video = |f: &dyn Fn(&MediaInfo) -> bool| k == Kind::Video && m.as_ref().is_some_and(f);
    let short = |m: &MediaInfo| m.w.min(m.h);
    match id {
        "or:land" => ratio > 1.05 && ratio < 2.0,
        "or:port" => ratio > 0.0 && ratio < 0.95,
        "or:sq" => (0.95..=1.05).contains(&ratio),
        "or:pano" => ratio >= 2.0,
        "res:big" => m.is_some_and(|m| m.w as u64 * m.h as u64 >= 20_000_000),
        // Czas trwania w milisekundach.
        "dur:short" => video(&|m| m.dur > 0 && m.dur < 60_000),
        "dur:mid" => video(&|m| m.dur >= 60_000 && m.dur < 600_000),
        "dur:long" => video(&|m| m.dur >= 600_000),
        "vres:4k" => video(&|m| short(m) >= 2000),
        "vres:hd" => video(&|m| (1000..2000).contains(&short(m))),
        "size:big" => e.size >= 100 << 20,
        _ if id.starts_with("fmt:") => !e.ext.is_empty() && format(e) == id[4..],
        _ if id.starts_with("doc:") => DOC_KINDS.iter().any(|(dk, key, _)| *dk == k && *key == &id[4..]),
        _ if id.starts_with("date:") => {
            // Zdjęcia i filmy według daty wykonania, reszta według modyfikacji.
            let when = if is_media(k) { m.map(|m| m.taken).filter(|t| *t > 0).unwrap_or(e.modified) } else { e.modified };
            let (today, year) = day_bounds();
            match &id[5..] {
                "today" => when >= today,
                "week" => when >= today - 6 * DAY,
                "month" => when >= today - 29 * DAY,
                "year" => when >= year,
                _ => when < year,
            }
        }
        _ => false,
    }
}

/// Definicje (id, etykieta) dopasowane do składu plików.
fn defs_for(files: &[&Entry]) -> Vec<(String, String)> {
    let mut kinds: HashMap<Kind, usize> = HashMap::new();
    for e in files {
        *kinds.entry(kind_of(e)).or_default() += 1;
    }
    let n = files.len().max(1) as f64;
    let share = |ks: &[Kind]| ks.iter().map(|k| kinds.get(k).copied().unwrap_or(0)).sum::<usize>() as f64 / n;
    let mut defs: Vec<(String, String)> = Vec::new();

    // Formaty – sześć najczęstszych.
    let mut fmts: Vec<(String, usize)> = Vec::new();
    for e in files.iter().filter(|e| !e.ext.is_empty()) {
        let f = format(e);
        match fmts.iter_mut().find(|(x, _)| *x == f) {
            Some(x) => x.1 += 1,
            None => fmts.push((f, 1)),
        }
    }
    fmts.sort_by(|a, b| b.1.cmp(&a.1));
    for (f, _) in fmts.into_iter().take(6) {
        defs.push((format!("fmt:{f}"), f));
    }

    let d = |id: &str, l: &str| (id.to_string(), l.to_string());
    if share(&[Kind::Image, Kind::Raw]) >= 0.3 {
        defs.extend([d("or:land", "Poziome"), d("or:port", "Pionowe"), d("or:sq", "Kwadratowe"), d("or:pano", "Panoramy"), d("res:big", "20 Mpx+")]);
    }
    let vids = share(&[Kind::Video]);
    if vids >= 0.15 {
        defs.extend([d("dur:short", "Do 1 min"), d("dur:mid", "1–10 min"), d("dur:long", "Ponad 10 min"), d("vres:4k", "4K"), d("vres:hd", "Full HD")]);
    }
    if share(&[Kind::Pdf, Kind::Doc, Kind::Sheet, Kind::Slides, Kind::Text]) >= 0.3 {
        for (_, key, label) in DOC_KINDS {
            defs.push(d(&format!("doc:{key}"), label));
        }
    }
    if vids >= 0.15 || share(&[Kind::Archive, Kind::App, Kind::Other]) >= 0.3 {
        defs.push(d("size:big", "Duże (100 MB+)"));
    }
    defs.extend([d("date:today", "Dziś"), d("date:week", "7 dni"), d("date:month", "30 dni"), d("date:year", "Ten rok"), d("date:old", "Starsze")]);
    defs
}

/// Filtr wybranych faset: w obrębie grupy – „lub”, między grupami – „i”. Foldery nie przechodzą.
pub fn matches<'a>(ids: impl IntoIterator<Item = &'a String> + Clone, e: &Entry, m: Option<MediaInfo>) -> bool {
    if e.dir {
        return false;
    }
    let mut groups: Vec<&'static str> = ids.clone().into_iter().map(|id| group_of(id)).collect();
    groups.dedup();
    groups.iter().all(|g| ids.clone().into_iter().filter(|id| group_of(id) == *g).any(|id| test(id, e, m)))
}

/// Dostępne filtry dla listy plików – tylko te, które realnie zawężają wynik, oraz już wybrane.
pub fn facets_for(list: &[Entry], media: impl Fn(&Entry) -> Option<MediaInfo>, selected: &BTreeSet<String>) -> Vec<Facet> {
    let files: Vec<&Entry> = list.iter().filter(|e| !e.dir).collect();
    if files.len() < 4 || files.len() > 20_000 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (id, label) in defs_for(&files) {
        let group = group_of(&id);
        // Licznik jak w sklepach: uwzględnia filtry wybrane w innych grupach.
        let others: Vec<&String> = selected.iter().filter(|s| group_of(s) != group).collect();
        let (mut all, mut c) = (0, 0);
        for e in &files {
            let m = media(e);
            if !others.is_empty() && !matches(others.iter().copied(), e, m) {
                continue;
            }
            all += 1;
            if test(&id, e, m) {
                c += 1;
            }
        }
        if selected.contains(&id) || (c > 0 && c < all) {
            out.push(Facet { id, label, group, count: c });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> Entry {
        Entry {
            name: name.into(),
            path: format!("C:/t/{name}"),
            dir: false,
            size,
            modified: 0,
            created: 0,
            ext: name.rsplit_once('.').map(|x| x.1).unwrap_or("").into(),
            hidden: false,
            system: false,
            readonly: false,
            link: false,
            cloud: false,
        }
    }

    #[test]
    fn facets() {
        let list = vec![file("a.jpg", 1), file("b.jpeg", 1), file("c.png", 1), file("d.cr2", 1), file("e.jpg", 1)];
        let m = |e: &Entry| Some(MediaInfo { w: if e.name.starts_with('a') { 3000 } else { 2000 }, h: 2000, dur: 0, taken: 0 });
        let f = facets_for(&list, m, &BTreeSet::new());
        let ids: Vec<&str> = f.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(&ids[..3], ["fmt:JPG", "fmt:PNG", "fmt:RAW"]);
        assert_eq!(f[0].count, 3);
        assert!(ids.contains(&"or:land") && ids.contains(&"or:sq"));
        // Wszystkie pliki są „starsze” (data 0) – taki filtr niczego nie zawęża, więc go nie ma.
        assert!(!ids.contains(&"date:old"));
        // Format „lub” w grupie, kadr „i” między grupami.
        let sel: Vec<String> = vec!["fmt:JPG".into(), "fmt:PNG".into(), "or:land".into()];
        assert!(matches(sel.iter(), &list[0], m(&list[0])));
        assert!(!matches(sel.iter(), &list[2], m(&list[2])));
    }
}
