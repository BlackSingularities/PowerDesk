//! Menu Start PowerDesk: aplikacje z wirtualnego folderu „Aplikacje” Windows (programy
//! i aplikacje ze Sklepu), uruchamianie, zasilanie i klawisz Win otwierający nasze menu.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_LWIN, VK_RWIN};
use windows::Win32::UI::Shell::{IEnumShellItems, IShellItem, SHGetKnownFolderItem, BHID_EnumItems, FOLDERID_AppsFolder, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING};
use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP};

/// Aplikacja z menu Start.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    /// Nazwa w folderze „Aplikacje” (AUMID albo ścieżka) – do uruchamiania i ikony.
    pub id: String,
}

impl App {
    /// Ścieżka powłoki – przyjmują ją i uruchamianie, i pobieranie ikony.
    pub fn shell_path(&self) -> String {
        format!("shell:AppsFolder\\{}", self.id)
    }
}

unsafe fn name_of(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    let p = item.GetDisplayName(kind).ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

/// Wszystkie aplikacje, alfabetycznie (bez deinstalatorów – w Windows 10 są, ale tylko przeszkadzają).
pub fn apps() -> Vec<App> {
    let mut out = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return out };
        let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut items = [None];
            let mut n = 0u32;
            if en.Next(&mut items, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let Some(item) = items[0].take() else { break };
            let (Some(name), Some(id)) = (name_of(&item, SIGDN_NORMALDISPLAY), name_of(&item, SIGDN_PARENTRELATIVEPARSING)) else { continue };
            let low = name.to_lowercase();
            if low.starts_with("uninstall") || low.starts_with("odinstaluj") || low.contains("deinstal") {
                continue;
            }
            out.push(App { name, id });
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.name == b.name && a.id == b.id);
    out
}

pub fn launch(app_path: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(app_path).spawn();
}

