//! PowerDesk w oknach „Otwórz” i „Zapisz jako” innych programów.
//!
//! Biblioteka jest zarejestrowana (tylko dla bieżącego użytkownika, `HKCU\Software\Classes\CLSID`)
//! jako serwer klas CLSID_FileOpenDialog i CLSID_FileSaveDialog. Program, który tworzy okno pliku,
//! dostaje od nas cienką nakładkę na prawdziwe okno Windows z comdlg32.dll: każde ustawienie trafia
//! do oryginału, a my je tylko zapamiętujemy. Dopiero `Show` pokazuje okno PowerDesk – osobny proces
//! `powerdesk.exe --pick-file`, więc jego awaria nie dotyka programu. Przy każdym problemie (brak
//! PowerDesk, interfejs, którego nie znamy, własne kontrolki programu, błąd, zawieszony start okna)
//! pokazuje się zwykłe okno Windows z tymi samymi ustawieniami – zapis w cudzym programie ma działać
//! zawsze.
//!
//! Protokół z `powerdesk.exe --pick-file`: jeden wiersz JSON z opisem okna na stdin; na stdout
//! „ready”, gdy okno jest widoczne, a na końcu wiersz JSON z wynikiem:
//! `{"result":"ok","paths":[…],"type":n}`, `{"result":"cancel"}` albo `{"result":"fallback"}`.

#![allow(non_snake_case)]

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::io::{BufRead, BufReader, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use windows::core::{s, w, IUnknown, IUnknown_Vtbl, Interface, BOOL, GUID, HRESULT, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, CLASS_E_CLASSNOTAVAILABLE, ERROR_CANCELLED, E_NOINTERFACE, E_POINTER, E_UNEXPECTED, HANDLE, HMODULE, HWND, S_FALSE, S_OK};
use windows::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel, TokenIsAppContainer, TOKEN_MANDATORY_LABEL, TOKEN_QUERY};
use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree, IClassFactory, IClassFactory_Vtbl};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW, GetProcAddress, LoadLibraryExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};
use windows::Win32::UI::Shell::Common::{COMDLG_FILTERSPEC, ITEMIDLIST};
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileDialog, IFileDialog2, IFileDialog2_Vtbl, IFileDialogCustomize, IFileDialogCustomize_Vtbl, IFileDialogEvents, IFileDialog_Vtbl, IFileOpenDialog, IFileOpenDialog_Vtbl, IFileSaveDialog, IFileSaveDialog_Vtbl, IModalWindow, IModalWindow_Vtbl, IShellItem, IShellItemArray, SHAddToRecentDocs, SHCreateItemFromIDList,
    SHCreateShellItemArrayFromIDLists, SHParseDisplayName, SHSimpleIDListFromPath, CDCONTROLSTATEF, FDAP, FILEOPENDIALOGOPTIONS, FOS_ALLNONSTORAGEITEMS, FOS_ALLOWMULTISELECT, FOS_CREATEPROMPT, FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST, FOS_FORCESHOWHIDDEN, FOS_NOREADONLYRETURN, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, DispatchMessageW, IsWindow, MsgWaitForMultipleObjects, PeekMessageW, PostQuitMessage, SetForegroundWindow, TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT, WM_QUIT};

/// Argument `powerdesk.exe`: okno wyboru pliku zamiast zwykłego startu.
const PICK_FLAG: &str = "--pick-file";
/// Okno PowerDesk musi się pokazać w tym czasie – inaczej zwykłe okno Windows.
const START_LIMIT: Duration = Duration::from_secs(20);

// ---------- Dziennik ----------

/// Powody, dla których pokazało się okno Windows: `%LOCALAPPDATA%\PowerDesk\dialog.log` (do 256 KB).
fn log(msg: impl AsRef<str>) {
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("PowerDesk")) else { return };
    let f = dir.join("dialog.log");
    if std::fs::metadata(&f).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::remove_file(&f);
    }
    let app = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
    if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(out, "{secs} {app} [{}] {}", std::process::id(), msg.as_ref());
    }
}

// ---------- Kiedy w ogóle próbujemy ----------

/// `powerdesk.exe` obok tej biblioteki – albo `None`, gdy w tym procesie zostaje okno Windows:
/// sam PowerDesk, proces z uprawnieniami albo w piaskownicy (inny poziom integralności niż zwykły
/// program użytkownika) i zmienna `POWERDESK_SYSTEM_DIALOGS` (awaryjne wyłączenie).
fn picker() -> Option<&'static PathBuf> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| {
        if std::env::var_os("POWERDESK_SYSTEM_DIALOGS").is_some() {
            return None;
        }
        let me = std::env::current_exe().ok()?;
        if me.file_name()?.to_string_lossy().eq_ignore_ascii_case("powerdesk.exe") {
            return None;
        }
        if !medium_integrity() {
            return None;
        }
        // Ścieżkę zapisuje PowerDesk przy rejestracji (biblioteka leży w folderze użytkownika,
        // nie obok programu); obok biblioteki – przy ręcznych próbach.
        let exe = registered_exe().or_else(|| Some(module_path()?.parent()?.join("powerdesk.exe")))?;
        exe.is_file().then_some(exe)
    })
    .as_ref()
}

