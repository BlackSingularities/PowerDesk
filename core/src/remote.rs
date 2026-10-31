//! Serwery FTP przeglądane jak foldery: `ftp:\\serwer:21\katalog\plik`.
//! Hasła trafiają do Menedżera poświadczeń Windows (opcjonalnie) albo tylko do pamięci na czas sesji.

use std::collections::HashMap;
use std::io::Read;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use suppaftp::types::FileType;
use suppaftp::{FtpStream, Mode};

use crate::fsx::{io_err, Entry};

pub const PREFIX: &str = "ftp:\\\\";

#[derive(Clone)]
struct Cred {
    user: String,
    pass: String,
}

static CREDS: Lazy<Mutex<HashMap<String, Cred>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn is_remote(path: &str) -> bool {
    path.len() > PREFIX.len() && path[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
}

/// (klucz „serwer:port”, ścieżka na serwerze „/a/b”).
fn split(path: &str) -> Result<(String, String), String> {
    if !is_remote(path) {
        return Err(crate::lang::tr("To nie jest ścieżka FTP").into());
    }
    let rest = &path[PREFIX.len()..];
    let mut parts = rest.splitn(2, '\\');
    let host = parts.next().unwrap_or("").to_lowercase();
    let inner = parts.next().unwrap_or("").trim_matches('\\').replace('\\', "/");
    let key = if host.contains(':') { host } else { format!("{host}:21") };
    Ok((key, format!("/{inner}")))
}

fn root_of(key: &str) -> String {
    let shown = key.strip_suffix(":21").unwrap_or(key);
    format!("{PREFIX}{shown}")
}

fn to_local(key: &str, remote: &str) -> String {
    let r = remote.trim_matches('/').replace('/', "\\");
    if r.is_empty() {
        root_of(key)
    } else {
        format!("{}\\{}", root_of(key), r)
    }
}

// ---------- Menedżer poświadczeń Windows ----------

fn target(key: &str) -> windows::core::HSTRING {
    windows::core::HSTRING::from(format!("OpenFilePower:ftp:{key}"))
}

fn cred_save(key: &str, c: &Cred) {
    use windows::Win32::Security::Credentials::*;
    let t = target(key);
    let user = windows::core::HSTRING::from(c.user.as_str());
    let blob: Vec<u16> = c.pass.encode_utf16().collect();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: windows::core::PWSTR(t.as_ptr() as *mut _),
        CredentialBlobSize: (blob.len() * 2) as u32,
        CredentialBlob: blob.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: windows::core::PWSTR(user.as_ptr() as *mut _),
        ..Default::default()
    };
    unsafe {
        let _ = CredWriteW(&cred, 0);
    }
}

fn cred_load(key: &str) -> Option<Cred> {
    use windows::Win32::Security::Credentials::*;
    let t = target(key);
    let mut p: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        CredReadW(&t, CRED_TYPE_GENERIC, None, &mut p).ok()?;
        let c = &*p;
        let user = c.UserName.to_string().unwrap_or_default();
        let blob = std::slice::from_raw_parts(c.CredentialBlob as *const u16, c.CredentialBlobSize as usize / 2);
        let pass = String::from_utf16_lossy(blob);
        CredFree(p as *const _);
        Some(Cred { user, pass })
    }
}

pub fn forget(key_or_path: &str) {
    use windows::Win32::Security::Credentials::*;
    let key = split(key_or_path).map(|(k, _)| k).unwrap_or_else(|_| key_or_path.to_lowercase());
    CREDS.lock().remove(&key);
    unsafe {
        let _ = CredDeleteW(&target(&key), CRED_TYPE_GENERIC, None);
    }
}

// ---------- Połączenie ----------

