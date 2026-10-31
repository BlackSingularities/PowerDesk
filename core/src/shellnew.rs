//! Menu „Nowy” jak w Eksploratorze: typy plików, które programy rejestrują w `HKCR\.ext\ShellNew`
//! (albo `HKCR\.ext\<ProgID>\ShellNew`) – pusty plik, plik z szablonu albo zapisane w rejestrze dane.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ, REG_BINARY, REG_VALUE_TYPE};

/// Typ do utworzenia: rozszerzenie (z kropką), nazwa typu i zawartość nowego pliku.
#[derive(Clone, Debug)]
pub struct NewType {
    pub ext: String,
    pub name: String,
    source: Source,
}

#[derive(Clone, Debug)]
enum Source {
    Empty,
    Template(String),
    Data(Vec<u8>),
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open(parent: HKEY, sub: &str) -> Option<Key> {
    let mut k = HKEY::default();
    let ok = unsafe { RegOpenKeyExW(parent, &HSTRING::from(sub), Some(0), KEY_READ, &mut k) } == ERROR_SUCCESS;
    ok.then_some(Key(k))
}

/// Wartość (pusta nazwa – domyślna): typ i surowe bajty. None – brak wartości.
fn value(k: &Key, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let name = HSTRING::from(name);
    let mut ty = REG_VALUE_TYPE::default();
    let mut len = 0u32;
    unsafe {
        if RegQueryValueExW(k.0, PCWSTR(name.as_ptr()), None, Some(&mut ty), None, Some(&mut len)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if RegQueryValueExW(k.0, PCWSTR(name.as_ptr()), None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut len)) != ERROR_SUCCESS {
            return None;
        }
        buf.truncate(len as usize);
        Some((ty, buf))
    }
}

fn text(bytes: &[u8]) -> String {
    let w: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
}

fn string(k: &Key, name: &str) -> Option<String> {
    value(k, name).map(|(_, b)| text(&b)).filter(|s| !s.is_empty())
}

/// „@plik.dll,-123” – napis z zasobów programu.
fn indirect(s: String) -> String {
    if !s.starts_with('@') {
        return s;
    }
    let mut out = vec![0u16; 512];
    match unsafe { windows::Win32::UI::Shell::SHLoadIndirectString(&HSTRING::from(s.as_str()), &mut out, None) } {
        Ok(()) => String::from_utf16_lossy(&out[..out.iter().position(|&c| c == 0).unwrap_or(out.len())]),
        Err(_) => String::new(),
    }
}

fn shell_new(ext: &Key, progid: &Option<String>) -> Option<Key> {
    open(ext.0, "ShellNew").or_else(|| progid.as_ref().and_then(|p| open(ext.0, &format!(r"{p}\ShellNew"))))
}

/// Typy z menu „Nowy” (bez folderu, skrótu i poleceń programów), posortowane po nazwie.
/// Przegląda cały HKCR – wywoływać w tle.
pub fn types() -> Vec<NewType> {
    let mut out: Vec<NewType> = Vec::new();
    let mut i = 0u32;
    loop {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        let res = unsafe { RegEnumKeyExW(HKEY_CLASSES_ROOT, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        i += 1;
        if res == windows::Win32::Foundation::ERROR_NO_MORE_ITEMS {
            break;
        }
        if res != ERROR_SUCCESS {
            continue;
        }
        let ext = String::from_utf16_lossy(&name[..len as usize]);
        if !ext.starts_with('.') || matches!(ext.to_lowercase().as_str(), ".lnk" | ".library-ms" | ".url") {
            continue;
        }
        let Some(ek) = open(HKEY_CLASSES_ROOT, &ext) else { continue };
        let progid = string(&ek, "");
        let Some(sn) = shell_new(&ek, &progid) else { continue };
        let source = if value(&sn, "NullFile").is_some() {
            Source::Empty
        } else if let Some(f) = string(&sn, "FileName") {
            Source::Template(f)
        } else if let Some((ty, data)) = value(&sn, crate::lang::tr("Data")) {
            Source::Data(if ty == REG_BINARY { data } else { text(&data).into_bytes() })
        } else {
            continue;
        };
        let friendly = progid.as_ref().and_then(|p| open(HKEY_CLASSES_ROOT, p)).and_then(|k| string(&k, "FriendlyTypeName").map(indirect).filter(|s| !s.is_empty()).or_else(|| string(&k, "")));
        let Some(friendly) = friendly else { continue };
        if out.iter().any(|t| t.name.eq_ignore_ascii_case(&friendly)) {
            continue;
        }
        out.push(NewType { ext, name: friendly, source });
    }
    out.sort_by_key(|t| t.name.to_lowercase());
    out
}

fn template_path(f: &str) -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(f);
    if p.is_absolute() {
        return p.is_file().then_some(p);
    }
    let env = |k: &str| std::env::var(k).ok().map(std::path::PathBuf::from);
    [
        env("APPDATA").map(|d| d.join(r"Microsoft\Windows\Templates")),
        env("ProgramData").map(|d| d.join(r"Microsoft\Windows\Templates")),
        env("SystemRoot").map(|d| d.join("ShellNew")),
    ]
    .into_iter()
    .flatten()
    .map(|d| d.join(f))
    .find(|p| p.is_file())
}

/// Tworzy w `dir` nowy plik tego typu („Nowy <typ>.ext”, zajęta nazwa dostaje numer). Zwraca ścieżkę.
pub fn create(dir: &str, t: &NewType) -> Result<String, String> {
    let path = crate::ops::create(dir, &format!("{} {}{}", "Nowy", t.name, t.ext), false)?;
    let res = match &t.source {
        Source::Empty => Ok(()),
        Source::Template(f) => match template_path(f) {
            Some(src) => std::fs::copy(src, &path).map(|_| ()).map_err(|e| e.to_string()),
            None => Ok(()),
        },
        Source::Data(d) => std::fs::write(&path, d).map_err(|e| e.to_string()),
    };
    res.map(|_| path)
}
