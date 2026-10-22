//! Masowa zmiana nazw (BulkRename.svelte): wzorzec z tokenami, numeracja, znajdź i zamień,
//! wielkość liter, rozszerzenie. Podgląd liczony przy każdej zmianie pola; całość cofa jedno Ctrl+Z.

use std::collections::HashMap;
use std::path::Path;

use chrono::{Local, TimeZone};
use powerdesk_core::fsx::Entry;
use powerdesk_core::ops;
use powerdesk_core::shell::MediaInfo;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::transfers::{self, Kind};
use crate::{Bulk, BulkRow, Shared};

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Case {
    #[default]
    Keep,
    Lower,
    Upper,
    Title,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Ext {
    #[default]
    Keep,
    Lower,
    Upper,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Order {
    #[default]
    List,
    Name,
    Taken,
    Modified,
}

pub struct Opts {
    pub pattern: String,
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub match_case: bool,
    pub start: u64,
    pub pad: usize,
    pub case: Case,
    pub ext: Ext,
    pub order: Order,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { pattern: "{nazwa}".into(), find: String::new(), replace: String::new(), regex: false, match_case: false, start: 1, pad: 3, case: Case::Keep, ext: Ext::Keep, order: Order::List }
    }
}

pub struct Row {
    pub path: String,
    pub old: String,
    pub new: String,
    pub problem: &'static str,
}

/// Pierwsza litera po początku, spacji, „_”, „-” albo „.” wielka, reszta małe.
fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut boundary = true;
    for c in s.to_lowercase().chars() {
        if boundary && c.is_alphabetic() {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        boundary = c.is_whitespace() || matches!(c, '_' | '-' | '.');
    }
    out
}

fn stem(e: &Entry) -> &str {
    if e.dir || e.ext.is_empty() {
        &e.name
    } else {
        &e.name[..e.name.len() - e.ext.len() - 1]
    }
}

fn folder(e: &Entry) -> String {
    let parent = Path::new(&e.path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    parent.split('\\').filter(|s| !s.is_empty()).last().unwrap_or("").replace(':', "")
}

/// Wyrażenie „Znajdź”; zwykły tekst jest cytowany. Błąd tylko dla niepełnego wyrażenia regularnego.
fn finder(o: &Opts) -> Result<Option<regex::Regex>, String> {
    if o.find.is_empty() {
        return Ok(None);
    }
    let src = if o.regex { o.find.clone() } else { regex::escape(&o.find) };
    regex::RegexBuilder::new(&src).case_insensitive(!o.match_case).build().map(Some).map_err(|_| "Błędne wyrażenie regularne".to_string())
}

/// Nowe nazwy w kolejności numeracji; drugi element to błąd wyrażenia (podgląd i tak działa – bez zamiany).
pub fn rows(entries: &[Entry], media: impl Fn(&Entry) -> Option<MediaInfo>, o: &Opts) -> (Vec<Row>, String) {
    let taken = |e: &Entry| media(e).map(|m| m.taken).filter(|t| *t > 0).unwrap_or(e.modified);
    let mut list: Vec<&Entry> = entries.iter().collect();
    match o.order {
        Order::List => {}
        Order::Name => list.sort_by(|a, b| crate::model::natural(&a.name, &b.name)),
        Order::Taken => list.sort_by_key(|e| taken(e)),
        Order::Modified => list.sort_by_key(|e| e.modified),
    }
    let (re, err) = match finder(o) {
        Ok(r) => (r, String::new()),
        Err(e) => (None, e),
    };
    let mut out: Vec<Row> = list
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let t = Local.timestamp_millis_opt(taken(e)).earliest().unwrap_or_else(Local::now);
            let dims = media(e).filter(|m| m.w > 0).map(|m| format!("{}x{}", m.w, m.h)).unwrap_or_default();
            let mut s = o
                .pattern
                .replace("{nazwa}", stem(e))
                .replace("{nr}", &format!("{:0w$}", o.start + i as u64, w = o.pad))
                .replace("{data}", &t.format("%Y-%m-%d").to_string())
                .replace("{czas}", &t.format("%H.%M.%S").to_string())
                .replace("{folder}", &folder(e))
                .replace("{rozmiar}", &dims);
            if let Some(re) = &re {
                s = if o.regex { re.replace_all(&s, o.replace.as_str()).into_owned() } else { re.replace_all(&s, regex::NoExpand(&o.replace)).into_owned() };
            }
            s = match o.case {
                Case::Keep => s,
                Case::Lower => s.to_lowercase(),
                Case::Upper => s.to_uppercase(),
                Case::Title => title_case(&s),
            };
            let s = s.trim();
            let new = if e.dir || e.ext.is_empty() {
                s.to_string()
            } else {
                let ext = match o.ext {
                    Ext::Keep => e.name[e.name.len() - e.ext.len()..].to_string(),
                    Ext::Lower => e.ext.to_lowercase(),
                    Ext::Upper => e.ext.to_uppercase(),
                };
                format!("{s}.{ext}")
            };
            Row { path: e.path.clone(), old: e.name.clone(), new, problem: "" }
        })
        .collect();
    let mut count: HashMap<String, usize> = HashMap::new();
    for r in &out {
        *count.entry(r.new.to_lowercase()).or_default() += 1;
    }
    for r in &mut out {
        r.problem = if r.new.is_empty() || r.new.starts_with('.') {
            "Pusta nazwa"
        } else if r.new.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            "Niedozwolony znak"
        } else if count[&r.new.to_lowercase()] > 1 {
            "Powtarza się"
        } else {
            ""
        };
    }
    (out, err)
}

