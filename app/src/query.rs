//! Zapytanie z filtrami (query.ts): zwykłe słowa szukają w nazwie, a słowa z dwukropkiem zawężają wynik.
//!   typ:zdjęcia   rozmiar:>100mb   data:tydzień   ext:jpg,png   data:2024   rozmiar:1mb-1gb

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use powerdesk_core::fsx::Entry;

use crate::fmt::{kind_of, Kind};

pub type Test = Box<dyn Fn(&Entry) -> bool>;

pub struct Parsed {
    /// Część tekstowa – trafia do wyszukiwarki (nazwa, * i ?).
    pub text: String,
    /// Filtr dodatkowy albo None, gdy zapytanie ma same słowa.
    pub test: Option<Test>,
    /// Rozpoznane filtry – do pokazania w interfejsie.
    pub chips: Vec<String>,
    /// `zawiera:` – szukanie w treści plików (indeks Windows).
    pub content: Option<String>,
}

enum TypeSel {
    Kinds(&'static [Kind]),
    Dir,
    File,
}

const IMAGES: &[Kind] = &[Kind::Image, Kind::Raw];
const DOCS: &[Kind] = &[Kind::Pdf, Kind::Doc, Kind::Sheet, Kind::Slides, Kind::Text];

fn type_sel(v: &str) -> Option<TypeSel> {
    Some(match v {
        "zdjecia" | "zdjęcia" | "zdjecie" | "zdjęcie" | "foto" | "obrazy" | "image" => TypeSel::Kinds(IMAGES),
        "raw" => TypeSel::Kinds(&[Kind::Raw]),
        "wideo" | "video" | "filmy" | "film" => TypeSel::Kinds(&[Kind::Video]),
        "audio" | "muzyka" | "music" => TypeSel::Kinds(&[Kind::Audio]),
        "dokumenty" | "dokument" | "doc" => TypeSel::Kinds(DOCS),
        "pdf" => TypeSel::Kinds(&[Kind::Pdf]),
        "arkusze" => TypeSel::Kinds(&[Kind::Sheet]),
        "prezentacje" => TypeSel::Kinds(&[Kind::Slides]),
        "tekst" => TypeSel::Kinds(&[Kind::Text]),
        "kod" | "code" => TypeSel::Kinds(&[Kind::Code]),
        "archiwa" | "archiwum" | "zip" => TypeSel::Kinds(&[Kind::Archive]),
        "programy" | "app" => TypeSel::Kinds(&[Kind::App]),
        "folder" | "foldery" | "katalog" => TypeSel::Dir,
        "plik" | "pliki" | "file" => TypeSel::File,
        _ => return None,
    })
}

/// „100mb”, „1,5 GB”, „300k” → bajty.
fn bytes(s: &str) -> Option<u64> {
    let s = s.trim().replace(',', ".").to_lowercase();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mul: f64 = match unit.trim() {
        "" | "b" => 1.0,
        "k" | "kb" => 1024.0,
        "m" | "mb" => 1024.0f64.powi(2),
        "g" | "gb" => 1024.0f64.powi(3),
        "t" | "tb" => 1024.0f64.powi(4),
        _ => return None,
    };
    Some((n * mul) as u64)
}

const DAY: i64 = 86_400_000;

fn local_ms(d: NaiveDate) -> i64 {
    Local.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap()).earliest().map(|t| t.timestamp_millis()).unwrap_or(0)
}

