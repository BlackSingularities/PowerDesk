//! Porównanie dwóch folderów z menu (dwa zaznaczone foldery): różnice w nowej karcie jak wyniki
//! wyszukiwania, podsumowanie w powiadomieniu z „Wyrównaj” – brakujące i nowsze pliki kopiowane
//! w obie strony (bez usuwania czegokolwiek). Porównanie – core::compare.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use powerdesk_core::compare::{self, Diff};
use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::ops::{self, Conflict};

use crate::model::Search;
use crate::transfers::{self, Kind};
use crate::Shared;

fn name(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string())
}

pub fn run(s: &Shared, a: String, b: String) {
    transfers::toast(s, powerdesk_core::trf!("Porównuję „{}” i „{}”…", name(&a), name(&b)), Kind::Info, None);
    std::thread::spawn(move || {
        let d = compare::compare(&a, &b);
        let _ = slint::invoke_from_event_loop(move || crate::with_state(|s| show(s, a, b, d)));
    });
}

fn show(s: &Shared, a: String, b: String, d: Diff) {
    let (na, nb) = (name(&a), name(&b));
    let diffs = d.only_a.len() + d.only_b.len() + d.newer_a.len() + d.newer_b.len();
    if diffs == 0 {
        transfers::toast(s, powerdesk_core::trf!("„{na}” i „{nb}” są zgodne ({} plików)", d.same), Kind::Ok, None);
        return;
    }
    let summary = powerdesk_core::trf!(
        "„{na}” ↔ „{nb}”: tylko w {na} – {}, tylko w {nb} – {}, nowsze w {na} – {}, nowsze w {nb} – {}, zgodne – {}",
        d.only_a.len(),
        d.only_b.len(),
        d.newer_a.len(),
        d.newer_b.len(),
        d.same
    );
    let to_b: Vec<String> = d.only_a.iter().chain(&d.newer_a).map(|e| e.path.clone()).collect();
    let to_a: Vec<String> = d.only_b.iter().chain(&d.newer_b).map(|e| e.path.clone()).collect();
    let results: Vec<_> = d.only_a.into_iter().chain(d.newer_a).chain(d.only_b).chain(d.newer_b).collect();
    crate::new_tab_at(s, &a, true);
    crate::tab_mut(s, |t| {
        t.selected.clear();
        t.focus = None;
        t.search = Some(Search { query: powerdesk_core::trf!("Różnice: {na} ↔ {nb}"), id: 0, running: false, scanned: 0, results });
    });
    crate::refresh(s);
    let (a2, b2) = (a.clone(), b.clone());
    transfers::toast(
        s,
        summary,
        Kind::Info,
        Some((crate::tr("Wyrównaj"), Rc::new(move |_: &Shared| {
            copy_all(&to_b, &a2, &b2);
            copy_all(&to_a, &b2, &a2);
        }))),
    );
}

/// Kopiuje pliki z `from` do `to` z zachowaniem podfolderów; istniejące – tylko gdy źródło nowsze.
fn copy_all(files: &[String], from: &str, to: &str) {
    let mut by_dir: HashMap<String, Vec<String>> = HashMap::new();
    for f in files {
        by_dir.entry(compare::target_dir(f, from, to)).or_default().push(f.clone());
    }
    for (dir, srcs) in by_dir {
        let _ = std::fs::create_dir_all(&dir);
        ops::start(Core, "copy".into(), srcs, dir, Conflict::Newer);
    }
}
