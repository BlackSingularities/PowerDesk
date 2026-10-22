//! Biblioteki Windows (Dokumenty, Obrazy… i własne) przez IShellLibrary –
//! zmiany są widoczne także w Eksploratorze.

use serde::Serialize;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ, STGM_READWRITE};
use windows::Win32::UI::Shell::{
    FOLDERID_UsersLibraries, IShellItem, IShellItemArray, IShellLibrary, SHCreateItemFromParsingName, ShellLibrary, LFF_ALLITEMS,
    LSF_MAKEUNIQUENAME, SIGDN_FILESYSPATH,
};

#[derive(Serialize)]
pub struct Library {
    pub name: String,
    /// Ścieżka pliku .library-ms.
    pub path: String,
    pub folders: Vec<String>,
}

fn libs_dir() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_default().join("Microsoft").join("Windows").join("Libraries")
}

unsafe fn item(path: &str) -> Result<IShellItem, String> {
    SHCreateItemFromParsingName(&HSTRING::from(path), None).map_err(|e| e.message().to_string())
}

unsafe fn fs_path(it: &IShellItem) -> Option<String> {
    let p: PWSTR = it.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

unsafe fn open(path: &str, write: bool) -> Result<IShellLibrary, String> {
    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let lib: IShellLibrary = CoCreateInstance(&ShellLibrary, None, CLSCTX_INPROC_SERVER).map_err(|e| e.message().to_string())?;
    lib.LoadLibraryFromItem(&item(path)?, if write { STGM_READWRITE.0 } else { STGM_READ.0 }).map_err(|e| e.message().to_string())?;
    Ok(lib)
}

unsafe fn folders_of(lib: &IShellLibrary) -> Vec<String> {
    let Ok(arr) = lib.GetFolders::<IShellItemArray>(LFF_ALLITEMS) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0);
    (0..n).filter_map(|i| arr.GetItemAt(i).ok().and_then(|it| fs_path(&it))).collect()
}

pub fn list() -> Vec<Library> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(libs_dir()) else { return out };
    for de in rd.flatten() {
        let p = de.path();
        if p.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("library-ms".into()) {
            continue;
        }
        let path = p.to_string_lossy().to_string();
        let folders = unsafe { open(&path, false).map(|l| folders_of(&l)).unwrap_or_default() };
        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        out.push(Library { name: localized(&name), path, folders });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Biblioteki systemowe mają angielskie nazwy plików – pokazujemy polskie.
fn localized(name: &str) -> String {
    match name {
        "Documents" => "Dokumenty",
        "Pictures" => "Obrazy",
        "Music" => "Muzyka",
        "Videos" => "Wideo",
        "CameraRoll" | "Camera Roll" => "Z aparatu",
        "SavedPictures" | "Saved Pictures" => "Zapisane obrazy",
        n => n,
    }
    .to_string()
}

pub fn create(name: &str, folder: Option<&str>) -> Result<String, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let lib: IShellLibrary = CoCreateInstance(&ShellLibrary, None, CLSCTX_INPROC_SERVER).map_err(|e| e.message().to_string())?;
        if let Some(f) = folder {
            lib.AddFolder(&item(f)?).map_err(|e| e.message().to_string())?;
        }
        let saved: IShellItem = lib.SaveInKnownFolder(&FOLDERID_UsersLibraries, &HSTRING::from(name), LSF_MAKEUNIQUENAME).map_err(|e| e.message().to_string())?;
        fs_path(&saved).ok_or_else(|| "Nie udało się zapisać biblioteki".into())
    }
}

pub fn add_folder(lib_path: &str, folder: &str) -> Result<(), String> {
    unsafe {
        let lib = open(lib_path, true)?;
        lib.AddFolder(&item(folder)?).map_err(|e| e.message().to_string())?;
        lib.Commit().map_err(|e| e.message().to_string())
    }
}

pub fn remove_folder(lib_path: &str, folder: &str) -> Result<(), String> {
    unsafe {
        let lib = open(lib_path, true)?;
        lib.RemoveFolder(&item(folder)?).map_err(|e| e.message().to_string())?;
        lib.Commit().map_err(|e| e.message().to_string())
    }
}

#[allow(dead_code)]
fn _assert_interface<T: Interface>() {}
