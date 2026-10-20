mod agent;
mod connectors;
mod hooks;
mod linear;
mod pty;

use portable_pty::CommandBuilder;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const OPACITIES: [u8; 6] = [100, 90, 80, 60, 40, 20];
const DEFAULT_OPACITY: u8 = 94;

const PILL: LogicalSize<f64> = LogicalSize { width: 210.0, height: 40.0 };

struct Launch {
    claude: PathBuf,
    path_env: Option<String>,
    settings: PathBuf,
    sock: PathBuf,
}

/// One tab in the panel: its own folder and claude session.
struct Tab {
    cwd: PathBuf,
    session: Option<pty::Session>,
    /// Extra `claude` arguments for the first start only, so a restart doesn't redo the task.
    launch: Vec<String>,
    /// Environment for every start, such as the issue a connector's session reports on.
    env: Vec<(&'static str, String)>,
}

struct Harness {
    launch: Launch,
    default_cwd: PathBuf,
    tabs: Mutex<HashMap<String, Tab>>,
    next_tab: AtomicU32,
    expanded: Mutex<LogicalSize<f64>>,
    update: Mutex<Option<Update>>,
    opacity: AtomicU8,
}

impl Harness {
    /// Runs `f` on the tab `id`, failing once the tab is closed.
    fn tab<R>(&self, id: &str, f: impl FnOnce(&mut Tab) -> R) -> Result<R, String> {
        self.tabs.lock().unwrap().get_mut(id).map(f).ok_or_else(|| format!("no tab {id}"))
    }

