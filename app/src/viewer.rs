//! Przeglądarka zdjęć (Viewer.svelte) – w osobnej karcie tego samego okna, pod paskiem tytułu
//! z przyciskami okna, więc okno da się normalnie zminimalizować; pełny ekran tylko na życzenie (F11, przycisk). Zdjęcia z dysku dekoduje `image` (z orientacją EXIF),
//! RAW, HEIC i wideo pokazują duży obraz z powłoki Windows. Zoom, przesuwanie i obrót są w Slint.

use std::cell::Cell;

use image::ImageDecoder;
use powerdesk_core::fsx::Entry;
use powerdesk_core::shell;
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::fmt::{self, Kind};
use crate::{Shared, ViewerState, ViewerThumb};

/// Ścieżka karty z przeglądarką – nie wczytuje żadnego folderu.
pub const TAB: &str = "ofp:viewer";

pub struct Viewer {
    pub list: Vec<Entry>,
    pub index: usize,
}

thread_local! {
    /// Numer wczytania – zdjęcie z tła trafia tylko do tego, które wciąż jest pokazane.
    static GEN: Cell<u64> = const { Cell::new(0) };
}

/// Formaty dekodowane bezpośrednio (pełna rozdzielczość); resztę daje powłoka.
const DECODED: &[&str] = &["jpg", "jpeg", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff"];
/// Dłuższy bok dekodowanego zdjęcia – więcej nie widać nawet przy powiększeniu na 4K.
const MAX_SIDE: u32 = 4096;
/// Obraz z powłoki dla RAW, HEIC i wideo.
const SHELL_PX: u32 = 2560;

fn with_ui(s: &Shared, f: impl FnOnce(&ViewerState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<ViewerState>());
    }
}

/// Otwiera przeglądarkę na elemencie `i` listy; po kolei idą zdjęcia i filmy z bieżącego widoku.
pub fn open(s: &Shared, i: usize) -> bool {
    let (list, index) = crate::tab_mut(s, |t| {
        let Some(cur) = t.items.get(i) else { return (Vec::new(), 0) };
        let list: Vec<Entry> = t.items.iter().filter(|e| fmt::is_media(e) && !crate::archives::inside(&e.path)).cloned().collect();
        let index = list.iter().position(|e| e.path == cur.path).unwrap_or(0);
        (list, index)
    });
    if list.is_empty() {
        return false;
    }
    crate::new_viewer_tab(s, Viewer { list, index });
    with_ui(s, |g| {
        g.set_info_open(s.borrow().prefs.viewer_info);
        g.set_slideshow(false);
        g.set_open(true);
    });
    show(s);
    true
}

/// Przeglądarka otwartej karty (albo nic, gdy to zwykła karta z listą).
fn viewer<R>(s: &Shared, f: impl FnOnce(&Viewer) -> R) -> Option<R> {
    let b = s.borrow();
    let a = b.active;
    b.tabs[a].viewer.as_ref().map(f)
}

/// Przełączono kartę: przeglądarkę pokazujemy tylko wtedy, gdy to jej karta.
pub fn sync_tab(s: &Shared) {
    let on = viewer(s, |_| ()).is_some();
    with_ui(s, |g| g.set_open(on));
    if on {
        with_ui(s, |g| {
            g.set_info_open(s.borrow().prefs.viewer_info);
            g.set_slideshow(false);
        });
        show(s);
    }
}

pub fn close(s: &Shared) {
    GEN.with(|g| g.set(g.get() + 1));
    let path = {
        let mut b = s.borrow_mut();
        let a = b.active;
        b.tabs[a].viewer.take().map(|v| v.list[v.index].path.clone())
    };
    if path.is_none() {
        return;
    }
    if let Some(ui) = s.borrow().ui.upgrade() {
        if ui.window().is_fullscreen() {
            ui.window().set_fullscreen(false);
        }
        let g = ui.global::<ViewerState>();
        g.set_open(false);
        g.set_fullscreen(false);
        g.set_img(Default::default());
        g.set_thumbs(ModelRc::default());
        g.set_sections(ModelRc::default());
    }
    // Karta przeglądarki znika, a lista wraca zaznaczona na ostatnio oglądanym zdjęciu.
    let active = s.borrow().active;
    crate::close_tab(s, active);
    if let Some(p) = path {
        crate::select_path(s, &p);
    }
}

pub fn step(s: &Shared, d: i32) {
    let moved = {
        let mut b = s.borrow_mut();
        let a = b.active;
        let Some(v) = b.tabs[a].viewer.as_mut() else { return };
        let n = v.list.len() as i32;
        let i = (v.index as i32 + d).rem_euclid(n) as usize;
        let moved = i != v.index;
        v.index = i;
        moved
    };
    if moved {
        show(s);
    }
}

pub fn go(s: &Shared, i: usize) {
    {
        let mut b = s.borrow_mut();
        let a = b.active;
        let Some(v) = b.tabs[a].viewer.as_mut() else { return };
        if i >= v.list.len() || i == v.index {
            return;
        }
        v.index = i;
    }
    show(s);
}

pub fn first_last(s: &Shared, last: bool) {
    let n = viewer(s, |v| v.list.len()).unwrap_or(0);
    if n > 0 {
        go(s, if last { n - 1 } else { 0 });
    }
}

pub fn toggle_fullscreen(s: &Shared) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let on = !ui.window().is_fullscreen();
    ui.window().set_fullscreen(on);
    ui.global::<ViewerState>().set_fullscreen(on);
}

