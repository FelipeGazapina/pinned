mod agent;
mod awake;
mod connectors;
mod history;
mod hooks;
mod linear;
mod projects;
mod pty;
mod sessions;

use portable_pty::CommandBuilder;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, State, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

const OPACITIES: [u8; 6] = [100, 90, 80, 60, 40, 20];
const DEFAULT_OPACITY: u8 = 94;
/// Slack after a usage limit's reset time before the session is told to continue.
const LIMIT_MARGIN_SECS: u64 = 5;

/// The pill's starting size, and the smallest it can be resized to.
const PILL: LogicalSize<f64> = LogicalSize { width: 210.0, height: 40.0 };
const EXPANDED_MIN: LogicalSize<f64> = LogicalSize { width: 150.0, height: 40.0 };

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
    env: Vec<(String, String)>,
    /// Claude Code's id for the conversation in progress, known once a hook reports it.
    claude_session: Option<String>,
    /// A turn is running, or died on an error: the session has work left if pinned quits now.
    interrupted: bool,
    /// Unix time a usage limit that stopped the turn lifts; the session then continues by itself.
    resume_at: Option<u64>,
    /// Working tree and prompt as the running turn began, to diff against when it ends.
    turn: Option<(String, Option<String>)>,
}

struct Harness {
    launch: Launch,
    default_cwd: PathBuf,
    tabs: Mutex<HashMap<String, Tab>>,
    next_tab: AtomicU32,
    expanded: Mutex<LogicalSize<f64>>,
    pill: Mutex<LogicalSize<f64>>,
    /// Where the pill sat when it was last expanded, so shrinking again puts it back there.
    pill_pos: Mutex<Option<tauri::PhysicalPosition<i32>>>,
    update: Mutex<Option<Update>>,
    opacity: AtomicU8,
    sessions_file: Option<PathBuf>,
    projects: Mutex<Vec<projects::Project>>,
    projects_file: Option<PathBuf>,
    /// One file of finished turns per project.
    history_dir: Option<PathBuf>,
    /// Set while the app shuts down, when sessions die without having finished.
    quitting: AtomicBool,
}

impl Harness {
    /// Runs `f` on the tab `id`, failing once the tab is closed.
    fn tab<R>(&self, id: &str, f: impl FnOnce(&mut Tab) -> R) -> Result<R, String> {
        self.tabs.lock().unwrap().get_mut(id).map(f).ok_or_else(|| format!("no tab {id}"))
    }

    fn add_tab(&self, cwd: PathBuf, launch: Vec<String>, env: Vec<(String, String)>, claude_session: Option<String>) -> String {
        let id = format!("tab-{}", self.next_tab.fetch_add(1, Ordering::Relaxed));
        self.tabs.lock().unwrap().insert(id.clone(), Tab { cwd, session: None, launch, env, claude_session, interrupted: false, resume_at: None, turn: None });
        id
    }

    /// Writes the tabs whose claude session is still going, in the order they were opened.
    fn save_sessions(&self) {
        let Some(file) = &self.sessions_file else { return };
        let saved: Vec<sessions::Saved> = {
            let tabs = self.tabs.lock().unwrap();
            let mut open: Vec<(u32, &Tab)> = tabs
                .iter()
                .filter(|(_, t)| t.claude_session.is_some())
                .map(|(id, t)| (id.trim_start_matches("tab-").parse().unwrap_or(0), t))
                .collect();
            open.sort_by_key(|(n, _)| *n);
            open.into_iter()
                .map(|(_, t)| sessions::Saved {
                    cwd: t.cwd.clone(),
                    session: t.claude_session.clone().unwrap(),
                    env: t.env.clone(),
                    interrupted: t.interrupted,
                    resume_at: t.resume_at,
                })
                .collect()
        };
        sessions::save(file, &saved);
    }

    /// Records `path` as a project opened now and returns its canonical form.
    fn open_project(&self, path: PathBuf) -> PathBuf {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let mut projects = self.projects.lock().unwrap();
        projects::upsert(&mut projects, path.clone(), unix_now());
        if let Some(file) = &self.projects_file {
            projects::save(file, &projects);
        }
        path
    }

