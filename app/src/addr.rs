//! Podpowiedzi przy edycji ścieżki (Toolbar.svelte `suggest`): podfoldery folderu przed
//! ostatnim „\”, zaczynające się od wpisanego początku nazwy – najwyżej 12.

use std::cell::Cell;

use powerdesk_core::fsx;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{AddrSugg, Shared};

thread_local! {
    static TOKEN: Cell<u64> = const { Cell::new(0) };
}

fn set(s: &Shared, items: Vec<SharedString>) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        let g = ui.global::<AddrSugg>();
        g.set_index(-1);
        g.set_items(ModelRc::new(VecModel::from(items)));
    }
}

fn query(s: &Shared, text: &str) {
    let token = TOKEN.with(|t| {
        t.set(t.get() + 1);
        t.get()
    });
    let v = text.trim().trim_matches('"').replace('/', "\\");
    let Some(i) = v.rfind('\\') else { return set(s, Vec::new()) };
    let (dir, pre) = (v[..=i].to_string(), v[i + 1..].to_lowercase());
    let hidden = s.borrow().prefs.show_hidden;
    std::thread::spawn(move || {
        let mut list: Vec<String> = fsx::list_dir(&dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.dir && e.name.to_lowercase().starts_with(&pre) && (hidden || !e.hidden))
            .map(|e| e.path)
            .collect();
        list.sort_by(|a, b| crate::model::natural(a, b));
        list.truncate(12);
        let _ = slint::invoke_from_event_loop(move || {
            crate::with_state(|s| {
                if TOKEN.with(|t| t.get()) == token {
                    set(s, list.into_iter().map(Into::into).collect());
                }
            })
        });
    });
}

pub fn bind(ui: &crate::MainWindow, st: &Shared) {
    let g = ui.global::<AddrSugg>();
    let s = st.clone();
    g.on_query(move |t| query(&s, &t));
    let s = st.clone();
    g.on_clear(move || {
        TOKEN.with(|t| t.set(t.get() + 1));
        set(&s, Vec::new());
    });
}
