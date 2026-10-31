//! Ustawienia pojedynczego widżetu („Dostosuj…”): zamiana zapisu (`WidgetCfg`) na `WCfg` dla
//! Slint, miejsca danych wg źródła (strefa, miejscowość, folder – tablice `clk`, `wx`, `folders`)
//! i panel ustawień rysowany w oknie pulpitu albo Startu (globalny `WidgetEdit`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::model::WidgetCfg;
use crate::starttiles::At;
use crate::{WCfg, WidgetEdit};

/// Edytowany widżet: na pulpicie (okno i miejsce na liście w ustawieniach) albo kafelek Startu.
#[derive(Clone, Copy, PartialEq)]
pub enum Target {
    Desk { win: usize, index: usize },
    Start(At),
}

thread_local! {
    /// Znane źródła każdej grupy; miejsce na liście to `WCfg.slot` (0 – domyślne).
    static SLOTS: RefCell<HashMap<&'static str, Vec<String>>> = RefCell::new(HashMap::new());
    static TARGET: Cell<Option<Target>> = const { Cell::new(None) };
}

/// Grupa źródła danych rodzaju: strefa, miejscowość albo folder.
pub fn group(kind: &str) -> Option<&'static str> {
    match kind {
        "clock" | "analog" => Some("zone"),
        "weather" | "forecast" => Some("city"),
        "folder" => Some("folder"),
        _ => None,
    }
}

/// Klucz źródła: strefa – nazwa strefy (pusto – lokalna, także przy nieznanej), reszta – jak wpisana.
pub fn key(kind: &str, cfg: &WidgetCfg) -> String {
    let src = cfg.source.trim();
    match group(kind) {
        Some("zone") if !src.is_empty() => crate::widgetfeeds::zone_of(src).unwrap_or_default(),
        Some(_) => src.to_string(),
        None => String::new(),
    }
}

/// Miejsce danych widżetu w tablicy jego grupy (nowe źródło dopisuje się na końcu).
pub fn slot(kind: &str, cfg: &WidgetCfg) -> i32 {
    let Some(g) = group(kind) else { return 0 };
    let k = key(kind, cfg);
    SLOTS.with(|s| {
        let mut s = s.borrow_mut();
        let list = s.entry(g).or_insert_with(|| vec![String::new()]);
        match list.iter().position(|x| *x == k) {
            Some(i) => i as i32,
            None => {
                list.push(k);
                list.len() as i32 - 1
            }
        }
    })
}

/// Źródła grupy w kolejności miejsc (zawsze z domyślnym na początku).
pub fn sources(g: &'static str) -> Vec<String> {
    SLOTS.with(|s| s.borrow_mut().entry(g).or_insert_with(|| vec![String::new()]).clone())
}

fn hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn to_hex(c: Color) -> String {
    if c.alpha() == 0 {
        return String::new();
    }
    format!("#{:02x}{:02x}{:02x}", c.red(), c.green(), c.blue())
}

/// Ustawienia dla Slint (z miejscem danych wg źródła).
pub fn to_slint(kind: &str, cfg: &WidgetCfg) -> WCfg {
    WCfg {
        accent: hex(&cfg.accent).unwrap_or(Color::from_argb_u8(0, 0, 0, 0)),
        alpha: (cfg.alpha / 100.0).clamp(0.0, 1.0),
        style: cfg.style as i32,
        variant: cfg.variant as i32,
        h12: cfg.h12,
        seconds: cfg.seconds,
        fahrenheit: cfg.fahrenheit,
        title: cfg.title.clone().into(),
        slot: slot(kind, cfg),
    }
}

/// Opcje panelu dla rodzaju: układy, format czasu, jednostka temperatury, źródło (0 – brak,
/// 1 – tekst, 2 – folder) z opisem i podpowiedzią.
struct Opts {
    variants: &'static [&'static str],
    time: bool,
    temp: bool,
    src: (i32, &'static str, &'static str),
}

