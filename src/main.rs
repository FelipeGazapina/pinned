mod agent;
mod hooks;
mod pty;

use portable_pty::CommandBuilder;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, State, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const PILL: LogicalSize<f64> = LogicalSize { width: 210.0, height: 40.0 };

struct Launch {
    claude: PathBuf,
    path_env: Option<String>,
    settings: PathBuf,
    sock: PathBuf,
}

/// One pinned panel: its own folder, claude session and expanded size.
struct Tab {
    cwd: PathBuf,
    session: Option<pty::Session>,
    expanded: LogicalSize<f64>,
}

struct Harness {
    launch: Launch,
    default_cwd: PathBuf,
    tabs: Mutex<HashMap<String, Tab>>,
    next_tab: AtomicU32,
    update: Mutex<Option<Update>>,
}

impl Harness {
    /// Runs `f` on the tab owned by the window `label`, creating it on first use.
    fn tab<R>(&self, label: &str, f: impl FnOnce(&mut Tab) -> R) -> R {
        let mut tabs = self.tabs.lock().unwrap();
        let tab = tabs.entry(label.to_owned()).or_insert_with(|| Tab {
            cwd: self.default_cwd.clone(),
            session: None,
            expanded: LogicalSize::new(760.0, 460.0),
        });
        f(tab)
    }
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
    window: WebviewWindow,
    harness: State<Harness>,
    cols: u16,
    rows: u16,
    output: Channel<InvokeResponseBody>,
) -> Result<String, String> {
    let l = &harness.launch;
    let label = window.label().to_owned();
    let cwd = harness.tab(&label, |t| t.cwd.clone());
    let mut cmd = CommandBuilder::new(&l.claude);
    cmd.arg("--settings");
    cmd.arg(&l.settings);
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env(agent::SOCK_ENV, &l.sock);
    cmd.env(agent::WINDOW_ENV, &label);
    if let Some(path) = &l.path_env {
        cmd.env("PATH", path);
    }

    // Drop the old session first so its exit event fires before the new one starts.
    harness.tab(&label, |t| t.session.take());
    let session = pty::Session::spawn(
        cmd,
        pty::size(cols, rows),
        move |bytes| {
            let _ = output.send(InvokeResponseBody::Raw(bytes));
        },
        move || {
            let _ = app.emit_to(label.as_str(), "claude-exit", ());
        },
    )?;
    harness.tab(window.label(), |t| t.session = Some(session));
    Ok(cwd.display().to_string())
}

#[tauri::command]
fn write_input(window: WebviewWindow, harness: State<Harness>, data: String) -> Result<(), String> {
    harness.tab(window.label(), |t| match t.session.as_mut() {
        Some(s) => s.write(data.as_bytes()).map_err(|e| e.to_string()),
        None => Ok(()),
    })
}

#[tauri::command]
fn resize_pty(window: WebviewWindow, harness: State<Harness>, cols: u16, rows: u16) -> Result<(), String> {
    harness.tab(window.label(), |t| match t.session.as_ref() {
        Some(s) => s.resize(pty::size(cols, rows)),
        None => Ok(()),
    })
}

