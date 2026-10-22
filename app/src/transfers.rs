//! Transfery w tle i to, co je otacza (Overlay.svelte, Dialogs.svelte): pasek transferów z postępem,
//! wstrzymaniem i anulowaniem, powiadomienia z akcją „Cofnij”, okno konfliktu nazw i potwierdzenia.
//! Wszystko w oknie aplikacji – żadnych systemowych okienek kopiowania.

use std::path::Path;
use std::time::Duration;

use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::ops::{self, Conflict, Progress};
use powerdesk_core::drives;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::menus::Act;
use crate::{fmt, ConflictRow, OpItem, Shared, ToastItem};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Info,
    Ok,
    Error,
}

pub struct Toast {
    id: i32,
    text: String,
    kind: Kind,
    action: Option<(String, Act)>,
}

/// Operacja czekająca na decyzję w oknie konfliktu.
pub struct Pending {
    kind: String,
    sources: Vec<String>,
    dest: String,
}

#[derive(Default)]
pub struct Transfers {
    ops: Vec<Progress>,
    toasts: Vec<Toast>,
    seq: i32,
    hidden: bool,
    pending: Option<Pending>,
    confirm: Option<Act>,
    prompt: Option<std::rc::Rc<dyn Fn(&Shared, &str)>>,
}

fn base(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string())
}

fn same(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

// ---------- Powiadomienia ----------

/// Powiadomienie w prawym dolnym rogu; znika samo (błędy i te z akcją – później).
pub fn toast(s: &Shared, text: impl Into<String>, kind: Kind, action: Option<(&str, Act)>) {
    let id = {
        let mut b = s.borrow_mut();
        let t = &mut b.transfers;
        t.seq += 1;
        let id = t.seq;
        t.toasts.push(Toast { id, text: text.into(), kind, action: action.map(|(l, a)| (l.to_string(), a)) });
        // Najwyżej cztery naraz – najstarsze odpadają.
        while t.toasts.len() > 4 {
            t.toasts.remove(0);
        }
        id
    };
    render(s);
    let long = kind == Kind::Error || s.borrow().transfers.toasts.iter().any(|t| t.id == id && t.action.is_some());
    let s2 = s.clone();
    slint::Timer::single_shot(Duration::from_secs(if long { 8 } else { 4 }), move || close_toast(&s2, id));
}

pub fn close_toast(s: &Shared, id: i32) {
    s.borrow_mut().transfers.toasts.retain(|t| t.id != id);
    render(s);
}

pub fn toast_action(s: &Shared, id: i32) {
    let act = s.borrow().transfers.toasts.iter().find(|t| t.id == id).and_then(|t| t.action.as_ref().map(|a| a.1.clone()));
    close_toast(s, id);
    if let Some(act) = act {
        act(s);
    }
}

// ---------- Cofanie ----------

pub fn undo(s: &Shared) {
    match ops::undo_last() {
        Ok(m) => toast(s, m, Kind::Ok, Some(("Ponów", std::rc::Rc::new(redo)))),
        Err(e) => toast(s, e, Kind::Info, None),
    }
}

pub fn redo(s: &Shared) {
    match ops::redo_last() {
        Ok(m) => toast(s, m, Kind::Ok, Some(("Cofnij", std::rc::Rc::new(undo)))),
        Err(e) => toast(s, e, Kind::Info, None),
    }
}

// ---------- Operacje ----------

/// Kopiowanie albo przenoszenie (transfer w state.svelte.ts): sprawdzenia, potem konflikty, potem praca w tle.
pub fn transfer(s: &Shared, sources: Vec<String>, dest: &str, kind: &str) {
    if dest.starts_with("ofp:") {
        return;
    }
    // Do archiwum nie da się nic dodać; z archiwum się wypakowuje (oryginał zostaje).
    if crate::archives::read_only_dest(s, dest) {
        return;
    }
    let (packed, sources): (Vec<String>, Vec<String>) = sources.into_iter().partition(|p| crate::archives::inside(p));
    if !packed.is_empty() {
        crate::archives::extract_to(s, packed, dest);
    }
    if sources.is_empty() {
        return;
    }
    // Przeniesienie do tego samego folderu nic nie zmienia.
    let sources: Vec<String> = sources
        .into_iter()
        .filter(|p| kind == "copy" || !Path::new(p).parent().is_some_and(|d| same(&d.to_string_lossy(), dest)))
        .collect();
    if sources.is_empty() {
        return;
    }
    let low = dest.to_lowercase();
    if sources.iter().any(|p| same(p, dest) || low.starts_with(&format!("{}\\", p.to_lowercase().trim_end_matches('\\')))) {
        toast(s, "Nie można przenieść folderu do jego własnego wnętrza", Kind::Error, None);
        return;
    }
    let conflicts: Vec<ops::ConflictInfo> = ops::check_conflicts(&sources, dest).into_iter().filter(|c| !same(&c.src, &c.dst)).collect();
    if conflicts.is_empty() {
        ops::start(Core, kind.into(), sources, dest.into(), Conflict::Rename);
        return;
    }
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let n = conflicts.len();
    ui.set_conflict_title(if n == 1 { "Element już istnieje".into() } else { format!("{n} elementów już istnieje").into() });
    let info = |dir: bool, size: u64, t: i64, other: i64, who: &str| {
        let newer = if t > other + 2000 { " · nowszy" } else { "" };
        format!("{who}: {} · {}{newer}", if dir { "folder".into() } else { fmt::size(size) }, fmt::date(t))
    };
    let rows: Vec<ConflictRow> = conflicts
        .iter()
        .take(6)
        .map(|c| ConflictRow {
            name: c.name.clone().into(),
            new_info: info(c.dir, c.src_size, c.src_modified, c.dst_modified, "Nowy").into(),
            old_info: info(c.dir, c.dst_size, c.dst_modified, c.src_modified, "Istniejący").into(),
        })
        .collect();
    ui.set_conflict_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_conflict_more(if n > 6 { format!("…i {} więcej", n - 6).into() } else { SharedString::new() });
    ui.set_conflict_dirs(conflicts.iter().any(|c| c.dir));
    ui.set_conflict_moving(kind == "move");
    ui.set_conflict_open(true);
    s.borrow_mut().transfers.pending = Some(Pending { kind: kind.into(), sources, dest: dest.into() });
}

pub fn conflict_pick(s: &Shared, i: i32) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_conflict_open(false);
    }
    let Some(p) = s.borrow_mut().transfers.pending.take() else { return };
    let policy = match i {
        0 => Conflict::Replace,
        1 => Conflict::Newer,
        2 => Conflict::Rename,
        3 => Conflict::Skip,
        _ => return,
    };
    ops::start(Core, p.kind, p.sources, p.dest, policy);
}