    /// Hooks report the session on every event, since /clear or /resume inside claude swaps it.
    fn record_session(&self, tab: &str, session: String, reported_cwd: Option<&str>) {
        let changed = self.tab(tab, |t| {
            let mut changed = t.claude_session.as_ref() != Some(&session);
            if let Some(worktree) = reported_cwd.and_then(|c| sessions::worktree(&t.cwd, Path::new(c))) {
                t.cwd = worktree;
                changed = true;
            }
            t.claude_session = Some(session);
            changed
        });
        if changed == Ok(true) {
            self.save_sessions();
        }
    }

    /// Tracks whether the tab's turn has work left, from the status its hooks report.
    fn record_status(&self, tab: &str, event: &agent::AgentEvent) {
        let changed = self.tab(tab, |t| {
            let before = (t.interrupted, t.resume_at);
            match event.status {
                agent::AgentStatus::Working | agent::AgentStatus::Done => {
                    t.interrupted = event.status == agent::AgentStatus::Working;
                    t.resume_at = None;
                }
                agent::AgentStatus::Failed => {
                    t.interrupted = true;
                    t.resume_at = event.resume_at;
                }
                agent::AgentStatus::NeedsInput => {}
            }
            before != (t.interrupted, t.resume_at)
        });
        if changed == Ok(true) {
            self.save_sessions();
        }
    }

    /// Starts a turn on a prompt, or on its end saves what the session changed since.
    fn record_turn(&self, tab: &str, event: &agent::AgentEvent) {
        use agent::AgentStatus::*;
        let Some(tree) = event.snapshot.clone() else { return };
        match event.status {
            Working => {
                let _ = self.tab(tab, |t| t.turn = Some((tree, event.prompt.clone())));
            }
            Done | Failed => {
                let Ok(Some(((before, prompt), cwd, session))) =
                    self.tab(tab, |t| Some((t.turn.take()?, t.cwd.clone(), t.claude_session.clone()?)))
                else {
                    return;
                };
                let (Some(dir), Some((files, patch, truncated))) = (&self.history_dir, history::diff(&cwd, &before, &tree)) else { return };
                let turn = history::Turn { session, title: event.title.clone(), prompt, ended: unix_now(), files, patch, truncated };
                history::append(&history::file_for(dir, &history::project_of(&cwd)), &turn);
            }
            NeedsInput => {}
        }
    }