/// Zakres dat [od, do) dla słów: dziś, wczoraj, tydzień, miesiąc, rok, 2024, 2024-05, 2024-05-17, >2024-01-01.
fn date_range(v: &str) -> Option<(i64, i64)> {
    let today_d = Local::now().date_naive();
    let today = local_ms(today_d);
    let w = v.to_lowercase();
    match w.as_str() {
        "dziś" | "dzis" | "today" => return Some((today, i64::MAX)),
        "wczoraj" | "yesterday" => return Some((today - DAY, today)),
        "tydzień" | "tydzien" | "week" => return Some((today - 6 * DAY, i64::MAX)),
        "miesiąc" | "miesiac" | "month" => return Some((today - 29 * DAY, i64::MAX)),
        "rok" | "year" => return Some((local_ms(NaiveDate::from_ymd_opt(today_d.year(), 1, 1)?), i64::MAX)),
        _ => {}
    }
    if let Some(rest) = w.strip_prefix('>') {
        let r = date_range(rest)?;
        return Some((if r.1 == i64::MAX { r.0 } else { r.1 }, i64::MAX));
    }
    if let Some(rest) = w.strip_prefix('<') {
        return Some((0, date_range(rest)?.0));
    }
    let parts: Vec<&str> = w.split('-').collect();
    if parts.is_empty() || parts.len() > 3 || parts[0].len() != 4 || !parts.iter().all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    match parts.len() {
        1 => Some((local_ms(NaiveDate::from_ymd_opt(y, 1, 1)?), local_ms(NaiveDate::from_ymd_opt(y + 1, 1, 1)?))),
        2 => {
            let m: u32 = parts[1].parse().ok()?;
            let start = NaiveDate::from_ymd_opt(y, m, 1)?;
            let end = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1)? } else { NaiveDate::from_ymd_opt(y, m + 1, 1)? };
            Some((local_ms(start), local_ms(end)))
        }
        _ => {
            let d = NaiveDate::from_ymd_opt(y, parts[1].parse().ok()?, parts[2].parse().ok()?)?;
            Some((local_ms(d), local_ms(d.succ_opt()?)))
        }
    }
}

/// Podpowiedzi pokazywane pod polem wyszukiwania.
pub const HINTS: [(&str, &str); 11] = [
    ("typ:zdjęcia", "Tylko zdjęcia"),
    ("typ:wideo", "Tylko filmy"),
    ("typ:dokumenty", "Dokumenty"),
    ("typ:folder", "Tylko foldery"),
    ("rozmiar:>100mb", "Większe niż 100 MB"),
    ("rozmiar:<1mb", "Mniejsze niż 1 MB"),
    ("data:dziś", "Zmienione dziś"),
    ("data:tydzień", "W ostatnim tygodniu"),
    ("data:2024", "W roku 2024"),
    ("ext:jpg,png", "Rozszerzenia"),
    ("zawiera:\"tekst\"", "Pliki zawierające tekst (indeks Windows – także Word, PDF)"),
];

