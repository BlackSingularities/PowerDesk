//! Panel podglądu (PreviewPane.svelte): po prawej stronie listy, z regulowaną szerokością.
//! Jeden element – obraz z powłoki Windows (zdjęcia, RAW, klatka wideo, pierwsza strona PDF, dokumenty),
//! tekst i kod albo spis archiwum, pod spodem nazwa, akcje, daty i metadane. Kilka – podsumowanie.
//! Nic – folder i dysk. Treść wczytuje się w tle, z krótkim opóźnieniem przy przewijaniu strzałkami.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::Duration;

use powerdesk_core::fsx::{self, Entry};
use powerdesk_core::{archive, maps, shell};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::fmt::{self, Kind};
use crate::props::{self, plural, row, sections};
use crate::syntax;
use crate::{CodeLine, CodeTok, PreviewState, PropRow, Shared};

thread_local! {
    /// Co pokazuje panel (ścieżki zaznaczenia albo folder) – żeby nie wczytywać tego samego od nowa.
    static SHOWN: RefCell<String> = const { RefCell::new(String::new()) };
    /// Numer wczytania – spóźnione wyniki z tła są pomijane.
    static GEN: Cell<u64> = const { Cell::new(0) };
    static TIMER: slint::Timer = slint::Timer::default();
    /// Obraz, o który panel poprosił: (ścieżka, piksele, sama ikona) – do podmiany, gdy dotrze.
    static IMG: RefCell<Option<(String, u32, bool)>> = const { RefCell::new(None) };
    /// Współrzędne pokazanego zdjęcia – do otwarcia w przeglądarce.
    static COORDS: Cell<Option<(f64, f64)>> = const { Cell::new(None) };
    /// Pokazany PDF: ścieżka i szerokość strony w pikselach; numer renderu – spóźnione strony są pomijane.
    static PDF: RefCell<Option<(String, u32)>> = const { RefCell::new(None) };
    static PDF_GEN: Cell<u64> = const { Cell::new(0) };
}

/// Strona PDF w tle; po dotarciu zastępuje obraz z powłoki (i nie podmienią jej już miniatury).
fn pdf_page(gen: u64, page: i32) {
    let Some((path, px)) = PDF.with(|p| p.borrow().clone()) else { return };
    let pg = PDF_GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    std::thread::spawn(move || {
        let Some((png, count)) = powerdesk_core::pdf::render(&path, page.max(0) as u32, px) else { return };
        let Some(buf) = props::decode_png(&png) else { return };
        deliver(gen, move |_, g| {
            if PDF_GEN.with(|x| x.get()) != pg {
                return;
            }
            IMG.with(|i| *i.borrow_mut() = None);
            g.set_img(slint::Image::from_rgba8(buf));
            g.set_has_img(true);
            g.set_pdf_pages(count as i32);
            g.set_pdf_page(page.clamp(0, count as i32 - 1));
        });
    });
}

/// Strzałki i kółko: następna albo poprzednia strona PDF.
pub fn pdf_step(s: &Shared, d: i32) {
    let mut target = None;
    with_ui(s, |g| {
        let n = g.get_pdf_pages();
        let p = (g.get_pdf_page() + d).clamp(0, (n - 1).max(0));
        if n > 1 && p != g.get_pdf_page() {
            // Numer zmienia się od razu – obraz dociąga się w tle.
            g.set_pdf_page(p);
            target = Some(p);
        }
    });
    if let Some(p) = target {
        pdf_page(GEN.with(|g| g.get()), p);
    }
}

/// Obraz z pamięci podręcznej albo zamówienie go; po dotarciu `thumbs_arrived` go podmieni.
fn image(g: &PreviewState, path: &str, px: u32, icon_only: bool) {
    IMG.with(|i| *i.borrow_mut() = Some((path.to_string(), px, icon_only)));
    let img = crate::thumbs::get(path, px, icon_only);
    g.set_has_img(img.is_some());
    g.set_img(img.unwrap_or_default());
}

/// Najwięcej znaków tekstu w panelu – dłuższe pliki i tak otwiera się w programie.
const TEXT_LIMIT: usize = 60_000;

/// Dokumenty, z których tekst wyciągamy z ich wnętrza (spakowany XML), a nie czytamy wprost.
const OFFICE_EXT: [&str; 11] = ["docx", "docm", "dotx", "xlsx", "xlsm", "pptx", "ppsx", "odt", "ods", "odp", "epub"];

fn with_ui(s: &Shared, f: impl FnOnce(&PreviewState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<PreviewState>());
    }
}

/// Wynik z tła – tylko gdy panel wciąż pokazuje to samo.
fn deliver(gen: u64, f: impl FnOnce(&Shared, &PreviewState) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if GEN.with(|g| g.get()) != gen {
            return;
        }
        crate::with_state(|s| {
            if let Some(ui) = s.borrow().ui.upgrade() {
                f(s, &ui.global::<PreviewState>());
            }
        })
    });
}

