//! Connectors make the harness proactive: each one watches an integration and opens a tab
//! with a claude session already working on what it found.

use crate::agent::Stage;
use crate::linear;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::Notify;

const INTERVAL: Duration = Duration::from_secs(60);

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// Serialized names match `claude --permission-mode`.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Manual,
    AcceptEdits,
    Auto,
    DontAsk,
    BypassPermissions,
    Plan,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Trigger {
    /// A Linear issue created with this label.
    LinearLabel { label: String },
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub folder: PathBuf,
    /// Passed to `claude --model`; empty keeps claude's default.
    pub model: String,
    pub effort: Effort,
    pub permission_mode: PermissionMode,
    /// Runs each session in its own git worktree, so parallel issues don't share a checkout.
    pub worktree: bool,
    pub instructions: String,
    #[serde(default)]
    pub columns: Columns,
    /// Only issues created after this instant (RFC 3339) trigger, so saving a connector
    /// doesn't start sessions for the backlog.
    #[serde(default)]
    pub since: String,
    /// Issues that already started a session.
    #[serde(default)]
    pub handled: Vec<String>,
}

/// The Linear column each stage moves the issue to. Empty leaves the issue where it is.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Columns {
    pub picked: String,
    pub coding: String,
    pub review: String,
}

impl Columns {
    fn get(&self, stage: Stage) -> &str {
        match stage {
            Stage::Picked => &self.picked,
            Stage::Coding => &self.coding,
            Stage::Review => &self.review,
        }
        .trim()
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    linear: Option<linear::Account>,
    connectors: Vec<Connector>,
}

#[derive(Serialize, Clone)]
pub struct Check {
    at: String,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct Settings {
    linear: Option<linear::Account>,
    connectors: Vec<Connector>,
    checked: Option<Check>,
}

/// A session a connector wants opened.
pub struct Run {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub connector: String,
    /// Linear's id for the issue, which `pinned stage` reports on.
    pub issue: String,
}

#[derive(Serialize)]
pub struct Suggestions {
    labels: Vec<String>,
    columns: Vec<String>,
}

pub struct Connectors {
    dir: PathBuf,
    /// This binary, which sessions call back as `pinned stage`.
    exe: PathBuf,
    store: Mutex<Store>,
    checked: Mutex<Option<Check>>,
    wake: Notify,
}

impl Connectors {
    pub fn load(dir: PathBuf, exe: PathBuf) -> Self {
        let store = std::fs::read(dir.join("connectors.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { dir, exe, store: Mutex::new(store), checked: Mutex::new(None), wake: Notify::new() }
    }

    fn save(&self, store: &Store) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join("connectors.json"), bytes).map_err(|e| e.to_string())
    }

    fn key_file(&self) -> PathBuf {
        self.dir.join("linear-key")
    }

    fn linear_key(&self) -> Option<String> {
        let key = std::fs::read_to_string(self.key_file()).ok()?;
        Some(key.trim().to_owned()).filter(|k| !k.is_empty())
    }

    fn write_key(&self, key: &str) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(self.key_file())
            .and_then(|mut f| f.write_all(key.as_bytes()))
            .map_err(|e| e.to_string())
    }

    /// Starts a session for every new issue the enabled connectors match.
    async fn poll(&self) -> Result<Vec<Run>, String> {
        let Some(key) = self.linear_key() else { return Ok(vec![]) };
        let watching: Vec<Connector> =
            self.store.lock().unwrap().connectors.iter().filter(|c| c.enabled).cloned().collect();
        let mut found = vec![];
        for connector in watching {
            let Trigger::LinearLabel { label } = &connector.trigger;
            for issue in linear::labeled_issues(&key, label, &connector.since).await? {
                found.push((connector.id.clone(), issue));
            }
        }

        let mut store = self.store.lock().unwrap();
        let mut runs = vec![];
        for (id, issue) in found {
            // The connector may have been edited or deleted while Linear answered.
            let Some(connector) = store.connectors.iter_mut().find(|c| c.id == id && c.enabled) else { continue };
            if connector.handled.contains(&issue.id) {
                continue;
            }
            connector.handled.push(issue.id.clone());
            runs.push(run(connector, &issue, &self.exe));
        }
        if !runs.is_empty() {
            self.save(&store)?;
        }
        Ok(runs)
    }

