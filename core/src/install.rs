//! Instalacja i deinstalacja PowerDesk bez osobnego instalatora: ten sam plik .exe kopiuje się
//! do folderu programu, zakłada skrót w menu Start i wpis w „Aplikacje i funkcje”.
//! Dla bieżącego użytkownika: `%LOCALAPPDATA%\Programs\PowerDesk` + HKCU (bez administratora);
//! dla wszystkich: `%ProgramFiles%\PowerDesk` + HKLM (Windows pyta o uprawnienia).
//! Integracja z Windows (powłoka, Win+E, autostart) jest zawsze per użytkownik – odnawia ją
//! zainstalowany program przy starcie, a zdejmuje `integration::remove`.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

use crate::winkey::reg;

pub const INSTALL_FLAG: &str = "--install";
pub const ALL_USERS_FLAG: &str = "--all-users";
pub const UNINSTALL_FLAG: &str = "--uninstall";
pub const QUIET_FLAG: &str = "--quiet";
/// Aktualizacja z działającego PowerDesk: bez pytań, bez zamykania działającej kopii (ta się
/// zamknie sama i uruchomi nową) i bez uruchamiania programu z uprawnieniami administratora.
pub const UPDATE_FLAG: &str = "--update";
/// Kopia z uprawnieniami instaluje, ale programu nie uruchamia – robi to proces użytkownika,
/// który ją wywołał (PowerDesk jako powłoka nie może działać z uprawnieniami administratora).
pub const NO_LAUNCH_FLAG: &str = "--no-launch";

const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\PowerDesk";
const NO_WINDOW: u32 = 0x0800_0000;

fn hive(all_users: bool) -> &'static str {
    if all_users {
        "HKLM"
    } else {
        "HKCU"
    }
}

fn env_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var(var).map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(fallback))
}

/// Folder programu.
pub fn install_dir(all_users: bool) -> PathBuf {
    if all_users {
        env_dir("ProgramFiles", r"C:\Program Files").join("PowerDesk")
    } else {
        env_dir("LOCALAPPDATA", r"C:\Users\Default\AppData\Local").join("Programs").join("PowerDesk")
    }
}

/// Skrót w menu Start (bieżącego użytkownika albo wszystkich).
fn shortcut(all_users: bool) -> PathBuf {
    let base = if all_users { env_dir("ProgramData", r"C:\ProgramData") } else { env_dir("APPDATA", r"C:\Users\Default\AppData\Roaming") };
    base.join(r"Microsoft\Windows\Start Menu\Programs\PowerDesk.lnk")
}

fn query(all_users: bool, value: &str) -> Option<String> {
    let out = reg(&["query", &format!(r"{}\{KEY}", hive(all_users)), "/v", value]).ok()?;
    out.lines().find(|l| l.trim_start().starts_with(value)).and_then(|l| l.split("REG_").nth(1)).and_then(|v| v.split_once(char::is_whitespace)).map(|(_, v)| v.trim().to_string())
}

/// Zainstalowana kopia: (plik .exe, dla wszystkich).
pub fn installed() -> Option<(PathBuf, bool)> {
    for all in [false, true] {
        if let Some(dir) = query(all, "InstallLocation") {
            let exe = PathBuf::from(dir).join("powerdesk.exe");
            if exe.is_file() {
                return Some((exe, all));
            }
        }
    }
    None
}

fn same_file(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    canon(a) == canon(b)
}

/// Wersja zapisana we wpisie „Aplikacje i funkcje” zainstalowanej kopii.
pub fn installed_version() -> Option<String> {
    let (_, all) = installed()?;
    query(all, "DisplayVersion")
}

/// Czy ten proces to zainstalowana kopia.
pub fn running_installed() -> bool {
    let Ok(me) = std::env::current_exe() else { return false };
    installed().is_some_and(|(exe, _)| same_file(&exe, &me))
}

pub fn is_admin() -> bool {
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
}

/// Uruchamia plik `exe` z argumentami – jako administrator (UAC), gdy `elevated` – i czeka na
/// jego koniec. Zwraca kod wyjścia.
pub fn run_wait(exe: &Path, args: &str, elevated: bool) -> Result<u32, String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: if elevated { w!("runas") } else { PCWSTR::null() },
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|_| if elevated { "Brak zgody na uprawnienia administratora".to_string() } else { "Nie można uruchomić instalatora".to_string() })?;
        if info.hProcess.is_invalid() {
            return Ok(0);
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        Ok(code)
    }
}

