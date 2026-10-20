mod agent;
mod hooks;
mod pty;

use portable_pty::CommandBuilder;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const PILL: LogicalSize<f64> = LogicalSize { width: 210.0, height: 40.0 };

struct Launch {
    claude: PathBuf,
    path_env: Option<String>,
    settings: PathBuf,
    sock: PathBuf,
}

struct Harness {
    launch: Launch,
    cwd: Mutex<PathBuf>,
    session: Mutex<Option<pty::Session>>,
    expanded: Mutex<LogicalSize<f64>>,
    update: Mutex<Option<Update>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum View {
    Expanded,
    Pill,
}

#[tauri::command]
fn start_claude(
    app: AppHandle,
    harness: State<Harness>,
    cols: u16,
    rows: u16,
    output: Channel<InvokeResponseBody>,
) -> Result<String, String> {
    let l = &harness.launch;
    let cwd = harness.cwd.lock().unwrap().clone();
    let mut cmd = CommandBuilder::new(&l.claude);
    cmd.arg("--settings");
    cmd.arg(&l.settings);
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env(agent::SOCK_ENV, &l.sock);
    if let Some(path) = &l.path_env {
        cmd.env("PATH", path);
    }

    // Drop the old session first so its exit event fires before the new one starts.
    harness.session.lock().unwrap().take();
    let session = pty::Session::spawn(
        cmd,
        pty::size(cols, rows),
        move |bytes| {
            let _ = output.send(InvokeResponseBody::Raw(bytes));
        },
        move || {
            let _ = app.emit("claude-exit", ());
        },
    )?;
    *harness.session.lock().unwrap() = Some(session);
    Ok(cwd.display().to_string())
}

#[tauri::command]
fn write_input(harness: State<Harness>, data: String) -> Result<(), String> {
    match harness.session.lock().unwrap().as_mut() {
        Some(s) => s.write(data.as_bytes()).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
fn resize_pty(harness: State<Harness>, cols: u16, rows: u16) -> Result<(), String> {
    match harness.session.lock().unwrap().as_ref() {
        Some(s) => s.resize(pty::size(cols, rows)),
        None => Ok(()),
    }
}

#[tauri::command]
fn set_view(window: WebviewWindow, harness: State<Harness>, view: View) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = match view {
        View::Pill => {
            let current = window.inner_size().map_err(|e| e.to_string())?.to_logical(scale);
            *harness.expanded.lock().unwrap() = current;
            PILL
        }
        View::Expanded => *harness.expanded.lock().unwrap(),
    };
    window.set_size(size).map_err(|e| e.to_string())
}

/// A nonactivating panel never becomes key on its own, so keystrokes would go to the app
/// underneath. Called on every click and on start so the terminal receives typing.
#[tauri::command]
fn focus_panel(app: AppHandle) {
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        if let Ok(panel) = app.get_webview_panel("main") {
            panel.make_key_window();
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Returns true when the user picked a folder; the caller then restarts claude there.
#[tauri::command]
async fn choose_folder(app: AppHandle, harness: State<'_, Harness>) -> Result<bool, String> {
    let current = harness.cwd.lock().unwrap().clone();
    let Some(picked) = app.dialog().file().set_directory(current).blocking_pick_folder() else {
        return Ok(false);
    };
    let dir = picked.into_path().map_err(|e| e.to_string())?;
    if let Ok(config) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&config);
        let _ = std::fs::write(config.join("last-folder"), dir.as_os_str().as_encoded_bytes());
    }
    *harness.cwd.lock().unwrap() = dir;
    Ok(true)
}

/// Returns the newer version when one is published, keeping it ready for `install_update`.
#[tauri::command]
async fn check_update(app: AppHandle, harness: State<'_, Harness>) -> Result<Option<String>, String> {
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let version = update.as_ref().map(|u| u.version.clone());
    *harness.update.lock().unwrap() = update;
    Ok(version)
}

#[tauri::command]
async fn install_update(app: AppHandle, harness: State<'_, Harness>) -> Result<(), String> {
    let Some(update) = harness.update.lock().unwrap().take() else {
        return Err("no update pending".into());
    };
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    harness.session.lock().unwrap().take();
    app.restart();
}

fn find_claude(path_env: Option<&str>) -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    path_env
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("claude"))
        .find(|p| p.is_file())
        .unwrap_or_else(|| home.join(".local/bin/claude"))
}