fn opts(kind: &str) -> Opts {
    let none = (0, "", "");
    let zone = (1, crate::tr("Strefa czasowa"), "Lokalna – np. Tokio, Londyn, Europe/Paris");
    let city = (1, crate::tr("Miejscowość"), crate::tr("Wspólna z menu widżetu – np. Kraków"));
    let o = |variants: &'static [&'static str]| Opts { variants, time: false, temp: false, src: none };
    match kind {
        "clock" => Opts { variants: &[crate::tr("Klasyczny"), crate::tr("Minimalny"), crate::tr("Pionowy")], time: true, temp: false, src: zone },
        "analog" => Opts { variants: &[crate::tr("Kreski"), crate::tr("Minimalny"), crate::tr("Cyfry")], time: true, temp: false, src: zone },
        "weather" => Opts { variants: &[crate::tr("Pełny"), crate::tr("Duża temperatura")], time: false, temp: true, src: city },
        "forecast" => Opts { variants: &[crate::tr("Dni"), crate::tr("Wykres 24 h")], time: false, temp: true, src: city },
        "folder" => Opts { variants: &[crate::tr("Lista"), crate::tr("Siatka")], time: false, temp: false, src: (2, crate::tr("Folder"), crate::tr("Pobrane")) },
        "calendar" => o(&[crate::tr("Miesiąc"), crate::tr("Dzień"), crate::tr("Tydzień")]),
        "system" | "drives" | "progress" => o(&[crate::tr("Paski"), crate::tr("Kręgi")]),
        "cpu" | "ram" | "gpu" => o(&[crate::tr("Wykres"), crate::tr("Słupki"), crate::tr("Krąg")]),
        "network" => o(&[crate::tr("Wykres"), crate::tr("Liczby")]),
        "battery" => o(&[crate::tr("Krąg"), crate::tr("Bateria")]),
        "media" => o(&[crate::tr("Okładka obok"), crate::tr("Okładka w tle")]),
        "photos" | "daypic" => o(&[crate::tr("Wypełnij"), crate::tr("Dopasuj")]),
        "recent" | "favorites" => o(&[crate::tr("Lista"), crate::tr("Siatka")]),
        "launcher" => o(&[crate::tr("Siatka"), crate::tr("Lista")]),
        "stopwatch" | "timer" => o(&[crate::tr("Krąg"), crate::tr("Cyfry")]),
        "quote" => o(&[crate::tr("Cytat"), crate::tr("Przysłowie")]),
        _ => o(&[]),
    }
}

/// Rodzaj i ustawienia edytowanego widżetu.
fn read(t: Target) -> Option<(String, WidgetCfg)> {
    let mut out = None;
    crate::with_state(|s| {
        let b = s.borrow();
        out = match t {
            Target::Desk { index, .. } => b.prefs.desktop_widgets.get(index).map(|w| (w.kind.clone(), w.cfg.clone())),
            Target::Start(at) => crate::starttiles::get(&b.prefs.start_groups, at).map(|p| (p.widget.clone(), p.cfg.clone())),
        };
    });
    out
}

/// Zapis ustawień i przerysowanie widżetów (nowe źródło dostaje miejsce przy rysowaniu).
fn write(t: Target, cfg: WidgetCfg) {
    match t {
        Target::Desk { index, .. } => {
            crate::with_state(|s| {
                let mut b = s.borrow_mut();
                if let Some(w) = b.prefs.desktop_widgets.get_mut(index) {
                    w.cfg = cfg;
                }
                b.prefs.save();
            });
            crate::desktop::refresh_widgets();
        }
        Target::Start(at) => crate::start::edit_groups(|gs| {
            if let Some(p) = crate::starttiles::get_mut(gs, at) {
                p.cfg = cfg;
            }
        }),
    }
    crate::widgetlive::sources_changed();
    crate::widgets::push_all();
}

fn with_editor(t: Target, f: impl Fn(&WidgetEdit)) {
    match t {
        Target::Desk { win, .. } => crate::desktop::with_window(win, |w| f(&w.global::<WidgetEdit>())),
        Target::Start(_) => crate::start::each_window(|w| f(&w.global::<WidgetEdit>())),
    }
}

fn fill(g: &WidgetEdit, kind: &str, cfg: &WidgetCfg) {
    let o = opts(kind);
    g.set_name(crate::widgets::name(kind).into());
    g.set_accent(hex(&cfg.accent).unwrap_or(Color::from_argb_u8(0, 0, 0, 0)));
    g.set_alpha((cfg.alpha / 100.0).clamp(0.0, 1.0));
    g.set_style(cfg.style as i32);
    g.set_variant((cfg.variant as usize).min(o.variants.len().saturating_sub(1)) as i32);
    g.set_variants(ModelRc::new(VecModel::from(o.variants.iter().map(|v| SharedString::from(*v)).collect::<Vec<_>>())));
    g.set_has_time(o.time);
    g.set_h12(cfg.h12);
    g.set_seconds(cfg.seconds);
    g.set_has_temp(o.temp);
    g.set_fahrenheit(cfg.fahrenheit);
    g.set_title(cfg.title.clone().into());
    g.set_src_kind(o.src.0);
    g.set_src_label(o.src.1.into());
    let hint = match kind {
        "weather" | "forecast" => powerdesk_core::trf!("Wspólna: {}", crate::widgetfeeds::city()),
        "folder" => dirs::download_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| o.src.2.to_string()),
        _ => o.src.2.to_string(),
    };
    g.set_src_hint(hint.into());
    g.set_src(cfg.source.clone().into());
    let note = if group(kind) == Some("zone") && !cfg.source.trim().is_empty() && key(kind, cfg).is_empty() {
        crate::tr("Nieznana strefa – pokazuję czas lokalny")
    } else {
        ""
    };
    g.set_src_note(note.into());
}

