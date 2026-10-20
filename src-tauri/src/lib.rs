mod drives;
mod fsx;
mod ops;
mod shell;
mod sync;
pub mod sync_core;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::http::{header, Response, StatusCode};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(true);
static START_HIDDEN: AtomicBool = AtomicBool::new(false);
const NO_WINDOW: u32 = 0x0800_0000;
const REPO: &str = "BlackSingularities/OpenFilePowerWin";

type R<T> = Result<T, String>;

fn hwnd(w: &WebviewWindow) -> isize {
    w.hwnd().map(|h| h.0 as isize).unwrap_or(0)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> R<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

// ---------- System plików ----------

#[tauri::command]
async fn list_dir(path: String) -> R<Vec<fsx::Entry>> {
    blocking(move || fsx::list_dir(&path)).await?
}

#[tauri::command]
async fn stat(path: String) -> R<fsx::Entry> {
    blocking(move || fsx::stat(&path)).await?
}

#[tauri::command]
async fn list_drives() -> R<Vec<drives::Drive>> {
    blocking(drives::list).await
}

#[derive(Serialize)]
struct Known {
    id: &'static str,
    name: &'static str,
    path: String,
}

#[tauri::command]
fn known_folders() -> Vec<Known> {
    let mut v = Vec::new();
    let mut add = |id, name, p: Option<PathBuf>| {
        if let Some(p) = p {
            if p.is_dir() {
                v.push(Known { id, name, path: p.to_string_lossy().to_string() });
            }
        }
    };
    add("home", "Folder domowy", dirs::home_dir());
    add("desktop", "Pulpit", dirs::desktop_dir());
    add("downloads", "Pobrane", dirs::download_dir());
    add("documents", "Dokumenty", dirs::document_dir());
    add("pictures", "Obrazy", dirs::picture_dir());
    add("videos", "Wideo", dirs::video_dir());
    add("music", "Muzyka", dirs::audio_dir());
    add("onedrive", "OneDrive", std::env::var("OneDrive").ok().map(PathBuf::from));
    v
}

#[tauri::command]
fn open_path(path: String) -> R<()> {
    open::that_detached(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_with(window: WebviewWindow, path: String) -> bool {
    shell::open_with(hwnd(&window), &path)
}

#[tauri::command]
fn show_properties(window: WebviewWindow, path: String) -> bool {
    shell::show_properties(hwnd(&window), &path)
}

#[tauri::command]
fn native_menu(window: WebviewWindow, paths: Vec<String>, x: f64, y: f64, extended: bool) -> R<bool> {
    let scale = window.scale_factor().unwrap_or(1.0);
    shell::native_menu(hwnd(&window), &paths, (x * scale) as i32, (y * scale) as i32, extended)
}

#[tauri::command]
fn reveal(path: String) {
    let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{path}\"")).spawn();
}

#[tauri::command]
fn open_terminal(path: String) {
    let wt = std::process::Command::new("wt").arg("-d").arg(&path).creation_flags(NO_WINDOW).spawn();
    if wt.is_err() {
        let _ = std::process::Command::new("powershell").arg("-NoExit").current_dir(&path).spawn();
    }
}

#[tauri::command]
fn create_folder(parent: String, name: String) -> R<String> {
    let p = ops::unique_path(&Path::new(&parent).join(name.trim()));
    std::fs::create_dir(&p).map_err(|e| fsx::io_err(&e))?;
    let s = p.to_string_lossy().to_string();
    ops::push_undo(ops::Undo::Created(vec![s.clone()]));
    Ok(s)
}

#[tauri::command]
fn create_file(parent: String, name: String) -> R<String> {
    let p = ops::unique_path(&Path::new(&parent).join(name.trim()));
    std::fs::File::create(&p).map_err(|e| fsx::io_err(&e))?;
    let s = p.to_string_lossy().to_string();
    ops::push_undo(ops::Undo::Created(vec![s.clone()]));
    Ok(s)
}

#[tauri::command]
fn rename(path: String, name: String) -> R<String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        return Err("Nazwa nie może zawierać znaków \\ / : * ? \" < > |".into());
    }
    let src = PathBuf::from(&path);
    let dst = src.with_file_name(name);
    if dst == src {
        return Ok(path);
    }
    let case_only = dst.to_string_lossy().to_lowercase() == src.to_string_lossy().to_lowercase();
    if dst.exists() && !case_only {
        return Err("Element o tej nazwie już istnieje".into());
    }
    std::fs::rename(&src, &dst).map_err(|e| fsx::io_err(&e))?;
    shell::invalidate(&path);
    let d = dst.to_string_lossy().to_string();
    ops::push_undo(ops::Undo::Renamed(d.clone(), path));
    Ok(d)
}

