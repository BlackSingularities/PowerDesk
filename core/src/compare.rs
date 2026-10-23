//! Porównanie dwóch folderów (z podfolderami): pliki tylko po jednej stronie, nowsze po jednej
//! albo drugiej stronie i zgodne. Pliki porównujemy po rozmiarze i dacie zmiany (bez czytania
//! treści – szybko także na dyskach sieciowych); różnica dat do 2 s to ten sam plik (FAT, NAS).

use std::collections::HashMap;
use std::path::Path;

use crate::fsx::{self, Entry};

/// Różnice: pliki (pełne wpisy) po każdej stronie i liczba zgodnych.
#[derive(Default)]
pub struct Diff {
    pub only_a: Vec<Entry>,
    pub only_b: Vec<Entry>,
    pub newer_a: Vec<Entry>,
    pub newer_b: Vec<Entry>,
    pub same: usize,
}

const LIMIT: usize = 100_000;

/// Pliki folderu: ścieżka względna (małe litery) → wpis. Bez ukrytych systemowych.
fn files(root: &str) -> HashMap<String, Entry> {
    let mut out = HashMap::new();
    let base = Path::new(root);
    let mut stack = vec![base.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            let path = e.path();
            if md.is_dir() {
                stack.push(path);
                continue;
            }
            let entry = fsx::entry_from(&path, &md);
            if entry.system && entry.hidden {
                continue;
            }
            let rel = path.strip_prefix(base).map(|r| r.to_string_lossy().to_lowercase()).unwrap_or_default();
            out.insert(rel, entry);
            if out.len() >= LIMIT {
                return out;
            }
        }
    }
    out
}

pub fn compare(a: &str, b: &str) -> Diff {
    let (fa, fb) = (files(a), files(b));
    let mut d = Diff::default();
    for (rel, ea) in &fa {
        match fb.get(rel) {
            None => d.only_a.push(ea.clone()),
            Some(eb) => {
                let close = (ea.modified - eb.modified).abs() <= 2000;
                if ea.size == eb.size && close {
                    d.same += 1;
                } else if close {
                    // Ten sam czas, inny rozmiar – pokazujemy po stronie większego.
                    if ea.size >= eb.size {
                        d.newer_a.push(ea.clone());
                    } else {
                        d.newer_b.push(eb.clone());
                    }
                } else if ea.modified > eb.modified {
                    d.newer_a.push(ea.clone());
                } else {
                    d.newer_b.push(eb.clone());
                }
            }
        }
    }
    for (rel, eb) in &fb {
        if !fa.contains_key(rel) {
            d.only_b.push(eb.clone());
        }
    }
    for v in [&mut d.only_a, &mut d.only_b, &mut d.newer_a, &mut d.newer_b] {
        v.sort_by(|x, y| x.path.to_lowercase().cmp(&y.path.to_lowercase()));
    }
    d
}

/// Folder docelowy pliku `file` z `from` przeniesionego do `to` (ta sama ścieżka względna).
pub fn target_dir(file: &str, from: &str, to: &str) -> String {
    let rel = Path::new(file).strip_prefix(from).ok().and_then(|r| r.parent()).map(|p| p.to_path_buf()).unwrap_or_default();
    Path::new(to).join(rel).to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_differences() {
        let root = std::env::temp_dir().join(format!("pd-compare-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("tylko-a.txt"), "1").unwrap();
        std::fs::write(b.join("tylko-b.txt"), "1").unwrap();
        std::fs::write(a.join("sub").join("x.txt"), "1").unwrap();
        std::fs::write(a.join("wspolny.txt"), "abc").unwrap();
        std::fs::write(b.join("wspolny.txt"), "abc").unwrap();
        let d = compare(&a.to_string_lossy(), &b.to_string_lossy());
        assert_eq!(d.only_a.len(), 2);
        assert_eq!(d.only_b.len(), 1);
        assert_eq!(d.same, 1);
        let t = target_dir(&a.join("sub").join("x.txt").to_string_lossy(), &a.to_string_lossy(), &b.to_string_lossy());
        assert!(t.ends_with("sub"));
        let _ = std::fs::remove_dir_all(root);
    }
}
