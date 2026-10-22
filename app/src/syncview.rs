//! Ekran synchronizacji (SyncView.svelte) na karcie `ofp:sync`: profile ze stanem, postępem
//! i historią, edytor profilu, podgląd planu, ochrona przed masowym usunięciem.
//! Silnik (`powerdesk_core::sync`) startuje dopiero po pierwszym wejściu na ekran – i tylko,
//! gdy nie działa stary OpenFilePower, który korzysta z tych samych profili.

use std::cell::Cell;

use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::sync::{self, Counts, Profile, Status};
use powerdesk_core::sync_core::Mode;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::fmt;
use crate::{Shared, SyncCard, SyncHist, SyncState};

pub const TAB: &str = "ofp:sync";

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
    static LOADED: Cell<bool> = const { Cell::new(false) };
}

fn with_ui(s: &Shared, f: impl FnOnce(&SyncState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<SyncState>());
    }
}

/// Czy działa stara wersja (Tauri) – ona też synchronizuje profile z sync.json.
fn old_app_running() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq openfilepower-win.exe", "/NH"])
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("openfilepower-win.exe"))
        .unwrap_or(false)
}

/// Uruchamia silnik, jeśli jeszcze nie działa i nikt inny nie synchronizuje.
fn ensure_started(s: &Shared) {
    if STARTED.with(|x| x.get()) {
        return;
    }
    let blocked = old_app_running();
    with_ui(s, |g| g.set_blocked(blocked));
    if blocked {
        if !LOADED.with(|x| x.replace(true)) {
            sync::load();
        }
        return;
    }
    STARTED.with(|x| x.set(true));
    sync::init(Core);
}

fn blocked(s: &Shared) -> bool {
    let mut b = false;
    with_ui(s, |g| b = g.get_blocked());
    b
}

pub fn open(s: &Shared) {
    let at = s.borrow().tabs.iter().position(|t| t.path == TAB);
    match at {
        Some(i) => {
            crate::save_scroll(s);
            {
                let mut b = s.borrow_mut();
                b.active = i;
                let f = b.focused;
                b.panes[f] = i;
            }
            crate::show_tab(s);
        }
        None => crate::new_tab_at(s, TAB, true),
    }
}

pub fn sync_tab(s: &Shared) {
    let on = crate::tab_mut(s, |t| t.path == TAB);
    let was = s.borrow().ui.upgrade().is_some_and(|u| u.global::<SyncState>().get_open());
    with_ui(s, |g| g.set_open(on));
    if on && !was {
        ensure_started(s);
        render(s);
    }
}

/// Zdarzenie „sync-status” z silnika – stan jednego profilu się zmienił.
pub fn on_status(s: &Shared) {
    let open = s.borrow().ui.upgrade().is_some_and(|u| u.global::<SyncState>().get_open());
    if open {
        render(s);
    }
}

fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Mirror => "kopia lustrzana",
        Mode::TwoWay => "dwukierunkowa",
        Mode::Backup => "kopia zapasowa",
    }
}

fn ago(ms: i64) -> String {
    if ms <= 0 {
        return "nigdy".into();
    }
    let mins = (sync::now_ms() - ms).max(0) / 60_000;
    match mins {
        0 => "przed chwilą".into(),
        1..=59 => format!("{mins} min temu"),
        60..=1439 => format!("{} godz. temu", mins / 60),
        _ => format!("{} dni temu", mins / 1440),
    }
}

fn counts_text(c: &Counts) -> String {
    let mut parts = Vec::new();
    if c.to_target > 0 {
        parts.push(format!("{} do celu", c.to_target));
    }
    if c.to_source > 0 {
        parts.push(format!("{} do źródła", c.to_source));
    }
    if c.del_target > 0 {
        parts.push(format!("{} usunięć w celu", c.del_target));
    }
    if c.del_source > 0 {
        parts.push(format!("{} usunięć w źródle", c.del_source));
    }
    if c.conflicts > 0 {
        parts.push(format!("{} konfliktów", c.conflicts));
    }
    if c.errors > 0 {
        parts.push(format!("{} błędów", c.errors));
    }
    if parts.is_empty() {
        return "bez zmian".into();
    }
    if c.bytes > 0 {
        parts.push(fmt::size(c.bytes));
    }
    parts.join(" · ")
}