pub fn toggle(s: &Shared) {
    {
        let mut b = s.borrow_mut();
        b.prefs.preview_open = !b.prefs.preview_open;
        b.prefs.save();
    }
    SHOWN.with(|x| x.borrow_mut().clear());
    crate::player::stop_target(s, crate::player::PREVIEW);
    sync(s);
}

pub fn resize(s: &Shared, w: f32) {
    let w = w.clamp(260.0, 760.0);
    s.borrow_mut().prefs.preview_width = w;
    with_ui(s, |g| g.set_width(w));
}

/// Zapis szerokości po puszczeniu uchwytu (nie przy każdym ruchu).
pub fn resize_done(s: &Shared) {
    s.borrow().prefs.save();
    SHOWN.with(|x| x.borrow_mut().clear());
    sync(s);
}

/// Po każdej zmianie zaznaczenia albo folderu – panel nadąża z opóźnieniem 90 ms.
pub fn sync(s: &Shared) {
    let (open, width) = {
        let b = s.borrow();
        (b.prefs.preview_open, b.prefs.preview_width)
    };
    with_ui(s, |g| {
        g.set_open(open);
        g.set_width(width);
    });
    if !open {
        return;
    }
    let key = crate::tab_mut(s, |t| {
        let sel: Vec<&str> = t.items.iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.path.as_str()).collect();
        if sel.is_empty() { format!("dir|{}|{}", t.path, t.items.len()) } else { sel.join("|") }
    });
    if SHOWN.with(|x| *x.borrow() == key) {
        return;
    }
    SHOWN.with(|x| *x.borrow_mut() = key);
    let s2 = s.clone();
    TIMER.with(|t| t.start(slint::TimerMode::SingleShot, Duration::from_millis(90), move || load(&s2)));
}

fn load(s: &Shared) {
    let gen = GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    crate::player::stop_target(s, crate::player::PREVIEW);
    let (sel, dir, count) = crate::tab_mut(s, |t| (t.items.iter().filter(|e| t.selected.contains(&e.path)).cloned().collect::<Vec<_>>(), t.path.clone(), t.items.len()));
    match sel.len() {
        0 => folder(s, &dir, count),
        1 => one(s, gen, sel.into_iter().next().unwrap()),
        _ => many(s, &sel),
    }
}

fn clear(g: &PreviewState) {
    IMG.with(|i| *i.borrow_mut() = None);
    g.set_img(Default::default());
    g.set_has_img(false);
    g.set_text("".into());
    g.set_code(ModelRc::default());
    g.set_list(ModelRc::default());
    g.set_rows(ModelRc::default());
    g.set_sections(ModelRc::default());
    g.set_usage(-1.0);
    g.set_path("".into());
    g.set_has_map(false);
    g.set_media(0);
    g.set_pdf_pages(0);
    g.set_pdf_page(0);
    PDF.with(|p| *p.borrow_mut() = None);
    g.set_coords("".into());
    COORDS.with(|c| c.set(None));
}

/// Otwiera pokazany punkt w przeglądarce (OpenStreetMap).
pub fn open_map() {
    if let Some((lat, lon)) = COORDS.with(|c| c.get()) {
        let _ = open::that(maps::web_url(lat, lon));
    }
}

/// Bez zaznaczenia: folder, liczba elementów i miejsce na dysku.
fn folder(s: &Shared, dir: &str, count: usize) {
    let (title, drive) = {
        let b = s.borrow();
        let t = &b.tabs[b.active];
        let low = dir.to_lowercase();
        let d = b.drives.iter().filter(|d| low.starts_with(&d.path.to_lowercase())).max_by_key(|d| d.path.len()).cloned();
        (t.title(), d)
    };
    with_ui(s, |g| {
        clear(g);
        g.set_mode(0);
        g.set_title(title.into());
        g.set_subtitle(if dir.starts_with("ofp:") { "".into() } else { plural(count as u64, "element", "elementy", "elementów").into() });
        if let Some(d) = drive.filter(|d| d.total > 0) {
            g.set_usage(1.0 - d.free as f32 / d.total as f32);
            g.set_drive(format!("{} wolne z {}", fmt::size(d.free), fmt::size(d.total)).into());
        }
    });
}

