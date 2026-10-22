//! Archiwa ZIP, 7z i RAR w menedżerze (state.svelte.ts → extractTo, extractHere, zipSelection):
//! przeglądanie jak folderu, otwieranie plików ze środka, wypakowanie i pakowanie w tle.
//! Wnętrze archiwum jest tylko do odczytu.

use std::path::Path;

use powerdesk_core::archive;
use powerdesk_core::fsx::Entry;

use crate::transfers::{self, Kind};
use crate::Shared;

/// Plik, który da się otworzyć jak folder.
pub fn browsable(e: &Entry) -> bool {
    !e.dir && ["zip", "7z", "rar"].iter().any(|x| e.ext.eq_ignore_ascii_case(x))
}

/// Ścieżka prowadzi do archiwum albo do jego wnętrza (folder bieżący karty).
pub fn browsing(path: &str) -> bool {
    !path.starts_with("ofp:") && archive::split(path).is_some()
}

/// Element leży wewnątrz archiwum (nie sam plik archiwum).
pub fn inside(path: &str) -> bool {
    !path.starts_with("ofp:") && archive::split(path).is_some_and(|(_, inner)| !inner.is_empty())
}

/// Blokuje zmiany elementów z wnętrza archiwum; `true` – operację trzeba przerwać.
pub fn read_only(s: &Shared, paths: &[String]) -> bool {
    blocked(s, paths.iter().any(|p| inside(p)))
}

/// Blokuje zapis do folderu w archiwum (także do jego korzenia, który jest plikiem).
pub fn read_only_dest(s: &Shared, dest: &str) -> bool {
    blocked(s, browsing(dest))
}

fn blocked(s: &Shared, hit: bool) -> bool {
    if hit {
        transfers::toast(s, "Archiwa są tylko do odczytu", Kind::Error, None);
    }
    hit
}

fn base(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string())
}

/// Praca w tle; wynik (komunikat albo błąd) wraca do wątku UI.
fn background(work: impl FnOnce() -> Result<(String, Option<String>), String> + Send + 'static) {
    std::thread::spawn(move || {
        let res = work();
        let _ = slint::invoke_from_event_loop(move || {
            crate::with_state(|s| match res {
                Ok((msg, select)) => {
                    reload(s);
                    if let Some(p) = select {
                        crate::select_path(s, &p);
                    }
                    if !msg.is_empty() {
                        transfers::toast(s, msg, Kind::Ok, None);
                    }
                }
                Err(e) => transfers::toast(s, e, Kind::Error, None),
            })
        });
    });
}

/// Odświeża bieżącą kartę, o ile nie jest ekranem specjalnym.
fn reload(s: &Shared) {
    if !crate::tab_mut(s, |t| t.path.starts_with("ofp:")) {
        crate::load(s, true);
    }
}

/// Plik z archiwum: wypakowanie do folderu tymczasowego i otwarcie w domyślnym programie.
pub fn open_file(path: &str) {
    let path = path.to_string();
    background(move || {
        let out = archive::extract_temp(&path)?;
        open::that_detached(&out).map_err(|e| e.to_string())?;
        Ok((String::new(), None))
    });
}

/// Wypakowanie elementów archiwum do folderu (kopiowanie, wklejanie, przeciąganie z archiwum).
pub fn extract_to(s: &Shared, sources: Vec<String>, dest: &str) {
    if dest.starts_with("ofp:") || read_only_dest(s, dest) {
        return;
    }
    let what = if sources.len() == 1 { format!("„{}”", base(&sources[0])) } else { format!("{} elem.", sources.len()) };
    transfers::toast(s, format!("Wypakowuję {what}…"), Kind::Info, None);
    let dest = dest.to_string();
    background(move || {
        let out = archive::extract(&sources, &dest)?;
        Ok((format!("Wypakowano do „{}”", base(&dest)), out.into_iter().next()))
    });
}

/// Całe archiwum do folderu obok, o tej samej nazwie.
pub fn extract_here(s: &Shared, arc: &str) {
    transfers::toast(s, format!("Wypakowuję „{}”…", base(arc)), Kind::Info, None);
    let arc = arc.to_string();
    background(move || {
        let out = archive::extract_here(&arc)?;
        Ok((format!("Wypakowano do „{}”", base(&out)), Some(out)))
    });
}

/// Nowe archiwum ZIP z zaznaczonych, obok nich; nazwa jak w zipSelection.
pub fn zip(s: &Shared, sources: &[String]) {
    if sources.is_empty() || read_only(s, sources) {
        return;
    }
    let dir = crate::tab_mut(s, |t| t.path.clone());
    let name = if sources.len() == 1 {
        Path::new(&sources[0]).file_stem().map(|n| n.to_string_lossy().to_string())
    } else {
        Path::new(&dir).file_name().map(|n| n.to_string_lossy().to_string())
    }
    .unwrap_or_else(|| "Archiwum".into());
    let dest = Path::new(&dir).join(format!("{name}.zip")).to_string_lossy().to_string();
    transfers::toast(s, format!("Pakuję {} elem.…", sources.len()), Kind::Info, None);
    let sources = sources.to_vec();
    background(move || {
        let out = archive::create_zip(&sources, &dest)?;
        Ok((format!("Utworzono „{}”", base(&out)), Some(out)))
    });
}
