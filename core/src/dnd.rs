//! Przeciąganie plików przez OLE – tak jak w Eksploratorze, więc działa w obie strony z innymi programami.
//!
//! - [`start_drag`]: przeciąganie z aplikacji. `SHDoDragDrop` w osobnym wątku STA (ze swoją pętlą komunikatów),
//!   żeby okno aplikacji dalej się rysowało; podgląd przeciąganych plików daje powłoka.
//! - [`register`]: cel upuszczania na oknie. Wywołania przychodzą w wątku okna; handler decyduje,
//!   dokąd trafią pliki i jaki jest efekt (kopiuj / przenieś / brak).

use std::cell::RefCell;

use windows::core::{implement, Ref, Result as WinResult};
use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE,
    DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{BHID_DataObject, DragQueryFileW, IShellItemArray, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHParseDisplayName, HDROP};

/// Efekt upuszczenia wybrany przez handler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    None,
    Copy,
    Move,
}

impl Effect {
    fn win(self) -> DROPEFFECT {
        match self {
            Effect::None => DROPEFFECT_NONE,
            Effect::Copy => DROPEFFECT_COPY,
            Effect::Move => DROPEFFECT_MOVE,
        }
    }
}

/// Klawisze przy upuszczaniu: Ctrl – kopiuj, Shift – przenieś.
#[derive(Clone, Copy, Debug, Default)]
pub struct Keys {
    pub ctrl: bool,
    pub shift: bool,
}

/// Obsługa celu upuszczania (w wątku okna). Współrzędne – piksele fizyczne względem obszaru klienta.
pub trait DropHandler {
    fn over(&self, x: i32, y: i32, keys: Keys, paths: &[String]) -> Effect;
    fn leave(&self);
    fn dropped(&self, x: i32, y: i32, keys: Keys, paths: &[String]) -> Effect;
}

fn keys(k: MODIFIERKEYS_FLAGS) -> Keys {
    Keys { ctrl: k.0 & MK_CONTROL.0 != 0, shift: k.0 & MK_SHIFT.0 != 0 }
}

/// Ścieżki plików z danych przeciągania (CF_HDROP); pusto, gdy to nie pliki.
fn paths_of(data: &IDataObject) -> Vec<String> {
    let fmt = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let mut out = Vec::new();
    unsafe {
        let Ok(mut med) = data.GetData(&fmt) else { return out };
        let drop = HDROP(med.u.hGlobal.0);
        let n = DragQueryFileW(drop, u32::MAX, None);
        for i in 0..n {
            let len = DragQueryFileW(drop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(drop, i, Some(&mut buf));
            out.push(String::from_utf16_lossy(&buf[..len]));
        }
        ReleaseStgMedium(&mut med);
    }
    out
}

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    handler: Box<dyn DropHandler>,
    /// Pliki z bieżącego przeciągania (odczytane przy wejściu).
    paths: RefCell<Vec<String>>,
}

impl Target {
    fn client(&self, pt: &POINTL) -> (i32, i32) {
        let mut p = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut p);
        }
        (p.x, p.y)
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, keystate: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> WinResult<()> {
        *self.paths.borrow_mut() = data.as_ref().map(paths_of).unwrap_or_default();
        self.DragOver(keystate, pt, effect)
    }

    fn DragOver(&self, keystate: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> WinResult<()> {
        let (x, y) = self.client(pt);
        let paths = self.paths.borrow().clone();
        let e = if paths.is_empty() { Effect::None } else { self.handler.over(x, y, keys(keystate), &paths) };
        unsafe {
            *effect = e.win();
        }
        Ok(())
    }

    fn DragLeave(&self) -> WinResult<()> {
        self.paths.borrow_mut().clear();
        self.handler.leave();
        Ok(())
    }

    fn Drop(&self, data: Ref<'_, IDataObject>, keystate: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> WinResult<()> {
        let (x, y) = self.client(pt);
        let mut paths = std::mem::take(&mut *self.paths.borrow_mut());
        if paths.is_empty() {
            paths = data.as_ref().map(paths_of).unwrap_or_default();
        }
        let e = if paths.is_empty() { Effect::None } else { self.handler.dropped(x, y, keys(keystate), &paths) };
        unsafe {
            *effect = e.win();
        }
        Ok(())
    }
}

/// Rejestruje okno jako cel upuszczania (zastępuje ewentualny cel zarejestrowany przez winit).
/// Wywoływać w wątku okna.
pub fn register(hwnd: isize, handler: Box<dyn DropHandler>) -> Result<(), String> {
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        let _ = OleInitialize(None);
        let _ = RevokeDragDrop(hwnd);
        let target: IDropTarget = Target { hwnd, handler, paths: RefCell::new(Vec::new()) }.into();
        RegisterDragDrop(hwnd, &target).map_err(|e| e.message())
    }
}

/// Dane przeciągania dla plików – ten sam obiekt, który tworzy Eksplorator (z podglądem i miniaturami).
unsafe fn data_object(paths: &[String]) -> WinResult<IDataObject> {
    let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
    for p in paths {
        let wide: Vec<u16> = p.encode_utf16().chain(std::iter::once(0)).collect();
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(windows::core::PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            pidls.push(pidl);
        }
    }
    let res = (|| {
        let arr: IShellItemArray = SHCreateShellItemArrayFromIDLists(&pidls)?;
        arr.BindToHandler(None, &BHID_DataObject)
    })();
    for p in pidls {
        CoTaskMemFree(Some(p as *const _));
    }
    res
}

/// Przeciąganie plików z aplikacji (w osobnym wątku). `done` dostaje wynik: kopiowanie, przeniesienie albo nic.
pub fn start_drag(paths: Vec<String>, done: impl FnOnce(Effect) + Send + 'static) {
    std::thread::spawn(move || {
        let effect = unsafe {
            let _ = OleInitialize(None);
            match data_object(&paths) {
                Ok(data) => match SHDoDragDrop(None, &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK) {
                    Ok(e) if e.0 & DROPEFFECT_MOVE.0 != 0 => Effect::Move,
                    Ok(e) if e.0 & DROPEFFECT_COPY.0 != 0 => Effect::Copy,
                    _ => Effect::None,
                },
                Err(_) => Effect::None,
            }
        };
        done(effect);
    });
}