    fn add_tab(&self, cwd: PathBuf, launch: Vec<String>, env: Vec<(&'static str, String)>) -> String {
        let id = format!("tab-{}", self.next_tab.fetch_add(1, Ordering::Relaxed));
        self.tabs.lock().unwrap().insert(id.clone(), Tab { cwd, session: None, launch, env });
        id
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum View {
    Expanded,
    Pill,
}

/// Adds a tab on `from`'s folder, or the launch folder, and returns its id.
#[tauri::command]
fn open_tab(harness: State<Harness>, from: Option<String>) -> String {
    let cwd = from
        .and_then(|from| harness.tab(&from, |t| t.cwd.clone()).ok())
        .unwrap_or_else(|| harness.default_cwd.clone());
    harness.add_tab(cwd, vec![], vec![])
}

#[tauri::command]
fn start_claude(
    app: AppHandle,
    harness: State<Harness>,
    tab: String,
    cols: u16,
    rows: u16,
    output: Channel<InvokeResponseBody>,
) -> Result<String, String> {
    let l = &harness.launch;
    let (cwd, launch, env) = harness.tab(&tab, |t| (t.cwd.clone(), std::mem::take(&mut t.launch), t.env.clone()))?;
    let mut cmd = CommandBuilder::new(&l.claude);
    cmd.arg("--settings");
    cmd.arg(&l.settings);
    cmd.args(launch);
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env(agent::SOCK_ENV, &l.sock);
    cmd.env(agent::TAB_ENV, &tab);
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(path) = &l.path_env {
        cmd.env("PATH", path);
    }

    // Dropping the old session kills it silently, so its exit can't be mistaken for the new one's.
    harness.tab(&tab, |t| t.session.take())?;
    let exited = tab.clone();
    let session = pty::Session::spawn(
        cmd,
        pty::size(cols, rows),
        move |bytes| {
            let _ = output.send(InvokeResponseBody::Raw(bytes));
        },
        move || {
            let _ = app.emit("claude-exit", exited);
        },
    )?;
    harness.tab(&tab, |t| t.session = Some(session))?;
    Ok(cwd.display().to_string())
}

#[tauri::command]
fn write_input(harness: State<Harness>, tab: String, data: String) -> Result<(), String> {
    harness.tab(&tab, |t| match t.session.as_mut() {
        Some(s) => s.write(data.as_bytes()).map_err(|e| e.to_string()),
        None => Ok(()),
    })?
}

#[tauri::command]
fn resize_pty(harness: State<Harness>, tab: String, cols: u16, rows: u16) -> Result<(), String> {
    harness.tab(&tab, |t| match t.session.as_ref() {
        Some(s) => s.resize(pty::size(cols, rows)),
        None => Ok(()),
    })?
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

/// Ends the tab's session; the app exits with the last tab.
#[tauri::command]
fn close_tab(app: AppHandle, harness: State<Harness>, tab: String) {
    let last = {
        let mut tabs = harness.tabs.lock().unwrap();
        tabs.remove(&tab);
        tabs.is_empty()
    };
    if last {
        app.exit(0);
    }
}

/// Returns true when the user picked a folder; the caller then restarts claude there.
#[tauri::command]
async fn choose_folder(app: AppHandle, harness: State<'_, Harness>, tab: String) -> Result<bool, String> {
    let current = harness.tab(&tab, |t| t.cwd.clone())?;
    let Some(picked) = app.dialog().file().set_directory(current).blocking_pick_folder() else {
        return Ok(false);
    };
    let dir = picked.into_path().map_err(|e| e.to_string())?;
    if let Ok(config) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&config);
        let _ = std::fs::write(config.join("last-folder"), dir.as_os_str().as_encoded_bytes());
    }
    harness.tab(&tab, |t| t.cwd = dir)?;
    Ok(true)
}

#[tauri::command]
fn get_opacity(harness: State<Harness>) -> u8 {
    harness.opacity.load(Ordering::Relaxed)
}

fn opacity_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|c| c.join("opacity"))
}

fn saved_opacity(config_dir: Option<PathBuf>) -> u8 {
    config_dir
        .and_then(|c| std::fs::read_to_string(c.join("opacity")).ok())
        .and_then(|t| t.trim().parse::<u8>().ok())
        .filter(|p| (1..=100).contains(p))
        .unwrap_or(DEFAULT_OPACITY)
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
    let current = app.state::<Harness>().opacity.load(Ordering::Relaxed);
    let levels = OPACITIES
        .iter()
        .map(|&p| CheckMenuItem::with_id(app, format!("opacity-{p}"), format!("{p}%"), true, p == current, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let level_refs: Vec<&dyn tauri::menu::IsMenuItem<_>> = levels.iter().map(|i| i as _).collect();
    let opacity = Submenu::with_items(app, "Opacity", true, &level_refs)?;
    let quit = MenuItem::with_id(app, "quit", "Quit pinned", true, None::<&str>)?;
    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&Menu::with_items(app, &[&opacity, &quit])?)
        .on_menu_event(move |app, e| {
            if e.id() == "quit" {
                app.exit(0);
                return;
            }
            let Some(percent) = e.id().as_ref().strip_prefix("opacity-").and_then(|p| p.parse::<u8>().ok()) else {
                return;
            };
            for item in &levels {
                let _ = item.set_checked(item.id() == e.id());
            }
            app.state::<Harness>().opacity.store(percent, Ordering::Relaxed);
            if let Some(file) = opacity_file(app) {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(file, percent.to_string());
            }
            let _ = app.emit("opacity", percent);
        })
        .build(app)?;
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some(hooks::SUBCOMMAND) => return hooks::forward(args.next().as_deref()),
        Some(hooks::STAGE_SUBCOMMAND) => match hooks::stage(args.next().as_deref()) {
            Ok(done) => return println!("{done}"),
            Err(e) => {
                eprintln!("pinned: {e}");
                std::process::exit(1);
            }
        },
        _ => {}
    }

    // The updater and the Linear client share reqwest's rustls, which ships without a crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let launch = launch().expect("could not prepare claude launch");
    let sock = launch.sock.clone();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    builder
        .invoke_handler(tauri::generate_handler![
            open_tab,
            start_claude,
            write_input,
            resize_pty,
            set_view,
            focus_panel,
            close_tab,
            choose_folder,
            check_update,
            install_update,
            get_opacity,
            connectors::get_settings,
            connectors::open_linear_keys,
            connectors::connect_linear,
            connectors::disconnect_linear,
            connectors::linear_suggestions,
            connectors::save_connector,
            connectors::delete_connector,
            connectors::pick_folder
        ])
        .setup(move |app| {
            app.manage(Harness {
                launch,
                default_cwd: initial_cwd(app.path().app_config_dir().ok()),
                tabs: Mutex::new(HashMap::new()),
                next_tab: AtomicU32::new(1),
                expanded: Mutex::new(LogicalSize::new(760.0, 460.0)),
                update: Mutex::new(None),
                opacity: AtomicU8::new(saved_opacity(app.path().app_config_dir().ok())),
            });
            app.manage(connectors::Connectors::load(app.path().app_config_dir()?, std::env::current_exe()?));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(connectors::watch(handle.clone(), move |run| {
                let env = vec![(agent::CONNECTOR_ENV, run.connector), (agent::ISSUE_ENV, run.issue)];
                let id = handle.state::<Harness>().add_tab(run.cwd, run.args, env);
                let _ = handle.emit("tab-opened", id);
            }));
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            add_tray(app)?;
            let handle = app.handle().clone();
            hooks::listen(sock, move |message| match message {
                agent::Message::Agent(event) => {
                    let _ = handle.emit("agent-event", event);
                    Ok(String::new())
                }
                agent::Message::Stage { connector, issue, stage } => tauri::async_runtime::block_on(
                    handle.state::<connectors::Connectors>().advance(&connector, &issue, stage),
                ),
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