pub fn parse(q: &str) -> Parsed {
    let mut words: Vec<&str> = Vec::new();
    let mut tests: Vec<Test> = Vec::new();
    let mut chips = Vec::new();
    // `zawiera:"dwa słowa"` albo `zawiera:słowo` – reszta zapytania to nazwa i filtry.
    let mut content = None;
    let mut rest = q.to_string();
    for key in ["zawiera:", "treść:", "content:"] {
        if let Some(at) = rest.to_lowercase().find(key) {
            let after = &rest[at + key.len()..];
            let (val, end) = match after.strip_prefix('"') {
                Some(r) => r.find('"').map_or((r, after.len()), |e| (&r[..e], e + 2)),
                None => after.find(char::is_whitespace).map_or((after, after.len()), |e| (&after[..e], e)),
            };
            if !val.trim().is_empty() {
                chips.push(format!("zawiera: {}", val.trim()));
                content = Some(val.trim().to_string());
            }
            rest = format!("{}{}", &rest[..at], &after[end..]);
            break;
        }
    }
    for tok in rest.split_whitespace() {
        let Some((key, val)) = tok.split_once(':').filter(|(k, v)| !k.is_empty() && !v.is_empty() && k.chars().all(char::is_alphabetic)) else {
            words.push(tok);
            continue;
        };
        match key.to_lowercase().as_str() {
            "typ" | "type" | "rodzaj" => {
                let sels: Vec<TypeSel> = val.split(',').filter_map(|v| type_sel(&v.to_lowercase())).collect();
                if sels.is_empty() {
                    words.push(tok);
                    continue;
                }
                tests.push(Box::new(move |e| {
                    sels.iter().any(|s| match s {
                        TypeSel::Dir => e.dir,
                        TypeSel::File => !e.dir,
                        TypeSel::Kinds(ks) => !e.dir && ks.contains(&kind_of(e)),
                    })
                }));
                chips.push(format!("typ: {val}"));
            }
            "ext" | "rozszerzenie" => {
                let exts: Vec<String> = val.to_lowercase().split(',').map(|x| x.trim_start_matches('.').to_string()).collect();
                chips.push(format!(".{}", exts.join(", .")));
                tests.push(Box::new(move |e| !e.dir && exts.contains(&e.ext.to_lowercase())));
            }
            "rozmiar" | "size" => {
                let test: Test = if let Some((lo, hi)) = val.split_once('-').and_then(|(a, b)| Some((bytes(a)?, bytes(b)?))) {
                    Box::new(move |e| !e.dir && e.size >= lo && e.size <= hi)
                } else if let Some(op) = ["<=", ">=", "<", ">"].into_iter().find(|op| val.starts_with(op)) {
                    let Some(b) = bytes(&val[op.len()..]) else {
                        words.push(tok);
                        continue;
                    };
                    match op {
                        ">" => Box::new(move |e| !e.dir && e.size > b),
                        ">=" => Box::new(move |e| !e.dir && e.size >= b),
                        "<" => Box::new(move |e| !e.dir && e.size < b),
                        _ => Box::new(move |e| !e.dir && e.size <= b),
                    }
                } else if let Some(b) = bytes(val) {
                    Box::new(move |e| !e.dir && e.size >= b)
                } else {
                    words.push(tok);
                    continue;
                };
                tests.push(test);
                chips.push(format!("rozmiar {val}"));
            }
            "data" | "date" | "zmieniony" | "modified" => {
                let Some((lo, hi)) = date_range(val) else {
                    words.push(tok);
                    continue;
                };
                tests.push(Box::new(move |e| e.modified >= lo && e.modified < hi));
                chips.push(format!("data: {val}"));
            }
            "utworzony" | "created" => {
                let Some((lo, hi)) = date_range(val) else {
                    words.push(tok);
                    continue;
                };
                tests.push(Box::new(move |e| e.created >= lo && e.created < hi));
                chips.push(format!("utworzony: {val}"));
            }
            _ => words.push(tok),
        }
    }
    let test: Option<Test> = if tests.is_empty() { None } else { Some(Box::new(move |e| tests.iter().all(|t| t(e)))) };
    Parsed { text: words.join(" "), test, chips, content }
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
    fn filters() {
        let q = parse("wakacje typ:zdjęcia rozmiar:>1mb");
        assert_eq!(q.text, "wakacje");
        assert_eq!(q.chips.len(), 2);
        let t = q.test.unwrap();
        assert!(t(&file("a.jpg", 2 << 20)));
        assert!(!t(&file("a.jpg", 10)));
        assert!(!t(&file("a.mp4", 2 << 20)));
        let r = parse("rozmiar:1kb-2kb").test.unwrap();
        assert!(r(&file("x", 1500)) && !r(&file("x", 5000)));
        assert!(parse("ext:.JPG,png").test.unwrap()(&file("a.png", 1)));
        // Nieznany klucz albo wartość zostaje zwykłym słowem.
        assert_eq!(parse("typ:coś c:\\x").text, "typ:coś c:\\x");
    }

    #[test]
    fn dates() {
        let (lo, hi) = date_range("2024-02").unwrap();
        assert_eq!(hi - lo, 29 * DAY);
        assert!(date_range(">2024").unwrap().0 > lo);
        assert!(date_range("2024-13").is_none());
    }
}
