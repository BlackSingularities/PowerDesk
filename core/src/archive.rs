//! Archiwa ZIP i 7z przeglądane jak zwykłe foldery: `C:\a\zdjecia.zip\2024\x.jpg`.
//! Tylko do odczytu – wypakowanie przez przeciągnięcie, kopiowanie albo otwarcie pliku.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::fsx::{io_err, Entry};

const EXTS: [&str; 2] = ["zip", "7z"];

/// Pozycja w archiwum (ścieżka z ukośnikami, bez wiodącego).
#[derive(Clone)]
struct Item {
    name: String,
    dir: bool,
    size: u64,
    modified: i64,
}

/// Spis archiwum trzymamy w pamięci (klucz: ścieżka + czas modyfikacji pliku).
static CACHE: Lazy<Mutex<HashMap<String, (i64, Vec<Item>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn is_archive_ext(p: &Path) -> bool {
    p.extension().map(|e| EXTS.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false)
}

/// Dzieli ścieżkę na (plik archiwum, ścieżka wewnątrz). `None`, gdy ścieżka nie prowadzi do archiwum.
pub fn split(path: &str) -> Option<(PathBuf, String)> {
    let p = Path::new(path);
    for anc in p.ancestors() {
        if is_archive_ext(anc) && anc.is_file() {
            let inner = p.strip_prefix(anc).ok()?.to_string_lossy().replace('\\', "/");
            return Some((anc.to_path_buf(), inner.trim_matches('/').to_string()));
        }
        if anc.is_dir() {
            return None;
        }
    }
    None
}

fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn zip_time(dt: Option<zip::DateTime>) -> i64 {
    let Some(d) = dt else { return 0 };
    chrono::NaiveDate::from_ymd_opt(d.year() as i32, d.month() as u32, d.day() as u32)
        .and_then(|day| day.and_hms_opt(d.hour() as u32, d.minute() as u32, d.second() as u32))
        .map(|t| t.and_utc().timestamp_millis())
        .unwrap_or(0)
}

fn read_items(arc: &Path) -> Result<Vec<Item>, String> {
    let ext = arc.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut out = Vec::new();
    if ext == "zip" {
        let f = std::fs::File::open(arc).map_err(|e| io_err(&e))?;
        let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|_| "Nieobsługiwane lub uszkodzone archiwum".to_string())?;
        for i in 0..z.len() {
            if let Ok(f) = z.by_index_raw(i) {
                out.push(Item { name: f.name().replace('\\', "/"), dir: f.is_dir(), size: f.size(), modified: zip_time(f.last_modified()) });
            }
        }
    } else {
        let a = sevenz_rust::Archive::open(arc).map_err(|e| format!("Nie udało się otworzyć archiwum 7z: {e}"))?;
        for f in &a.files {
            let ft: u64 = f.last_modified_date.into();
            let modified = if f.has_last_modified_date { (ft / 10_000) as i64 - 11_644_473_600_000 } else { 0 };
            out.push(Item { name: f.name.replace('\\', "/"), dir: f.is_directory, size: f.size, modified });
        }
    }
    Ok(out)
}

fn items(arc: &Path) -> Result<Vec<Item>, String> {
    let key = arc.to_string_lossy().to_lowercase();
    let m = mtime(arc);
    if let Some((t, v)) = CACHE.lock().get(&key) {
        if *t == m {
            return Ok(v.clone());
        }
    }
    let v = read_items(arc)?;
    CACHE.lock().insert(key, (m, v.clone()));
    Ok(v)
}

fn entry(arc: &Path, inner: &str, dir: bool, size: u64, modified: i64) -> Entry {
    let name = inner.rsplit('/').next().unwrap_or(inner).to_string();
    let ext = if dir { String::new() } else { Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default() };
    Entry {
        path: format!("{}\\{}", arc.to_string_lossy(), inner.replace('/', "\\")),
        name,
        dir,
        size,
        modified,
        created: modified,
        ext,
        hidden: false,
        system: false,
        readonly: true,
        link: false,
        cloud: false,
    }
}