/// Uruchamia ten plik ponownie jako administrator z podanymi argumentami (Windows pyta przez UAC).
pub fn relaunch_elevated(args: &str) -> Result<(), String> {
    use windows::core::w;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let r = unsafe { ShellExecuteW(None, w!("runas"), &HSTRING::from(me.as_os_str()), &HSTRING::from(args), None, SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err("Brak zgody na uprawnienia administratora".into())
    }
}

fn make_shortcut(link: &Path, target: &Path) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        sl.SetPath(&HSTRING::from(target.as_os_str())).map_err(|e| e.to_string())?;
        if let Some(dir) = target.parent() {
            let _ = sl.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()));
        }
        let _ = sl.SetDescription(&HSTRING::from("PowerDesk – pliki, pulpit i pasek zadań"));
        let pf: IPersistFile = sl.cast().map_err(|e| e.to_string())?;
        if let Some(dir) = link.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        pf.Save(&HSTRING::from(link.as_os_str()), true).map_err(|e| e.to_string())
    }
}

fn dir_size_kb(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum::<u64>() / 1024).unwrap_or(0)
}

/// Wpis w „Aplikacje i funkcje” (wersja, rozmiar, polecenie deinstalacji).
pub fn write_entry(exe: &Path, all_users: bool, version: &str) -> Result<(), String> {
    let key = format!(r"{}\{KEY}", hive(all_users));
    let exe_s = exe.to_string_lossy().to_string();
    let dir = exe.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    let size = dir_size_kb(exe.parent().unwrap_or(exe)).to_string();
    let sz = |name: &str, val: &str| reg(&["add", &key, "/v", name, "/t", "REG_SZ", "/d", val, "/f"]).map(|_| ());
    let dw = |name: &str, val: &str| reg(&["add", &key, "/v", name, "/t", "REG_DWORD", "/d", val, "/f"]).map(|_| ());
    sz("DisplayName", "PowerDesk")?;
    sz("DisplayVersion", version)?;
    sz("Publisher", "adam001d")?;
    sz("DisplayIcon", &exe_s)?;
    sz("InstallLocation", &dir)?;
    sz("UninstallString", &format!("\"{exe_s}\" {UNINSTALL_FLAG}"))?;
    sz("QuietUninstallString", &format!("\"{exe_s}\" {UNINSTALL_FLAG} {QUIET_FLAG}"))?;
    dw("NoModify", "1")?;
    dw("NoRepair", "1")?;
    dw("EstimatedSize", &size)?;
    Ok(())
}

/// Wersja we wpisie po aktualizacji (program zainstalowany, wpis starszy).
pub fn refresh_entry(version: &str) {
    if let Some((exe, all)) = installed() {
        if running_installed() && query(all, "DisplayVersion").as_deref() != Some(version) && (!all || is_admin()) {
            let _ = write_entry(&exe, all, version);
        }
    }
}

/// Kopiuje ten plik do folderu programu, zakłada skrót i wpis. Zwraca ścieżkę zainstalowanego .exe.
/// Starsza kopia, która właśnie działa, zostaje przemianowana na .old (Windows pozwala na to
/// przy działającym pliku) – nowa wejdzie po ponownym uruchomieniu.
pub fn install(all_users: bool, version: &str) -> Result<PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir(all_users);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Nie można utworzyć {}: {e}", dir.display()))?;
    let target = dir.join("powerdesk.exe");
    if !same_file(&me, &target) {
        if target.exists() && std::fs::copy(&me, &target).is_err() {
            let old = dir.join("powerdesk.exe.old");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&target, &old).map_err(|e| format!("Nie można zastąpić starej wersji: {e}"))?;
            std::fs::copy(&me, &target).map_err(|e| e.to_string())?;
        } else if !target.exists() {
            std::fs::copy(&me, &target).map_err(|e| format!("Nie można skopiować programu: {e}"))?;
        }
    }
    make_shortcut(&shortcut(all_users), &target)?;
    write_entry(&target, all_users, version)?;
    Ok(target)
}