/// Esc: najpierw wyjście z pełnego ekranu, potem zamknięcie.
pub fn escape(s: &Shared) {
    let full = s.borrow().ui.upgrade().is_some_and(|ui| ui.window().is_fullscreen());
    if full {
        toggle_fullscreen(s);
    } else {
        close(s);
    }
}

pub fn toggle_info(s: &Shared) {
    let on = {
        let mut b = s.borrow_mut();
        b.prefs.viewer_info = !b.prefs.viewer_info;
        b.prefs.save();
        b.prefs.viewer_info
    };
    with_ui(s, |g| g.set_info_open(on));
}

pub fn open_external(s: &Shared) {
    let path = viewer(s, |v| v.list[v.index].path.clone());
    if let Some(p) = path {
        let _ = open::that_detached(p);
    }
}

fn current(s: &Shared) -> Option<(Entry, usize, usize)> {
    viewer(s, |v| (v.list[v.index].clone(), v.index, v.list.len()))
}

/// Pokazuje bieżące zdjęcie: od razu obraz z pamięci podręcznej (jeśli jest), pełny dociąga się w tle.
fn show(s: &Shared) {
    let Some((e, index, n)) = current(s) else { return };
    let gen = GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let video = fmt::kind_of(&e) == Kind::Video;
    let decoded = DECODED.contains(&e.ext.to_lowercase().as_str()) && !e.cloud;
    let dims = s.borrow().media.get(&e.path).filter(|m| m.w > 0).map(|m| format!("{} × {} · ", m.w, m.h)).unwrap_or_default();
    with_ui(s, |g| {
        g.set_name(e.name.clone().into());
        g.set_counter(format!("{} / {n}", index + 1).into());
        g.set_details(format!("{dims}{} · {}", fmt::size(e.size), fmt::date(e.modified)).into());
        g.set_is_video(video);
        g.set_rotation(0);
        g.set_sections(ModelRc::default());
        // Na start – największa miniatura, którą już mamy; pełny obraz zastąpi ją po wczytaniu.
        let quick = crate::thumbs::get(&e.path, SHELL_PX, false).or_else(|| crate::thumbs::get(&e.path, 256, false));
        g.set_loading(quick.is_none() || decoded);
        g.set_has_img(quick.is_some());
        g.set_img(quick.unwrap_or_default());
    });
    render_thumbs(s);
    if decoded {
        let p = e.path.clone();
        std::thread::spawn(move || {
            let buf = decode(&p);
            let _ = slint::invoke_from_event_loop(move || {
                if GEN.with(|g| g.get()) != gen {
                    return;
                }
                crate::with_state(|s| {
                    with_ui(s, |g| {
                        g.set_loading(false);
                        if let Some(b) = buf {
                            g.set_has_img(true);
                            g.set_img(Image::from_rgba8(b));
                        }
                    })
                })
            });
        });
    }
    // Metadane do panelu informacji.
    let p = e.path.clone();
    std::thread::spawn(move || {
        let props = shell::properties(&p);
        let _ = slint::invoke_from_event_loop(move || {
            if GEN.with(|g| g.get()) != gen {
                return;
            }
            crate::with_state(|s| with_ui(s, |g| g.set_sections(ModelRc::new(VecModel::from(crate::props::sections(&props))))));
        });
    });
}

/// Pełne zdjęcie z dysku, obrócone według EXIF i zmniejszone do MAX_SIDE.
fn decode(path: &str) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let mut dec = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
    let orientation = dec.orientation().ok();
    let mut img = image::DynamicImage::from_decoder(dec).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    if img.width().max(img.height()) > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    Some(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}

/// Pasek miniatur: do 30 zdjęć z każdej strony bieżącego.
fn render_thumbs(s: &Shared) {
    let b = s.borrow();
    let Some(v) = b.tabs[b.active].viewer.as_ref() else { return };
    let Some(ui) = b.ui.upgrade() else { return };
    let px = (64.0 * ui.window().scale_factor()) as u32;
    let lo = v.index.saturating_sub(30);
    let hi = (v.index + 31).min(v.list.len());
    let thumbs: Vec<ViewerThumb> = (lo..hi)
        .map(|i| {
            let img = crate::thumbs::get(&v.list[i].path, px, false);
            ViewerThumb { index: i as i32, current: i == v.index, has_img: img.is_some(), img: img.unwrap_or_default() }
        })
        .collect();
    ui.global::<ViewerState>().set_thumbs(ModelRc::new(VecModel::from(thumbs)));
}

/// Dotarły miniatury: pasek i – gdy pełny obraz jeszcze nie przyszedł – zdjęcie z powłoki.
pub fn thumbs_arrived(s: &Shared, paths: &std::collections::HashSet<String>) {
    let Some((e, ..)) = current(s) else { return };
    let any = viewer(s, |v| v.list.iter().any(|x| paths.contains(&x.path))).unwrap_or(false);
    if !any {
        return;
    }
    render_thumbs(s);
    if paths.contains(&e.path) && !DECODED.contains(&e.ext.to_lowercase().as_str()) {
        if let Some(img) = crate::thumbs::get(&e.path, SHELL_PX, false) {
            with_ui(s, |g| {
                g.set_loading(false);
                g.set_has_img(true);
                g.set_img(img);
            });
        }
    }
}
