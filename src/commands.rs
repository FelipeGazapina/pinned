use pinned_core::connectors::Connectors;
use pinned_core::github;
use pinned_core::linear;
use pinned_core::link::Local;
use pinned_core::local_hub::{Answer, LocalHub, NEEDS_THE_HUB_OFF};
use pinned_core::machine::Machine;
use pinned_core::process;
use pinned_core::wire::HubCall;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

/// What this Mac answers from its copy while signed in, and from its own store while signed out.
#[tauri::command]
pub async fn local_call(app: AppHandle, call: HubCall) -> Result<Value, String> {
    if let Some(standby) = app.state::<crate::account::Account>().standby() {
        return tauri::async_runtime::spawn_blocking(move || match standby.answer(&call) {
            Some(result) => result,
            None => Err(NEEDS_THE_HUB_OFF.into()),
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let connectors = app.state::<Connectors>();
    let machine = app.state::<Arc<Machine>>();
    let hub = LocalHub { connectors: connectors.inner(), machine: machine.inner(), place_as: "local", taste_dir: None, account: None };
    match hub.answer(call).await {
        Answer::Done(result) => result,
        Answer::HubOnly => Err(NEEDS_THE_HUB_OFF.into()),
    }
}

#[tauri::command]
pub fn open_linear_keys() -> Result<(), String> {
    open(linear::KEY_PAGE)
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(format!("not a web address: {url}"));
    }
    open(&url)
}

#[tauri::command]
pub fn open_github_tokens() -> Result<(), String> {
    open(github::TOKEN_PAGE)
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle, start: Option<String>) -> Option<String> {
    let mut dialog = app.dialog().file();
    if let Some(start) = start.filter(|s| !s.is_empty()) {
        dialog = dialog.set_directory(start);
    }
    dialog.blocking_pick_folder()?.into_path().ok().map(|p| p.display().to_string())
}

/// Opens `url` in the default browser. On Windows through explorer.exe, since cmd's `start` would
/// cut the URL at its first `&`.
pub(crate) fn open(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let opener = "explorer.exe";
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(not(any(windows, target_os = "linux")))]
    let opener = "open";
    process::command(opener).arg(url).spawn().map(drop).map_err(|e| e.to_string())
}