#[tauri::command]
fn start_op(app: AppHandle, kind: String, sources: Vec<String>, dest: String, conflict: ops::Conflict) -> u64 {
    ops::start(app, kind, sources, dest, conflict)
}

#[tauri::command]
fn cancel_op(id: u64) {
    ops::cancel(id)
}

#[tauri::command]
async fn check_conflicts(sources: Vec<String>, dest: String) -> R<Vec<ops::ConflictInfo>> {
    blocking(move || ops::check_conflicts(&sources, &dest)).await
}

#[tauri::command]
fn undo() -> R<String> {
    ops::undo_last()
}

#[tauri::command]
fn search(app: AppHandle, root: String, query: String, hidden: bool) -> u64 {
    fsx::search(app, root, query, hidden)
}

#[tauri::command]
fn cancel_search() {
    fsx::cancel_search()
}

#[tauri::command]
async fn dir_size(path: String) -> R<fsx::DirSize> {
    blocking(move || fsx::dir_size(&path)).await
}

#[tauri::command]
async fn file_hash(path: String) -> R<String> {
    blocking(move || fsx::sha256(&path)).await?
}

#[tauri::command]
async fn shell_props(path: String) -> R<Vec<shell::Prop>> {
    blocking(move || shell::properties(&path)).await
}

#[tauri::command]
async fn media_info(items: Vec<(String, i64)>) -> R<Vec<shell::MediaInfo>> {
    blocking(move || shell::media_info(items)).await
}

#[tauri::command]
async fn eject(window: WebviewWindow, path: String) -> R<()> {
    let h = hwnd(&window);
    blocking(move || shell::invoke_verb(h, &path, "eject")).await?
}