fn open(key: &str, c: &Cred) -> Result<FtpStream, String> {
    let addr = key.to_socket_addrs().map_err(|e| crate::trf!("Nie znaleziono serwera: {e}"))?.next().ok_or(crate::lang::tr("Nie znaleziono serwera"))?;
    let mut f = FtpStream::connect_timeout(addr, Duration::from_secs(8)).map_err(|e| crate::trf!("Brak połączenia z serwerem: {e}"))?;
    f.get_ref().set_read_timeout(Some(Duration::from_secs(30))).ok();
    let user = if c.user.is_empty() { "anonymous" } else { c.user.as_str() };
    f.login(user, c.pass.as_str()).map_err(|_| crate::lang::tr("AUTH: Serwer odrzucił login lub hasło").to_string())?;
    f.set_mode(Mode::Passive);
    f.transfer_type(FileType::Binary).map_err(|e| e.to_string())?;
    Ok(f)
}

fn connect(key: &str) -> Result<FtpStream, String> {
    let c = CREDS.lock().get(key).cloned().or_else(|| cred_load(key));
    let Some(c) = c else { return Err(crate::lang::tr("AUTH: Podaj login i hasło do serwera").into()) };
    CREDS.lock().insert(key.to_string(), c.clone());
    open(key, &c)
}

/// Logowanie z okna „Połącz z lokalizacją sieciową”. Zwraca ścieżkę główną serwera.
pub fn login(host: &str, port: u16, user: &str, pass: &str, remember: bool, path: &str) -> Result<String, String> {
    let host = host.trim().trim_start_matches("ftp://").trim_end_matches('/').to_lowercase();
    if host.is_empty() {
        return Err(crate::lang::tr("Podaj adres serwera").into());
    }
    let key = format!("{host}:{port}");
    let c = Cred { user: user.trim().to_string(), pass: pass.to_string() };
    let mut f = open(&key, &c).map_err(|e| e.trim_start_matches("AUTH: ").to_string())?;
    let _ = f.quit();
    CREDS.lock().insert(key.clone(), c.clone());
    if remember {
        cred_save(&key, &c);
    }
    Ok(to_local(&key, path))
}

fn parse_line(line: &str) -> Option<suppaftp::list::File> {
    suppaftp::list::File::try_from(line).ok()
}

fn mk_entry(key: &str, dir_remote: &str, f: &suppaftp::list::File) -> Entry {
    let name = f.name().to_string();
    let remote = format!("{}/{}", dir_remote.trim_end_matches('/'), name);
    let dir = f.is_directory();
    let modified = f.modified().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    Entry {
        path: to_local(key, &remote),
        ext: if dir { String::new() } else { Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default() },
        name,
        dir,
        size: if dir { 0 } else { f.size() as u64 },
        modified,
        created: modified,
        hidden: f.name().starts_with('.'),
        system: false,
        readonly: false,
        link: f.is_symlink(),
        cloud: false,
    }
}

fn list_remote(f: &mut FtpStream, key: &str, remote: &str) -> Result<Vec<Entry>, String> {
    let lines = f.list(Some(remote)).map_err(|e| crate::trf!("Nie udało się odczytać katalogu: {e}"))?;
    Ok(lines
        .iter()
        .filter_map(|l| parse_line(l))
        .filter(|x| x.name() != "." && x.name() != "..")
        .map(|x| mk_entry(key, remote, &x))
        .collect())
}

pub fn list(path: &str) -> Result<Vec<Entry>, String> {
    let (key, remote) = split(path)?;
    let mut f = connect(&key)?;
    let out = list_remote(&mut f, &key, &remote);
    let _ = f.quit();
    out
}

// ---------- Transfer ----------

fn download_into(f: &mut FtpStream, key: &str, remote: &str, is_dir: bool, dest: &Path) -> Result<(), String> {
    let name = remote.rsplit('/').next().unwrap_or("plik");
    let target = crate::ops::unique_path(&dest.join(name));
    if is_dir {
        std::fs::create_dir_all(&target).map_err(|e| io_err(&e))?;
        for e in list_remote(f, key, remote)? {
            let (_, r) = split(&e.path)?;
            download_into(f, key, &r, e.dir, &target)?;
        }
        return Ok(());
    }
    let mut s = f.retr_as_stream(remote).map_err(|e| format!("{name}: {e}"))?;
    let mut w = std::fs::File::create(&target).map_err(|e| io_err(&e))?;
    std::io::copy(&mut s, &mut w).map_err(|e| io_err(&e))?;
    f.finalize_retr_stream(s).map_err(|e| e.to_string())?;
    Ok(())
}

