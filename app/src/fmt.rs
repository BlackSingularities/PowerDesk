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

/// Czas trwania jak `duration` w util.ts: 3:07, 1:02:03.
pub fn duration(ms: u64) -> String {
    let t = (ms as f64 / 1000.0).round() as u64;
    let (h, m, sec) = (t / 3600, (t % 3600) / 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
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

/// Zdjęcia, RAW i wideo – mają miniatury i własne proporcje w galerii.
pub fn is_media(e: &Entry) -> bool {
    !e.dir && matches!(kind_label(&e.ext.to_lowercase()), "Obraz" | "Zdjęcie RAW" | "Wideo")
}

/// Kategoria pliku – ta sama tabela rozszerzeń co `kindOf` w util.ts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Folder,
    Image,
    Raw,
    Video,
    Audio,
    Pdf,
    Doc,
    Sheet,
    Slides,
    Text,
    Code,
    Archive,
    App,
    Other,
}

pub fn kind_of(e: &Entry) -> Kind {
    if e.dir {
        return Kind::Folder;
    }
    match e.ext.to_lowercase().as_str() {
        "jpg" | "jpeg" | "jfif" | "png" | "gif" | "webp" | "bmp" | "ico" | "avif" | "svg" | "tif" | "tiff" | "heic" | "heif" | "jxl" => Kind::Image,
        "cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf" | "sr2" | "dng" | "raf" | "orf" | "rw2" | "pef" | "srw" | "x3f" | "3fr" | "erf" | "kdc" | "mrw" | "mos" | "iiq" | "rwl" => Kind::Raw,
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "mts" | "m2ts" | "3gp" | "flv" | "mpg" | "mpeg" | "vob" | "ogv" => Kind::Video,
        "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "opus" | "wma" | "aiff" | "ape" | "alac" => Kind::Audio,
        "pdf" => Kind::Pdf,
        "doc" | "docx" | "docm" | "odt" | "rtf" | "dotx" | "pages" | "epub" => Kind::Doc,
        "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "numbers" => Kind::Sheet,
        "ppt" | "pptx" | "ppsx" | "odp" | "key" => Kind::Slides,
        "txt" | "md" | "log" | "ini" | "cfg" | "conf" | "nfo" | "srt" | "vtt" => Kind::Text,
        "js" | "ts" | "jsx" | "tsx" | "json" | "xml" | "html" | "htm" | "css" | "scss" | "py" | "rs" | "kt" | "kts" | "java" | "c" | "h" | "cpp" | "hpp" | "cs" | "go" | "rb" | "php" | "sh" | "ps1" | "bat" | "cmd" | "yml" | "yaml" | "toml" | "sql" | "lua" | "swift" | "dart" | "vue" | "svelte" | "gradle" | "properties" | "reg" => Kind::Code,
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "cab" | "iso" => Kind::Archive,
        "exe" | "msi" | "lnk" | "appx" | "msix" | "url" => Kind::App,
        _ => Kind::Other,
    }
}