/// Pytanie w oknie aplikacji; `act` wykonuje się po potwierdzeniu.
pub fn confirm(s: &Shared, title: &str, text: &str, ok: &str, danger: bool, act: Act) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    ui.set_confirm_title(title.into());
    ui.set_confirm_text(text.into());
    ui.set_confirm_ok(ok.into());
    ui.set_confirm_danger(danger);
    ui.set_confirm_open(true);
    s.borrow_mut().transfers.confirm = Some(act);
}

/// Pytanie o tekst (nazwa) w oknie aplikacji; `act` dostaje wpisany tekst po zatwierdzeniu.
pub fn prompt(s: &Shared, title: &str, value: &str, ok: &str, act: std::rc::Rc<dyn Fn(&Shared, &str)>) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let g = ui.global::<crate::Prompt>();
    g.set_title(title.into());
    g.set_value(value.into());
    g.set_ok(ok.into());
    g.set_open(true);
    s.borrow_mut().transfers.prompt = Some(act);
}

pub fn prompt_answer(s: &Shared, text: Option<&str>) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.global::<crate::Prompt>().set_open(false);
    }
    let act = s.borrow_mut().transfers.prompt.take();
    if let (Some(t), Some(act)) = (text.map(str::trim).filter(|t| !t.is_empty()), act) {
        act(s, t);
    }
}

pub fn confirm_answer(s: &Shared, yes: bool) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        ui.set_confirm_open(false);
    }
    let act = s.borrow_mut().transfers.confirm.take();
    if let (true, Some(act)) = (yes, act) {
        act(s);
    }
}

/// Trwałe usunięcie (Shift+Del) – zawsze po potwierdzeniu.
pub fn delete_permanently(s: &Shared, paths: Vec<String>) {
    if paths.is_empty() || crate::archives::read_only(s, &paths) {
        return;
    }
    let what = if paths.len() == 1 { format!("„{}”", base(&paths[0])) } else { format!("{} elementów", paths.len()) };
    confirm(
        s,
        "Usunąć trwale?",
        &format!("{what} zostanie usunięte bez przenoszenia do Kosza. Tej operacji nie można cofnąć."),
        "Usuń trwale",
        true,
        std::rc::Rc::new(move |_| {
            ops::start(Core, "delete".into(), paths.clone(), String::new(), Conflict::Skip);
        }),
    );
}

fn done_text(p: &Progress) -> String {
    match p.kind.as_str() {
        "copy" => format!("Skopiowano do „{}”", base(&p.dest)),
        "move" => format!("Przeniesiono do „{}”", base(&p.dest)),
        "trash" => format!("Przeniesiono do Kosza ({})", p.total_items),
        _ => format!("Usunięto ({})", p.total_items),
    }
}

