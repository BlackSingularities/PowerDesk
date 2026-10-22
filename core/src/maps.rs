//! Mapa do właściwości i podglądu zdjęć z GPS: kafelki OpenStreetMap zszyte w jeden obrazek,
//! wyśrodkowany na podanym punkcie. Kafelki trafiają do pamięci podręcznej na dysku,
//! żeby te same okolice nie były pobierane dwa razy.

use std::io::Read;
use std::path::PathBuf;

const TILE: u32 = 256;
const AGENT: &str = concat!("PowerDesk/", env!("CARGO_PKG_VERSION"), " (menedżer plików dla Windows)");

/// Adres strony z mapą dla tego punktu – do otwarcia w przeglądarce.
pub fn web_url(lat: f64, lon: f64) -> String {
    format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=16/{lat:.6}/{lon:.6}")
}

fn cache_dir() -> Option<PathBuf> {
    let d = dirs::cache_dir()?.join("PowerDesk").join("tiles");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn tile_bytes(z: u32, x: u32, y: u32) -> Option<Vec<u8>> {
    let file = cache_dir().map(|d| d.join(format!("{z}_{x}_{y}.png")));
    if let Some(f) = &file {
        if let Ok(b) = std::fs::read(f) {
            if !b.is_empty() {
                return Some(b);
            }
        }
    }
    let url = format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png");
    let res = ureq::get(&url).set("User-Agent", AGENT).timeout(std::time::Duration::from_secs(8)).call().ok()?;
    let mut buf = Vec::new();
    res.into_reader().take(2_000_000).read_to_end(&mut buf).ok()?;
    if buf.is_empty() {
        return None;
    }
    if let Some(f) = &file {
        let _ = std::fs::write(f, &buf);
    }
    Some(buf)
}

/// Mapa `w` × `h` pikseli wyśrodkowana na punkcie, jako PNG. `None`, gdy nie ma sieci.
pub fn map_png(lat: f64, lon: f64, zoom: u32, w: u32, h: u32) -> Option<Vec<u8>> {
    let zoom = zoom.clamp(1, 18);
    let n = (1u64 << zoom) as f64;
    let lat = lat.clamp(-85.0, 85.0);
    // Globalne współrzędne w pikselach (Web Mercator).
    let gx = (lon + 180.0) / 360.0 * n * TILE as f64;
    let r = lat.to_radians();
    let gy = (1.0 - ((r.tan() + 1.0 / r.cos()).ln()) / std::f64::consts::PI) / 2.0 * n * TILE as f64;
    let (left, top) = (gx - w as f64 / 2.0, gy - h as f64 / 2.0);
    let (tx0, ty0) = ((left / TILE as f64).floor() as i64, (top / TILE as f64).floor() as i64);
    let (tx1, ty1) = (((left + w as f64) / TILE as f64).floor() as i64, ((top + h as f64) / TILE as f64).floor() as i64);
    let span = ((tx1 - tx0 + 1) * (ty1 - ty0 + 1)) as u32;
    if span > 16 {
        return None;
    }
    let cw = (tx1 - tx0 + 1) as u32 * TILE;
    let ch = (ty1 - ty0 + 1) as u32 * TILE;
    let mut canvas = image::RgbaImage::from_pixel(cw, ch, image::Rgba([235, 235, 235, 255]));
    let max = (1i64 << zoom) - 1;
    let mut got = false;
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            if ty < 0 || ty > max {
                continue;
            }
            // W poziomie świat się zawija.
            let wx = tx.rem_euclid(max + 1) as u32;
            let Some(bytes) = tile_bytes(zoom, wx, ty as u32) else { continue };
            let Ok(img) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png) else { continue };
            image::imageops::overlay(&mut canvas, &img.to_rgba8(), (tx - tx0) * TILE as i64, (ty - ty0) * TILE as i64);
            got = true;
        }
    }
    if !got {
        return None;
    }
    let ox = (left - tx0 as f64 * TILE as f64).max(0.0) as u32;
    let oy = (top - ty0 as f64 * TILE as f64).max(0.0) as u32;
    let cut = image::imageops::crop_imm(&canvas, ox, oy, w.min(cw - ox), h.min(ch - oy)).to_image();
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(cut).write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}
