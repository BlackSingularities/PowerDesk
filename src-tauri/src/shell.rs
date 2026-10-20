//! Integracja z powłoką Windows: miniatury i ikony z systemowego cache (thumbcache),
//! metadane z systemu właściwości, okno „Właściwości", „Otwórz za pomocą".
//! Aplikacja nie generuje i nie zapisuje własnych miniatur – wszystko pochodzi z Shella.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, PROPERTYKEY, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyStore, PSFormatForDisplayAlloc, PSGetNameFromPropertyKey, PSGetPropertyDescription, IPropertyDescription,
    SHGetPropertyStoreFromParsingName, GPS_BESTEFFORT, PDFF_DEFAULT,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SHObjectProperties, SHOpenWithDialog,
    OAIF_EXEC, OAIF_ALLOW_REGISTRATION, OPENASINFO, SHOP_FILEPATH, SIIGBF_BIGGERSIZEOK,
    SIIGBF_ICONONLY, SIIGBF_THUMBNAILONLY,
};

pub struct ThumbReq {
    pub path: String,
    pub size: u32,
    pub icon_only: bool,
    pub reply: Box<dyn FnOnce(Option<(Vec<u8>, &'static str)>) + Send>,
}

struct Pool {
    hi: Sender<ThumbReq>,
    /// Ikony mają pierwszeństwo przed miniaturami – są tanie, a bez nich widok wygląda na pusty.
    icons: Sender<ThumbReq>,
}

/// Pamięć podręczna tylko w RAM (zakodowane obrazy), żeby przewijanie było płynne.
struct MemCache {
    map: HashMap<String, Arc<(Vec<u8>, &'static str)>>,
    order: VecDeque<String>,
    bytes: usize,
}

const CACHE_LIMIT: usize = 160 * 1024 * 1024;

static CACHE: Lazy<Mutex<MemCache>> =
    Lazy::new(|| Mutex::new(MemCache { map: HashMap::new(), order: VecDeque::new(), bytes: 0 }));

static POOL: Lazy<Pool> = Lazy::new(|| {
    let (tx, rx) = unbounded::<ThumbReq>();
    let (itx, irx) = unbounded::<ThumbReq>();
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 6);
    for _ in 0..workers {
        let rx: Receiver<ThumbReq> = rx.clone();
        let irx: Receiver<ThumbReq> = irx.clone();
        std::thread::Builder::new()
            .name("thumb".into())
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                loop {
                    // Najpierw zaległe ikony, dopiero potem miniatury.
                    let req = match irx.try_recv() {
                        Ok(r) => r,
                        Err(_) => crossbeam_channel::select! {
                            recv(irx) -> r => match r { Ok(r) => r, Err(_) => break },
                            recv(rx) -> r => match r { Ok(r) => r, Err(_) => break },
                        },
                    };
                    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        render(&req.path, req.size, req.icon_only)
                    }))
                    .ok()
                    .flatten();
                    (req.reply)(out);
                }
            })
            .ok();
    }
    Pool { hi: tx, icons: itx }
});

fn cache_key(path: &str, size: u32, icon_only: bool) -> String {
    // Ikony zwykłych plików są wspólne dla rozszerzenia – jedna na typ zamiast tysięcy.
    if icon_only {
        let lower = path.to_lowercase();
        let ext = std::path::Path::new(&lower)
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        let p = std::path::Path::new(path);
        // Zwykłe foldery mają tę samą ikonę – jedna dla wszystkich. Własną mają tylko te z desktop.ini
        // (Pobrane, Dokumenty, foldery z ustawioną ikoną) oraz dyski i udziały sieciowe.
        if lower.len() > 3 && !lower.starts_with("\\\\") && p.is_dir() {
            return if p.join("desktop.ini").exists() { format!("i{size}:{path}") } else { format!("i{size}:<folder>") };
        }
        let per_file = matches!(ext.as_str(), "exe" | "lnk" | "ico" | "url" | "msi" | "appref-ms" | "")
            || lower.len() <= 3;
        if !per_file {
            return format!("i{size}:.{ext}");
        }
    }
    format!("{}{size}:{path}", if icon_only { "i" } else { "t" })
}