// ---------- Okno ----------

/// Otwiera okno dla zaznaczonych elementów; daty wykonania z EXIF dochodzą w tle.
pub fn open(s: &Shared) {
    let entries: Vec<Entry> = crate::tab_mut(s, |t| t.items.iter().filter(|e| t.selected.contains(&e.path)).cloned().collect());
    if entries.len() < 2 {
        return crate::start_rename(s);
    }
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let g = ui.global::<Bulk>();
    let d = Opts::default();
    g.set_pattern(d.pattern.into());
    g.set_find("".into());
    g.set_replace("".into());
    g.set_regex(false);
    g.set_match_case(false);
    g.set_start(d.start.to_string().into());
    g.set_pad(d.pad.to_string().into());
    g.set_case_mode(0);
    g.set_ext_mode(0);
    g.set_order(0);
    g.set_subtitle(format!("{} elementów · podgląd aktualizuje się na bieżąco", entries.len()).into());
    let todo: Vec<(String, i64)> = {
        let b = s.borrow();
        entries.iter().filter(|e| crate::fmt::is_media(e) && !b.media.contains_key(&e.path)).map(|e| (e.path.clone(), e.modified)).collect()
    };
    s.borrow_mut().bulk = Some(entries);
    refresh(s);
    g.set_open(true);
    if !todo.is_empty() {
        std::thread::spawn(move || {
            let info = powerdesk_core::shell::media_info(todo.clone());
            let _ = slint::invoke_from_event_loop(move || {
                crate::with_state(|s| {
                    s.borrow_mut().media.extend(todo.into_iter().map(|(p, _)| p).zip(info));
                    if s.borrow().bulk.is_some() {
                        refresh(s);
                    }
                })
            });
        });
    }
}

fn opts(g: &Bulk) -> Opts {
    let num = |v: slint::SharedString, d: u64| v.trim().parse::<u64>().unwrap_or(d);
    Opts {
        pattern: g.get_pattern().into(),
        find: g.get_find().into(),
        replace: g.get_replace().into(),
        regex: g.get_regex(),
        match_case: g.get_match_case(),
        start: num(g.get_start(), 1),
        pad: num(g.get_pad(), 3).clamp(1, 8) as usize,
        case: [Case::Keep, Case::Lower, Case::Upper, Case::Title][g.get_case_mode().clamp(0, 3) as usize],
        ext: [Ext::Keep, Ext::Lower, Ext::Upper][g.get_ext_mode().clamp(0, 2) as usize],
        order: [Order::List, Order::Name, Order::Taken, Order::Modified][g.get_order().clamp(0, 3) as usize],
    }
}

fn compute(s: &Shared) -> Option<Vec<Row>> {
    let ui = s.borrow().ui.upgrade()?;
    let b = s.borrow();
    let entries = b.bulk.as_ref()?;
    let (rows, err) = rows(entries, |e| b.media.get(&e.path).copied(), &opts(&ui.global::<Bulk>()));
    ui.global::<Bulk>().set_error(err.into());
    Some(rows)
}

