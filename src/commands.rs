//! The settings page's Tauri commands, answered by the app's own connectors.

use pinned_core::connectors::{Connector, Connectors, Settings, Suggestions};
use pinned_core::{linear, slack};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub fn get_settings(connectors: State<Connectors>) -> Settings {
    connectors.get_settings()
}

#[tauri::command]
pub fn open_linear_keys() -> Result<(), String> {
    std::process::Command::new("open").arg(linear::KEY_PAGE).spawn().map(drop).map_err(|e| e.to_string())
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