    /// Moves the issue to the column the connector maps `stage` to.
    pub async fn advance(&self, connector: &str, issue: &str, stage: Stage) -> Result<String, String> {
        let column = {
            let store = self.store.lock().unwrap();
            let c = store.connectors.iter().find(|c| c.id == connector).ok_or("the connector was deleted")?;
            c.columns.get(stage).to_owned()
        };
        if column.is_empty() {
            return Ok("this connector has no column for that stage, so the issue stays where it is".into());
        }
        let key = self.linear_key().ok_or("Linear is not connected")?;
        linear::move_issue(&key, issue, &column).await?;
        Ok(format!("moved the issue to {column}"))
    }
}

fn arg<T: Serialize>(value: T) -> String {
    serde_json::to_value(value).unwrap().as_str().unwrap().to_owned()
}

fn run(c: &Connector, issue: &linear::Issue, exe: &Path) -> Run {
    let mut args = vec![];
    if !c.model.trim().is_empty() {
        args.extend(["--model".into(), c.model.trim().to_owned()]);
    }
    args.extend(["--effort".into(), arg(c.effort), "--permission-mode".into(), arg(c.permission_mode)]);
    if c.worktree {
        args.push(format!("--worktree={}", issue.identifier.to_lowercase()));
    }
    args.extend(["--name".into(), format!("{} {}", issue.identifier, issue.title)]);
    let mut prompt = c.instructions.trim().to_owned();
    let report = [
        (Stage::Coding, "coding", "right before you start changing code"),
        (Stage::Review, "review", "right after the pull request is open"),
    ];
    let steps: Vec<String> = report
        .iter()
        .filter(|(stage, ..)| !c.columns.get(*stage).is_empty())
        .map(|(_, arg, when)| format!("- Run `'{}' stage {arg}` {when}.", exe.display()))
        .collect();
    if !steps.is_empty() {
        prompt += "\n\nReport your progress to pinned, the app that started this session. Each command moves the Linear issue to its next column. Run each one once:\n";
        prompt += &steps.join("\n");
    }
    prompt += &format!(
        "\n\nLinear issue {}: {}\n{}\n\n{}",
        issue.identifier,
        issue.title,
        issue.url,
        issue.description.as_deref().unwrap_or("(no description)"),
    );
    args.push(prompt);
    Run { cwd: c.folder.clone(), args, connector: c.id.clone(), issue: issue.id.clone() }
}

/// Polls forever; `open` receives each session to start. A settings change wakes it early.
pub async fn watch(app: AppHandle, open: impl Fn(Run)) {
    let connectors = app.state::<Connectors>();
    loop {
        let result = connectors.poll().await;
        let mut error = result.as_ref().err().cloned();
        for run in result.unwrap_or_default() {
            let (connector, issue) = (run.connector.clone(), run.issue.clone());
            open(run);
            // The session starts even when Linear won't move the issue; the check reports why.
            if let Err(e) = connectors.advance(&connector, &issue, Stage::Picked).await {
                error.get_or_insert(e);
            }
        }
        let check = Check { at: chrono::Utc::now().to_rfc3339(), error };
        *connectors.checked.lock().unwrap() = Some(check.clone());
        let _ = app.emit("connectors-checked", check);
        let _ = tokio::time::timeout(INTERVAL, connectors.wake.notified()).await;
    }
}

#[tauri::command]
pub fn get_settings(connectors: State<Connectors>) -> Settings {
    let store = connectors.store.lock().unwrap();
    Settings {
        linear: store.linear.clone(),
        connectors: store.connectors.clone(),
        checked: connectors.checked.lock().unwrap().clone(),
    }
}

#[tauri::command]
pub fn open_linear_keys() -> Result<(), String> {
    std::process::Command::new("open").arg(linear::KEY_PAGE).spawn().map(drop).map_err(|e| e.to_string())
}

/// Checks the key against Linear before keeping it.
#[tauri::command]
pub async fn connect_linear(connectors: State<'_, Connectors>, key: String) -> Result<linear::Account, String> {
    let key = key.trim();
    let account = linear::account(key).await?;
    connectors.write_key(key)?;
    let mut store = connectors.store.lock().unwrap();
    store.linear = Some(account.clone());
    connectors.save(&store)?;
    connectors.wake.notify_one();
    Ok(account)
}

#[tauri::command]
pub fn disconnect_linear(connectors: State<Connectors>) -> Result<(), String> {
    let _ = std::fs::remove_file(connectors.key_file());
    let mut store = connectors.store.lock().unwrap();
    store.linear = None;
    connectors.save(&store)
}

#[tauri::command]
pub async fn linear_suggestions(connectors: State<'_, Connectors>) -> Result<Suggestions, String> {
    let key = connectors.linear_key().ok_or("Linear is not connected")?;
    Ok(Suggestions { labels: linear::label_names(&key).await?, columns: linear::state_names(&key).await? })
}

/// Adds or replaces a connector. Its watermark and handled issues stay with the stored copy.
#[tauri::command]
pub fn save_connector(connectors: State<Connectors>, mut connector: Connector) -> Result<Connector, String> {
    let mut store = connectors.store.lock().unwrap();
    match store.connectors.iter_mut().find(|c| c.id == connector.id && !c.id.is_empty()) {
        Some(existing) => {
            connector.since = existing.since.clone();
            connector.handled = std::mem::take(&mut existing.handled);
            *existing = connector.clone();
        }
        None => {
            connector.id = format!("c{}", chrono::Utc::now().timestamp_millis());
            connector.since = chrono::Utc::now().to_rfc3339();
            connector.handled.clear();
            store.connectors.push(connector.clone());
        }
    }
    connectors.save(&store)?;
    connectors.wake.notify_one();
    Ok(connector)
}

#[tauri::command]
pub fn delete_connector(connectors: State<Connectors>, id: String) -> Result<(), String> {
    let mut store = connectors.store.lock().unwrap();
    store.connectors.retain(|c| c.id != id);
    connectors.save(&store)
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle, start: Option<String>) -> Option<String> {
    let mut dialog = app.dialog().file();
    if let Some(start) = start.filter(|s| !s.is_empty()) {
        dialog = dialog.set_directory(start);
    }
    dialog.blocking_pick_folder()?.into_path().ok().map(|p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_passes_connector_settings_to_claude() {
        let connector = Connector {
            id: "c1".into(),
            name: "Resolve".into(),
            enabled: true,
            trigger: Trigger::LinearLabel { label: "claude".into() },
            folder: "/repo".into(),
            model: " sonnet ".into(),
            effort: Effort::Xhigh,
            permission_mode: PermissionMode::AcceptEdits,
            worktree: true,
            instructions: "Fix it.\n".into(),
            columns: Columns { picked: "Analyzing".into(), coding: "In Dev".into(), review: String::new() },
            since: String::new(),
            handled: vec![],
        };
        let issue = linear::Issue {
            id: "uuid".into(),
            identifier: "ENG-7".into(),
            title: "Broken button".into(),
            description: None,
            url: "https://linear.app/x/issue/ENG-7".into(),
        };
        let run = run(&connector, &issue, Path::new("/Applications/pinned.app/Contents/MacOS/pinned"));
        assert_eq!(run.cwd, PathBuf::from("/repo"));
        assert_eq!((run.connector.as_str(), run.issue.as_str()), ("c1", "uuid"));
        assert_eq!(
            run.args,
            [
                "--model",
                "sonnet",
                "--effort",
                "xhigh",
                "--permission-mode",
                "acceptEdits",
                "--worktree=eng-7",
                "--name",
                "ENG-7 Broken button",
                "Fix it.\n\nReport your progress to pinned, the app that started this session. Each command moves the Linear issue to its next column. Run each one once:\n- Run `'/Applications/pinned.app/Contents/MacOS/pinned' stage coding` right before you start changing code.\n\nLinear issue ENG-7: Broken button\nhttps://linear.app/x/issue/ENG-7\n\n(no description)",
            ]
        );
    }
}
