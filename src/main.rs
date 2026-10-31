#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod awake;
mod browser;
mod commands;
mod fonts;
mod navigation;

use pinned_core::machine::{Launch, Machine, SelfUpdate};
use pinned_core::wire::{Call, MachineEvent, TabId, UpdateStatus};
use pinned_core::{agent, connectors, reminders, uploads};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, State, WebviewWindow, Window};
use tauri_plugin_updater::{Update, UpdaterExt};

const OPACITIES: [u8; 6] = [100, 90, 80, 60, 40, 20];
const DEFAULT_OPACITY: u8 = 94;
const LOCAL: &str = "local";

/// The pill's starting size, and the smallest it can be resized to.
const PILL: LogicalSize<f64> = LogicalSize { width: 210.0, height: 40.0 };
const EXPANDED_MIN: LogicalSize<f64> = LogicalSize { width: 150.0, height: 40.0 };

struct Panel {
    expanded: Mutex<LogicalSize<f64>>,
    pill: Mutex<LogicalSize<f64>>,
    /// Where the pill sat when it was last expanded, so shrinking again puts it back there.
    pill_pos: Mutex<Option<tauri::PhysicalPosition<i32>>>,
    update: Mutex<Option<Update>>,
    opacity: AtomicU8,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum View {
    Expanded,
    Pill,
}

/// Sync commands run one at a time on the main thread, which keeps keystrokes in order.
#[tauri::command]
fn machine_call(machine: State<Arc<Machine>>, call: Call) -> Result<serde_json::Value, String> {
    finish_local(&machine, call)
}

#[tauri::command]
async fn machine_call_slow(machine: State<'_, Arc<Machine>>, call: Call) -> Result<serde_json::Value, String> {
    let machine = machine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || finish_local(&machine, call)).await.map_err(|e| e.to_string())?
}

fn finish_local(machine: &Machine, call: Call) -> Result<serde_json::Value, String> {
    let (result, armed) = machine.finish_for_restart(call);
    if armed {
        machine.restart_now();
    }
    result
}

/// A dropped file's bytes, for a tab on another machine, which can't read this Mac's paths.
#[tauri::command]
async fn read_dropped(path: String) -> Result<tauri::ipc::Response, String> {
    let read = |e: std::io::Error| format!("could not read {path}: {e}");
    if std::fs::metadata(&path).map_err(read)?.len() > uploads::LIMIT as u64 {
        return Err(uploads::too_big(&path));
    }
    std::fs::read(&path).map(tauri::ipc::Response::new).map_err(read)
}

#[tauri::command]
fn attach_tab(machine: State<Arc<Machine>>, tab: String, output: Channel<InvokeResponseBody>) -> Result<(), String> {
    let sink = move |bytes: &[u8]| drop(output.send(InvokeResponseBody::Raw(bytes.to_vec())));
    machine.attach(&tab, LOCAL.into(), Box::new(sink))
}

#[tauri::command]
fn detach_tab(machine: State<Arc<Machine>>, tab: String) {
    machine.detach(&tab, LOCAL);
}

#[tauri::command]
fn restore_tabs(machine: State<Arc<Machine>>) -> Vec<TabId> {
    machine.restore_tabs()
}

#[tauri::command]
fn set_view(window: Window, panel: State<Panel>, view: View) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let current = window.inner_size().map_err(|e| e.to_string())?.to_logical(scale);
    let (leaving, entering, min) = match view {
        View::Pill => (&panel.expanded, &panel.pill, PILL),
        View::Expanded => (&panel.pill, &panel.expanded, EXPANDED_MIN),
    };
    *leaving.lock().unwrap() = current;
    if matches!(view, View::Expanded) {
        *panel.pill_pos.lock().unwrap() = window.outer_position().ok();
    }
    let size = *entering.lock().unwrap();
    window.set_min_size(Some(min)).map_err(|e| e.to_string())?;
    window.set_size(size).map_err(|e| e.to_string())?;
    match view {
        View::Expanded => {}
        View::Pill => {
            if let Some(pos) = *panel.pill_pos.lock().unwrap() {
                window.set_position(pos).map_err(|e| e.to_string())?;
            }
        }
    }
    keep_on_screen(&window, size, scale).map_err(|e| e.to_string())?;
    Ok(())
}