#[tauri::command]
fn set_view(window: WebviewWindow, harness: State<Harness>, view: View) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = match view {
        View::Pill => {
            let current = window.inner_size().map_err(|e| e.to_string())?.to_logical(scale);
            harness.tab(window.label(), |t| t.expanded = current);
            PILL
        }
        View::Expanded => harness.tab(window.label(), |t| t.expanded),
    };
    window.set_size(size).map_err(|e| e.to_string())?;
    if matches!(view, View::Expanded) {
        keep_on_screen(&window, size, scale).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Moves the window so a `size`-sized frame at its position fits inside the monitor's work area.
fn keep_on_screen(window: &WebviewWindow, size: LogicalSize<f64>, scale: f64) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else { return Ok(()) };
    let area = monitor.work_area();
    let size = size.to_physical::<i32>(scale);
    let pos = window.outer_position()?;
    let fit = |pos: i32, len: i32, start: i32, avail: u32| {
        pos.min(start + avail as i32 - len).max(start)
    };
    let x = fit(pos.x, size.width, area.position.x, area.size.width);
    let y = fit(pos.y, size.height, area.position.y, area.size.height);
    if (x, y) != (pos.x, pos.y) {
        window.set_position(tauri::PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

/// A nonactivating panel never becomes key on its own, so keystrokes would go to the app
/// underneath. Called on every click and on start so the terminal receives typing.
#[tauri::command]
fn focus_panel(app: AppHandle, window: WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        if let Ok(panel) = app.get_webview_panel(window.label()) {
            panel.make_key_window();
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, window);
}

/// Closes this panel and ends its session; the app exits with the last panel.
#[tauri::command]
fn close_tab(window: WebviewWindow, harness: State<Harness>) -> Result<(), String> {
    harness.tabs.lock().unwrap().remove(window.label());
    window.destroy().map_err(|e| e.to_string())
}

/// Opens another panel next to this one, on the same folder, with its own claude session.
#[tauri::command]
async fn new_tab(app: AppHandle, window: WebviewWindow, harness: State<'_, Harness>) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size: LogicalSize<f64> = window.inner_size().map_err(|e| e.to_string())?.to_logical(scale);
    let origin = window.outer_position().map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    let label = format!("pinned-{}", harness.next_tab.fetch_add(1, Ordering::Relaxed));
    let cwd = harness.tab(window.label(), |t| t.cwd.clone());
    harness.tabs.lock().unwrap().insert(
        label.clone(),
        Tab { cwd, session: None, expanded: size },
    );

    let mut config = app.config().app.windows[0].clone();
    config.label = label.clone();
    config.x = Some(origin.x + 32.0);
    config.y = Some(origin.y + 32.0);
    config.width = size.width;
    config.height = size.height;
    let built = WebviewWindowBuilder::from_config(&app, &config)
        .and_then(|b| b.build())
        .map_err(|e| e.to_string());
    let new_window = match built {
        Ok(w) => w,
        Err(e) => {
            harness.tabs.lock().unwrap().remove(&label);
            return Err(e);
        }
    };
    keep_on_screen(&new_window, size, scale).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    app.run_on_main_thread(move || {
        if let Err(e) = pin_above_fullscreen(&new_window) {
            eprintln!("pinned: could not pin {}: {e}", new_window.label());
        }
    })
    .map_err(|e| e.to_string())
}

/// Returns true when the user picked a folder; the caller then restarts claude there.
#[tauri::command]
async fn choose_folder(app: AppHandle, window: WebviewWindow, harness: State<'_, Harness>) -> Result<bool, String> {
    let current = harness.tab(window.label(), |t| t.cwd.clone());
    let Some(picked) = app.dialog().file().set_directory(current).blocking_pick_folder() else {
        return Ok(false);
    };
    let dir = picked.into_path().map_err(|e| e.to_string())?;
    if let Ok(config) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&config);
        let _ = std::fs::write(config.join("last-folder"), dir.as_os_str().as_encoded_bytes());
    }
    harness.tab(window.label(), |t| t.cwd = dir);
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
    harness.tabs.lock().unwrap().clear();
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
            close_tab,
            new_tab,
            choose_folder,
            check_update,
            install_update
        ])
        .setup(move |app| {
            app.manage(Harness {
                launch,
                default_cwd: initial_cwd(app.path().app_config_dir().ok()),
                tabs: Mutex::new(HashMap::new()),
                next_tab: AtomicU32::new(2),
                update: Mutex::new(None),
            });
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            add_tray(app)?;
            let handle = app.handle().clone();
            hooks::listen(sock, move |event| {
                let label = event.window.clone().unwrap_or_else(|| "main".into());
                let _ = handle.emit_to(label.as_str(), "agent-event", event);
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