/// Pobiera elementy z serwera do lokalnego folderu. `dirs` – które z nich są folderami.
pub fn download(sources: &[(String, bool)], dest: &str) -> Result<usize, String> {
    let mut by_key: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for (s, d) in sources {
        let (k, r) = split(s)?;
        by_key.entry(k).or_default().push((r, *d));
    }
    let mut n = 0;
    for (k, items) in by_key {
        let mut f = connect(&k)?;
        for (r, d) in items {
            download_into(&mut f, &k, &r, d, Path::new(dest))?;
            n += 1;
        }
        let _ = f.quit();
    }
    Ok(n)
}

fn upload_from(f: &mut FtpStream, local: &Path, remote_dir: &str) -> Result<(), String> {
    let name = local.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let target = format!("{}/{}", remote_dir.trim_end_matches('/'), name);
    if local.is_dir() {
        let _ = f.mkdir(&target);
        for de in std::fs::read_dir(local).map_err(|e| io_err(&e))?.flatten() {
            upload_from(f, &de.path(), &target)?;
        }
        return Ok(());
    }
    let mut r = std::fs::File::open(local).map_err(|e| io_err(&e))?;
    f.put_file(&target, &mut r).map_err(|e| format!("{name}: {e}"))?;
    Ok(())
}

pub fn upload(sources: &[String], dest: &str) -> Result<usize, String> {
    let (k, r) = split(dest)?;
    let mut f = connect(&k)?;
    for s in sources {
        upload_from(&mut f, Path::new(s), &r)?;
    }
    let _ = f.quit();
    Ok(sources.len())
}

/// Pobiera plik do folderu tymczasowego (otwarcie w programie).
pub fn fetch_temp(path: &str) -> Result<String, String> {
    let (k, r) = split(path)?;
    let dest: PathBuf = std::env::temp_dir().join("OpenFilePower").join("ftp").join(k.replace(':', "_"));
    std::fs::create_dir_all(&dest).map_err(|e| io_err(&e))?;
    let name = r.rsplit('/').next().unwrap_or("plik").to_string();
    let target = dest.join(&name);
    let _ = std::fs::remove_file(&target);
    let mut f = connect(&k)?;
    let mut s = f.retr_as_stream(&r).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).map_err(|e| io_err(&e))?;
    f.finalize_retr_stream(s).map_err(|e| e.to_string())?;
    let _ = f.quit();
    std::fs::write(&target, buf).map_err(|e| io_err(&e))?;
    Ok(target.to_string_lossy().to_string())
}

// ---------- Zmiany na serwerze ----------

fn remove(f: &mut FtpStream, key: &str, remote: &str, dir: bool) -> Result<(), String> {
    if dir {
        for e in list_remote(f, key, remote)? {
            let (_, r) = split(&e.path)?;
            remove(f, key, &r, e.dir)?;
        }
        f.rmdir(remote).map_err(|e| e.to_string())
    } else {
        f.rm(remote).map_err(|e| e.to_string())
    }
}

pub fn delete(items: &[(String, bool)]) -> Result<usize, String> {
    let mut n = 0;
    for (p, d) in items {
        let (k, r) = split(p)?;
        let mut f = connect(&k)?;
        remove(&mut f, &k, &r, *d)?;
        let _ = f.quit();
        n += 1;
    }
    Ok(n)
}

pub fn mkdir(parent: &str, name: &str) -> Result<String, String> {
    let (k, r) = split(&format!("{}\\{}", parent.trim_end_matches('\\'), name))?;
    let mut f = connect(&k)?;
    f.mkdir(&r).map_err(|e| e.to_string())?;
    let _ = f.quit();
    Ok(to_local(&k, &r))
}

pub fn rename(path: &str, name: &str) -> Result<String, String> {
    let (k, r) = split(path)?;
    let to = format!("{}/{}", r.rsplit_once('/').map(|x| x.0).unwrap_or(""), name);
    let mut f = connect(&k)?;
    f.rename(&r, &to).map_err(|e| e.to_string())?;
    let _ = f.quit();
    Ok(to_local(&k, &to))
}
