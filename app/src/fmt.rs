//! Formatowanie jak w src/lib/util.ts (rozmiary, daty, typy) – żeby teksty były identyczne.

use chrono::{Local, TimeZone};
use powerdesk_core::fsx::Entry;

pub fn size(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut i = 0;
    v /= 1024.0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    // pl-PL: przecinek dziesiętny, jedna cyfra po przecinku poniżej 10.
    let s = if v < 10.0 { format!("{:.1}", v).trim_end_matches(".0").to_string() } else { format!("{:.0}", v) };
    format!("{} {}", s.replace('.', ","), units[i])
}

/// Odpowiednik `Intl.DateTimeFormat('pl-PL', { dateStyle: 'short', timeStyle: 'short' })`: 22.10.2026, 14:05
pub fn date(ms: i64) -> String {
    if ms <= 0 {
        return "—".into();
    }
    match Local.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(d) => d.format("%d.%m.%Y, %H:%M").to_string(),
        _ => "—".into(),
    }
}

fn kind_label(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "heif" | "avif" | "svg" | "ico" => "Obraz",
        "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "raf" => "Zdjęcie RAW",
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" | "mts" => "Wideo",
        "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "opus" | "wma" => "Dźwięk",
        "pdf" => "Dokument",
        "doc" | "docx" | "odt" | "rtf" => "Dokument",
        "xls" | "xlsx" | "ods" | "csv" => "Arkusz",
        "ppt" | "pptx" | "odp" => "Prezentacja",
        "zip" | "7z" | "rar" | "tar" | "gz" => "Archiwum",
        "txt" | "md" | "log" | "json" | "xml" | "yml" | "yaml" | "ini" => "Tekst",
        "exe" | "msi" => "Aplikacja",
        "lnk" | "url" => "Skrót",
        _ => "Plik",
    }
}

pub fn type_label(e: &Entry) -> String {
    if e.dir {
        return "Folder".into();
    }
    if e.ext.is_empty() {
        return "Plik".into();
    }
    format!("{} {}", kind_label(&e.ext.to_lowercase()), e.ext.to_uppercase())
}

pub fn items(n: usize) -> String {
    let w = if n == 1 {
        "element"
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        "elementy"
    } else {
        "elementów"
    };
    format!("{n} {w}")
}
