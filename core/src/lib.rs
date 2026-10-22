//! PowerDesk – backend: system plików, operacje, synchronizacja, miniatury i integracja z Windows.
//! Bez zależności od warstwy UI; zdarzenia wychodzą przez [`events`].

pub mod archive;
pub mod autostart;
pub mod clip;
pub mod dnd;
pub mod drives;
pub mod events;
pub mod fsx;
pub mod instance;
pub mod library;
pub mod maps;
pub mod media;
pub mod ops;
pub mod pdf;
pub mod remote;
pub mod shell;
pub mod space;
pub mod sync;
pub mod sync_core;
pub mod sysmon;
pub mod system;
pub mod terminal;
pub mod tray;
pub mod winkey;
