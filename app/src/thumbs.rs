//! Miniatury i ikony z systemowej pamięci podręcznej Windows (`shell::request`).
//!
//! Obrazy (`slint::Image`) żyją tylko w wątku UI, więc pamięć podręczna jest lokalna dla wątku.
//! Dekodowanie odbywa się w puli `shell`, a gotowe piksele wracają przez `invoke_from_event_loop`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use powerdesk_core::shell;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Rozmiary zaokrąglamy w górę do kroku, żeby zmiana wielkości kafelków nie pobierała miniatur od nowa.
fn bucket(px: u32) -> u32 {
    match px {
        0..=24 => px.max(16),
        25..=64 => 64,
        _ => px.div_ceil(96) * 96,
    }
    .min(2560)
}

const LIMIT: usize = 3000;

#[derive(Default)]
struct Cache {
    map: HashMap<(String, u32, bool), Image>,
    order: VecDeque<(String, u32, bool)>,
    pending: HashSet<(String, u32, bool)>,
    missing: HashSet<(String, u32, bool)>,
    /// Nieudane próby – powłoka przy starcie bywa zajęta i chwilowo nie oddaje ikony.
    failed: HashMap<(String, u32, bool), u8>,
    redraw: bool,
    /// Ścieżki, których obrazy dotarły od ostatniego przerysowania.
    arrived: HashSet<String>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Zapomina obrazy pliku (np. ikona Kosza pustego / pełnego) – następne `get` pobierze nowe.
pub fn invalidate(path: &str) {
    shell::invalidate(path);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.map.retain(|k, _| k.0 != path);
        c.order.retain(|k| k.0 != path);
        c.missing.retain(|k| k.0 != path);
        c.failed.retain(|k, _| k.0 != path);
    });
}

/// Miniatura, jeśli już jest; w przeciwnym razie zamawia ją i zwraca `None`
/// (po dotarciu widok przerysuje się sam).
pub fn get(path: &str, px: u32, icon_only: bool) -> Option<Image> {
    let key = (path.to_string(), bucket(px), icon_only);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(img) = c.map.get(&key) {
            return Some(img.clone());
        }
        if c.missing.contains(&key) || !c.pending.insert(key.clone()) {
            return None;
        }
        let k = key.clone();
        shell::request(shell::ThumbReq {
            path: key.0.clone(),
            size: key.1,
            icon_only,
            reply: Box::new(move |res| {
                let buf = res.and_then(|(data, _)| image::load_from_memory(&data).ok()).map(|img| {
                    let rgba = img.to_rgba8();
                    SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height())
                });
                let _ = slint::invoke_from_event_loop(move || arrived(k, buf));
            }),
        });
        None
    })
}

fn arrived(key: (String, u32, bool), buf: Option<SharedPixelBuffer<Rgba8Pixel>>) {
    let schedule = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.pending.remove(&key);
        c.arrived.insert(key.0.clone());
        match buf {
            Some(b) => {
                c.map.insert(key.clone(), Image::from_rgba8(b));
                c.order.push_back(key);
                while c.order.len() > LIMIT {
                    if let Some(old) = c.order.pop_front() {
                        c.map.remove(&old);
                    }
                }
            }
            None => {
                let n = c.failed.entry(key.clone()).or_default();
                *n += 1;
                let n = *n;
                if n >= 5 {
                    c.missing.insert(key);
                } else if key.2 {
                    // Ikona: powłoka bywa chwilowo zajęta (start systemu) – kolejna próba za chwilę,
                    // za każdym razem dłuższą; widoki odświeżają się i pytają ponownie.
                    let path = key.0.clone();
                    c.arrived.remove(&path);
                    slint::Timer::single_shot(std::time::Duration::from_millis(1500 * n as u64), move || {
                        let set = HashSet::from([path]);
                        crate::picker::thumbs_arrived(&set);
                        crate::with_state(|s| crate::thumbs_arrived(s, &set));
                    });
                }
            }
        }
        // Wiele miniatur naraz – jedno przerysowanie na klatkę.
        !std::mem::replace(&mut c.redraw, true)
    });
    if schedule {
        slint::Timer::single_shot(std::time::Duration::from_millis(16), || {
            let paths = CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.redraw = false;
                std::mem::take(&mut c.arrived)
            });
            // Tylko wiersze z nowymi obrazami – bez przebudowy całej listy i panelu.
            crate::picker::thumbs_arrived(&paths);
            crate::with_state(|s| crate::thumbs_arrived(s, &paths));
        });
    }
}