fn card(p: &Profile, st: &Status) -> SyncCard {
    let busy = matches!(st.state.as_str(), "scanning" | "syncing");
    let (tone, state) = if !p.enabled {
        (0, "Wstrzymana".to_string())
    } else {
        match st.state.as_str() {
            "scanning" => (2, "Porównuję foldery…".into()),
            "syncing" => (2, "Synchronizuję…".into()),
            "offline" => (0, "Cel odłączony".into()),
            "confirm" => (3, "Czeka na potwierdzenie".into()),
            "error" => (3, format!("Błąd: {}", st.last_result)),
            _ if st.last_sync == 0 => (1, "Czeka na pierwsze porównanie".into()),
            _ => (1, "Aktualna".into()),
        }
    };
    let detail = match st.state.as_str() {
        "offline" => {
            let pend = if st.pending > 0 { format!(" · czeka zmian: {}", st.pending) } else { String::new() };
            format!("widziany {}{pend}", ago(st.last_seen))
        }
        "error" => format!("ostatnia udana {}", ago(st.last_sync)),
        _ if st.last_sync > 0 => format!("ostatnio {} · {}", ago(st.last_sync), counts_text(&st.last_counts)),
        _ => String::new(),
    };
    let confirm = st
        .needs_confirm
        .as_ref()
        .filter(|_| st.state == "confirm")
        .map(|c| format!("Ochrona przed masowym usunięciem: synchronizacja usunęłaby {} plików w celu i {} w źródle. Sprawdź podgląd i potwierdź, jeśli to zamierzone.", c.del_target, c.del_source))
        .unwrap_or_default();
    let target = if st.connected && !st.resolved_target.is_empty() { st.resolved_target.clone() } else { p.target.path.clone() };
    let target = if !p.target.label.is_empty() && !p.target.network { format!("{target}  ({})", p.target.label) } else { target };
    let history: Vec<SyncHist> = st.history.iter().rev().take(10).map(|h| SyncHist { when: fmt::date(h.at).into(), text: h.summary.clone().into(), ok: h.ok }).collect();
    SyncCard {
        id: p.id.clone().into(),
        name: if p.name.is_empty() { p.source.clone() } else { p.name.clone() }.into(),
        source: p.source.clone().into(),
        target: target.into(),
        mode: mode_label(p.mode).into(),
        tone,
        state: state.into(),
        detail: detail.into(),
        enabled: p.enabled,
        busy,
        progress: st.progress,
        current: st.current.clone().into(),
        confirm: confirm.into(),
        history: ModelRc::new(VecModel::from(history)),
    }
}

fn render(s: &Shared) {
    let (profiles, status) = sync::profiles();
    let cards: Vec<SyncCard> = profiles.iter().zip(status.iter()).map(|(p, st)| card(p, st)).collect();
    with_ui(s, |g| g.set_cards(ModelRc::new(VecModel::from(cards))));
}

fn profile(id: &str) -> Option<Profile> {
    sync::profiles().0.into_iter().find(|p| p.id == id)
}

// ---------- Edytor ----------

pub fn add(s: &Shared) {
    if blocked(s) {
        return;
    }
    with_ui(s, |g| {
        g.set_e_id("".into());
        g.set_e_name("".into());
        g.set_e_source("".into());
        g.set_e_target("".into());
        g.set_e_mode(0);
        g.set_e_continuous(true);
        g.set_e_interval("15".into());
        g.set_e_versioning(true);
        g.set_e_hidden(false);
        g.set_e_exclude("".into());
        g.set_e_error("".into());
        g.set_editing(true);
    });
}

pub fn edit(s: &Shared, id: &str) {
    if blocked(s) {
        return;
    }
    let Some(p) = profile(id) else { return };
    with_ui(s, |g| {
        g.set_e_id(p.id.clone().into());
        g.set_e_name(p.name.clone().into());
        g.set_e_source(p.source.clone().into());
        g.set_e_target(p.target.path.clone().into());
        g.set_e_mode(match p.mode {
            Mode::Mirror => 0,
            Mode::TwoWay => 1,
            Mode::Backup => 2,
        });
        g.set_e_continuous(p.continuous);
        g.set_e_interval(p.interval_min.to_string().into());
        g.set_e_versioning(p.versioning);
        g.set_e_hidden(p.include_hidden);
        g.set_e_exclude(p.exclude.join("\n").into());
        g.set_e_error("".into());
        g.set_editing(true);
    });
}

fn pick(s: &Shared, title: &str, current: SharedString) -> Option<String> {
    let cur = current.to_string();
    powerdesk_core::shell::pick_folder(crate::hwnd(s), title, (!cur.is_empty()).then_some(cur.as_str()))
}

pub fn pick_source(s: &Shared) {
    let mut cur = SharedString::new();
    with_ui(s, |g| cur = g.get_e_source());
    if let Some(p) = pick(s, "Folder źródłowy", cur) {
        let name = std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        with_ui(s, |g| {
            if g.get_e_name().is_empty() {
                g.set_e_name(name.into());
            }
            g.set_e_source(p.into());
        });
    }
}

pub fn pick_target(s: &Shared) {
    let mut cur = SharedString::new();
    with_ui(s, |g| cur = g.get_e_target());
    if let Some(p) = pick(s, "Folder docelowy (dysk zewnętrzny albo sieciowy)", cur) {
        with_ui(s, |g| g.set_e_target(p.into()));
    }
}