/// Kilka elementów: ile folderów i plików, łączny rozmiar plików.
fn many(s: &Shared, sel: &[Entry]) {
    let dirs = sel.iter().filter(|e| e.dir).count() as u64;
    let files = sel.len() as u64 - dirs;
    let size: u64 = sel.iter().filter(|e| !e.dir).map(|e| e.size).sum();
    let scale = s.borrow().ui.upgrade().map(|u| u.window().scale_factor()).unwrap_or(1.0);
    with_ui(s, |g| {
        clear(g);
        g.set_mode(2);
        g.set_title(format!("Zaznaczono {}", plural(sel.len() as u64, "element", "elementy", "elementów")).into());
        g.set_subtitle(format!("{} · {} · {}", plural(dirs, "folder", "foldery", "folderów"), plural(files, "plik", "pliki", "plików"), fmt::size(size)).into());
        if let Some(e) = sel.first() {
            image(g, &e.path, (160.0 * scale) as u32, e.dir);
        }
    });
}

/// Tekst z dokumentu bywa poszatkowany na akapity z samymi spacjami – zwijamy puste linie.
fn tidy(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut blanks = 0;
    for line in src.lines() {
        let line = line.trim_end().trim_end_matches('\t');
        if line.trim().is_empty() {
            blanks += 1;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blanks > 0 {
                out.push('\n');
            }
        }
        blanks = 0;
        out.push_str(line);
    }
    // Po ostatnim slajdzie zostaje sama kreska – ucinamy ją.
    out.trim_matches(|c: char| c == '―' || c.is_whitespace()).to_string()
}

/// Najwięcej wierszy i znaków w wierszu – dalej w panelu i tak nic nie widać.
const CODE_LINES: usize = 500;
const CODE_COLS: usize = 300;

/// Pokolorowane wiersze liczone w tle (modele Slint powstają dopiero w wątku UI).
fn code_pieces(text: &str, ext: &str) -> Vec<Vec<syntax::Piece>> {
    if !syntax::supported(ext) {
        return Vec::new();
    }
    syntax::highlight(text, ext, CODE_LINES, CODE_COLS)
}

/// Wiersze z tła na model dla Slint – już w wątku UI.
fn code_lines(lines: Vec<Vec<syntax::Piece>>) -> Vec<CodeLine> {
    lines
        .into_iter()
        .map(|line| {
            let toks: Vec<CodeTok> = line.into_iter().map(|(k, t)| CodeTok { text: t.into(), kind: k as i32 }).collect();
            CodeLine { toks: ModelRc::new(VecModel::from(toks)) }
        })
        .collect()
}

