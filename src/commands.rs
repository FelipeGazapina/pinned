use pinned_core::connectors::{Connector, Connectors, Settings, Suggestions};
use pinned_core::machine::Machine;
use pinned_core::wire::Call;
use pinned_core::{github, linear, process, slack, vercel};
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub fn get_settings(connectors: State<Connectors>) -> Settings {
    connectors.get_settings()
}

#[tauri::command]
pub fn open_linear_keys() -> Result<(), String> {
    open(linear::KEY_PAGE)
}

#[tauri::command]
pub async fn connect_linear(connectors: State<'_, Connectors>, key: String) -> Result<linear::Account, String> {
    connectors.connect_linear(&key).await
}

#[tauri::command]
pub fn disconnect_linear(connectors: State<Connectors>) -> Result<(), String> {
    connectors.disconnect_linear()
}

#[tauri::command]
pub async fn linear_suggestions(connectors: State<'_, Connectors>) -> Result<Suggestions, String> {
    connectors.linear_suggestions().await
}

#[tauri::command]
pub async fn connect_slack(connectors: State<'_, Connectors>, token: String) -> Result<slack::Account, String> {
    connectors.connect_slack(&token).await
}

#[tauri::command]
pub fn disconnect_slack(connectors: State<Connectors>) -> Result<(), String> {
    connectors.disconnect_slack()
}

#[tauri::command]
pub async fn slack_suggestions(connectors: State<'_, Connectors>) -> Result<Vec<String>, String> {
    connectors.slack_suggestions().await
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
pub async fn connect_github(connectors: State<'_, Connectors>, token: String) -> Result<github::Account, String> {
    connectors.connect_github(&token).await
}

#[tauri::command]
pub fn disconnect_github(connectors: State<Connectors>) -> Result<(), String> {
    connectors.disconnect_github()
}

#[tauri::command]
pub async fn github_repos(connectors: State<'_, Connectors>) -> Result<Vec<github::Repo>, String> {
    connectors.github_repos().await
}

#[tauri::command]
pub async fn connect_vercel(connectors: State<'_, Connectors>, key: String) -> Result<vercel::Account, String> {
    connectors.connect_vercel(&key).await
}

#[tauri::command]
pub fn disconnect_vercel(connectors: State<Connectors>) -> Result<(), String> {
    connectors.disconnect_vercel()
}

#[tauri::command]
pub async fn vercel_models(connectors: State<'_, Connectors>) -> Result<Vec<vercel::Model>, String> {
    connectors.vercel_models().await
}

/// Clones on this Mac while it's signed out, with this Mac's own GitHub token.
#[tauri::command]
pub async fn clone_repo(connectors: State<'_, Connectors>, machine: State<'_, Arc<Machine>>, repo: String, dest: Option<String>) -> Result<Value, String> {
    let repo = github::clone_url(&repo);
    let token = connectors.github_token().filter(|_| github::hosts(&repo));
    let machine = machine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || machine.call(Call::Clone { repo, token, dest })).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn save_connector(connectors: State<Connectors>, connector: Connector) -> Result<Connector, String> {
    connectors.save_connector(connector)
}

#[tauri::command]
pub fn delete_connector(connectors: State<Connectors>, id: String) -> Result<(), String> {
    connectors.delete_connector(&id)
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
fn open(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let opener = "explorer.exe";
    #[cfg(not(windows))]
    let opener = "open";
    process::command(opener).arg(url).spawn().map(drop).map_err(|e| e.to_string())
}
