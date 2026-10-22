//! Kanał zdarzeń backendu, niezależny od warstwy UI.
//!
//! Moduły wołają `app.emit("nazwa", dane)` jak dawniej; dokąd trafia zdarzenie, decyduje UI
//! przez [`set_sink`] (Slint – kolejka do wątku interfejsu, Tauri – `AppHandle::emit`).

use once_cell::sync::OnceCell;
use serde::Serialize;

type Sink = Box<dyn Fn(&str, serde_json::Value) + Send + Sync>;
type Exit = Box<dyn Fn(i32) + Send + Sync>;

static SINK: OnceCell<Sink> = OnceCell::new();
static EXIT: OnceCell<Exit> = OnceCell::new();

pub fn set_sink(f: impl Fn(&str, serde_json::Value) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(f));
}

pub fn set_exit(f: impl Fn(i32) + Send + Sync + 'static) {
    let _ = EXIT.set(Box::new(f));
}

pub fn emit(name: &str, payload: impl Serialize) {
    if let (Some(s), Ok(v)) = (SINK.get(), serde_json::to_value(payload)) {
        s(name, v)
    }
}

/// Uchwyt przekazywany modułom backendu. Nie niesie stanu – wszystko idzie przez zarejestrowany kanał.
#[derive(Clone, Copy, Default)]
pub struct AppHandle;

pub trait Emitter {
    fn emit<S: Serialize>(&self, name: &str, payload: S) -> Result<(), ()>;
}

impl Emitter for AppHandle {
    fn emit<S: Serialize>(&self, name: &str, payload: S) -> Result<(), ()> {
        emit(name, payload);
        Ok(())
    }
}

impl AppHandle {
    pub fn exit(&self, code: i32) {
        match EXIT.get() {
            Some(f) => f(code),
            None => std::process::exit(code),
        }
    }
}