/// Apps opened from Finder inherit launchd's bare PATH. Ask the user's login shell for the
/// PATH their terminal sees, so claude and the tools it runs (git, node, brew) resolve.
fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let out = std::process::Command::new(shell)
        .args(["-ilc", "printf '\\n__PINNED_PATH__%s__PINNED_PATH__' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split("__PINNED_PATH__").nth(1).map(str::to_owned)
}

fn initial_cwd(config_dir: Option<PathBuf>) -> PathBuf {
    let from_arg = std::env::args().nth(1).and_then(|d| std::fs::canonicalize(d).ok());
    let remembered = config_dir
        .and_then(|c| std::fs::read_to_string(c.join("last-folder")).ok())
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    from_arg
        .or(remembered)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
}

fn launch() -> std::io::Result<Launch> {
    let tmp = std::env::temp_dir();
    let settings = tmp.join("pinned-claude-settings.json");
    hooks::write_settings(&settings, &std::env::current_exe()?)?;
    let path_env = login_shell_path().or_else(|| std::env::var("PATH").ok());
    Ok(Launch {
        claude: find_claude(path_env.as_deref()),
        path_env,
        settings,
        sock: tmp.join(format!("pinned-{}.sock", std::process::id())),
    })
}

fn add_tray(app: &tauri::App) -> tauri::Result<()> {
    let quit = MenuItem::with_id(app, "quit", "Quit pinned", true, None::<&str>)?;
    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&Menu::with_items(app, &[&quit])?)
        .on_menu_event(|app, e| {
            if e.id() == "quit" {
                app.exit(0);
            }
        })
        .build(app)?;
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some(hooks::SUBCOMMAND) {
        return hooks::forward(args.next().as_deref());
    }

    let launch = launch().expect("could not prepare claude launch");
    let sock = launch.sock.clone();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    builder
        .invoke_handler(tauri::generate_handler![
            start_claude,
            write_input,
            resize_pty,
            set_view,
            focus_panel,
            quit_app,
            choose_folder,
            check_update,
            install_update
        ])
        .setup(move |app| {
            app.manage(Harness {
                launch,
                cwd: Mutex::new(initial_cwd(app.path().app_config_dir().ok())),
                session: Mutex::new(None),
                expanded: Mutex::new(LogicalSize::new(760.0, 460.0)),
                update: Mutex::new(None),
            });
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            add_tray(app)?;
            let handle = app.handle().clone();
            hooks::listen(sock, move |event| {
                let _ = handle.emit("agent-event", event);
            })?;
            #[cfg(target_os = "macos")]
            pin_above_fullscreen(&app.get_webview_window("main").unwrap())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("tauri build failed")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let _ = std::fs::remove_file(&app.state::<Harness>().launch.sock);
            }
        });
}

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(PinnedPanel {
        config: {
            can_become_key_window: true,
            is_floating_panel: true
        }
    })
}

// Only a nonactivating NSPanel may enter another app's fullscreen Space; a plain NSWindow
// stays on the regular desktop no matter its level or collection behavior.
#[cfg(target_os = "macos")]
fn pin_above_fullscreen(window: &WebviewWindow) -> tauri::Result<()> {
    use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};
    let panel = window.to_panel::<PinnedPanel>()?;
    panel.set_level(PanelLevel::Status.value());
    panel
        .add_style_mask(StyleMask::empty().nonactivating_panel().value())
        .expect("nonactivating style mask rejected");
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .stationary()
            .value(),
    );
    panel.set_hides_on_deactivate(false);
    panel.order_front_regardless();
    Ok(())
}