pub fn save(s: &Shared) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let g = ui.global::<SyncState>();
    let id = g.get_e_id().to_string();
    let old = profile(&id);
    let source = g.get_e_source().to_string();
    let target_path = g.get_e_target().to_string();
    // Cel się nie zmienił – zostaje jego zapamiętana identyfikacja (numer woluminu).
    let target = match &old {
        Some(o) if o.target.path.eq_ignore_ascii_case(&target_path) => o.target.clone(),
        _ => sync::describe_target(&target_path),
    };
    let p = Profile {
        id,
        name: g.get_e_name().trim().to_string(),
        source,
        target,
        mode: match g.get_e_mode() {
            1 => Mode::TwoWay,
            2 => Mode::Backup,
            _ => Mode::Mirror,
        },
        enabled: old.as_ref().map(|o| o.enabled).unwrap_or(true),
        continuous: g.get_e_continuous(),
        interval_min: g.get_e_interval().trim().parse().unwrap_or(15).clamp(1, 1440),
        versioning: g.get_e_versioning(),
        exclude: g.get_e_exclude().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        include_hidden: g.get_e_hidden(),
    };
    match sync::save_profile(p) {
        Ok(_) => {
            g.set_editing(false);
            render(s);
        }
        Err(e) => g.set_e_error(e.into()),
    }
}

pub fn remove(s: &Shared, id: String) {
    if blocked(s) {
        return;
    }
    let name = profile(&id).map(|p| if p.name.is_empty() { p.source } else { p.name }).unwrap_or_default();
    crate::transfers::confirm(
        s,
        "Usunąć synchronizację?",
        &format!("„{name}” przestanie się synchronizować. Pliki w źródle i w celu zostaną bez zmian."),
        "Usuń",
        true,
        std::rc::Rc::new(move |s| {
            sync::delete_profile(&id);
            render(s);
        }),
    );
}

pub fn toggle(s: &Shared, id: &str) {
    if blocked(s) {
        return;
    }
    if let Some(p) = profile(id) {
        sync::set_enabled(id, !p.enabled);
        render(s);
    }
}

// ---------- Podgląd ----------

pub fn preview(s: &Shared, id: String) {
    let name = profile(&id).map(|p| if p.name.is_empty() { p.source } else { p.name }).unwrap_or_default();
    with_ui(s, |g| {
        g.set_preview_title(format!("Podgląd: {name}").into());
        g.set_preview_summary("".into());
        g.set_preview_items(ModelRc::default());
        g.set_preview_loading(true);
        g.set_preview_open(true);
    });
    std::thread::spawn(move || {
        let res = sync::preview(&id);
        let _ = slint::invoke_from_event_loop(move || {
            crate::with_state(|s| {
                with_ui(s, |g| {
                    g.set_preview_loading(false);
                    match res {
                        Ok(p) => {
                            let more = if p.items.len() >= 500 { " · pokazano pierwsze 500" } else { "" };
                            g.set_preview_summary(format!("{}{more}", counts_text(&p.counts)).into());
                            let items: Vec<SharedString> = p.items.into_iter().map(Into::into).collect();
                            g.set_preview_items(ModelRc::new(VecModel::from(items)));
                        }
                        Err(e) => g.set_preview_summary(e.into()),
                    }
                })
            })
        });
    });
}

pub fn bind(ui: &crate::MainWindow, st: &Shared) {
    let g = ui.global::<SyncState>();
    let s = st.clone();
    g.on_add(move || add(&s));
    let s = st.clone();
    g.on_edit(move |id| edit(&s, &id));
    let s = st.clone();
    g.on_pick_source(move || pick_source(&s));
    let s = st.clone();
    g.on_pick_target(move || pick_target(&s));
    let s = st.clone();
    g.on_save(move || save(&s));
    let s = st.clone();
    g.on_cancel_edit(move || with_ui(&s, |g| g.set_editing(false)));
    let s = st.clone();
    g.on_remove(move |id| remove(&s, id.into()));
    let s = st.clone();
    g.on_sync_now(move |id| {
        if !blocked(&s) {
            sync::sync_now(Some(id.into()), false);
        }
    });
    let s = st.clone();
    g.on_sync_all(move || {
        if !blocked(&s) {
            sync::sync_now(None, false);
        }
    });
    let s = st.clone();
    g.on_confirm(move |id| {
        if !blocked(&s) {
            sync::sync_now(Some(id.into()), true);
        }
    });
    let s = st.clone();
    g.on_toggle(move |id| toggle(&s, &id));
    g.on_stop(|id| sync::cancel(&id));
    let s = st.clone();
    g.on_preview(move |id| preview(&s, id.into()));
    let s = st.clone();
    g.on_open_folder(move |p| {
        // Cel może mieć dopisaną etykietę dysku – ścieżka jest przed nią.
        let p = p.split("  (").next().unwrap_or_default().to_string();
        crate::new_tab_at(&s, &p, true);
    });
    let s = st.clone();
    g.on_retry_start(move || {
        ensure_started(&s);
        render(&s);
    });
}
