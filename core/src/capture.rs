//! Zrzuty ekranu w trybie powłoki (bez Eksploratora skróty PrtScn i Win+Shift+S nie mają kto
//! obsłużyć): wycinek – systemowe narzędzie Wycinanie (`ms-screenclip:`), cały ekran – zapis
//! do Obrazy\Zrzuty ekranu i do schowka, jak Win+PrtScn w Windows.

use std::path::PathBuf;

use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};

/// Wycinek ekranu: systemowe Wycinanie (zaznaczenie obszaru, okna albo całości).
/// Gdy go nie ma – zrzut całego ekranu.
pub fn snip() -> Result<Option<PathBuf>, String> {
    if open::that_detached("ms-screenclip:").is_ok() {
        return Ok(None);
    }
    full().map(Some)
}

/// Piksele całego pulpitu wirtualnego (wszystkie monitory): szerokość, wysokość, RGBA.
fn grab() -> Result<(u32, u32, Vec<u8>), String> {
    unsafe {
        let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (w, h) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        if w <= 0 || h <= 0 {
            return Err("brak ekranu".into());
        }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let ok = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT).is_ok();
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let got = ok && GetDIBits(mem, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS) != 0;
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if !got {
            return Err("nie udało się odczytać ekranu".into());
        }
        // BGRA → RGBA, pełna nieprzezroczystość.
        for p in px.chunks_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        Ok((w as u32, h as u32, px))
    }
}

/// Zrzut całego ekranu: plik PNG w Obrazy\Zrzuty ekranu (nazwa jak w Windows) i obraz w schowku.
pub fn full() -> Result<PathBuf, String> {
    let (w, h, px) = grab()?;
    let img = image::RgbaImage::from_raw(w, h, px).ok_or("zły rozmiar obrazu")?;
    let dir = dirs::picture_dir().unwrap_or_else(std::env::temp_dir).join("Screenshots");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = chrono::Local::now().format("Zrzut ekranu %Y-%m-%d %H%M%S.png").to_string();
    let path = dir.join(name);
    img.save(&path).map_err(|e| e.to_string())?;
    // Do schowka jako bitmapa (BMP bez nagłówka pliku – tak ją przyjmuje CF_DIB).
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    let mut bmp = Vec::new();
    if image::codecs::bmp::BmpEncoder::new(&mut bmp).encode(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8).is_ok() {
        let _ = clipboard_win::set_clipboard(clipboard_win::formats::Bitmap, &bmp);
    }
    Ok(path)
}