/// Zwykły proces użytkownika: średni poziom integralności i nie AppContainer.
fn medium_integrity() -> bool {
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).is_err() {
            return false;
        }
        let mut buf = [0u64; 16];
        let mut len = 0u32;
        let ok = GetTokenInformation(tok, TokenIntegrityLevel, Some(buf.as_mut_ptr() as *mut c_void), std::mem::size_of_val(&buf) as u32, &mut len).is_ok();
        let mut app = 0u32;
        let app_ok = GetTokenInformation(tok, TokenIsAppContainer, Some(&mut app as *mut u32 as *mut c_void), 4, &mut len).is_ok();
        let _ = CloseHandle(tok);
        if !ok || !app_ok || app != 0 {
            return false;
        }
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let n = *GetSidSubAuthorityCount(sid);
        if n == 0 {
            return false;
        }
        let rid = *GetSidSubAuthority(sid, n as u32 - 1);
        // SECURITY_MANDATORY_MEDIUM_RID = 0x2000 (z „plus” 0x2100), HIGH = 0x3000.
        (0x2000..0x3000).contains(&rid)
    }
}

/// `HKCU\Software\PowerDesk` → `DialogExe` (ten klucz nie jest przekierowywany dla 32 bitów).
fn registered_exe() -> Option<PathBuf> {
    unsafe {
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        RegGetValueW(HKEY_CURRENT_USER, w!("Software\\PowerDesk"), w!("DialogExe"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut c_void), Some(&mut len)).ok().ok()?;
        let n = (len as usize / 2).saturating_sub(1).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..n]);
        (!s.is_empty()).then(|| PathBuf::from(s))
    }
}

/// Ścieżka tej biblioteki.
fn module_path() -> Option<PathBuf> {
    unsafe {
        let mut h = HMODULE::default();
        GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, PCWSTR(module_path as *const () as *const u16), &mut h).ok()?;
        let mut buf = vec![0u16; 2048];
        let n = GetModuleFileNameW(Some(h), &mut buf) as usize;
        (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
    }
}

// ---------- Oryginalne okno Windows ----------

type GetClassObjectFn = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