    /// Forgets the tab's session once it finished, unless it only died because the app is quitting.
    fn end_session(&self, tab: &str) {
        if self.quitting.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.tab(tab, |t| (t.interrupted, t.resume_at) = (false, None));
        if self.tab(tab, |t| t.claude_session.take()).ok().flatten().is_some() {
            self.save_sessions();
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum View {
    Expanded,
    Pill,
}

/// Adds a tab on the project at `path`, or the launch folder, and returns its id.
#[tauri::command]
fn open_tab(harness: State<Harness>, path: Option<String>) -> String {
    let cwd = match path {
        Some(path) => harness.open_project(path.into()),
        None => harness.default_cwd.clone(),
    };
    harness.add_tab(cwd, vec![], vec![], None)
}

/// The turns saved for the project at `path`, newest first.
#[tauri::command]
fn project_history(harness: State<Harness>, path: String) -> Vec<history::Turn> {
    harness.history_dir.as_ref().map(|dir| history::load(&history::file_for(dir, Path::new(&path)))).unwrap_or_default()
}

#[derive(serde::Serialize)]
struct ProjectView {
    path: String,
    name: String,
}

#[tauri::command]
fn list_projects(harness: State<Harness>) -> Vec<ProjectView> {
    let projects = harness.projects.lock().unwrap();
    projects::recent(&projects)
        .into_iter()
        .map(|p| ProjectView {
            path: p.path.display().to_string(),
            name: p.path.file_name().map_or_else(|| "/".into(), |n| n.to_string_lossy().into_owned()),
        })
        .collect()
}

/// Asks for a folder and records it as a project, returning its path.
#[tauri::command]
async fn pick_project(app: AppHandle, harness: State<'_, Harness>) -> Result<Option<String>, ()> {
    let picked = connectors::pick_folder(app, None).await;
    Ok(picked.map(|dir| harness.open_project(dir.into()).display().to_string()))
}

/// Types `CONTINUE_PROMPT` into the tab's session once its usage limit lifts. Safe to call again
/// for the same limit: whichever waiter fires first clears `resume_at`, so the rest stand down.
fn continue_when_limit_lifts(app: &AppHandle, tab: &str) {
    let harness = app.state::<Harness>();
    let Ok(Some(at)) = harness.tab(tab, |t| t.resume_at) else { return };
    let (app, tab) = (app.clone(), tab.to_owned());
    // A short sleep loop, since a laptop asleep past `at` must fire on wake.
    std::thread::spawn(move || {
        let harness = app.state::<Harness>();
        while unix_now() < at + LIMIT_MARGIN_SECS {
            std::thread::sleep(std::time::Duration::from_secs(15));
            if harness.tab(&tab, |t| t.resume_at) != Ok(Some(at)) {
                return;
            }
        }
        let fired = harness.tab(&tab, |t| t.resume_at.take() == Some(at) && t.session.is_some());
        if fired != Ok(true) {
            return;
        }
        let type_into = |data: &str| harness.tab(&tab, |t| t.session.as_mut().map(|s| s.write(data.as_bytes())));
        let _ = type_into(sessions::CONTINUE_PROMPT);
        // Enter arrives separately, or the TUI takes it as part of a paste.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = type_into("\r");
        harness.save_sessions();
    });
}

/// Reopens the sessions still going at the last quit, resumed, and returns their tab ids.
#[tauri::command]
fn restore_tabs(harness: State<Harness>) -> Vec<String> {
    // A reloaded webview must not duplicate the tabs that are already open.
    let Some(file) = harness.sessions_file.as_ref().filter(|_| harness.tabs.lock().unwrap().is_empty()) else {
        return vec![];
    };
    let now = unix_now();
    sessions::load(file)
        .into_iter()
        .filter(|s| s.cwd.is_dir())
        .map(|s| {
            let launch = s.launch_args(now);
            let waiting = s.interrupted && launch.len() == 2;
            let id = harness.add_tab(s.cwd, launch, s.env, Some(s.session));
            // A limit that already lifted continues through the launch prompt; one still in force waits.
            let _ = harness.tab(&id, |t| {
                t.interrupted = s.interrupted;
                t.resume_at = s.resume_at.filter(|_| waiting);
            });
            id
        })
        .collect()
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
    let (cwd, launch, env) = harness.tab(&tab, |t| {
        // Without launch arguments this is a fresh conversation, not the one recorded before.
        if t.launch.is_empty() {
            t.claude_session = None;
        }
        (t.cwd.clone(), std::mem::take(&mut t.launch), t.env.clone())
    })?;
    harness.save_sessions();
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
    let on_exit_app = app.clone();
    let session = pty::Session::spawn(
        cmd,
        pty::size(cols, rows),
        move |bytes| {
            let _ = output.send(InvokeResponseBody::Raw(bytes));
        },
        move || {
            on_exit_app.state::<Harness>().end_session(&exited);
            let _ = on_exit_app.emit("claude-exit", exited);
        },
    )?;
    harness.tab(&tab, |t| t.session = Some(session))?;
    continue_when_limit_lifts(&app, &tab);
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
    let current = window.inner_size().map_err(|e| e.to_string())?.to_logical(scale);
    let (leaving, entering, min) = match view {
        View::Pill => (&harness.expanded, &harness.pill, PILL),
        View::Expanded => (&harness.pill, &harness.expanded, EXPANDED_MIN),
    };
    *leaving.lock().unwrap() = current;
    if matches!(view, View::Expanded) {
        *harness.pill_pos.lock().unwrap() = window.outer_position().ok();
    }
    let size = *entering.lock().unwrap();
    window.set_min_size(Some(min)).map_err(|e| e.to_string())?;
    window.set_size(size).map_err(|e| e.to_string())?;
    match view {
        View::Expanded => {}
        View::Pill => {
            if let Some(pos) = *harness.pill_pos.lock().unwrap() {
                window.set_position(pos).map_err(|e| e.to_string())?;
            }
        }
    }
    keep_on_screen(&window, size, scale).map_err(|e| e.to_string())?;
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
    harness.save_sessions();
    if last {
        harness.quitting.store(true, Ordering::SeqCst);
        app.exit(0);
    }
}

/// Returns true when the user picked a folder; the caller then restarts claude there.
#[tauri::command]
async fn choose_folder(app: AppHandle, harness: State<'_, Harness>, tab: String) -> Result<bool, String> {
    let current = harness.tab(&tab, |t| t.cwd.display().to_string())?;
    let Some(picked) = connectors::pick_folder(app, Some(current)).await else {
        return Ok(false);
    };
    let dir = harness.open_project(picked.into());
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
    harness.quitting.store(true, Ordering::SeqCst);
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

fn initial_cwd(from_arg: Option<PathBuf>, projects: &[projects::Project]) -> PathBuf {
    from_arg
        .or_else(|| projects::recent(projects).first().map(|p| p.path.clone()))
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
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
                app.state::<Harness>().quitting.store(true, Ordering::SeqCst);
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
            list_projects,
            pick_project,
            restore_tabs,
            start_claude,
            write_input,
            resize_pty,
            set_view,
            focus_panel,
            close_tab,
            choose_folder,
            project_history,
            awake::get_keep_awake,
            awake::set_keep_awake,
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
            let config = app.path().app_config_dir().ok();
            let projects_file = config.as_ref().map(|c| c.join("projects.json"));
            let mut projects = config
                .as_ref()
                .zip(projects_file.as_ref())
                .map(|(c, file)| projects::load_or_migrate(file, &c.join("last-folder")))
                .unwrap_or_default();
            let from_arg = std::env::args().nth(1).and_then(|d| std::fs::canonicalize(d).ok()).filter(|p| p.is_dir());
            if let Some(dir) = &from_arg {
                projects::upsert(&mut projects, dir.clone(), unix_now());
                if let Some(file) = &projects_file {
                    projects::save(file, &projects);
                }
            }
            app.manage(Harness {
                launch,
                default_cwd: initial_cwd(from_arg, &projects),
                tabs: Mutex::new(HashMap::new()),
                next_tab: AtomicU32::new(1),
                expanded: Mutex::new(LogicalSize::new(760.0, 460.0)),
                pill: Mutex::new(PILL),
                pill_pos: Mutex::new(None),
                update: Mutex::new(None),
                opacity: AtomicU8::new(saved_opacity(app.path().app_config_dir().ok())),
                sessions_file: app.path().app_config_dir().ok().map(|c| c.join("sessions.json")),
                projects: Mutex::new(projects),
                projects_file,
                history_dir: app.path().app_config_dir().ok().map(|c| c.join("history")),
                quitting: AtomicBool::new(false),
            });
            app.manage(awake::Awake::load(app.path().app_config_dir().ok()));
            app.manage(connectors::Connectors::load(app.path().app_config_dir()?, std::env::current_exe()?));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(connectors::watch(handle.clone(), move |run| {
                let env = vec![(agent::CONNECTOR_ENV.to_owned(), run.connector), (agent::ISSUE_ENV.to_owned(), run.issue)];
                let id = handle.state::<Harness>().add_tab(run.cwd, run.args, env, None);
                let _ = handle.emit("tab-opened", id);
            }));
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            add_tray(app)?;
            let handle = app.handle().clone();
            hooks::listen(sock, move |message| match message {
                agent::Message::Agent(event) => {
                    if let Some(tab) = &event.tab {
                        let harness = handle.state::<Harness>();
                        if let Some(session) = &event.session {
                            harness.record_session(tab, session.clone(), event.cwd.as_deref());
                        }
                        harness.record_status(tab, &event);
                        harness.record_turn(tab, &event);
                        continue_when_limit_lifts(&handle, tab);
                    }
                    let _ = handle.emit("agent-event", event);
                    Ok(String::new())
                }
                agent::Message::Stage { connector, issue, stage } => tauri::async_runtime::block_on(
                    handle.state::<connectors::Connectors>().advance(&connector, &issue, stage),
                ),
            })?;
            let window = app.get_webview_window("main").unwrap();
            window.set_min_size(Some(EXPANDED_MIN))?;
            #[cfg(target_os = "macos")]
            pin_above_fullscreen(&window)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("tauri build failed")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                app.state::<Harness>().quitting.store(true, Ordering::SeqCst);
            }
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
