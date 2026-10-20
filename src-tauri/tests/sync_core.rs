use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

#[path = "../src/sync_core.rs"]
#[allow(dead_code)]
mod sync_core;
use sync_core::{execute, plan, scan, Action, ExecCtx, Excludes, FileState, Item, Mode};

fn f(size: u64, mtime: i64) -> Item {
    Item { dir: false, size, mtime }
}
fn d() -> Item {
    Item { dir: true, size: 0, mtime: 0 }
}
fn map(items: &[(&str, Item)]) -> BTreeMap<String, Item> {
    items.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn base(items: &[(&str, u64, i64)]) -> BTreeMap<String, FileState> {
    items.iter().map(|(k, s, t)| (k.to_string(), FileState { dir: false, size: *s, ms: *t, mt: *t })).collect()
}

#[test]
fn mirror_copies_new_and_deletes_extra() {
    let src = map(&[("a.txt", f(1, 1000)), ("dir", d()), ("dir\\b.txt", f(2, 1000))]);
    let dst = map(&[("old.txt", f(1, 1000))]);
    let p = plan(Mode::Mirror, &src, &dst, &BTreeMap::new());
    assert!(p.actions.contains(&Action::MkdirTarget("dir".into())));
    assert!(p.actions.contains(&Action::CopyToTarget("a.txt".into(), 1)));
    assert!(p.actions.contains(&Action::CopyToTarget("dir\\b.txt".into(), 2)));
    assert!(p.actions.contains(&Action::DeleteTarget("old.txt".into())));
    // Foldery przed plikami, usunięcia na końcu.
    assert!(matches!(p.actions.first(), Some(Action::MkdirTarget(_))));
    assert!(matches!(p.actions.last(), Some(Action::DeleteTarget(_))));
}

#[test]
fn backup_never_deletes() {
    let src = map(&[("a.txt", f(1, 1000))]);
    let dst = map(&[("a.txt", f(1, 1000)), ("keep.txt", f(5, 1000))]);
    let p = plan(Mode::Backup, &src, &dst, &BTreeMap::new());
    assert!(p.actions.is_empty());
}

#[test]
fn fat_time_tolerance() {
    let src = map(&[("a.txt", f(10, 1_000_000))]);
    let dst = map(&[("a.txt", f(10, 1_001_999))]);
    assert!(plan(Mode::Mirror, &src, &dst, &BTreeMap::new()).actions.is_empty());
}

#[test]
fn twoway_propagates_deletions_using_base() {
    let b = base(&[("gone_on_target.txt", 1, 1000), ("gone_on_source.txt", 1, 1000)]);
    let src = map(&[("gone_on_target.txt", f(1, 1000)), ("new_src.txt", f(3, 5000))]);
    let dst = map(&[("gone_on_source.txt", f(1, 1000)), ("new_dst.txt", f(4, 5000))]);
    let p = plan(Mode::TwoWay, &src, &dst, &b);
    assert!(p.actions.contains(&Action::DeleteSource("gone_on_target.txt".into())));
    assert!(p.actions.contains(&Action::DeleteTarget("gone_on_source.txt".into())));
    assert!(p.actions.contains(&Action::CopyToTarget("new_src.txt".into(), 3)));
    assert!(p.actions.contains(&Action::CopyToSource("new_dst.txt".into(), 4)));
}

#[test]
fn twoway_detects_direction_and_conflicts() {
    let b = base(&[("s.txt", 1, 1000), ("t.txt", 1, 1000), ("both.txt", 1, 1000)]);
    let src = map(&[("s.txt", f(2, 9000)), ("t.txt", f(1, 1000)), ("both.txt", f(3, 9000))]);
    let dst = map(&[("s.txt", f(1, 1000)), ("t.txt", f(5, 9000)), ("both.txt", f(4, 8000))]);
    let p = plan(Mode::TwoWay, &src, &dst, &b);
    assert!(p.actions.contains(&Action::CopyToTarget("s.txt".into(), 2)));
    assert!(p.actions.contains(&Action::CopyToSource("t.txt".into(), 5)));
    assert!(p.actions.contains(&Action::Conflict { rel: "both.txt".into(), source_wins: true, size: 3 }));
}

#[test]
fn twoway_modified_after_remote_delete_is_kept() {
    let b = base(&[("doc.txt", 1, 1000)]);
    let src = map(&[("doc.txt", f(7, 50_000))]);
    let p = plan(Mode::TwoWay, &src, &BTreeMap::new(), &b);
    assert_eq!(p.actions, vec![Action::CopyToTarget("doc.txt".into(), 7)]);
}

#[test]
fn excludes_match_names_and_paths() {
    let ex = Excludes::new(&["node_modules".into(), "Cache\\*".into()]);
    assert!(ex.excluded("a\\node_modules", "node_modules"));
    assert!(ex.excluded("x\\Thumbs.db", "Thumbs.db"));
    assert!(ex.excluded("~$raport.docx", "~$raport.docx"));
    assert!(ex.excluded("cache\\data.bin", "data.bin"));
    assert!(!ex.excluded("photos\\a.jpg", "a.jpg"));
}

fn tmpdir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("ofp-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

#[test]
fn end_to_end_mirror_with_versioning() {
    let root = tmpdir("e2e");
    let (s, t) = (root.join("src"), root.join("dst"));
    write(&s.join("a.txt"), "nowy");
    write(&s.join("sub\\b.txt"), "b");
    write(&t.join("a.txt"), "stary-dluzszy");
    write(&t.join("extra.txt"), "x");
    // Stary plik w celu ma inną datę.
    let old = SystemTime::now() - Duration::from_secs(3600);
    std::fs::OpenOptions::new().write(true).open(t.join("a.txt")).unwrap().set_modified(old).unwrap();

    let no = AtomicBool::new(false);
    let ex = Excludes::new(&[]);
    let sm = scan(&s, &ex, false, &no).unwrap();
    let tm = scan(&t, &ex, false, &no).unwrap();
    let p = plan(Mode::Mirror, &sm, &tm, &BTreeMap::new());
    let ctx = ExecCtx { source: s.clone(), target: t.clone(), versioning: true, stamp: "2026-01-01 000000".into() };
    for a in &p.actions {
        execute(&ctx, a).unwrap();
    }
    assert_eq!(std::fs::read_to_string(t.join("a.txt")).unwrap(), "nowy");
    assert_eq!(std::fs::read_to_string(t.join("sub\\b.txt")).unwrap(), "b");
    assert!(!t.join("extra.txt").exists());
    // Nadpisana i usunięta wersja trafiły do .ofp-versions.
    let v = t.join(".ofp-versions").join("2026-01-01 000000");
    assert_eq!(std::fs::read_to_string(v.join("a.txt")).unwrap(), "stary-dluzszy");
    assert!(v.join("extra.txt").exists());

    // Drugi przebieg: wszystko aktualne (wersje są pomijane przy skanowaniu).
    let sm = scan(&s, &ex, false, &no).unwrap();
    let tm = scan(&t, &ex, false, &no).unwrap();
    assert!(plan(Mode::Mirror, &sm, &tm, &BTreeMap::new()).actions.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn scan_fails_for_missing_root() {
    let no = AtomicBool::new(false);
    assert!(scan(Path::new("Q:\\nie\\istnieje\\nigdzie"), &Excludes::new(&[]), false, &no).is_err());
}