/// Zawartość „folderu” wewnątrz archiwum. Foldery pośrednie (bez własnych wpisów) też się pojawiają.
pub fn list(arc: &Path, inner: &str) -> Result<Vec<Entry>, String> {
    let prefix = if inner.is_empty() { String::new() } else { format!("{inner}/") };
    let mut children: BTreeMap<String, Entry> = BTreeMap::new();
    for it in items(arc)? {
        let name = it.name.trim_end_matches('/');
        let Some(rest) = name.strip_prefix(&prefix) else { continue };
        if rest.is_empty() {
            continue;
        }
        let rel = format!("{prefix}{}", rest.split('/').next().unwrap());
        let direct = !rest.contains('/');
        let key = rel.to_lowercase();
        if direct {
            children.insert(key, entry(arc, &rel, it.dir, it.size, it.modified));
        } else if !children.contains_key(&key) {
            children.insert(key, entry(arc, &rel, true, 0, it.modified));
        }
    }
    Ok(children.into_values().collect())
}

pub fn stat(path: &str) -> Option<Entry> {
    let (arc, inner) = split(path)?;
    if inner.is_empty() {
        return None;
    }
    let all = items(&arc).ok()?;
    let low = inner.to_lowercase();
    for it in &all {
        if it.name.trim_end_matches('/').to_lowercase() == low {
            return Some(entry(&arc, &inner, it.dir, it.size, it.modified));
        }
    }
    let pre = format!("{low}/");
    all.iter().any(|it| it.name.to_lowercase().starts_with(&pre)).then(|| entry(&arc, &inner, true, 0, 0))
}

/// Bezpieczna ścieżka docelowa (bez „..” i liter dysku z wnętrza archiwum).
fn safe_join(dest: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = dest.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return None;
        }
        p.push(part);
    }
    Some(p)
}