/// Jako administrator – ShellExecute z „runas” (Windows pyta przez UAC).
pub fn launch_admin(app_path: &str) {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        ShellExecuteW(None, w!("runas"), &HSTRING::from(app_path), None, None, SW_SHOWNORMAL);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Power {
    Lock,
    SignOut,
    /// Przełącz użytkownika – ekran logowania, sesja zostaje.
    SwitchUser,
    Sleep,
    Hibernate,
    Restart,
    Shutdown,
}

/// Czy hibernacja jest włączona (inaczej Windows nie pokazuje jej w menu).
pub fn can_hibernate() -> bool {
    unsafe { windows::Win32::System::Power::IsPwrHibernateAllowed() }
}

pub fn power(p: Power) {
    use windows::Win32::System::Power::SetSuspendState;
    let (cmd, args): (&str, &[&str]) = match p {
        Power::Lock => ("rundll32.exe", &["user32.dll,LockWorkStation"]),
        Power::SignOut => ("shutdown.exe", &["/l"]),
        Power::SwitchUser => ("tsdiscon.exe", &[]),
        // Uśpienie i hibernacja wprost – rundll32 z SetSuspendState hibernuje, gdy hibernacja jest włączona.
        Power::Sleep | Power::Hibernate => {
            let hibernate = matches!(p, Power::Hibernate);
            std::thread::spawn(move || unsafe {
                SetSuspendState(hibernate, false, false);
            });
            return;
        }
        // Jak w menu Start Windows: aktualizacje oczekujące na instalację instalują się (/hybrid tylko przy zamykaniu).
        Power::Restart => ("shutdown.exe", &["/r", "/t", "0"]),
        Power::Shutdown => ("shutdown.exe", &["/s", "/hybrid", "/t", "0"]),
    };
    let _ = std::process::Command::new(cmd).args(args).spawn();
}

// ---------- Klawisz Win ----------

static ON_START: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// Win+V – historia schowka PowerDesk zamiast systemowej.
static ON_CLIPBOARD: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static WIN_DOWN: AtomicBool = AtomicBool::new(false);
static COMBO: AtomicBool = AtomicBool::new(false);
/// Tryb powłoki: Win+E, Win+R, Win+D obsługuje PowerDesk (bez Eksploratora nie zrobi tego nikt).
static ON_SHELL_KEY: OnceLock<Box<dyn Fn(u8) + Send + Sync>> = OnceLock::new();
const SHELL_KEYS: [u8; 3] = [b'E', b'R', b'D'];
/// Klawisz z Win połknięty przez nas (Win+V) – system widział samo Win, trzeba je zamaskować.
static SWALLOWED: AtomicBool = AtomicBool::new(false);
/// Nieprzypisany kod klawisza – „maskuje” samotne Win, żeby Windows nie otwierał swojego Start.
const MASK: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

fn send_mask() {
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: MASK, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } },
    };
    unsafe {
        SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32);
    }
}

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let injected = k.flags.0 & LLKHF_INJECTED.0 != 0;
        let win = k.vkCode == VK_LWIN.0 as u32 || k.vkCode == VK_RWIN.0 as u32;
        let msg = wp.0 as u32;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        if !injected {
            if win && down {
                if !WIN_DOWN.swap(true, Ordering::SeqCst) {
                    COMBO.store(false, Ordering::SeqCst);
                }
            } else if WIN_DOWN.load(Ordering::SeqCst) && k.vkCode == b'V' as u32 && ON_CLIPBOARD.get().is_some() {
                // Win+V: nasza historia; V nie trafia do systemu (inaczej otworzyłby swoją).
                COMBO.store(true, Ordering::SeqCst);
                SWALLOWED.store(true, Ordering::SeqCst);
                if down {
                    if let Some(f) = ON_CLIPBOARD.get() {
                        f();
                    }
                }
                return LRESULT(1);
            } else if WIN_DOWN.load(Ordering::SeqCst) && SHELL_KEYS.iter().any(|&c| k.vkCode == c as u32) && ON_SHELL_KEY.get().is_some() && crate::shellmode::active() {
                COMBO.store(true, Ordering::SeqCst);
                SWALLOWED.store(true, Ordering::SeqCst);
                if down {
                    if let Some(f) = ON_SHELL_KEY.get() {
                        f(k.vkCode as u8);
                    }
                }
                return LRESULT(1);
            } else if down && WIN_DOWN.load(Ordering::SeqCst) {
                // Win+E, Win+D, Win+Tab… – skrót systemowy, nie Start.
                COMBO.store(true, Ordering::SeqCst);
            } else if win && up {
                let alone = WIN_DOWN.swap(false, Ordering::SeqCst) && !COMBO.load(Ordering::SeqCst);
                if SWALLOWED.swap(false, Ordering::SeqCst) {
                    send_mask();
                }
                if alone {
                    send_mask();
                    if let Some(f) = ON_START.get() {
                        f();
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// Win+V otwiera historię schowka PowerDesk (wywołać przed `hook_win_key`).
pub fn hook_win_v(on_clipboard: impl Fn() + Send + Sync + 'static) {
    let _ = ON_CLIPBOARD.set(Box::new(on_clipboard));
}

/// Skróty z Win w trybie powłoki (kod klawisza: E, R, D); wywołać przed `hook_win_key`.
pub fn hook_shell_keys(on_key: impl Fn(u8) + Send + Sync + 'static) {
    let _ = ON_SHELL_KEY.set(Box::new(on_key));
}

/// Samo Win (bez innego klawisza) otwiera menu Start PowerDesk. Hak żyje na własnym wątku
/// z pętlą komunikatów do końca programu; skróty z Win zostają systemowe.
pub fn hook_win_key(on_start: impl Fn() + Send + Sync + 'static) {
    if ON_START.set(Box::new(on_start)).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe {
        if SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0).is_err() {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0) != BOOL(0) {}
    });
}