/// Przelicza podgląd po zmianie dowolnego pola.
pub fn refresh(s: &Shared) {
    let Some(rows) = compute(s) else { return };
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let g = ui.global::<Bulk>();
    let changed = rows.iter().filter(|r| r.new != r.old).count();
    let problems = rows.iter().filter(|r| !r.problem.is_empty()).count();
    g.set_summary(if problems > 0 { format!("{changed} zmian · {problems} problemów") } else { format!("{changed} zmian") }.into());
    g.set_ok_label(if changed > 0 { format!("Zmień {changed}") } else { "Zmień".into() }.into());
    g.set_can_apply(changed > 0 && problems == 0);
    let model: Vec<BulkRow> = rows
        .into_iter()
        .map(|r| BulkRow { same: r.new == r.old, old: r.old.into(), new: r.new.into(), problem: r.problem.into() })
        .collect();
    g.set_rows(ModelRc::new(VecModel::from(model)));
}

pub fn close(s: &Shared) {
    s.borrow_mut().bulk = None;
    if let Some(ui) = s.borrow().ui.upgrade() {
        let g = ui.global::<Bulk>();
        g.set_open(false);
        g.set_rows(ModelRc::default());
    }
}

pub fn apply(s: &Shared) {
    let Some(rows) = compute(s) else { return };
    if rows.iter().any(|r| !r.problem.is_empty()) {
        return;
    }
    let pairs: Vec<(String, String)> = rows
        .iter()
        .filter(|r| r.new != r.old)
        .map(|r| (r.path.clone(), Path::new(&r.path).with_file_name(&r.new).to_string_lossy().to_string()))
        .collect();
    if pairs.is_empty() {
        return;
    }
    match ops::rename_batch(&pairs) {
        Ok(done) => {
            let n = done.len();
            ops::push_undo(ops::Undo::RenamedMany(done));
            close(s);
            crate::load(s, true);
            let undo: crate::menus::Act = std::rc::Rc::new(|s: &Shared| {
                transfers::undo(s);
                crate::load(s, true);
            });
            transfers::toast(s, format!("Zmieniono nazwy: {n}"), Kind::Ok, Some(("Cofnij", undo)));
        }
        Err(errors) => {
            crate::load(s, true);
            transfers::toast(s, errors.into_iter().take(3).collect::<Vec<_>>().join("\n"), Kind::Error, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, modified: i64) -> Entry {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
        Entry { name: name.into(), path: format!("C:\\Zdjęcia\\Wakacje\\{name}"), dir: false, size: 0, modified, created: 0, ext, hidden: false, system: false, readonly: false, link: false, cloud: false }
    }

    fn names(e: &[Entry], o: &Opts) -> Vec<String> {
        rows(e, |_| None, o).0.into_iter().map(|r| r.new).collect()
    }

    #[test]
    fn numeracja_i_folder() {
        let e = [file("b.JPG", 2), file("a.jpg", 1)];
        let o = Opts { pattern: "{folder} {nr}".into(), order: Order::Name, ext: Ext::Lower, ..Default::default() };
        assert_eq!(names(&e, &o), ["Wakacje 001.jpg", "Wakacje 002.jpg"]);
    }

    #[test]
    fn znajdz_i_zamien() {
        let e = [file("DSC0001.jpg", 0), file("dsc0002.jpg", 0)];
        let o = Opts { find: "dsc".into(), replace: "Foto_".into(), ..Default::default() };
        assert_eq!(names(&e, &o), ["Foto_0001.jpg", "Foto_0002.jpg"]);
        let o = Opts { find: r"(\d+)".into(), replace: "nr$1".into(), regex: true, match_case: true, ..Default::default() };
        assert_eq!(names(&e, &o), ["DSCnr0001.jpg", "dscnr0002.jpg"]);
        let o = Opts { find: "(".into(), regex: true, ..Default::default() };
        assert!(!rows(&e, |_| None, &o).1.is_empty());
    }

    #[test]
    fn wielkosc_liter_i_problemy() {
        let e = [file("ŻÓŁTA łódź.txt", 0), file("x.txt", 0)];
        let o = Opts { case: Case::Title, ..Default::default() };
        assert_eq!(names(&e, &o)[0], "Żółta Łódź.txt");
        let o = Opts { pattern: "same".into(), ..Default::default() };
        assert!(rows(&e, |_| None, &o).0.iter().all(|r| r.problem == "Powtarza się"));
        let o = Opts { pattern: "a:b".into(), ..Default::default() };
        assert_eq!(rows(&e, |_| None, &o).0[0].problem, "Niedozwolony znak");
    }
}
