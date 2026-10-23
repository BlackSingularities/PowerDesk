//! Tapety pulpitu PowerDesk: z Windows (domyślnie), pokaz slajdów z folderu (każdy monitor
//! inne zdjęcie, zmiana co N minut), zdjęcie dnia z Binga albo własny obraz dla wybranego
//! monitora. Rysuje je pulpit PowerDesk – tapeta Windows (i ekran blokady) zostaje bez zmian.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Ustawienia tapet w chwili odczytu (do wątku w tle).
#[derive(Clone, Default)]
pub struct Config {
    mode: String,
    folder: String,
    per_monitor: HashMap<usize, String>,
    pos: usize,
}

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    /// Przesunięcie w pokazie slajdów.
    static POS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub fn config() -> Config {
    let mut c = Config { pos: POS.with(|p| p.get()), ..Default::default() };
    crate::with_state(|s| {
        let b = s.borrow();
        c.mode = b.prefs.wallpaper_mode.clone();
        c.folder = b.prefs.wallpaper_folder.clone();
        c.per_monitor = b.prefs.wallpaper_monitors.clone();
    });
    c
}

impl Config {
    pub fn is_bing(&self) -> bool {
        self.mode == "bing"
    }

    /// Zdjęcia do pokazu slajdów (tylko w tym trybie; wołać w tle).
    pub fn images(&self) -> Vec<PathBuf> {
        if self.mode != "slideshow" || self.folder.is_empty() {
            return Vec::new();
        }
        let mut v = crate::widgetfeeds::scan(&self.folder);
        v.sort();
        v
    }

    /// Tapeta okna pulpitu `n` (kolejność okien: główny monitor pierwszy) i czy to nie tapeta
    /// Windows (wtedy zawsze „wypełnij”). `bing` – dzisiejsze zdjęcie, jeśli tryb Bing.
    pub fn choose(&self, n: usize, default: &str, images: &[PathBuf], bing: Option<&PathBuf>) -> (String, bool) {
        if let Some(p) = self.per_monitor.get(&n).filter(|p| std::path::Path::new(p).is_file()) {
            return (p.clone(), true);
        }
        match self.mode.as_str() {
            "slideshow" if !images.is_empty() => (images[(self.pos + n) % images.len()].to_string_lossy().to_string(), true),
            "bing" => match bing {
                Some(b) => (b.to_string_lossy().to_string(), true),
                None => (default.to_string(), false),
            },
            _ => (default.to_string(), false),
        }
    }
}

fn decode(path: &str) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::open(path).ok()?.to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

/// Tapety wszystkich okien pulpitu od nowa (zmiana ustawień, kolejne zdjęcie pokazu).
pub fn refresh() {
    let defaults = crate::desktop::wallpaper_defaults();
    if defaults.is_empty() {
        return;
    }
    let cfg = config();
    std::thread::spawn(move || {
        let images = cfg.images();
        let bing = if cfg.mode == "bing" { powerdesk_core::bing::today() } else { None };
        for (n, def) in defaults.iter().enumerate() {
            let (path, own) = cfg.choose(n, def, &images, bing.as_ref());
            let buf = if path.is_empty() { None } else { decode(&path) };
            let _ = slint::invoke_from_event_loop(move || crate::desktop::set_wallpaper(n, buf, own));
        }
    });
}

/// Zegar pokazu slajdów i zdjęcia dnia (co godzinę sprawdza, czy jest już nowe).
pub fn start_timer() {
    let minutes = {
        let mut m = 15;
        crate::with_state(|s| m = s.borrow().prefs.wallpaper_minutes.max(1));
        m
    };
    let mode = config().mode;
    TIMER.with(|t| match mode.as_str() {
        "slideshow" => t.start(slint::TimerMode::Repeated, Duration::from_secs(minutes as u64 * 60), next),
        "bing" => t.start(slint::TimerMode::Repeated, Duration::from_secs(3600), refresh),
        _ => t.stop(),
    });
}

/// Następne zdjęcia pokazu slajdów.
pub fn next() {
    POS.with(|p| p.set(p.get().wrapping_add(1)));
    refresh();
}

/// Własny obraz dla monitora `n` albo powrót do ustawienia ogólnego (None).
pub fn set_for_monitor(n: usize, path: Option<String>) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        match path {
            Some(p) => {
                b.prefs.wallpaper_monitors.insert(n, p);
            }
            None => {
                b.prefs.wallpaper_monitors.remove(&n);
            }
        }
        b.prefs.save();
    });
    refresh();
}

pub fn has_own(n: usize) -> bool {
    config().per_monitor.contains_key(&n)
}

/// Tryb z Ustawień: "" – Windows, "slideshow", "bing".
pub fn set_mode(mode: &str) {
    crate::with_state(|s| {
        let mut b = s.borrow_mut();
        b.prefs.wallpaper_mode = mode.to_string();
        b.prefs.save();
    });
    start_timer();
    refresh();
}
