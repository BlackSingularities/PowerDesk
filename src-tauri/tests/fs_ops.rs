//! Archiwa jak foldery i zmiana nazw wielu plików naraz.

use std::io::Write;

use ofp_lib::archive::{extract, list, split};
use ofp_lib::ops::rename_batch;


#[test]
fn zip_jak_folder() {
    let dir = std::env::temp_dir().join(format!("ofp-arch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let zp = dir.join("t.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("a/b/plik.txt", o).unwrap();
        z.write_all(b"czesc").unwrap();
        z.start_file("gora.txt", o).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
    }
    let root = list(&zp, "").unwrap();
    let names: Vec<_> = root.iter().map(|e| (e.name.clone(), e.dir)).collect();
    assert!(names.contains(&("a".into(), true)));
    assert!(names.contains(&("gora.txt".into(), false)));
    let inner = format!("{}\\a\\b", zp.display());
    let (arc, rel) = split(&inner).unwrap();
    assert_eq!(rel, "a/b");
    assert_eq!(list(&arc, &rel).unwrap()[0].name, "plik.txt");
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    extract(&[format!("{}\\a", zp.display())], &out.to_string_lossy()).unwrap();
    assert_eq!(std::fs::read_to_string(out.join("a").join("b").join("plik.txt")).unwrap(), "czesc");
    let _ = std::fs::remove_dir_all(&dir);
}



#[test]
fn zamiana_nazw() {
    let dir = std::env::temp_dir().join(format!("ofp-ren-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "A").unwrap();
    std::fs::write(&b, "B").unwrap();
    let s = |p: &std::path::Path| p.to_string_lossy().to_string();
    let done = rename_batch(&[(s(&a), s(&b)), (s(&b), s(&a))]).unwrap();
    assert_eq!(done.len(), 2);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "B");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "A");
    // Konflikt z plikiem spoza zmienianych – nic się nie dzieje.
    std::fs::write(dir.join("c.txt"), "C").unwrap();
    assert!(rename_batch(&[(s(&a), s(&dir.join("c.txt")))]).is_err());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "B");
    let _ = std::fs::remove_dir_all(&dir);
}