#[tauri::command]
fn disk_cleanup(letter: String) -> R<()> {
    let l: String = letter.chars().take(1).collect();
    std::process::Command::new("cleanmgr.exe").args(["/d", &l]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn read_text(path: String, limit: Option<usize>) -> R<fsx::TextPreview> {
    blocking(move || fsx::read_text(&path, limit.unwrap_or(512 * 1024))).await?
}

#[tauri::command]
async fn office_text(path: String) -> R<fsx::TextPreview> {
    blocking(move || fsx::office_text(&path, 400 * 1024)).await?
}

#[tauri::command]
async fn zip_list(path: String) -> R<Vec<fsx::ZipItem>> {
    blocking(move || fsx::zip_list(&path)).await?
}

#[tauri::command]
fn watch_dirs(paths: Vec<String>) {
    fsx::watch_dirs(paths)
}

// ---------- Schowek systemowy (zgodny z Eksploratorem) ----------

#[tauri::command]
fn clipboard_set_files(paths: Vec<String>, cut: bool) -> R<()> {
    use clipboard_win::{formats, raw, Clipboard, Setter};
    let _c = Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
    raw::empty().map_err(|e| e.to_string())?;
    formats::FileList.write_clipboard(&paths).map_err(|e| e.to_string())?;
    if let Some(f) = clipboard_win::register_format("Preferred DropEffect") {
        let effect: u32 = if cut { 2 } else { 5 };
        let _ = raw::set_without_clear(f.get(), &effect.to_le_bytes());
    }
    Ok(())
}

#[derive(Serialize)]
struct ClipFiles {
    paths: Vec<String>,
    cut: bool,
}

#[tauri::command]
fn clipboard_get_files() -> ClipFiles {
    use clipboard_win::{formats, raw, Clipboard, Getter};
    let mut paths: Vec<String> = Vec::new();
    let mut cut = false;
    if let Ok(_c) = Clipboard::new_attempts(10) {
        let _ = formats::FileList.read_clipboard(&mut paths);
        if let Some(f) = clipboard_win::register_format("Preferred DropEffect") {
            let mut buf = Vec::new();
            if raw::get_vec(f.get(), &mut buf).is_ok() && buf.len() >= 4 {
                cut = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) & 2 != 0;
            }
        }
    }
    ClipFiles { paths, cut }
}

// ---------- Stan aplikacji ----------

fn state_file() -> PathBuf {
    let d = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("OpenFilePowerWin");
    let _ = std::fs::create_dir_all(&d);
    d.join("state.json")
}

#[tauri::command]
fn load_state() -> String {
    std::fs::read_to_string(state_file()).unwrap_or_else(|_| "{}".into())
}

#[tauri::command]
fn save_state(json: String) {
    let p = state_file();
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

#[tauri::command]
fn set_close_to_tray(on: bool) {
    CLOSE_TO_TRAY.store(on, Ordering::SeqCst)
}

#[tauri::command]
fn start_hidden() -> bool {
    START_HIDDEN.load(Ordering::SeqCst)
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0)
}

#[tauri::command]
fn pick_folder(window: WebviewWindow, start: Option<String>) -> Option<String> {
    let mut d = rfd::FileDialog::new().set_title("Wybierz folder").set_parent(&window);
    if let Some(s) = start {
        d = d.set_directory(s);
    }
    d.pick_folder().map(|p| p.to_string_lossy().to_string())
}

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[tauri::command]
fn get_autostart() -> bool {
    std::process::Command::new("reg")
        .args(["query", RUN_KEY, "/v", "OpenFilePower"])
        .creation_flags(NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tauri::command]
fn set_autostart(on: bool) -> R<()> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let status = if on {
        std::process::Command::new("reg")
            .args(["add", RUN_KEY, "/v", "OpenFilePower", "/t", "REG_SZ", "/f", "/d"])
            .arg(format!("\"{}\" --tray", exe.display()))
            .creation_flags(NO_WINDOW)
            .status()
    } else {
        std::process::Command::new("reg").args(["delete", RUN_KEY, "/v", "OpenFilePower", "/f"]).creation_flags(NO_WINDOW).status()
    };
    status.map_err(|e| e.to_string()).map(|_| ())
}

#[derive(Serialize)]
struct UpdateInfo {
    current: String,
    latest: String,
    newer: bool,
    url: String,
    notes: String,
}

fn ver_tuple(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split(['.', '-']).filter_map(|p| p.parse().ok()).collect()
}

#[tauri::command]
async fn check_update(app: AppHandle) -> R<UpdateInfo> {
    let current = app.package_info().version.to_string();
    blocking(move || {
        let r: serde_json::Value = ureq::get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
            .set("User-Agent", "OpenFilePowerWin")
            .set("Accept", "application/vnd.github+json")
            .timeout(std::time::Duration::from_secs(10))
            .call()
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())?;
        let latest = r["tag_name"].as_str().unwrap_or("").to_string();
        Ok(UpdateInfo {
            newer: ver_tuple(&latest) > ver_tuple(&current),
            current,
            latest,
            url: r["html_url"].as_str().unwrap_or("").to_string(),
            notes: r["body"].as_str().unwrap_or("").chars().take(2000).collect(),
        })
    })
    .await?
}

// ---------- Dyski sieciowe ----------

#[tauri::command]
async fn map_drive(window: WebviewWindow, letter: Option<String>, remote: String, persistent: bool) -> R<String> {
    let h = hwnd(&window);
    blocking(move || drives::map(h, letter, &remote, persistent)).await?
}

#[tauri::command]
async fn unmap_drive(letter: String, forget: bool) -> R<()> {
    blocking(move || drives::unmap(&letter, forget)).await?
}

// ---------- Synchronizacja ----------

#[derive(Serialize)]
struct SyncList {
    profiles: Vec<sync::Profile>,
    status: Vec<sync::Status>,
}

