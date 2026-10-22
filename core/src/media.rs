//! Odtwarzanie wideo i audio przez Media Foundation (IMFMediaEngine w trybie „frame server”).
//! Silnik sam dekoduje, synchronizuje obraz z dźwiękiem i gra dźwięk; klatki odbieramy
//! do bitmapy WIC (BGRA w pamięci) i oddajemy interfejsowi jako RGBA.
//! Obiekt żyje w wątku interfejsu – klatki pobiera się z timera (`frame`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{implement, Interface, Result as WinResult, BSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICBitmap, IWICImagingFactory, WICBitmapCacheOnLoad, WICBitmapLockRead, WICRect,
};
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFMediaEngine, IMFMediaEngineClassFactory, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl,
    MFCreateAttributes, MFStartup, MFSTARTUP_FULL, MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR,
    MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_VERSION,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

/// Flagi ustawiane z wątków silnika.
#[derive(Default)]
struct Flags {
    ended: AtomicBool,
    error: AtomicBool,
}

#[implement(IMFMediaEngineNotify)]
struct Notify(Arc<Flags>);

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> WinResult<()> {
        if event == MF_MEDIA_ENGINE_EVENT_ENDED.0 as u32 {
            self.0.ended.store(true, Ordering::Relaxed);
        } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            self.0.error.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

/// Klatka gotowa do pokazania: RGBA, wiersz po wierszu.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Stan odtwarzania do paska sterowania.
#[derive(Clone, Copy, Debug, Default)]
pub struct Status {
    /// Sekundy; 0, dopóki silnik nie zna długości.
    pub duration: f64,
    pub position: f64,
    pub paused: bool,
    pub ended: bool,
    pub error: bool,
    pub has_video: bool,
    pub ready: bool,
}

pub struct Player {
    engine: IMFMediaEngine,
    flags: Arc<Flags>,
    wic: IWICImagingFactory,
    bitmap: Option<(IWICBitmap, u32, u32)>,
    last_pts: i64,
}

fn startup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    });
}

impl Player {
    /// Otwiera plik i od razu zaczyna grać (`autoplay`) albo czeka na `play`.
    pub fn open(path: &str, autoplay: bool) -> WinResult<Player> {
        startup();
        unsafe {
            let factory: IMFMediaEngineClassFactory = CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let flags = Arc::new(Flags::default());
            let notify: IMFMediaEngineNotify = Notify(flags.clone()).into();
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 2)?;
            let attrs = attrs.ok_or_else(windows::core::Error::empty)?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attrs.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)?;
            // Bez okna i bez urządzenia DXGI – tryb „frame server” z kopiowaniem klatek do WIC.
            let engine = factory.CreateInstance(0, &attrs)?;
            engine.SetAutoPlay(autoplay)?;
            engine.SetSource(&BSTR::from(path))?;
            Ok(Player { engine, flags, wic, bitmap: None, last_pts: -1 })
        }
    }

    pub fn play(&self) {
        unsafe {
            if self.engine.IsEnded().as_bool() {
                let _ = self.engine.SetCurrentTime(0.0);
            }
            self.flags.ended.store(false, Ordering::Relaxed);
            let _ = self.engine.Play();
        }
    }

    pub fn pause(&self) {
        unsafe {
            let _ = self.engine.Pause();
        }
    }

    pub fn toggle(&self) {
        if self.status().paused {
            self.play();
        } else {
            self.pause();
        }
    }

    /// Przewinięcie do `t` sekund (obcięte do długości).
    pub fn seek(&self, t: f64) {
        let d = self.status().duration;
        let t = if d > 0.0 { t.clamp(0.0, d) } else { t.max(0.0) };
        unsafe {
            let _ = self.engine.SetCurrentTime(t);
        }
    }

    /// Głośność 0–1.
    pub fn set_volume(&self, v: f64) {
        unsafe {
            let _ = self.engine.SetVolume(v.clamp(0.0, 1.0));
        }
    }

    pub fn set_muted(&self, m: bool) {
        unsafe {
            let _ = self.engine.SetMuted(m);
        }
    }

    pub fn status(&self) -> Status {
        unsafe {
            let d = self.engine.GetDuration();
            Status {
                duration: if d.is_finite() && d > 0.0 { d } else { 0.0 },
                position: self.engine.GetCurrentTime(),
                paused: self.engine.IsPaused().as_bool(),
                ended: self.flags.ended.load(Ordering::Relaxed) || self.engine.IsEnded().as_bool(),
                error: self.flags.error.load(Ordering::Relaxed) || self.engine.GetError().is_ok(),
                has_video: self.engine.HasVideo().as_bool(),
                ready: self.engine.GetReadyState() >= 1,
            }
        }
    }

    /// Nowa klatka, jeśli silnik ma inną niż ostatnio oddana.
    pub fn frame(&mut self) -> Option<Frame> {
        unsafe {
            if !self.engine.HasVideo().as_bool() {
                return None;
            }
            let pts = self.engine.OnVideoStreamTick().ok()?;
            if pts == self.last_pts {
                return None;
            }
            let (mut w, mut h) = (0u32, 0u32);
            self.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h)).ok()?;
            if w == 0 || h == 0 {
                return None;
            }
            if self.bitmap.as_ref().map(|b| (b.1, b.2)) != Some((w, h)) {
                let bmp = self.wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppBGRA, WICBitmapCacheOnLoad).ok()?;
                self.bitmap = Some((bmp, w, h));
            }
            let (bmp, ..) = self.bitmap.as_ref()?;
            let dst = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
            self.engine.TransferVideoFrame(&bmp.cast::<windows::core::IUnknown>().ok()?, None, &dst, None).ok()?;
            self.last_pts = pts;
            let lock = bmp.Lock(&WICRect { X: 0, Y: 0, Width: w as i32, Height: h as i32 }, WICBitmapLockRead.0 as u32).ok()?;
            let stride = lock.GetStride().ok()? as usize;
            let mut size = 0u32;
            let mut ptr = std::ptr::null_mut();
            lock.GetDataPointer(&mut size, &mut ptr).ok()?;
            let src = std::slice::from_raw_parts(ptr, size as usize);
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row = &src[y * stride..y * stride + w as usize * 4];
                for px in row.chunks_exact(4) {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
            Some(Frame { width: w, height: h, rgba })
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}