/// `DllGetClassObject` z systemowego comdlg32.dll (tam w HKLM wskazują obie klasy).
fn system_get_class_object() -> Option<GetClassObjectFn> {
    static F: OnceLock<usize> = OnceLock::new();
    let f = *F.get_or_init(|| unsafe {
        let Ok(h) = LoadLibraryExW(w!("comdlg32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) else { return 0 };
        GetProcAddress(h, s!("DllGetClassObject")).map(|p| p as usize).unwrap_or(0)
    });
    (f != 0).then(|| unsafe { std::mem::transmute::<usize, GetClassObjectFn>(f) })
}

fn system_factory(save: bool) -> Option<IClassFactory> {
    let get = system_get_class_object()?;
    let clsid = if save { FileSaveDialog } else { FileOpenDialog };
    let mut p = null_mut();
    unsafe {
        get(&clsid, &IClassFactory::IID, &mut p).ok().ok()?;
        (!p.is_null()).then(|| IClassFactory::from_raw(p))
    }
}

// ---------- Fabryka klas ----------

#[repr(C)]
struct Factory {
    vt: &'static IClassFactory_Vtbl,
    save: bool,
}

static FACTORY_VT: IClassFactory_Vtbl = IClassFactory_Vtbl {
    base__: IUnknown_Vtbl { QueryInterface: f_qi, AddRef: f_add_ref, Release: f_release },
    CreateInstance: f_create,
    LockServer: f_lock,
};
static OPEN_FACTORY: Factory = Factory { vt: &FACTORY_VT, save: false };
static SAVE_FACTORY: Factory = Factory { vt: &FACTORY_VT, save: true };

unsafe extern "system" fn f_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    if *iid == IUnknown::IID || *iid == IClassFactory::IID {
        *out = this;
        S_OK
    } else {
        *out = null_mut();
        E_NOINTERFACE
    }
}

// Fabryki są statyczne – licznik odwołań niczego nie zwalnia.
unsafe extern "system" fn f_add_ref(_: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn f_release(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn f_lock(_: *mut c_void, _: BOOL) -> HRESULT {
    S_OK
}

unsafe extern "system" fn f_create(this: *mut c_void, outer: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *out = null_mut();
    let save = (*(this as *const Factory)).save;
    let Some(sys) = system_factory(save) else { return CLASS_E_CLASSNOTAVAILABLE };
    let create = sys.vtable().CreateInstance;
    // Agregacja albo PowerDesk niedostępny w tym procesie – od razu oryginał.
    if !outer.is_null() || picker().is_none() {
        return create(sys.as_raw(), outer, riid, out);
    }
    match catch_unwind(AssertUnwindSafe(|| wrap(&sys, save))) {
        Ok(Some(obj)) => {
            let hr = qi::<0>(obj, riid, out);
            release::<0>(obj);
            hr
        }
        _ => create(sys.as_raw(), null_mut(), riid, out),
    }
}

// ---------- Nakładka na okno ----------

#[derive(Clone)]
struct Picked {
    paths: Vec<String>,
    /// Wybrany filtr typów (od 1, jak w IFileDialog).
    ty: u32,
}

#[derive(Default)]
struct Setup {
    filters: Vec<(String, String)>,
    type_index: u32,
    file_name: Option<String>,
    title: Option<String>,
    ok_label: Option<String>,
    name_label: Option<String>,
    cancel_label: Option<String>,
    default_ext: Option<String>,
    /// SetFolder – folder narzucony przez program.
    folder: Option<String>,
    default_folder: Option<String>,
    save_as: Option<String>,
    places: Vec<String>,
    sinks: Vec<(u32, IFileDialogEvents)>,
}

/// Obiekt COM: trzy tablice metod (IFileOpenDialog albo IFileSaveDialog, IFileDialog2,
/// IFileDialogCustomize) na początku, w tej kolejności – `me::<S>` cofa wskaźnik o S pól.
#[repr(C)]
struct Dialog {
    vt: *const c_void,
    vt2: *const IFileDialog2_Vtbl,
    vtc: *const IFileDialogCustomize_Vtbl,
    refs: AtomicU32,
    save: bool,
    real: IFileDialog,
    real2: Option<IFileDialog2>,
    real_open: Option<IFileOpenDialog>,
    real_save: Option<IFileSaveDialog>,
    cust: Option<IFileDialogCustomize>,
    setup: RefCell<Setup>,
    /// Program sięgnął po coś, czego okno PowerDesk nie odda (obcy interfejs, filtr elementów,
    /// ograniczenie nawigacji) – pokazujemy oryginał.
    foreign: Cell<bool>,
    /// Program dodał własne kontrolki (IFileDialogCustomize) – też oryginał.
    customized: Cell<bool>,
    busy: Cell<bool>,
    result: RefCell<Option<Picked>>,
}

impl Dialog {
    fn picked(&self) -> Option<Picked> {
        self.result.borrow().clone()
    }

    fn base(&self) -> *mut c_void {
        self as *const Dialog as *mut c_void
    }
}

unsafe fn me<'a, const S: usize>(this: *mut c_void) -> &'a Dialog {
    &*((this as *mut *const c_void).sub(S) as *const Dialog)
}

fn wrap(sys: &IClassFactory, save: bool) -> Option<*mut c_void> {
    let real: IFileDialog = unsafe {
        let mut p = null_mut();
        (sys.vtable().CreateInstance)(sys.as_raw(), null_mut(), &IFileDialog::IID, &mut p).ok().ok()?;
        if p.is_null() {
            return None;
        }
        IFileDialog::from_raw(p)
    };
    let (real_open, real_save) = if save { (None, Some(real.cast::<IFileSaveDialog>().ok()?)) } else { (Some(real.cast::<IFileOpenDialog>().ok()?), None) };
    let d = Box::new(Dialog {
        vt: if save { &SAVE_VT as *const IFileSaveDialog_Vtbl as *const c_void } else { &OPEN_VT as *const IFileOpenDialog_Vtbl as *const c_void },
        vt2: &D2_VT,
        vtc: &CUST_VT,
        refs: AtomicU32::new(1),
        save,
        real2: real.cast().ok(),
        cust: real.cast().ok(),
        real,
        real_open,
        real_save,
        setup: RefCell::new(Setup::default()),
        foreign: Cell::new(false),
        customized: Cell::new(false),
        busy: Cell::new(false),
        result: RefCell::new(None),
    });
    Some(Box::into_raw(d) as *mut c_void)
}

/// Interfejsy, o które pyta sam COM albo środowiska (.NET, WinRT) – bez nich obiekt działa
/// zwyczajnie (standardowy marshaling), więc odpowiadamy „nie ma” i nie ruszamy oryginału.
const PLUMBING: [GUID; 14] = [
    GUID::from_u128(0x00000003_0000_0000_c000_000000000046), // IMarshal
    GUID::from_u128(0x000001cf_0000_0000_c000_000000000046), // IMarshal2
    GUID::from_u128(0x00000018_0000_0000_c000_000000000046), // IStdMarshalInfo
    GUID::from_u128(0x00000019_0000_0000_c000_000000000046), // IExternalConnection
    GUID::from_u128(0x0000001b_0000_0000_c000_000000000046), // IdentityUnmarshal
    GUID::from_u128(0x00000144_0000_0000_c000_000000000046), // IRpcOptions
    GUID::from_u128(0x00000038_0000_0000_c000_000000000046), // IWeakReferenceSource
    GUID::from_u128(0x00020400_0000_0000_c000_000000000046), // IDispatch
    GUID::from_u128(0xecc8691b_c1db_4dc0_855e_65f6c551af49), // INoMarshal
    GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90), // IAgileObject
    GUID::from_u128(0xaf86e2e0_b12d_4c6a_9c5a_d7aa65101e90), // IInspectable
    GUID::from_u128(0xb196b283_bab4_101a_b69c_00aa00341d07), // IProvideClassInfo
    GUID::from_u128(0xc3fcc19e_a970_11d2_8b5a_00a0c9b7c9c4), // IManagedObject
    GUID::from_u128(0x1c733a30_2a1c_11ce_ade5_00aa0044773d), // ICallFactory
];

unsafe extern "system" fn qi<const S: usize>(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    *out = null_mut();
    let d = me::<S>(this);
    let iid = *iid;
    let base = d.base();
    let kind = if d.save { IFileSaveDialog::IID } else { IFileOpenDialog::IID };
    let p = if iid == IUnknown::IID || iid == IModalWindow::IID || iid == IFileDialog::IID || iid == kind {
        base
    } else if iid == IFileDialog2::IID && d.real2.is_some() {
        (base as *mut *const c_void).add(1) as *mut c_void
    } else if iid == IFileDialogCustomize::IID && d.cust.is_some() {
        (base as *mut *const c_void).add(2) as *mut c_void
    } else if PLUMBING.contains(&iid) {
        return E_NOINTERFACE;
    } else {
        // Interfejs, którego nie znamy (np. prywatne interfejsy powłoki): oddaje go oryginał,
        // a przy `Show` pokazujemy oryginał – tylko on wie, co program przez niego ustawił.
        let hr = d.real.query(&iid, out);
        if hr.is_ok() {
            d.foreign.set(true);
            log(format!("interfejs {iid:?} – okno Windows"));
        }
        return hr;
    };
    d.refs.fetch_add(1, Ordering::Relaxed);
    *out = p;
    S_OK
}

unsafe extern "system" fn add_ref<const S: usize>(this: *mut c_void) -> u32 {
    me::<S>(this).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn release<const S: usize>(this: *mut c_void) -> u32 {
    let d = me::<S>(this);
    let n = d.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        drop(Box::from_raw(d as *const Dialog as *mut Dialog));
    }
    n
}

// ---------- Pomocnicze ----------

/// Zapis ustawienia nie może przerwać wywołania – błąd po naszej stronie tylko gubi zapis.
fn quiet(f: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

fn text(p: PCWSTR) -> Option<String> {
    (!p.is_null()).then(|| unsafe { p.to_string().unwrap_or_default() })
}

unsafe fn item_path(p: *mut c_void) -> Option<String> {
    let item = IShellItem::from_raw_borrowed(&p)?;
    fs_path(item)
}

fn fs_path(item: &IShellItem) -> Option<String> {
    unsafe {
        let s = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let r = s.to_string().ok();
        CoTaskMemFree(Some(s.0 as *const c_void));
        r.filter(|p| !p.is_empty())
    }
}

/// PIDL ścieżki; plik, którego jeszcze nie ma (zapis), dostaje prosty PIDL.
unsafe fn pidl(path: &str) -> Option<*mut ITEMIDLIST> {
    let h = HSTRING::from(path);
    let mut p = null_mut();
    if SHParseDisplayName(&h, None, &mut p, 0, None).is_ok() && !p.is_null() {
        return Some(p);
    }
    let p = SHSimpleIDListFromPath(&h);
    (!p.is_null()).then_some(p)
}

unsafe fn shell_item(path: &str) -> Option<IShellItem> {
    let p = pidl(path)?;
    let r = SHCreateItemFromIDList(p).ok();
    CoTaskMemFree(Some(p as *const c_void));
    r
}

unsafe fn shell_items(paths: &[String]) -> Option<IShellItemArray> {
    let ids: Vec<*mut ITEMIDLIST> = paths.iter().filter_map(|p| pidl(p)).collect();
    let r = if ids.len() == paths.len() {
        let c: Vec<*const ITEMIDLIST> = ids.iter().map(|p| *p as *const ITEMIDLIST).collect();
        SHCreateShellItemArrayFromIDLists(&c).ok()
    } else {
        None
    };
    for p in ids {
        CoTaskMemFree(Some(p as *const c_void));
    }
    r
}

unsafe fn give<T: Interface>(out: *mut *mut c_void, v: Option<T>) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    match v {
        Some(v) => {
            *out = v.into_raw();
            S_OK
        }
        None => {
            *out = null_mut();
            E_UNEXPECTED
        }
    }
}

fn parent(p: &str) -> String {
    Path::new(p).parent().map(|d| d.to_string_lossy().to_string()).filter(|d| !d.is_empty()).unwrap_or_else(|| p.to_string())
}

// ---------- IFileDialog (wspólne dla tablicy głównej i IFileDialog2) ----------

macro_rules! forward {
    ($($name:ident($($a:ident: $t:ty),*);)*) => {$(
        unsafe extern "system" fn $name<const S: usize>(this: *mut c_void $(, $a: $t)*) -> HRESULT {
            let d = me::<S>(this);
            (d.real.vtable().$name)(d.real.as_raw() $(, $a)*)
        }
    )*};
}

forward! {
    SetOptions(fos: FILEOPENDIALOGOPTIONS);
    GetOptions(out: *mut FILEOPENDIALOGOPTIONS);
    Close(hr: HRESULT);
    SetClientGuid(guid: *const GUID);
    ClearClientData();
}

/// Wywołanie oryginału, a po powodzeniu – zapis ustawienia u nas.
macro_rules! record {
    ($($name:ident($($a:ident: $t:ty),*) |$d:ident| $body:block)*) => {$(
        unsafe extern "system" fn $name<const S: usize>(this: *mut c_void $(, $a: $t)*) -> HRESULT {
            let $d = me::<S>(this);
            let hr = ($d.real.vtable().$name)($d.real.as_raw() $(, $a)*);
            if hr.is_ok() {
                quiet(|| $body);
            }
            hr
        }
    )*};
}

record! {
    SetFileTypes(n: u32, spec: *const COMDLG_FILTERSPEC) |d| {
        let list = if spec.is_null() { Vec::new() } else { (0..n as usize).map(|i| { let f = &*spec.add(i); (text(f.pszName).unwrap_or_default(), text(f.pszSpec).unwrap_or_default()) }).collect() };
        d.setup.borrow_mut().filters = list;
    }
    SetFileTypeIndex(i: u32) |d| { d.setup.borrow_mut().type_index = i; }
    SetDefaultFolder(item: *mut c_void) |d| { d.setup.borrow_mut().default_folder = item_path(item); }
    SetFolder(item: *mut c_void) |d| { d.setup.borrow_mut().folder = item_path(item); }
    SetFileName(name: PCWSTR) |d| { d.setup.borrow_mut().file_name = text(name); }
    SetTitle(t: PCWSTR) |d| { d.setup.borrow_mut().title = text(t); }
    SetOkButtonLabel(t: PCWSTR) |d| { d.setup.borrow_mut().ok_label = text(t); }
    SetFileNameLabel(t: PCWSTR) |d| { d.setup.borrow_mut().name_label = text(t); }
    SetDefaultExtension(t: PCWSTR) |d| { d.setup.borrow_mut().default_ext = text(t); }
    AddPlace(item: *mut c_void, _where: FDAP) |d| {
        if let Some(p) = item_path(item) { d.setup.borrow_mut().places.push(p); }
    }
    SetFilter(_filter: *mut c_void) |d| { d.foreign.set(true); }
    Unadvise(cookie: u32) |d| { d.setup.borrow_mut().sinks.retain(|(c, _)| *c != cookie); }
}

unsafe extern "system" fn Advise<const S: usize>(this: *mut c_void, sink: *mut c_void, cookie: *mut u32) -> HRESULT {
    let d = me::<S>(this);
    let hr = (d.real.vtable().Advise)(d.real.as_raw(), sink, cookie);
    if hr.is_ok() && !cookie.is_null() {
        quiet(|| {
            if let Some(s) = IFileDialogEvents::from_raw_borrowed(&sink) {
                d.setup.borrow_mut().sinks.push((*cookie, s.clone()));
            }
        });
    }
    hr
}

unsafe extern "system" fn GetFileTypeIndex<const S: usize>(this: *mut c_void, out: *mut u32) -> HRESULT {
    let d = me::<S>(this);
    match d.picked() {
        Some(p) if !out.is_null() && p.ty > 0 => {
            *out = p.ty;
            S_OK
        }
        _ => (d.real.vtable().GetFileTypeIndex)(d.real.as_raw(), out),
    }
}

unsafe extern "system" fn GetFolder<const S: usize>(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    let d = me::<S>(this);
    match d.picked() {
        Some(p) => give(out, shell_item(&parent(&p.paths[0]))),
        None => (d.real.vtable().GetFolder)(d.real.as_raw(), out),
    }
}

unsafe extern "system" fn GetCurrentSelection<const S: usize>(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    let d = me::<S>(this);
    match d.picked() {
        Some(p) => give(out, shell_item(&p.paths[0])),
        None => (d.real.vtable().GetCurrentSelection)(d.real.as_raw(), out),
    }
}

unsafe extern "system" fn GetResult<const S: usize>(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    let d = me::<S>(this);
    match d.picked() {
        Some(p) => give(out, shell_item(&p.paths[0])),
        None => (d.real.vtable().GetResult)(d.real.as_raw(), out),
    }
}

unsafe extern "system" fn GetFileName<const S: usize>(this: *mut c_void, out: *mut PWSTR) -> HRESULT {
    let d = me::<S>(this);
    let Some(p) = d.picked() else { return (d.real.vtable().GetFileName)(d.real.as_raw(), out) };
    if out.is_null() {
        return E_POINTER;
    }
    let name = Path::new(&p.paths[0]).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mem = CoTaskMemAlloc(w.len() * 2) as *mut u16;
    if mem.is_null() {
        *out = PWSTR::null();
        return E_UNEXPECTED;
    }
    std::ptr::copy_nonoverlapping(w.as_ptr(), mem, w.len());
    *out = PWSTR(mem);
    S_OK
}

unsafe extern "system" fn Show<const S: usize>(this: *mut c_void, owner: HWND) -> HRESULT {
    let d = me::<S>(this);
    d.result.replace(None);
    let ours = catch_unwind(AssertUnwindSafe(|| show_ours(d, owner))).unwrap_or_else(|_| {
        log("błąd w obsłudze okna PowerDesk – okno Windows");
        None
    });
    match ours {
        Some(hr) => hr,
        None => {
            d.result.replace(None);
            d.busy.set(false);
            (d.real.vtable().base__.Show)(d.real.as_raw(), owner)
        }
    }
}

const fn fd_vtbl<const S: usize>() -> IFileDialog_Vtbl {
    IFileDialog_Vtbl {
        base__: IModalWindow_Vtbl { base__: IUnknown_Vtbl { QueryInterface: qi::<S>, AddRef: add_ref::<S>, Release: release::<S> }, Show: Show::<S> },
        SetFileTypes: SetFileTypes::<S>,
        SetFileTypeIndex: SetFileTypeIndex::<S>,
        GetFileTypeIndex: GetFileTypeIndex::<S>,
        Advise: Advise::<S>,
        Unadvise: Unadvise::<S>,
        SetOptions: SetOptions::<S>,
        GetOptions: GetOptions::<S>,
        SetDefaultFolder: SetDefaultFolder::<S>,
        SetFolder: SetFolder::<S>,
        GetFolder: GetFolder::<S>,
        GetCurrentSelection: GetCurrentSelection::<S>,
        SetFileName: SetFileName::<S>,
        GetFileName: GetFileName::<S>,
        SetTitle: SetTitle::<S>,
        SetOkButtonLabel: SetOkButtonLabel::<S>,
        SetFileNameLabel: SetFileNameLabel::<S>,
        GetResult: GetResult::<S>,
        AddPlace: AddPlace::<S>,
        SetDefaultExtension: SetDefaultExtension::<S>,
        Close: Close::<S>,
        SetClientGuid: SetClientGuid::<S>,
        ClearClientData: ClearClientData::<S>,
        SetFilter: SetFilter::<S>,
    }
}

// ---------- IFileOpenDialog / IFileSaveDialog / IFileDialog2 / IFileDialogCustomize ----------

/// Przekazanie do interfejsu oryginału trzymanego w polu `$field` (tablica `$slot`).
macro_rules! forward_to {
    ($slot:literal, $field:ident: $($name:ident($($a:ident: $t:ty),*);)*) => {$(
        unsafe extern "system" fn $name(this: *mut c_void $(, $a: $t)*) -> HRESULT {
            let d = me::<$slot>(this);
            match &d.$field {
                Some(r) => (r.vtable().$name)(r.as_raw() $(, $a)*),
                None => E_UNEXPECTED,
            }
        }
    )*};
}

unsafe extern "system" fn GetResults(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    let d = me::<0>(this);
    match (d.picked(), &d.real_open) {
        (Some(p), _) => give(out, shell_items(&p.paths)),
        (None, Some(r)) => (r.vtable().GetResults)(r.as_raw(), out),
        _ => E_UNEXPECTED,
    }
}

unsafe extern "system" fn GetSelectedItems(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    let d = me::<0>(this);
    match (d.picked(), &d.real_open) {
        (Some(p), _) => give(out, shell_items(&p.paths)),
        (None, Some(r)) => (r.vtable().GetSelectedItems)(r.as_raw(), out),
        _ => E_UNEXPECTED,
    }
}

unsafe extern "system" fn SetSaveAsItem(this: *mut c_void, item: *mut c_void) -> HRESULT {
    let d = me::<0>(this);
    let Some(r) = &d.real_save else { return E_UNEXPECTED };
    let hr = (r.vtable().SetSaveAsItem)(r.as_raw(), item);
    if hr.is_ok() {
        quiet(|| d.setup.borrow_mut().save_as = item_path(item));
    }
    hr
}

forward_to! { 0, real_save:
    SetProperties(store: *mut c_void);
    SetCollectedProperties(list: *mut c_void, append: BOOL);
    GetProperties(out: *mut *mut c_void);
    ApplyProperties(item: *mut c_void, store: *mut c_void, hwnd: HWND, sink: *mut c_void);
}

unsafe extern "system" fn SetCancelButtonLabel(this: *mut c_void, t: PCWSTR) -> HRESULT {
    let d = me::<1>(this);
    let Some(r) = &d.real2 else { return E_UNEXPECTED };
    let hr = (r.vtable().SetCancelButtonLabel)(r.as_raw(), t);
    if hr.is_ok() {
        quiet(|| d.setup.borrow_mut().cancel_label = text(t));
    }
    hr
}

unsafe extern "system" fn SetNavigationRoot(this: *mut c_void, item: *mut c_void) -> HRESULT {
    let d = me::<1>(this);
    let Some(r) = &d.real2 else { return E_UNEXPECTED };
    d.foreign.set(true);
    (r.vtable().SetNavigationRoot)(r.as_raw(), item)
}

forward_to! { 2, cust:
    SetControlLabel(id: u32, t: PCWSTR);
    GetControlState(id: u32, out: *mut CDCONTROLSTATEF);
    SetControlState(id: u32, st: CDCONTROLSTATEF);
    GetEditBoxText(id: u32, out: *mut *mut u16);
    SetEditBoxText(id: u32, t: PCWSTR);
    GetCheckButtonState(id: u32, out: *mut BOOL);
    SetCheckButtonState(id: u32, on: BOOL);
    AddControlItem(id: u32, item: u32, t: PCWSTR);
    RemoveControlItem(id: u32, item: u32);
    RemoveAllControlItems(id: u32);
    GetControlItemState(id: u32, item: u32, out: *mut CDCONTROLSTATEF);
    SetControlItemState(id: u32, item: u32, st: CDCONTROLSTATEF);
    GetSelectedControlItem(id: u32, out: *mut u32);
    SetSelectedControlItem(id: u32, item: u32);
    EndVisualGroup();
    SetControlItemText(id: u32, item: u32, t: PCWSTR);
}

/// Dodanie kontrolki do okna: program będzie czytał, co w niej wybrano – zostaje oryginał.
macro_rules! adds_control {
    ($($name:ident($($a:ident: $t:ty),*);)*) => {$(
        unsafe extern "system" fn $name(this: *mut c_void $(, $a: $t)*) -> HRESULT {
            let d = me::<2>(this);
            let Some(r) = &d.cust else { return E_UNEXPECTED };
            d.customized.set(true);
            (r.vtable().$name)(r.as_raw() $(, $a)*)
        }
    )*};
}

adds_control! {
    EnableOpenDropDown(id: u32);
    AddMenu(id: u32, t: PCWSTR);
    AddPushButton(id: u32, t: PCWSTR);
    AddComboBox(id: u32);
    AddRadioButtonList(id: u32);
    AddCheckButton(id: u32, t: PCWSTR, on: BOOL);
    AddEditBox(id: u32, t: PCWSTR);
    AddSeparator(id: u32);
    AddText(id: u32, t: PCWSTR);
    StartVisualGroup(id: u32, t: PCWSTR);
    MakeProminent(id: u32);
}

static OPEN_VT: IFileOpenDialog_Vtbl = IFileOpenDialog_Vtbl { base__: fd_vtbl::<0>(), GetResults, GetSelectedItems };
static SAVE_VT: IFileSaveDialog_Vtbl = IFileSaveDialog_Vtbl { base__: fd_vtbl::<0>(), SetSaveAsItem, SetProperties, SetCollectedProperties, GetProperties, ApplyProperties };
static D2_VT: IFileDialog2_Vtbl = IFileDialog2_Vtbl { base__: fd_vtbl::<1>(), SetCancelButtonLabel, SetNavigationRoot };
static CUST_VT: IFileDialogCustomize_Vtbl = IFileDialogCustomize_Vtbl {
    base__: IUnknown_Vtbl { QueryInterface: qi::<2>, AddRef: add_ref::<2>, Release: release::<2> },
    EnableOpenDropDown,
    AddMenu,
    AddPushButton,
    AddComboBox,
    AddRadioButtonList,
    AddCheckButton,
    AddEditBox,
    AddSeparator,
    AddText,
    SetControlLabel,
    GetControlState,
    SetControlState,
    GetEditBoxText,
    SetEditBoxText,
    GetCheckButtonState,
    SetCheckButtonState,
    AddControlItem,
    RemoveControlItem,
    RemoveAllControlItems,
    GetControlItemState,
    SetControlItemState,
    GetSelectedControlItem,
    SetSelectedControlItem,
    StartVisualGroup,
    EndVisualGroup,
    MakeProminent,
    SetControlItemText,
};

// ---------- Okno PowerDesk ----------

enum Outcome {
    Picked(Picked),
    Cancel,
    /// Powód (do dziennika) – pokazujemy okno Windows.
    Fallback(String),
}

/// `None` – pokaż oryginał; `Some(hr)` – wynik `Show`.
fn show_ours(d: &Dialog, owner: HWND) -> Option<HRESULT> {
    let exe = picker()?;
    if d.foreign.get() || d.customized.get() || d.busy.get() {
        return None;
    }
    let opts = unsafe { d.real.GetOptions() }.ok()?.0;
    // Elementy spoza systemu plików i pytanie o utworzenie pliku – tylko oryginał.
    if opts & (FOS_ALLNONSTORAGEITEMS.0 | FOS_CREATEPROMPT.0) != 0 {
        return None;
    }
    let req = request(d, opts, owner).to_string();
    d.busy.set(true);
    let out = run_picker(exe, owner, &req);
    d.busy.set(false);
    match out {
        Outcome::Fallback(why) => {
            log(why);
            None
        }
        Outcome::Cancel => Some(ERROR_CANCELLED.to_hresult()),
        Outcome::Picked(p) => {
            if p.paths.is_empty() {
                return None;
            }
            d.result.replace(Some(p.clone()));
            // Program może odrzucić wybór (IFileDialogEvents::OnFileOk zwraca S_FALSE) – wtedy
            // zwykłe okno z tymi samymi ustawieniami, żeby użytkownik zobaczył jego komunikat.
            let sinks: Vec<IFileDialogEvents> = d.setup.borrow().sinks.iter().map(|(_, s)| s.clone()).collect();
            for s in sinks {
                let hr = unsafe { (s.vtable().OnFileOk)(s.as_raw(), d.base()) };
                if hr != S_OK {
                    log(format!("program odrzucił wybór ({hr:?}) – okno Windows"));
                    d.result.replace(None);
                    return None;
                }
            }
            if opts & FOS_DONTADDTORECENT.0 == 0 {
                for path in &p.paths {
                    let h = HSTRING::from(path.as_str());
                    // SHARD_PATHW = 3
                    unsafe { SHAddToRecentDocs(3, Some(h.as_ptr() as *const c_void)) };
                }
            }
            Some(S_OK)
        }
    }
}

fn request(d: &Dialog, opts: u32, owner: HWND) -> Value {
    let suggested = unsafe { d.real.GetFolder() }.ok().and_then(|i| fs_path(&i));
    let st = d.setup.borrow();
    let save_as = st.save_as.clone();
    let name = st.file_name.clone().or_else(|| save_as.as_ref().and_then(|p| Path::new(p).file_name().map(|n| n.to_string_lossy().to_string())));
    let folder = st.folder.clone().or_else(|| save_as.as_ref().map(|p| parent(p)));
    let app = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
    let has = |f: FILEOPENDIALOGOPTIONS| opts & f.0 != 0;
    json!({
        "save": d.save,
        "title": st.title,
        "ok": st.ok_label,
        "cancel": st.cancel_label,
        "name_label": st.name_label,
        "name": name,
        "folder": folder,
        "suggested": suggested.or_else(|| st.default_folder.clone()),
        "ext": st.default_ext,
        "filters": st.filters.iter().map(|(n, s)| json!([n, s])).collect::<Vec<_>>(),
        "type": st.type_index,
        "multi": has(FOS_ALLOWMULTISELECT),
        "folders": has(FOS_PICKFOLDERS),
        "must_exist": has(FOS_FILEMUSTEXIST),
        "path_must_exist": has(FOS_PATHMUSTEXIST),
        "overwrite": has(FOS_OVERWRITEPROMPT),
        "no_readonly": has(FOS_NOREADONLYRETURN),
        "hidden": has(FOS_FORCESHOWHIDDEN),
        "places": st.places,
        "owner": owner.0 as isize as i64,
        "app": app,
    })
}

fn parse(line: &str) -> Option<Outcome> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    match v["result"].as_str()? {
        "ok" => {
            let paths: Vec<String> = v["paths"].as_array()?.iter().filter_map(|p| p.as_str().map(String::from)).collect();
            Some(Outcome::Picked(Picked { paths, ty: v["type"].as_u64().unwrap_or(0) as u32 }))
        }
        "cancel" => Some(Outcome::Cancel),
        _ => Some(Outcome::Fallback("wybrano okno Windows".into())),
    }
}

/// Uruchamia okno PowerDesk i czeka na wynik jak okno modalne: właściciel wyłączony, komunikaty
/// programu obsługiwane (żeby nie „przestał odpowiadać”).
fn run_picker(exe: &Path, owner: HWND, req: &str) -> Outcome {
    use std::os::windows::io::AsRawHandle;
    use std::process::{Command, Stdio};
    let mut child = match Command::new(exe).arg(PICK_FLAG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Fallback(format!("nie można uruchomić PowerDesk: {e}")),
    };
    unsafe {
        let _ = AllowSetForegroundWindow(child.id());
    }
    if let Some(mut input) = child.stdin.take() {
        if input.write_all(req.as_bytes()).and_then(|_| input.write_all(b"\n")).is_err() {
            let _ = child.kill();
            return Outcome::Fallback("PowerDesk nie przyjął opisu okna".into());
        }
    }
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let disabled = unsafe { !owner.is_invalid() && IsWindow(Some(owner)).as_bool() && IsWindowEnabled(owner).as_bool() };
    if disabled {
        unsafe {
            let _ = EnableWindow(owner, false);
        }
    }
    let process = HANDLE(child.as_raw_handle());
    let start = Instant::now();
    let mut ready = false;
    let mut quit = None;
    let mut res = None;
    loop {
        while let Ok(line) = rx.try_recv() {
            if line.trim() == "ready" {
                ready = true;
            } else if let Some(o) = parse(&line) {
                res = Some(o);
            }
        }
        if res.is_some() {
            break;
        }
        if let Ok(Some(_)) = child.try_wait() {
            // Ostatni wiersz mógł jeszcze nie przejść przez wątek czytający.
            while let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
                if let Some(o) = parse(&line) {
                    res = Some(o);
                    break;
                }
            }
            break;
        }
        if !ready && start.elapsed() > START_LIMIT {
            let _ = child.kill();
            res = Some(Outcome::Fallback("okno PowerDesk nie pokazało się na czas".into()));
            break;
        }
        unsafe {
            MsgWaitForMultipleObjects(Some(&[process]), false, 100, QS_ALLINPUT);
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    quit = Some(msg.wParam.0 as i32);
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if quit.is_some() {
            let _ = child.kill();
            res = Some(Outcome::Cancel);
            break;
        }
    }
    unsafe {
        if disabled {
            let _ = EnableWindow(owner, true);
        }
        if !owner.is_invalid() {
            let _ = SetForegroundWindow(owner);
        }
        if let Some(code) = quit {
            PostQuitMessage(code);
        }
    }
    res.unwrap_or_else(|| Outcome::Fallback("PowerDesk zakończył się bez odpowiedzi".into()))
}

// ---------- Eksport biblioteki ----------

/// Wejście COM: fabryka okna otwierania albo zapisu.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || clsid.is_null() || iid.is_null() {
        return E_POINTER;
    }
    *out = null_mut();
    let f: &'static Factory = if *clsid == FileOpenDialog {
        &OPEN_FACTORY
    } else if *clsid == FileSaveDialog {
        &SAVE_FACTORY
    } else {
        return CLASS_E_CLASSNOTAVAILABLE;
    };
    f_qi(f as *const Factory as *mut c_void, iid, out)
}

/// Biblioteka zostaje w procesie do jego końca (wątki czytające, obiekty w użyciu programu).
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