/// Otwiera panel dla widżetu `t` przy punkcie (x, y) okna; `size` – rozmiar okna (logiczny).
pub fn open(t: Target, x: f32, y: f32, size: (f32, f32)) {
    let Some((kind, cfg)) = read(t) else { return };
    if let Some(old) = TARGET.with(|c| c.get()) {
        if old != t {
            with_editor(old, |g| g.set_open(false));
        }
    }
    TARGET.with(|c| c.set(Some(t)));
    let px = x.min(size.0 - 312.0).max(8.0);
    let py = y.min(size.1 - 600.0).max(8.0);
    with_editor(t, |g| {
        fill(g, &kind, &cfg);
        g.set_x(px);
        g.set_y(py);
        g.set_open(true);
    });
}

fn changed() {
    let Some(t) = TARGET.with(|c| c.get()) else { return };
    let Some((kind, mut cfg)) = read(t) else { return };
    let got = RefCell::new(None);
    with_editor(t, |g| {
        *got.borrow_mut() = Some((g.get_accent(), g.get_alpha(), g.get_style(), g.get_variant(), g.get_h12(), g.get_seconds(), g.get_fahrenheit(), g.get_title().to_string(), g.get_src().to_string()));
    });
    let Some((accent, alpha, style, variant, h12, seconds, fahrenheit, title, src)) = got.into_inner() else { return };
    cfg.accent = to_hex(accent);
    cfg.alpha = (alpha * 100.0).round().clamp(0.0, 100.0);
    cfg.style = style.clamp(0, 3) as u8;
    cfg.variant = variant.max(0) as u8;
    cfg.h12 = h12;
    cfg.seconds = seconds;
    cfg.fahrenheit = fahrenheit;
    cfg.title = title.trim().to_string();
    cfg.source = src.trim().to_string();
    write(t, cfg.clone());
    with_editor(t, |g| fill(g, &kind, &cfg));
}

fn reset() {
    let Some(t) = TARGET.with(|c| c.get()) else { return };
    let Some((kind, _)) = read(t) else { return };
    let cfg = WidgetCfg::default();
    write(t, cfg.clone());
    with_editor(t, |g| fill(g, &kind, &cfg));
}

fn close() {
    if let Some(t) = TARGET.with(|c| c.take()) {
        with_editor(t, |g| g.set_open(false));
    }
}

fn pick_folder() {
    let Some(t) = TARGET.with(|c| c.get()) else { return };
    let Some((_, cfg)) = read(t) else { return };
    let cur = if cfg.source.is_empty() { dirs::download_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default() } else { cfg.source.clone() };
    let mut picked = None;
    crate::with_state(|s| picked = powerdesk_core::shell::pick_folder(crate::hwnd(s), crate::tr("Folder widżetu"), Some(cur.as_str())));
    if let Some(p) = picked {
        with_editor(t, |g| g.set_src(p.clone().into()));
        changed();
    }
}

/// Podpięcie panelu w nowym oknie (pulpit, Start).
pub fn wire(g: &WidgetEdit) {
    g.on_changed(changed);
    g.on_reset(reset);
    g.on_close(close);
    g.on_pick_folder(pick_folder);
}

/// Start zamknięty – edycja kafelka się kończy.
pub fn start_closed() {
    if matches!(TARGET.with(|c| c.get()), Some(Target::Start(_))) {
        TARGET.with(|c| c.set(None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_round_trip() {
        assert_eq!(to_hex(hex("#4f8cff").unwrap()), "#4f8cff");
        assert!(hex("motyw").is_none());
    }

    #[test]
    fn slots_by_source() {
        let mut a = WidgetCfg::default();
        assert_eq!(slot("weather", &a), 0);
        a.source = "Gdańsk".into();
        let s = slot("weather", &a);
        assert!(s > 0);
        assert_eq!(slot("forecast", &a), s);
        assert_eq!(slot("note", &a), 0);
    }
}