/// Wypakowuje wskazane elementy (pliki albo całe foldery) z jednego archiwum do `dest`.
/// Zwraca ścieżki utworzonych elementów najwyższego poziomu.
fn extract_from(arc: &Path, inners: &[String], dest: &Path) -> Result<Vec<String>, String> {
    // Element „a/b” wypakowujemy jako dest/b (z zawartością), jak przy zwykłym kopiowaniu.
    let wanted: Vec<(String, String)> = inners
        .iter()
        .map(|i| {
            let base = i.rsplit('/').next().unwrap_or(i).to_string();
            (i.to_lowercase(), base)
        })
        .collect();
    let target = |name: &str| -> Option<PathBuf> {
        let n = name.trim_end_matches('/');
        let low = n.to_lowercase();
        for (w, base) in &wanted {
            if w.is_empty() {
                return safe_join(dest, n);
            }
            if low == *w {
                return safe_join(dest, base);
            }
            if low.starts_with(&format!("{w}/")) {
                return safe_join(dest, &format!("{base}/{}", &n[w.len() + 1..]));
            }
        }
        None
    };
    let ext = arc.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "zip" {
        let f = std::fs::File::open(arc).map_err(|e| io_err(&e))?;
        let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|_| "Nieobsługiwane archiwum".to_string())?;
        for i in 0..z.len() {
            let mut f = z.by_index(i).map_err(|e| e.to_string())?;
            let Some(out) = target(&f.name().replace('\\', "/")) else { continue };
            if f.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| io_err(&e))?;
                continue;
            }
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).map_err(|e| io_err(&e))?;
            }
            let mut w = std::fs::File::create(&out).map_err(|e| io_err(&e))?;
            std::io::copy(&mut f, &mut w).map_err(|e| io_err(&e))?;
        }
    } else {
        let mut r = sevenz_rust::SevenZReader::open(arc, sevenz_rust::Password::empty()).map_err(|e| e.to_string())?;
        let mut err: Option<String> = None;
        r.for_each_entries(|en, rd| {
            let Some(out) = target(&en.name.replace('\\', "/")) else {
                // Trzeba przeczytać strumień, żeby przejść do następnego wpisu w bloku.
                std::io::copy(rd, &mut std::io::sink()).ok();
                return Ok(true);
            };
            if en.is_directory {
                let _ = std::fs::create_dir_all(&out);
                return Ok(true);
            }
            if let Some(p) = out.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            match std::fs::File::create(&out) {
                Ok(mut w) => {
                    let mut buf = [0u8; 1 << 16];
                    loop {
                        let n = rd.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        w.write_all(&buf[..n])?;
                    }
                }
                Err(e) => {
                    err = Some(io_err(&e));
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .map_err(|e| e.to_string())?;
        if let Some(e) = err {
            return Err(e);
        }
    }
    Ok(wanted.iter().map(|(_, b)| dest.join(b).to_string_lossy().to_string()).collect())
}

/// Wypakowuje elementy (mogą pochodzić z różnych archiwów) do folderu `dest`.
pub fn extract(sources: &[String], dest: &str) -> Result<Vec<String>, String> {
    let mut by_arc: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for s in sources {
        let (arc, inner) = split(s).ok_or_else(|| format!("To nie jest element archiwum: {s}"))?;
        by_arc.entry(arc).or_default().push(inner);
    }
    let mut out = Vec::new();
    for (arc, inners) in by_arc {
        out.extend(extract_from(&arc, &inners, Path::new(dest))?);
    }
    Ok(out)
}

/// Wypakowuje cały plik archiwum obok niego, do folderu o tej samej nazwie.
pub fn extract_here(arc: &str) -> Result<String, String> {
    let a = Path::new(arc);
    let stem = a.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "archiwum".into());
    let dest = crate::ops::unique_path(&a.with_file_name(stem));
    std::fs::create_dir_all(&dest).map_err(|e| io_err(&e))?;
    extract_from(a, &[String::new()], &dest)?;
    Ok(dest.to_string_lossy().to_string())
}

/// Wypakowuje pojedynczy plik do folderu tymczasowego (do otwarcia w programie albo podglądu).
pub fn extract_temp(path: &str) -> Result<String, String> {
    let (arc, inner) = split(path).ok_or("To nie jest element archiwum")?;
    let tag = format!("{:x}", { let mut h: u64 = 1469598103934665603; for b in arc.to_string_lossy().bytes() { h = (h ^ b as u64).wrapping_mul(1099511628211); } h });
    let dest = std::env::temp_dir().join("OpenFilePower").join(tag);
    let _ = std::fs::create_dir_all(&dest);
    let out = extract_from(&arc, &[inner], &dest)?;
    out.into_iter().next().ok_or_else(|| "Nie znaleziono pliku w archiwum".into())
}


/// Pakuje pliki i foldery do nowego archiwum ZIP (nazwa unikalna, obok źródeł).
pub fn create_zip(sources: &[String], dest: &str) -> Result<String, String> {
    let out = crate::ops::unique_path(Path::new(dest));
    let f = std::fs::File::create(&out).map_err(|e| io_err(&e))?;
    let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    fn add(z: &mut zip::ZipWriter<std::io::BufWriter<std::fs::File>>, p: &Path, rel: &str, opts: zip::write::SimpleFileOptions) -> Result<(), String> {
        if p.is_dir() {
            z.add_directory(format!("{rel}/"), opts).map_err(|e| e.to_string())?;
            for de in std::fs::read_dir(p).map_err(|e| io_err(&e))?.flatten() {
                let name = de.file_name().to_string_lossy().to_string();
                add(z, &de.path(), &format!("{rel}/{name}"), opts)?;
            }
        } else {
            z.start_file(rel, opts).map_err(|e| e.to_string())?;
            let mut r = std::fs::File::open(p).map_err(|e| io_err(&e))?;
            std::io::copy(&mut r, z).map_err(|e| io_err(&e))?;
        }
        Ok(())
    }
    for s in sources {
        let p = Path::new(s);
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Err(e) = add(&mut z, p, &name, opts) {
            drop(z);
            let _ = std::fs::remove_file(&out);
            return Err(e);
        }
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(out.to_string_lossy().to_string())
}