/// Zdarzenie „op-progress” z backendu.
pub fn on_progress(s: &Shared, p: Progress) {
    let finished = p.state != "running";
    {
        let mut b = s.borrow_mut();
        let ops = &mut b.transfers.ops;
        match ops.iter_mut().find(|o| o.id == p.id) {
            Some(o) => *o = p.clone(),
            None => ops.push(p.clone()),
        }
    }
    if finished {
        match p.state.as_str() {
            "done" if p.kind == "copy" || p.kind == "move" => toast(s, done_text(&p), Kind::Ok, Some(("Cofnij", std::rc::Rc::new(undo)))),
            "done" => toast(s, done_text(&p), Kind::Ok, None),
            "error" => toast(s, format!("{}: {} błędów", p.title, p.errors.len()), Kind::Error, None),
            _ => toast(s, format!("{}: anulowano", p.title), Kind::Info, None),
        }
        // Gotowa karta znika po chwili, z błędami – zostaje dłużej, żeby dało się je przeczytać.
        let (s2, id) = (s.clone(), p.id);
        slint::Timer::single_shot(Duration::from_millis(if p.state == "error" { 12_000 } else { 1_500 }), move || dismiss(&s2, id as i32));
        // Wolne miejsce na dyskach się zmieniło.
        s.borrow_mut().drives = drives::list();
        crate::render_side(s);
    }
    render(s);
}

pub fn dismiss(s: &Shared, id: i32) {
    s.borrow_mut().transfers.ops.retain(|o| o.id as i32 != id || o.state == "running");
    render(s);
}

pub fn toggle_hidden(s: &Shared) {
    {
        let mut b = s.borrow_mut();
        b.transfers.hidden = !b.transfers.hidden;
    }
    render(s);
}

/// Pozostały czas jak `eta` w Overlay.svelte.
fn eta(p: &Progress) -> String {
    if p.speed <= 0.0 || p.total_bytes == 0 {
        return String::new();
    }
    let secs = p.total_bytes.saturating_sub(p.done_bytes) as f64 / p.speed;
    if secs < 60.0 {
        format!("{} s", secs.round().max(1.0))
    } else if secs < 3600.0 {
        format!("{} min", (secs / 60.0).round())
    } else {
        format!("{:.1} h", secs / 3600.0).replace('.', ",")
    }
}

fn pct(p: &Progress) -> f32 {
    if p.state == "done" {
        1.0
    } else if p.total_bytes > 0 {
        p.done_bytes as f32 / p.total_bytes as f32
    } else if p.total_items > 0 {
        p.done_items as f32 / p.total_items as f32
    } else {
        0.0
    }
}

pub fn render(s: &Shared) {
    let b = s.borrow();
    let Some(ui) = b.ui.upgrade() else { return };
    let t = &b.transfers;
    let items: Vec<OpItem> = if t.hidden {
        Vec::new()
    } else {
        t.ops
            .iter()
            .map(|p| OpItem {
                id: p.id as i32,
                icon: crate::icons::icon(match p.kind.as_str() {
                    "copy" => "copy",
                    "move" => "cut",
                    _ => "trash",
                })
                .into(),
                title: p.title.clone().into(),
                current: match p.state.as_str() {
                    "running" => p.current.clone(),
                    "done" => "Gotowe".into(),
                    "cancelled" => "Anulowano".into(),
                    _ => format!("Zakończono z błędami ({})", p.errors.len()),
                }
                .into(),
                meta: if p.state != "running" {
                    String::new()
                } else if p.paused {
                    "Wstrzymano".into()
                } else if p.total_bytes > 0 {
                    format!("{} z {} · {}/s · {}", fmt::size(p.done_bytes), fmt::size(p.total_bytes), fmt::size(p.speed as u64), eta(p))
                } else {
                    format!("{} / {}", p.done_items, p.total_items)
                }
                .into(),
                pct: pct(p),
                state: match p.state.as_str() {
                    "running" => 0,
                    "done" => 1,
                    "error" => 2,
                    _ => 3,
                },
                paused: p.paused,
                errors: p.errors.join("\n").into(),
                error_count: p.errors.len() as i32,
            })
            .collect()
    };
    ui.set_ops(ModelRc::new(VecModel::from(items)));
    let running: Vec<&Progress> = t.ops.iter().filter(|p| p.state == "running").collect();
    ui.set_ops_summary(match running.len() {
        0 => SharedString::new(),
        1 => format!("{} · {:.0}%", running[0].title, pct(running[0]) * 100.0).into(),
        n => {
            let (d, a) = running.iter().fold((0u64, 0u64), |(d, a), p| (d + p.done_bytes, a + p.total_bytes));
            let all = if a > 0 { d as f32 / a as f32 } else { running.iter().map(|p| pct(p)).sum::<f32>() / n as f32 };
            format!("{n} {} · {:.0}%", if n < 5 { "operacje" } else { "operacji" }, all * 100.0).into()
        }
    });
    ui.set_transfers_hidden(t.hidden);
    let toasts: Vec<ToastItem> = t
        .toasts
        .iter()
        .map(|x| ToastItem {
            id: x.id,
            text: x.text.clone().into(),
            kind: match x.kind {
                Kind::Info => 0,
                Kind::Ok => 1,
                Kind::Error => 2,
            },
            action: x.action.as_ref().map(|a| a.0.clone()).unwrap_or_default().into(),
        })
        .collect();
    ui.set_toasts(ModelRc::new(VecModel::from(toasts)));
}
