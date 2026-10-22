//! Przeciąganie plików: start z listy (OLE, jak w Eksploratorze) i upuszczanie na okno – z aplikacji
//! albo z innych programów. Cel wskazuje sam interfejs: symulujemy ruch wskaźnika, a element pod nim
//! z ustawionym `drop` zapisuje folder w globalnym `Drag` (components.slint).

use std::cell::Cell;
use std::path::Path;

use powerdesk_core::dnd::{self, Effect, Keys};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

use crate::{transfers, Drag, MainWindow, Shared};

fn volume(p: &str) -> String {
    let s = p.to_lowercase();
    match s.strip_prefix(r"\\") {
        Some(rest) => rest.split('\\').take(2).collect::<Vec<_>>().join("\\"),
        None => s.chars().take(2).collect(),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

/// Kopiuj czy przenieś (dropMode w state.svelte.ts): Ctrl – kopiuj, Shift – przenieś,
/// bez klawiszy – przenieś w obrębie dysku, kopiuj między dyskami.
pub fn mode(sources: &[String], dest: &str, keys: Keys) -> &'static str {
    if keys.ctrl {
        "copy"
    } else if keys.shift || sources.iter().all(|s| volume(s) == volume(dest)) {
        "move"
    } else {
        "copy"
    }
}

/// Cel „Półka” – upuszczone pliki trafiają na półkę, bez kopiowania.
pub const SHELF: &str = "ofp:shelf";
/// Cel „Przypięte” w panelu bocznym – upuszczone foldery zostają przypięte.
pub const PIN: &str = "ofp:pin";

/// Efekt upuszczenia na `target`; brak, gdy cel nie przyjmie plików.
fn effect(paths: &[String], target: &str, keys: Keys) -> (Effect, &'static str) {
    if target == SHELF {
        return (Effect::Copy, "shelf");
    }
    if target == PIN {
        let dirs = paths.iter().any(|p| !p.starts_with("ofp:") && Path::new(p).is_dir());
        return if dirs { (Effect::Link, "pin") } else { (Effect::None, "") };
    }
    if target.is_empty() || target.starts_with("ofp:") || !Path::new(target).is_dir() {
        return (Effect::None, "");
    }
    let low = target.to_lowercase();
    // Folder na samego siebie albo do własnego wnętrza.
    if paths.iter().any(|p| same(p, target) || low.starts_with(&format!("{}\\", p.to_lowercase().trim_end_matches('\\')))) {
        return (Effect::None, "");
    }
    let m = mode(paths, target, keys);
    // Przeniesienie tam, gdzie pliki już są, nic nie zmienia.
    if m == "move" && paths.iter().all(|p| Path::new(p).parent().is_some_and(|d| same(&d.to_string_lossy(), target))) {
        return (Effect::None, "");
    }
    (if m == "copy" { Effect::Copy } else { Effect::Move }, m)
}

struct Handler {
    ui: slint::Weak<MainWindow>,
    /// Pierwszy ruch po wejściu – hover trzeba ustawić od nowa.
    fresh: Cell<bool>,
}

impl Handler {
    /// Przesuwa „wirtualny” wskaźnik Slint w miejsce przeciągania i zwraca wskazany folder.
    fn target_at(&self, x: i32, y: i32) -> String {
        let Some(ui) = self.ui.upgrade() else { return String::new() };
        let scale = ui.window().scale_factor();
        let pos = LogicalPosition::new(x as f32 / scale, y as f32 / scale);
        let g = ui.global::<Drag>();
        if !g.get_active() || self.fresh.replace(false) {
            g.set_active(true);
            g.set_target(Default::default());
            ui.window().dispatch_event(WindowEvent::PointerExited);
        }
        ui.window().dispatch_event(WindowEvent::PointerMoved { position: pos });
        g.get_target().to_string()
    }

    fn reset(&self) {
        if let Some(ui) = self.ui.upgrade() {
            let g = ui.global::<Drag>();
            g.set_active(false);
            g.set_target(Default::default());
            ui.window().dispatch_event(WindowEvent::PointerExited);
        }
        self.fresh.set(true);
    }
}

impl dnd::DropHandler for Handler {
    fn over(&self, x: i32, y: i32, keys: Keys, paths: &[String]) -> Effect {
        let target = self.target_at(x, y);
        effect(paths, &target, keys).0
    }

    fn leave(&self) {
        self.reset();
    }

    fn dropped(&self, x: i32, y: i32, keys: Keys, paths: &[String]) -> Effect {
        let target = self.target_at(x, y);
        self.reset();
        let (e, m) = effect(paths, &target, keys);
        if e != Effect::None {
            let paths = paths.to_vec();
            // Po powrocie z wywołania OLE – kopiowanie idzie przez pasek transferów (z oknem konfliktu).
            slint::Timer::single_shot(std::time::Duration::from_millis(1), move || {
                crate::with_state(|s| match m {
                    "shelf" => crate::shelf::add(s, &paths),
                    "pin" => crate::pin_folders(s, &paths),
                    _ => transfers::transfer(s, paths, &target, m),
                });
            });
        }
        e
    }
}

/// Okno przyjmuje upuszczane pliki (z aplikacji i z innych programów).
pub fn register(s: &Shared) {
    let hwnd = crate::hwnd(s);
    if hwnd == 0 {
        return;
    }
    let ui = s.borrow().ui.clone();
    let _ = dnd::register(hwnd, Box::new(Handler { ui, fresh: Cell::new(true) }));
}

/// Start przeciągania elementu `i` (z zaznaczeniem, jeśli należy do niego).
pub fn start(s: &Shared, i: usize) {
    let paths = crate::tab_mut(s, |t| {
        let e = t.items.get(i)?.clone();
        if !t.selected.contains(&e.path) {
            t.selected.clear();
            t.selected.insert(e.path.clone());
            t.focus = Some(i);
            t.anchor = Some(i);
        }
        Some(t.items.iter().filter(|x| t.selected.contains(&x.path)).map(|x| x.path.clone()).collect::<Vec<_>>())
    });
    let Some(paths) = paths.filter(|p| !p.is_empty() && !p.iter().any(|x| x.starts_with("ofp:"))) else { return };
    crate::render_items(s);
    start_paths(s, paths, |_| {});
}

/// Przeciąganie OLE podanych ścieżek; `done` – w wątku UI po upuszczeniu albo anulowaniu.
pub fn start_paths(s: &Shared, paths: Vec<String>, done: impl FnOnce(&Shared) + 'static) {
    let ui = s.borrow().ui.clone();
    // Element z wciśniętym przyciskiem trzyma wskaźnik – puszczamy go, żeby hover działał przy upuszczaniu.
    slint::Timer::single_shot(std::time::Duration::from_millis(1), move || {
        if let Some(ui) = ui.upgrade() {
            ui.window().dispatch_event(WindowEvent::PointerReleased { position: LogicalPosition::new(-1.0, -1.0), button: PointerEventButton::Left });
        }
        // `done` nie jest Send – czeka w wątku UI, a wątek OLE tylko go stamtąd wywołuje.
        DONE.with(|d| *d.borrow_mut() = Some(Box::new(done)));
        dnd::start_drag(paths, |_| {
            let _ = slint::invoke_from_event_loop(|| {
                if let Some(f) = DONE.with(|d| d.borrow_mut().take()) {
                    crate::with_state(|s| f(s));
                }
            });
        });
    });
}

thread_local! {
    static DONE: std::cell::RefCell<Option<Box<dyn FnOnce(&Shared)>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes() {
        let none = Keys::default();
        assert_eq!(mode(&["C:\\a\\x.txt".into()], "C:\\b", none), "move");
        assert_eq!(mode(&["C:\\a\\x.txt".into()], "D:\\b", none), "copy");
        assert_eq!(mode(&["C:\\a\\x.txt".into()], "C:\\b", Keys { ctrl: true, shift: false }), "copy");
        assert_eq!(mode(&["C:\\a\\x.txt".into()], "D:\\b", Keys { ctrl: false, shift: true }), "move");
    }
}