/// Moves the window so a `size`-sized frame at its position fits inside the monitor's work area.
fn keep_on_screen(window: &Window, size: LogicalSize<f64>, scale: f64) -> tauri::Result<()> {
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
fn focus_panel(app: AppHandle, window: Window) {
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

/// Ends the tab's session. Signed out, the app exits with the last tab; signed in, it stays up
/// so other devices can still open sessions on this Mac.
#[tauri::command]
fn close_tab(app: AppHandle, machine: State<Arc<Machine>>, account: State<account::Account>, tab: String) {
    if machine.close_tab(&tab) && !account.is_signed_in() {
        machine.quit();
        app.exit(0);
    }
}

#[tauri::command]
fn get_opacity(panel: State<Panel>) -> u8 {
    panel.opacity.load(Ordering::Relaxed)
}

#[tauri::command]
fn get_usage(machine: State<Arc<Machine>>) -> std::collections::BTreeMap<String, agent::Usage> {
    machine.usage()
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

async fn find_update(app: &AppHandle) -> Result<Option<Update>, String> {
    app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())
}

async fn download_pending(panel: &Panel) -> Result<(), String> {
    let Some(update) = panel.update.lock().unwrap().take() else {
        return Err("no update pending".into());
    };
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_update(app: AppHandle, panel: State<'_, Panel>) -> Result<Option<String>, String> {
    let update = find_update(&app).await?;
    let version = update.as_ref().map(|u| u.version.clone());
    *panel.update.lock().unwrap() = update;
    Ok(version)
}

#[tauri::command]
async fn install_update(machine: State<'_, Arc<Machine>>) -> Result<serde_json::Value, String> {
    let machine = machine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || finish_local(&machine, Call::UpdateSelf)).await.map_err(|e| e.to_string())?
}

struct DesktopUpdate {
    app: AppHandle,
}

impl SelfUpdate for DesktopUpdate {
    fn install(&self, machine: &Machine) -> UpdateStatus {
        let status = UpdateStatus::Updating { to: String::new() };
        machine.report(status.clone());
        machine.keep_updating();
        let app = self.app.clone();
        let machine = machine.arc();
        tauri::async_runtime::spawn(async move {
            let status = desktop_install(&app, &machine).await;
            if matches!(status, UpdateStatus::Restarting { .. }) {
                machine.restart_now();
            }
        });
        status
    }
}

async fn desktop_install(app: &AppHandle, machine: &Machine) -> UpdateStatus {
    let found = match find_update(app).await {
        Ok(update) => update,
        Err(reason) => return failed(machine, "", reason),
    };
    let to = found.as_ref().map(|update| update.version.clone());
    *app.state::<Panel>().update.lock().unwrap() = found;
    let Some(to) = to else {
        let status = UpdateStatus::UpToDate { version: env!("CARGO_PKG_VERSION").into() };
        machine.report(status.clone());
        return status;
    };
    machine.report(UpdateStatus::Updating { to: to.clone() });
    if let Err(reason) = download_pending(&app.state::<Panel>()).await {
        return failed(machine, &to, reason);
    }
    let (quitting, app) = (machine.arc(), app.clone());
    machine.schedule_restart(Box::new(move || {
        quitting.quit();
        app.restart();
    }));
    let status = UpdateStatus::Restarting { to };
    machine.report(status.clone());
    status
}

fn failed(machine: &Machine, to: &str, reason: String) -> UpdateStatus {
    let status = UpdateStatus::Failed { to: to.into(), reason };
    machine.report(status.clone());
    status
}

fn watch_reminders(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        for reminder in app.state::<reminders::Reminders>().take_due(now) {
            notify(&app, &reminder.message);
        }
    });
}