/// Czy jest zainstalowany OpenFilePower (poprzednia wersja programu).
pub fn has_openfilepower() -> bool {
    ["HKCU", "HKLM"].iter().any(|root| {
        reg(&["query", &format!(r"{root}\Software\Microsoft\Windows\CurrentVersion\Uninstall"), "/s", "/f", "OpenFilePower", "/d"]).is_ok_and(|o| o.contains("OpenFilePower"))
    })
}

/// Odinstalowuje OpenFilePower (poprzednia wersja programu), jeśli jest: jego deinstalator po cichu.
/// Zwraca true, gdy coś zostało odinstalowane.
pub fn remove_openfilepower() -> bool {
    let mut done = false;
    for root in ["HKCU", "HKLM"] {
        let base = format!(r"{root}\Software\Microsoft\Windows\CurrentVersion\Uninstall");
        let Ok(out) = reg(&["query", &base, "/s", "/f", "OpenFilePower", "/d"]) else { continue };
        let keys: Vec<String> = out.lines().filter(|l| l.starts_with("HKEY_")).map(|l| l.trim().to_string()).collect();
        for k in keys {
            let get = |v: &str| {
                reg(&["query", &k, "/v", v]).ok().and_then(|o| o.lines().find(|l| l.trim_start().starts_with(v)).and_then(|l| l.split("REG_SZ").nth(1)).map(|s| s.trim().to_string()))
            };
            if !get("DisplayName").is_some_and(|n| n.starts_with("OpenFilePower")) {
                continue;
            }
            let cmd = get("QuietUninstallString").or_else(|| get("UninstallString").map(|u| format!("{u} /S")));
            if let Some(cmd) = cmd {
                let ok = std::process::Command::new("cmd").raw_arg(format!("/c \"{cmd}\"")).creation_flags(NO_WINDOW).status().is_ok_and(|s| s.success());
                done |= ok;
            }
        }
    }
    done
}

/// Usuwa skrót, wpis i (po zakończeniu tego procesu) folder programu. Ustawienia – na życzenie.
pub fn uninstall(remove_settings: bool) -> Result<(), String> {
    let found = installed();
    let all = found.as_ref().is_some_and(|(_, a)| *a);
    if all && !is_admin() {
        return Err("PowerDesk zainstalowano dla wszystkich użytkowników – deinstalacja wymaga uprawnień administratora".into());
    }
    let _ = std::fs::remove_file(shortcut(all));
    let _ = reg(&["delete", &format!(r"{}\{KEY}", hive(all)), "/f"]);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some((exe, _)) = &found {
        if let Some(d) = exe.parent() {
            dirs.push(d.to_path_buf());
        }
    }
    if remove_settings {
        for var in ["APPDATA", "LOCALAPPDATA"] {
            if let Ok(v) = std::env::var(var) {
                dirs.push(PathBuf::from(v).join("PowerDesk"));
            }
        }
    }
    // Folder z działającym plikiem usuwa cmd po chwili, gdy ten proces już się zakończy.
    let pid = std::process::id();
    let mut script = format!("/c (for /l %i in (1,1,30) do (tasklist /fi \"PID eq {pid}\" | find \"{pid}\" >nul && timeout /t 1 /nobreak >nul))");
    for d in &dirs {
        script.push_str(&format!(" & rmdir /s /q \"{}\"", d.display()));
    }
    std::process::Command::new("cmd").raw_arg(script).creation_flags(NO_WINDOW).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Okno komunikatu instalatora.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ask {
    Info,
    Warning,
    YesNo,
    YesNoCancel,
}

/// Odpowiedź z okna komunikatu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answer {
    Yes,
    No,
    /// Anuluj albo OK w zwykłym komunikacie.
    Other,
}

pub fn message(text: &str, kind: Ask) -> Answer {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDNO, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO, MB_YESNOCANCEL};
    let style = match kind {
        Ask::Info => MB_OK | MB_ICONINFORMATION,
        Ask::Warning => MB_OK | MB_ICONWARNING,
        Ask::YesNo => MB_YESNO | MB_ICONQUESTION,
        Ask::YesNoCancel => MB_YESNOCANCEL | MB_ICONQUESTION,
    };
    let r = unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("PowerDesk"), style) };
    if r == IDYES {
        Answer::Yes
    } else if r == IDNO {
        Answer::No
    } else {
        Answer::Other
    }
}