#[tauri::command]
fn sync_list() -> SyncList {
    let (profiles, status) = sync::profiles();
    SyncList { profiles, status }
}

#[tauri::command]
async fn sync_save(profile: sync::Profile) -> R<sync::Profile> {
    blocking(move || sync::save_profile(profile)).await?
}

#[tauri::command]
fn sync_delete(id: String) {
    sync::delete_profile(&id)
}

#[tauri::command]
fn sync_now(id: Option<String>, confirm: Option<bool>) {
    sync::sync_now(id, confirm.unwrap_or(false))
}

#[tauri::command]
fn sync_enable(id: String, enabled: bool) {
    sync::set_enabled(&id, enabled)
}

#[tauri::command]
fn sync_cancel(id: String) {
    sync::cancel(&id)
}

#[tauri::command]
async fn sync_preview(id: String) -> R<sync::Preview> {
    blocking(move || sync::preview(&id)).await?
}

// ---------- Protokół miniatur ----------

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn run() {
    if std::env::args().any(|a| a == "--tray") {
        START_HIDDEN.store(true, Ordering::SeqCst);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            show_main(app);
            if let Some(p) = args.iter().skip(1).find(|a| !a.starts_with("--") && Path::new(a).exists()) {
                let _ = app.emit("open-path", p.clone());
            }
        }))
        .register_asynchronous_uri_scheme_protocol("thumb", |_ctx, request, responder| {
            let uri = request.uri();
            let mut parts = uri.path().trim_start_matches('/').split('/');
            let size: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(256).clamp(16, 2560);
            let icon_only = parts.next() == Some("i");
            let path = uri
                .query()
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("p=")))
                .map(pct_decode)
                .unwrap_or_default();
            shell::request(shell::ThumbReq {
                path,
                size,
                icon_only,
                reply: Box::new(move |res| {
                    let resp = match res {
                        Some((data, mime)) => Response::builder()
                            .status(StatusCode::OK)
                            .header(header::CONTENT_TYPE, mime)
                            .header(header::CACHE_CONTROL, "max-age=600")
                            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                            .body(data),
                        None => Response::builder().status(StatusCode::NOT_FOUND).header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*").body(Vec::new()),
                    };
                    responder.respond(resp.unwrap());
                }),
            });
        })
        .setup(|app| {
            let handle = app.handle().clone();
            fsx::init_watcher(handle.clone());
            drives::init_watch(handle.clone());
            sync::init(handle.clone());

            let show = MenuItem::with_id(app, "show", "Pokaż OpenFilePower", true, None::<&str>)?;
            let sync_all = MenuItem::with_id(app, "sync", "Synchronizuj teraz", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Zakończ", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &sync_all, &sep, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("ikona"))
                .tooltip("OpenFilePower")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id().as_ref() {
                    "show" => show_main(app),
                    "sync" => sync::sync_now(None, false),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            if let Some(p) = std::env::args().skip(1).find(|a| !a.starts_with("--") && Path::new(a).exists()) {
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    let _ = h.emit("open-path", p);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let has_sync = !sync::profiles().0.is_empty();
                if CLOSE_TO_TRAY.load(Ordering::SeqCst) && has_sync {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_dir,
            stat,
            list_drives,
            known_folders,
            open_path,
            open_with,
            show_properties,
            reveal,
            native_menu,
            open_terminal,
            create_folder,
            create_file,
            rename,
            start_op,
            cancel_op,
            check_conflicts,
            undo,
            search,
            cancel_search,
            dir_size,
            file_hash,
            shell_props,
            media_info,
            eject,
            disk_cleanup,
            read_text,
            office_text,
            zip_list,
            watch_dirs,
            clipboard_set_files,
            clipboard_get_files,
            load_state,
            save_state,
            set_close_to_tray,
            start_hidden,
            quit_app,
            pick_folder,
            get_autostart,
            set_autostart,
            check_update,
            map_drive,
            unmap_drive,
            sync_list,
            sync_save,
            sync_delete,
            sync_now,
            sync_enable,
            sync_cancel,
            sync_preview,
        ])
        .run(tauri::generate_context!())
        .expect("błąd uruchamiania aplikacji");
}