#[cfg(not(windows))]
fn notify(_app: &AppHandle, message: &str) {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{escaped}\" with title \"Reminder\" sound name \"Glass\"");
    let _ = pinned_core::process::command("osascript").args(["-e", &script]).status();
}

#[cfg(windows)]
fn notify(app: &AppHandle, message: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title("Reminder").body(message).show();
}

/// Debug builds run the script `PINNED_PROBE` names in the main window once it has loaded, so
/// scripts/native-probe can check what only the native window does.
#[cfg(debug_assertions)]
fn run_probe(app: &tauri::App) {
    let Some(script) = std::env::var_os("PINNED_PROBE").and_then(|path| std::fs::read_to_string(path).ok()) else { return };
    let Some(window) = app.get_webview_window("main") else { return };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(4));
        drop(window.eval(&script));
    });
}

fn add_tray(app: &tauri::App) -> tauri::Result<()> {
    let current = app.state::<Panel>().opacity.load(Ordering::Relaxed);
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
                app.state::<Arc<Machine>>().quit();
                app.exit(0);
                return;
            }
            let Some(percent) = e.id().as_ref().strip_prefix("opacity-").and_then(|p| p.parse::<u8>().ok()) else {
                return;
            };
            for item in &levels {
                let _ = item.set_checked(item.id() == e.id());
            }
            app.state::<Panel>().opacity.store(percent, Ordering::Relaxed);
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
    if let Some(code) = pinned_core::cli::dispatch(&std::env::args().skip(1).collect::<Vec<_>>()) {
        std::process::exit(code);
    }

    // The updater and the Linear client share reqwest's rustls, which ships without a crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let exe = std::env::current_exe().expect("could not find pinned's own binary");
    let launch = Launch::prepare(&exe).expect("could not prepare claude launch");
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_notification::init());
    builder
        .invoke_handler(tauri::generate_handler![
            machine_call,
            machine_call_slow,
            read_dropped,
            attach_tab,
            detach_tab,
            restore_tabs,
            set_view,
            focus_panel,
            close_tab,
            awake::get_keep_awake,
            awake::set_keep_awake,
            fonts::font_families,
            check_update,
            install_update,
            get_opacity,
            get_usage,
            commands::get_settings,
            commands::open_linear_keys,
            commands::connect_linear,
            commands::disconnect_linear,
            commands::linear_suggestions,
            commands::connect_slack,
            commands::disconnect_slack,
            commands::slack_suggestions,
            commands::open_github_tokens,
            commands::open_url,
            commands::connect_github,
            commands::disconnect_github,
            commands::github_repos,
            commands::connect_vercel,
            commands::disconnect_vercel,
            commands::vercel_models,
            commands::clone_repo,
            commands::save_connector,
            commands::delete_connector,
            commands::pick_folder,
            account::account,
            account::sign_in,
            account::change_password,
            account::sign_out,
            browser::browser_show,
            browser::browser_place,
            browser::browser_hide,
            browser::browser_close,
            browser::browser_state,
            browser::browser_do,
            browser::browser_eval,
            browser::browser_snapshot,
            browser::browser_window,
            browser::save_capture,
            browser::reveal_file,
            browser::browser_debug_key,
            navigation::permit_frame_navigation
        ])
        .setup(move |app| {
            let frames = Arc::new(Mutex::new(None));
            app.manage(navigation::FramePermit(frames.clone()));
            navigation::install_main_window(app, frames)?;
            let config = app.path().app_config_dir().ok();
            let from_arg = std::env::args().nth(1).and_then(|d| dunce::canonicalize(d).ok()).filter(|p| p.is_dir());
            let handle = app.handle().clone();
            let updater = handle.clone();
            let machine = Machine::new(launch, config.clone(), from_arg, move |event: MachineEvent| drop(handle.emit("machine", event)));
            machine.set_installer(Arc::new(DesktopUpdate { app: updater }));
            app.manage(machine.clone());
            app.manage(Panel {
                expanded: Mutex::new(LogicalSize::new(760.0, 460.0)),
                pill: Mutex::new(PILL),
                pill_pos: Mutex::new(None),
                update: Mutex::new(None),
                opacity: AtomicU8::new(saved_opacity(config.clone())),
            });
            app.manage(awake::Awake::load(config.clone()));
            app.manage(reminders::Reminders::load(config));
            watch_reminders(app.handle().clone());
            let storage = connectors::FileStorage::new(app.path().app_config_dir()?);
            app.manage(connectors::Connectors::new(storage, exe.display().to_string()));
            app.manage(account::Account::start(app.handle(), app.path().app_config_dir()?, machine.clone()));
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            add_tray(app)?;
            #[cfg(debug_assertions)]
            run_probe(app);
            let handle = app.handle().clone();
            machine.listen(move |message| match message {
                agent::Message::Remind { message, when } => handle.state::<reminders::Reminders>().add(message, &when),
                agent::Message::ListReminders => Ok(handle.state::<reminders::Reminders>().list()),
                agent::Message::CancelReminder { id } => handle.state::<reminders::Reminders>().cancel(id),
                agent::Message::Agent(_)
                | agent::Message::Usage { .. }
                | agent::Message::Delegate { .. }
                | agent::Message::TaskStatus { .. }
                | agent::Message::TaskCancel { .. }
                | agent::Message::Capabilities { .. }
                | agent::Message::Project { .. } => Err("the machine answers this itself".into()),
                callback => match handle.state::<account::Account>().forward(callback) {
                    Ok(answer) => answer,
                    Err(agent::Message::Stage { connector, subject, stage }) => tauri::async_runtime::block_on(
                        handle.state::<connectors::Connectors>().stage(&connector, &subject, stage),
                    ),
                    Err(agent::Message::SlackReply { connector, channel, thread, text }) => tauri::async_runtime::block_on(
                        handle.state::<connectors::Connectors>().slack_reply(&connector, &channel, &thread, &text),
                    ),
                    Err(agent::Message::LinearCreate { connector, channel, thread, title, description, assignee }) => tauri::async_runtime::block_on(
                        handle.state::<connectors::Connectors>().linear_create(&connector, &channel, &thread, &title, &description, &assignee),
                    ),
                    Err(agent::Message::LinearTake { connector, channel, thread, issue }) => tauri::async_runtime::block_on(
                        handle.state::<connectors::Connectors>().linear_take(&connector, &channel, &thread, &issue),
                    ),
                    Err(_) => unreachable!("forward hands back only the callback it was given"),
                },
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
                app.state::<Arc<Machine>>().quit();
            }
            if let tauri::RunEvent::Exit = event {
                let _ = std::fs::remove_file(&app.state::<Arc<Machine>>().launch().sock);
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

// An open panel, such as the one a file input shows, opens at the modal panel level, below the
// status level the panel floats at, so it would open under the panel and out of reach.
#[cfg(target_os = "macos")]
fn raise_open_panels() {
    use objc2_app_kit::{NSSavePanel, NSStatusWindowLevel, NSWindowDidBecomeKeyNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    let raise = block2::RcBlock::new(|note: std::ptr::NonNull<NSNotification>| {
        if let Some(panel) = unsafe { note.as_ref() }.object().and_then(|o| o.downcast::<NSSavePanel>().ok()) {
            panel.setLevel(NSStatusWindowLevel + 1);
        }
    });
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(Some(NSWindowDidBecomeKeyNotification), None, None, &raise)
    };
    std::mem::forget(observer);
}

// Only a nonactivating NSPanel may enter another app's fullscreen Space; a plain NSWindow
// stays on the regular desktop no matter its level or collection behavior.
#[cfg(target_os = "macos")]
fn pin_above_fullscreen(window: &WebviewWindow) -> tauri::Result<()> {
    use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};
    let panel = window.to_panel::<PinnedPanel>()?;
    panel.set_level(PanelLevel::Status.value());
    raise_open_panels();
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