pub fn request(req: ThumbReq) {
    let key = cache_key(&req.path, req.size, req.icon_only);
    if let Some(hit) = CACHE.lock().map.get(&key).cloned() {
        (req.reply)(if hit.0.is_empty() { None } else { Some((hit.0.clone(), hit.1)) });
        return;
    }
    let reply = req.reply;
    let k2 = key.clone();
    let wrapped = ThumbReq {
        path: req.path,
        size: req.size,
        icon_only: req.icon_only,
        reply: Box::new(move |res| {
            // Brak miniatury też zapamiętujemy, żeby nie pytać Shella w kółko.
            let entry = res.clone().unwrap_or((Vec::new(), "none"));
            {
                let (data, mime) = (&entry.0, entry.1);
                let mut c = CACHE.lock();
                if !c.map.contains_key(&k2) {
                    c.bytes += data.len();
                    c.map.insert(k2.clone(), Arc::new((data.clone(), mime)));
                    c.order.push_back(k2);
                    while c.bytes > CACHE_LIMIT {
                        if let Some(old) = c.order.pop_front() {
                            if let Some(v) = c.map.remove(&old) {
                                c.bytes -= v.0.len();
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
            reply(res)
        }),
    };
    let _ = if wrapped.icon_only { POOL.icons.send(wrapped) } else { POOL.hi.send(wrapped) };
}

pub fn invalidate(path: &str) {
    let mut c = CACHE.lock();
    let keys: Vec<String> = c.map.keys().filter(|k| k.ends_with(path)).cloned().collect();
    for k in keys {
        if let Some(v) = c.map.remove(&k) {
            c.bytes -= v.0.len();
        }
        c.order.retain(|o| o != &k);
    }
}

fn render(path: &str, size: u32, icon_only: bool) -> Option<(Vec<u8>, &'static str)> {
    unsafe {
        let p = HSTRING::from(path);
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&p, None).ok()?;
        let sz = SIZE { cx: size as i32, cy: size as i32 };
        let hbmp = if icon_only {
            factory.GetImage(sz, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?
        } else {
            factory.GetImage(sz, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK).ok()?
        };
        let res = encode_hbitmap(hbmp);
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        res
    }
}

unsafe fn encode_hbitmap(hbmp: HBITMAP) -> Option<(Vec<u8>, &'static str)> {
    let mut bm = BITMAP::default();
    if GetObjectW(HGDIOBJ(hbmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let dc = CreateCompatibleDC(None);
    let lines = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
    let _ = DeleteDC(dc);
    if lines == 0 {
        return None;
    }
    let mut any_alpha = false;
    let mut translucent = false;
    for px in buf.chunks_exact(4) {
        if px[3] != 0 {
            any_alpha = true;
        }
        if px[3] != 255 {
            translucent = true;
        }
    }
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        if !any_alpha {
            px[3] = 255;
        } else if px[3] > 0 && px[3] < 255 {
            // Shell zwraca alfę premnożoną – cofamy to dla PNG.
            let a = px[3] as u32;
            for c in 0..3 {
                px[c] = ((px[c] as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let img = image::RgbaImage::from_raw(w as u32, h as u32, buf)?;
    let mut out = Vec::with_capacity((w * h) as usize / 2);
    if any_alpha && translucent {
        image::DynamicImage::ImageRgba8(img)
            .write_with_encoder(image::codecs::png::PngEncoder::new_with_quality(
                &mut out,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Adaptive,
            ))
            .ok()?;
        Some((out, "image/png"))
    } else {
        let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88)
            .encode_image(&rgb)
            .ok()?;
        Some((out, "image/jpeg"))
    }
}

#[derive(Serialize, Clone)]
pub struct Prop {
    pub name: String,
    pub value: String,
    /// Nazwa kanoniczna, np. „System.Photo.FNumber” – do ładnej prezentacji.
    pub key: String,
    /// Wartość liczbowa, gdy właściwość jest liczbą.
    pub num: Option<f64>,
}

/// Wszystkie metadane, które zna system (wymiary, aparat, czas trwania, autor, strony…),
/// z nazwami w języku systemu.
pub fn properties(path: &str) -> Vec<Prop> {
    let mut out = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let p = HSTRING::from(path);
        let store: IPropertyStore = match SHGetPropertyStoreFromParsingName(&p, None, GPS_BESTEFFORT) {
            Ok(s) => s,
            Err(_) => return out,
        };
        let count = store.GetCount().unwrap_or(0);
        for i in 0..count {
            let mut key = PROPERTYKEY::default();
            if store.GetAt(i, &mut key).is_err() {
                continue;
            }
            let Ok(val) = store.GetValue(&key) else { continue };
            let mut raw: *mut std::ffi::c_void = std::ptr::null_mut();
            if PSGetPropertyDescription(&key, &IPropertyDescription::IID, &mut raw).is_err() || raw.is_null() {
                continue;
            }
            let desc = IPropertyDescription::from_raw(raw);
            let Ok(name_ptr) = desc.GetDisplayName() else { continue };
            let name = name_ptr.to_string().unwrap_or_default();
            CoTaskMemFree(Some(name_ptr.0 as *const _));
            if name.is_empty() {
                continue;
            }
            let Ok(vptr) = PSFormatForDisplayAlloc(&key, &val, PDFF_DEFAULT) else { continue };
            let value = vptr.to_string().unwrap_or_default();
            CoTaskMemFree(Some(vptr.0 as *const _));
            let value = value.trim_matches(|c: char| c == '\u{200e}' || c == '\u{200f}' || c.is_whitespace()).to_string();
            if value.is_empty() {
                continue;
            }
            let canon = PSGetNameFromPropertyKey(&key)
                .ok()
                .map(|p| {
                    let s = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const _));
                    s
                })
                .unwrap_or_default();
            let num = windows::Win32::System::Com::StructuredStorage::PropVariantToDouble(&val).ok().filter(|n| n.is_finite());
            out.push(Prop { name, value, key: canon, num });
        }
    }
    out.dedup_by(|a, b| a.name == b.name);
    out
}

pub fn show_properties(hwnd: isize, path: &str) -> bool {
    unsafe {
        let p = HSTRING::from(path);
        SHObjectProperties(Some(HWND(hwnd as *mut _)), SHOP_FILEPATH, &p, PCWSTR::null()).as_bool()
    }
}

pub fn open_with(hwnd: isize, path: &str) -> bool {
    unsafe {
        let p = HSTRING::from(path);
        let info = OPENASINFO {
            pcszFile: PCWSTR(p.as_ptr()),
            pcszClass: PCWSTR::null(),
            oaifInFlags: OAIF_EXEC | OAIF_ALLOW_REGISTRATION,
        };
        SHOpenWithDialog(Some(HWND(hwnd as *mut _)), &info).is_ok()
    }
}

/// Pełne menu kontekstowe Eksploratora (7-Zip, „Wyślij do", rozszerzenia innych programów…).
/// Zwraca true, gdy wybrano polecenie.
pub fn native_menu(hwnd: isize, paths: &[String], x: i32, y: i32, extended: bool) -> Result<bool, String> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::Shell::{CMF_EXPLORE, CMF_EXTENDEDVERBS, CMF_NORMAL, CMINVOKECOMMANDINFOEX};
    use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, TrackPopupMenuEx, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let hwnd = HWND(hwnd as *mut _);
        let (cm, pidls) = context_menu(hwnd, paths)?;
        let menu = CreatePopupMenu().map_err(|e| e.to_string())?;
        let mut flags = CMF_NORMAL | CMF_EXPLORE;
        if extended {
            flags |= CMF_EXTENDEDVERBS;
        }
        let _ = cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags);
        let mut pt = POINT { x, y };
        let _ = ClientToScreen(hwnd, &mut pt);
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None);
        let mut chosen = false;
        if cmd.0 > 0 {
            let mut info = CMINVOKECOMMANDINFOEX {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: 0x4000 | 0x2000_0000,
                hwnd,
                lpVerb: windows::core::PCSTR((cmd.0 - 1) as usize as *const u8),
                lpVerbW: PCWSTR((cmd.0 - 1) as usize as *const u16),
                nShow: SW_SHOWNORMAL.0,
                ptInvoke: pt,
                ..Default::default()
            };
            let _ = cm.InvokeCommand(&mut info as *mut _ as *mut _);
            chosen = true;
        }
        let _ = DestroyMenu(menu);
        free_pidls(&pidls);
        Ok(chosen)
    }
}

#[derive(Serialize, Clone, Copy, Default)]
pub struct MediaInfo {
    pub w: u32,
    pub h: u32,
    /// Czas trwania w ms (wideo, audio).
    pub dur: u64,
    /// Data wykonania zdjęcia / nagrania (ms od 1970, UTC); 0 – brak.
    pub taken: i64,
}

static MEDIA: Lazy<Mutex<HashMap<String, MediaInfo>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn filetime_ms(ft: windows::Win32::Foundation::FILETIME) -> i64 {
    let t = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    if t == 0 {
        0
    } else {
        (t / 10_000) as i64 - 11_644_473_600_000
    }
}

/// Wymiary (z obrotem z EXIF), czas trwania i data wykonania – z systemu właściwości Windows.
/// Wynik trzymany w RAM, klucz zawiera datę modyfikacji.
pub fn media_info(items: Vec<(String, i64)>) -> Vec<MediaInfo> {
    use windows::core::GUID;
    use windows::Win32::System::Com::StructuredStorage::{PropVariantToFileTime, PropVariantToUInt32, PropVariantToUInt64};
    use windows::Win32::System::Variant::PSTF_UTC;
    const IMG: GUID = GUID::from_u128(0x6444048F_4C8B_11D1_8B70_080036B11A03);
    const VID: GUID = GUID::from_u128(0x64440491_4C8B_11D1_8B70_080036B11A03);
    const MED: GUID = GUID::from_u128(0x64440490_4C8B_11D1_8B70_080036B11A03);
    const PHO: GUID = GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99);
    const MEDIA_CREATED: GUID = GUID::from_u128(0x2E4B640D_5019_46D8_8881_55414CC5CAA0);
    let one = |path: &str| -> MediaInfo {
        unsafe {
            let Ok(store): windows::core::Result<IPropertyStore> = SHGetPropertyStoreFromParsingName(&HSTRING::from(path), None, GPS_BESTEFFORT) else {
                return MediaInfo::default();
            };
            let val = |fmtid: GUID, pid: u32| store.GetValue(&PROPERTYKEY { fmtid, pid }).ok();
            let u32v = |fmtid: GUID, pid: u32| val(fmtid, pid).and_then(|v| PropVariantToUInt32(&v).ok()).unwrap_or(0);
            let ft = |fmtid: GUID, pid: u32| val(fmtid, pid).and_then(|v| PropVariantToFileTime(&v, PSTF_UTC).ok()).map(filetime_ms).unwrap_or(0);
            let (mut w, mut h) = (u32v(IMG, 3), u32v(IMG, 4));
            if w == 0 || h == 0 {
                (w, h) = (u32v(VID, 3), u32v(VID, 4));
            } else if matches!(u32v(PHO, 274), 5..=8) {
                std::mem::swap(&mut w, &mut h);
            }
            let dur = val(MED, 3).and_then(|v| PropVariantToUInt64(&v).ok()).unwrap_or(0) / 10_000;
            let mut taken = ft(PHO, 36867);
            if taken == 0 {
                taken = ft(MEDIA_CREATED, 100);
            }
            MediaInfo { w, h, dur, taken }
        }
    };
    let mut out = vec![MediaInfo::default(); items.len()];
    let mut todo = Vec::new();
    {
        let c = MEDIA.lock();
        for (i, (p, m)) in items.iter().enumerate() {
            match c.get(&format!("{m}:{p}")) {
                Some(d) => out[i] = *d,
                None => todo.push(i),
            }
        }
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let chunk = todo.len().div_ceil(threads).max(1);
    let found: Vec<(usize, MediaInfo)> = std::thread::scope(|s| {
        let hs: Vec<_> = todo
            .chunks(chunk)
            .map(|part| {
                let items = &items;
                s.spawn(move || {
                    unsafe {
                        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                    }
                    part.iter().map(|&i| (i, one(&items[i].0))).collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    let mut c = MEDIA.lock();
    if c.len() > 200_000 {
        c.clear();
    }
    for (i, d) in found {
        out[i] = d;
        c.insert(format!("{}:{}", items[i].1, items[i].0), d);
    }
    out
}

/// Polecenie z menu kontekstowego Shella po nazwie kanonicznej (np. „eject”).
pub fn invoke_verb(hwnd: isize, path: &str, verb: &str) -> Result<(), String> {
    use windows::Win32::UI::Shell::{CMF_NORMAL, CMINVOKECOMMANDINFOEX};
    use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, SW_SHOWNORMAL};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let hwnd = HWND(hwnd as *mut _);
        let (cm, pidls) = context_menu(hwnd, &[path.to_string()])?;
        let menu = CreatePopupMenu().map_err(|e| e.to_string())?;
        let _ = cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL);
        let verb_a = std::ffi::CString::new(verb).unwrap_or_default();
        let verb_w = HSTRING::from(verb);
        let mut info = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: 0x4000,
            hwnd,
            lpVerb: windows::core::PCSTR(verb_a.as_ptr() as *const u8),
            lpVerbW: PCWSTR(verb_w.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        let res = cm.InvokeCommand(&mut info as *mut _ as *mut _);
        let _ = DestroyMenu(menu);
        free_pidls(&pidls);
        res.map_err(|e| e.message().to_string())
    }
}

unsafe fn free_pidls(v: &[*mut windows::Win32::UI::Shell::Common::ITEMIDLIST]) {
    v.iter().for_each(|p| windows::Win32::UI::Shell::ILFree(Some(*p)));
}

type Pidls = Vec<*mut windows::Win32::UI::Shell::Common::ITEMIDLIST>;

/// IContextMenu dla elementów z jednego folderu (zwraca też PIDL-e do zwolnienia).
unsafe fn context_menu(hwnd: HWND, paths: &[String]) -> Result<(windows::Win32::UI::Shell::IContextMenu, Pidls), String> {
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{ILFindLastID, IContextMenu, IShellFolder, SHBindToParent, SHParseDisplayName};
    let mut pidls: Pidls = Vec::new();
    for p in paths {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(p.as_str()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            pidls.push(pidl);
        }
    }
    if pidls.is_empty() {
        return Err("Nie można odczytać elementów".into());
    }
    let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
    let folder: IShellFolder = match SHBindToParent(pidls[0], Some(&mut child as *mut _ as *mut _)) {
        Ok(f) => f,
        Err(e) => {
            free_pidls(&pidls);
            return Err(e.to_string());
        }
    };
    let children: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| ILFindLastID(*p) as *const _).collect();
    match folder.GetUIObjectOf::<IContextMenu>(hwnd, &children, None) {
        Ok(c) => Ok((c, pidls)),
        Err(e) => {
            free_pidls(&pidls);
            Err(e.to_string())
        }
    }
}