fn one(s: &Shared, gen: u64, e: Entry) {
    let (scale, width) = {
        let b = s.borrow();
        (b.ui.upgrade().map(|u| u.window().scale_factor()).unwrap_or(1.0), b.prefs.preview_width)
    };
    let k = fmt::kind_of(&e);
    let packed = crate::archives::inside(&e.path);
    let ext = e.ext.to_lowercase();
    let textual = !e.dir && !e.cloud && !packed && (matches!(k, Kind::Text | Kind::Code) || ext == "csv" || (k == Kind::Other && e.size < 2_000_000));
    let document = !e.cloud && !packed && OFFICE_EXT.contains(&ext.as_str());
    let listing = !e.cloud && !packed && crate::archives::browsable(&e);
    with_ui(s, |g| {
        clear(g);
        g.set_mode(1);
        g.set_title(e.name.clone().into());
        g.set_subtitle(if e.dir { fmt::type_label(&e) } else { format!("{} · {}", fmt::type_label(&e), fmt::size(e.size)) }.into());
        g.set_path(e.path.clone().into());
        g.set_is_dir(e.dir);
        if !e.dir && !packed && !e.cloud {
            g.set_media(match k {
                Kind::Video => 1,
                Kind::Audio => 2,
                _ => 0,
            });
        }
        let mut rows = vec![row("Zmodyfikowano", fmt::date(e.modified)), row("Utworzono", fmt::date(e.created))];
        if e.dir && !packed {
            rows.insert(0, row("Zawartość", "liczę…"));
        }
        g.set_rows(ModelRc::new(VecModel::from(rows)));
        // Obraz z powłoki (miniatura w rozmiarze panelu); dla tekstu i archiwów tylko ikona.
        let px = ((width - 32.0) * scale) as u32;
        if !packed {
            image(g, &e.path, if textual || listing { (96.0 * scale) as u32 } else { px }, e.dir || textual || listing);
        }
    });
    // SVG, czcionki i modele 3D – obraz liczony w tle (Windows nie ma dla nich miniatur).
    if !packed && !e.cloud && matches!(ext.as_str(), "svg" | "ttf" | "otf" | "stl" | "obj") {
        let p = e.path.clone();
        let w = ((width - 32.0) * scale) as u32;
        let light = s.borrow().ui.upgrade().is_some_and(|u| u.global::<crate::Theme>().get_light());
        let ink = if light { [20, 20, 22] } else { [235, 235, 238] };
        let ext2 = ext.clone();
        std::thread::spawn(move || {
            let img = match ext2.as_str() {
                "svg" => powerdesk_core::render::svg(&p, w),
                "ttf" | "otf" => powerdesk_core::render::font(&p, w, ink),
                _ => powerdesk_core::render::model(&p, w, [120, 160, 230]),
            };
            let Some((iw, ih, px)) = img else { return };
            deliver(gen, move |_, g| {
                g.set_img(slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&px, iw, ih)));
                g.set_has_img(true);
            });
        });
    }
    if ext == "pdf" && !packed && !e.cloud {
        PDF.with(|p| *p.borrow_mut() = Some((e.path.clone(), ((width - 32.0) * scale) as u32)));
        pdf_page(gen, 0);
    }
    if textual {
        let p = e.path.clone();
        let ext = ext.clone();
        std::thread::spawn(move || {
            let Ok(t) = fsx::read_text(&p, TEXT_LIMIT) else { return };
            if t.kind == "binary" || t.text.is_empty() {
                return;
            }
            let text = if t.truncated { format!("{}\n\n…", t.text) } else { t.text };
            // Składnię kolorujemy jeszcze w tle – wiersze idą do UI gotowe; Markdown – sformatowany.
            let code = if ext == "md" || ext == "markdown" { syntax::markdown(&text, CODE_LINES) } else { code_pieces(&text, &ext) };
            deliver(gen, move |_, g| {
                g.set_text(text.into());
                g.set_code(ModelRc::new(VecModel::from(code_lines(code))));
            });
        });
    }
    if document {
        let p = e.path.clone();
        std::thread::spawn(move || {
            let Ok(t) = fsx::office_text(&p, TEXT_LIMIT) else { return };
            let body = tidy(&t.text);
            if body.is_empty() {
                return;
            }
            let text = if t.truncated { format!("{body}\n\n…") } else { body };
            deliver(gen, move |_, g| g.set_text(text.into()));
        });
    }
    if listing {
        let p = e.path.clone();
        std::thread::spawn(move || {
            let Ok(list) = archive::list(Path::new(&p), "") else { return };
            let rows: Vec<PropRow> = list.iter().take(300).map(|x| row(&x.name, if x.dir { String::new() } else { fmt::size(x.size) })).collect();
            deliver(gen, move |_, g| g.set_list(ModelRc::new(VecModel::from(rows))));
        });
    }
    if e.dir && !packed {
        let p = e.path.clone();
        let (m, c) = (e.modified, e.created);
        std::thread::spawn(move || {
            let d = fsx::dir_size(&p);
            let text = format!("{} w {} · {}", plural(d.files, "plik", "pliki", "plików"), plural(d.dirs, "folderze", "folderach", "folderach"), fmt::size(d.size));
            deliver(gen, move |_, g| {
                let rows = vec![row("Zawartość", text), row("Zmodyfikowano", fmt::date(m)), row("Utworzono", fmt::date(c))];
                g.set_rows(ModelRc::new(VecModel::from(rows)));
            });
        });
    }
    if !packed && !e.cloud {
        let p = e.path.clone();
        std::thread::spawn(move || {
            let meta = shell::properties(&p);
            let coords = props::gps(&meta);
            deliver(gen, move |_, g| g.set_sections(ModelRc::new(VecModel::from(sections(&meta)))));
            // Mapa dochodzi osobno – kafelki trzeba pobrać z sieci.
            let Some((lat, lon)) = coords else { return };
            deliver(gen, move |_, g| {
                COORDS.with(|c| c.set(Some((lat, lon))));
                g.set_coords(format!("{lat:.5}, {lon:.5}").into());
            });
            let Some(buf) = maps::map_png(lat, lon, 15, 680, 300).and_then(|png| props::decode_png(&png)) else { return };
            deliver(gen, move |_, g| {
                g.set_map(slint::Image::from_rgba8(buf));
                g.set_has_map(true);
            });
        });
    }
}

/// Dotarła miniatura – jeśli to obraz panelu, podmieniamy tylko jego (bez wczytywania reszty od nowa).
pub fn thumbs_arrived(s: &Shared, paths: &std::collections::HashSet<String>) {
    let Some((path, px, icon_only)) = IMG.with(|i| i.borrow().clone()) else { return };
    if paths.contains(&path) {
        with_ui(s, |g| image(g, &path, px, icon_only));
    }
}

pub fn open_item(s: &Shared) {
    let mut path = String::new();
    with_ui(s, |g| path = g.get_path().into());
    if let Some(i) = crate::tab_mut(s, |t| t.items.iter().position(|e| e.path == path)) {
        crate::open(s, i);
    }
}

pub fn open_with(s: &Shared) {
    let mut path = String::new();
    with_ui(s, |g| path = g.get_path().into());
    if !path.is_empty() {
        shell::open_with(crate::hwnd(s), &path);
    }
}

pub fn copy_path(s: &Shared) {
    let mut path = String::new();
    with_ui(s, |g| path = g.get_path().into());
    crate::props::copy(s, &path);
}
