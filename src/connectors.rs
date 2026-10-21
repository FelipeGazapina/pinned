//! Connectors make the harness proactive: each one watches an integration and opens a tab
//! with a claude session already working on what it found.

use crate::agent::{Stage, ISSUE_ENV, SLACK_CHANNEL_ENV, SLACK_THREAD_ENV};
use crate::{linear, slack};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::Notify;

const INTERVAL: Duration = Duration::from_secs(60);
/// Files in the config dir that hold each integration's secret, readable only by the user.
const LINEAR_KEY: &str = "linear-key";
const SLACK_KEY: &str = "slack-key";
/// A Slack watermark trails the check that moved it, so a message Slack shows a little late
/// is still after it. Threads already handled keep the overlap from starting a session twice.
const SLACK_OVERLAP: chrono::Duration = chrono::Duration::seconds(60);
/// A mention older than this when pinned picks it up arrived while pinned was closed.
const OFFLINE_AFTER: chrono::Duration = chrono::Duration::minutes(5);

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
    /// A Slack message in this channel, by name or id, that mentions the bot.
    SlackMention { channel: String },
}

impl Trigger {
    fn integration(&self) -> &'static str {
        match self {
            Trigger::LinearLabel { .. } => "Linear",
            Trigger::SlackMention { .. } => "Slack",
        }
    }
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
    /// doesn't start sessions for the backlog. Slack connectors move it forward on every check.
    #[serde(default)]
    pub since: String,
    /// Issues, or Slack threads as `<channel id>:<thread ts>`, that already started a session.
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
    #[serde(default)]
    slack: Option<slack::Account>,
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
    slack: Option<slack::Account>,
    connectors: Vec<Connector>,
    checked: Option<Check>,
}

/// A session a connector wants opened.
pub struct Run {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub connector: String,
    pub subject: Subject,
}

/// What a connector's session works on, which its callbacks into pinned report against.
#[derive(Clone, Debug, PartialEq)]
pub enum Subject {
    /// Linear's id for the issue, which `pinned stage` moves.
    LinearIssue(String),
    /// The Slack thread `pinned slack reply` answers in.
    SlackThread { channel: String, thread: String },
}

impl Subject {
    /// The session's environment that names this subject.
    pub fn env(&self) -> Vec<(String, String)> {
        match self {
            Subject::LinearIssue(id) => vec![(ISSUE_ENV.to_owned(), id.clone())],
            Subject::SlackThread { channel, thread } => {
                vec![(SLACK_CHANNEL_ENV.to_owned(), channel.clone()), (SLACK_THREAD_ENV.to_owned(), thread.clone())]
            }
        }
    }
}

/// Something a connector's check turned up that starts a session unless it already did.
enum Found {
    Issue(linear::Issue),
    Mention { channel: String, channel_name: String, mention: slack::Mention, posts: Vec<slack::Post> },
}

impl Found {
    /// What `Connector::handled` records, so each issue or thread starts one session.
    fn key(&self) -> String {
        match self {
            Found::Issue(issue) => issue.id.clone(),
            Found::Mention { channel, mention, .. } => thread_key(channel, &mention.thread),
        }
    }
}

fn thread_key(channel: &str, thread: &str) -> String {
    format!("{channel}:{thread}")
}

#[derive(Serialize)]
pub struct Suggestions {
    labels: Vec<String>,
    columns: Vec<String>,
    teams: Vec<String>,
}

pub struct Connectors {
    dir: PathBuf,
    /// This binary, which sessions call back as `pinned stage`.
    exe: PathBuf,
    store: Mutex<Store>,
    checked: Mutex<Option<Check>>,
    wake: Notify,
    /// Slack channel ids by the name a connector gives, so a check doesn't list every channel.
    channels: Mutex<HashMap<String, String>>,
}

impl Connectors {
    pub fn load(dir: PathBuf, exe: PathBuf) -> Self {
        let store = std::fs::read(dir.join("connectors.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            dir,
            exe,
            store: Mutex::new(store),
            checked: Mutex::new(None),
            wake: Notify::new(),
            channels: Mutex::new(HashMap::new()),
        }
    }

    fn save(&self, store: &Store) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join("connectors.json"), bytes).map_err(|e| e.to_string())
    }

    fn key(&self, file: &str) -> Option<String> {
        let key = std::fs::read_to_string(self.dir.join(file)).ok()?;
        Some(key.trim().to_owned()).filter(|k| !k.is_empty())
    }

    fn write_key(&self, file: &str, key: &str) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(self.dir.join(file))
            .and_then(|mut f| f.write_all(key.as_bytes()))
            .map_err(|e| e.to_string())
    }

    /// Starts a session for everything new the enabled connectors match. A failing connector
    /// doesn't hold up the others, but a rate limit skips the rest of that integration until the
    /// next check instead of hammering it.
    async fn poll(&self) -> (Vec<Run>, Option<String>) {
        let now = Utc::now();
        let watching: Vec<Connector> =
            self.store.lock().unwrap().connectors.iter().filter(|c| c.enabled).cloned().collect();
        let (mut found, mut checked, mut limited, mut errors) = (vec![], vec![], vec![], vec![]);
        for connector in watching {
            let integration = connector.trigger.integration();
            if limited.contains(&integration) {
                continue;
            }
            match self.scan(&connector, now).await {
                Ok(None) => {}
                Ok(Some(items)) => {
                    found.extend(items.into_iter().map(|f| (connector.id.clone(), f)));
                    checked.push(connector.id);
                }
                Err(e) => {
                    if e == slack::RATE_LIMITED {
                        limited.push(integration);
                    }
                    errors.push(e);
                }
            }
        }

        let mut store = self.store.lock().unwrap();
        let mut changed = false;
        let watermark = now - SLACK_OVERLAP;
        for connector in store.connectors.iter_mut().filter(|c| checked.contains(&c.id)) {
            let behind = DateTime::parse_from_rfc3339(&connector.since).map_or(true, |since| since < watermark);
            if matches!(connector.trigger, Trigger::SlackMention { .. }) && behind {
                connector.since = watermark.to_rfc3339();
                changed = true;
            }
        }
        let mut runs = vec![];
        for (id, found) in found {
            // The connector may have been edited or deleted while the integration answered.
            let Some(connector) = store.connectors.iter_mut().find(|c| c.id == id && c.enabled) else { continue };
            let key = found.key();
            if connector.handled.contains(&key) {
                continue;
            }
            connector.handled.push(key);
            changed = true;
            runs.push(match &found {
                Found::Issue(issue) => run(connector, issue, &self.exe),
                Found::Mention { channel, channel_name, mention, posts } => {
                    slack_run(connector, channel, channel_name, mention, posts, now, &self.exe)
                }
            });
        }
        if changed {
            if let Err(e) = self.save(&store) {
                errors.push(e);
            }
        }
        (runs, (!errors.is_empty()).then(|| errors.join("; ")))
    }

    /// What one connector finds that it hasn't handled yet, or None while its integration isn't connected.
    async fn scan(&self, c: &Connector, now: DateTime<Utc>) -> Result<Option<Vec<Found>>, String> {
        match &c.trigger {
            Trigger::LinearLabel { label } => {
                let Some(key) = self.key(LINEAR_KEY) else { return Ok(None) };
                Ok(Some(linear::labeled_issues(&key, label, &c.since).await?.into_iter().map(Found::Issue).collect()))
            }
            Trigger::SlackMention { channel: name } => {
                let bot = self.store.lock().unwrap().slack.as_ref().map(|a| a.user_id.clone());
                let (Some(token), Some(bot)) = (self.key(SLACK_KEY), bot) else { return Ok(None) };
                let channel = self.channel_id(&token, name).await?;
                let since = DateTime::parse_from_rfc3339(&c.since).map_or(now, Into::into);
                let mut found: Vec<Found> = vec![];
                // Oldest first, so a thread's first new mention is the one its session answers.
                for mention in slack::mentions(&token, &channel, &bot, since).await? {
                    let key = thread_key(&channel, &mention.thread);
                    if c.handled.contains(&key) || found.iter().any(|f| f.key() == key) {
                        continue;
                    }
                    let posts = slack::thread(&token, &channel, &mention.thread).await?;
                    let channel_name = name.trim().trim_start_matches('#').to_owned();
                    found.push(Found::Mention { channel: channel.clone(), channel_name, mention, posts });
                }
                Ok(Some(found))
            }
        }
    }

    async fn channel_id(&self, token: &str, name: &str) -> Result<String, String> {
        if let Some(id) = self.channels.lock().unwrap().get(name) {
            return Ok(id.clone());
        }
        let id = slack::channel_id(token, name).await?;
        self.channels.lock().unwrap().insert(name.to_owned(), id.clone());
        Ok(id)
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
        let key = self.key(LINEAR_KEY).ok_or("Linear is not connected")?;
        linear::move_issue(&key, issue, &column).await?;
        Ok(format!("moved the issue to {column}"))
    }

    /// Posts `text` in the Slack thread the connector's session answers.
    pub async fn reply(&self, connector: &str, channel: &str, thread: &str, text: &str) -> Result<String, String> {
        if !self.store.lock().unwrap().connectors.iter().any(|c| c.id == connector) {
            return Err("the connector was deleted".into());
        }
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected: connect it in pinned's Integrations settings")?;
        slack::post(&token, channel, thread, text).await?;
        Ok("posted the reply in the Slack thread".into())
    }
}

fn arg<T: Serialize>(value: T) -> String {
    serde_json::to_value(value).unwrap().as_str().unwrap().to_owned()
}

/// The `claude` arguments every connector session starts with, before its prompt.
fn claude_args(c: &Connector, worktree: &str, name: String) -> Vec<String> {
    let mut args = vec![];
    if !c.model.trim().is_empty() {
        args.extend(["--model".into(), c.model.trim().to_owned()]);
    }
    args.extend(["--effort".into(), arg(c.effort), "--permission-mode".into(), arg(c.permission_mode)]);
    if c.worktree {
        args.push(format!("--worktree={worktree}"));
    }
    args.extend(["--name".into(), name]);
    args
}

fn run(c: &Connector, issue: &linear::Issue, exe: &Path) -> Run {
    let mut args = claude_args(c, &issue.identifier.to_lowercase(), format!("{} {}", issue.identifier, issue.title));
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
    Run { cwd: c.folder.clone(), args, connector: c.id.clone(), subject: Subject::LinearIssue(issue.id.clone()) }
}

const TAB_NAME_CHARS: usize = 50;

fn slack_run(
    c: &Connector,
    channel: &str,
    channel_name: &str,
    mention: &slack::Mention,
    posts: &[slack::Post],
    now: DateTime<Utc>,
    exe: &Path,
) -> Run {
    let opening = posts.first().and_then(|p| p.text.lines().next()).unwrap_or_default();
    let opening: String = opening.chars().take(TAB_NAME_CHARS).collect();
    let mut args = claude_args(c, &format!("slack-{}", mention.thread.replace('.', "-")), format!("#{channel_name} {opening}"));
    let mut prompt = c.instructions.trim().to_owned();
    prompt += &format!(
        "\n\nAnswer in the Slack thread below by running `'{}' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn.",
        exe.display()
    );
    prompt += &format!("\n\nSlack thread in #{channel_name}, oldest first:\n");
    let transcript: Vec<String> = posts.iter().map(|p| format!("{}: {}", p.author, p.text)).collect();
    prompt += &transcript.join("\n");
    if now - mention.at > OFFLINE_AFTER {
        prompt += "\n\npinned was closed when this mention arrived and is only seeing it now. Open your first reply by saying so, in the language of the thread.";
    }
    args.push(prompt);
    let subject = Subject::SlackThread { channel: channel.to_owned(), thread: mention.thread.clone() };
    Run { cwd: c.folder.clone(), args, connector: c.id.clone(), subject }
}

/// Polls forever; `open` receives each session to start. A settings change wakes it early.
pub async fn watch(app: AppHandle, open: impl Fn(Run)) {
    let connectors = app.state::<Connectors>();
    loop {
        let (runs, mut error) = connectors.poll().await;
        for run in runs {
            let (connector, subject) = (run.connector.clone(), run.subject.clone());
            open(run);
            // The session starts even when Linear won't move the issue; the check reports why.
            if let Subject::LinearIssue(issue) = subject {
                if let Err(e) = connectors.advance(&connector, &issue, Stage::Picked).await {
                    error.get_or_insert(e);
                }
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
        slack: store.slack.clone(),
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
    connectors.write_key(LINEAR_KEY, key)?;
    let mut store = connectors.store.lock().unwrap();
    store.linear = Some(account.clone());
    connectors.save(&store)?;
    connectors.wake.notify_one();
    Ok(account)
}

#[tauri::command]
pub fn disconnect_linear(connectors: State<Connectors>) -> Result<(), String> {
    let _ = std::fs::remove_file(connectors.dir.join(LINEAR_KEY));
    let mut store = connectors.store.lock().unwrap();
    store.linear = None;
    connectors.save(&store)
}

#[tauri::command]
pub async fn linear_suggestions(connectors: State<'_, Connectors>) -> Result<Suggestions, String> {
    let key = connectors.key(LINEAR_KEY).ok_or("Linear is not connected")?;
    Ok(Suggestions {
        labels: linear::label_names(&key).await?,
        columns: linear::state_names(&key).await?,
        teams: linear::team_names(&key).await?,
    })
}

/// Checks the bot token against Slack before keeping it.
#[tauri::command]
pub async fn connect_slack(connectors: State<'_, Connectors>, token: String) -> Result<slack::Account, String> {
    let token = token.trim();
    let account = slack::account(token).await?;
    connectors.write_key(SLACK_KEY, token)?;
    connectors.channels.lock().unwrap().clear();
    let mut store = connectors.store.lock().unwrap();
    store.slack = Some(account.clone());
    connectors.save(&store)?;
    connectors.wake.notify_one();
    Ok(account)
}

#[tauri::command]
pub fn disconnect_slack(connectors: State<Connectors>) -> Result<(), String> {
    let _ = std::fs::remove_file(connectors.dir.join(SLACK_KEY));
    connectors.channels.lock().unwrap().clear();
    let mut store = connectors.store.lock().unwrap();
    store.slack = None;
    connectors.save(&store)
}

/// Channels the bot was invited to, which a Slack connector can watch.
#[tauri::command]
pub async fn slack_suggestions(connectors: State<'_, Connectors>) -> Result<Vec<String>, String> {
    let token = connectors.key(SLACK_KEY).ok_or("Slack is not connected")?;
    slack::member_channels(&token).await
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
        assert_eq!((run.connector.as_str(), &run.subject), ("c1", &Subject::LinearIssue("uuid".into())));
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

    fn slack_connector() -> Connector {
        Connector {
            id: "c2".into(),
            name: "Triage".into(),
            enabled: true,
            trigger: Trigger::SlackMention { channel: "#support".into() },
            folder: "/repo".into(),
            model: String::new(),
            effort: Effort::High,
            permission_mode: PermissionMode::Plan,
            worktree: false,
            instructions: "Triage the thread.".into(),
            columns: Columns::default(),
            since: String::new(),
            handled: vec![],
        }
    }

    fn thread_run(mentioned: &str, picked_up: &str) -> Run {
        let at = |t: &str| DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc);
        let mention = slack::Mention { thread: "1792400050.000200".into(), at: at(mentioned) };
        let posts = [
            slack::Post { author: "ana".into(), text: "@pinned export 500s since today\nsee the log".into() },
            slack::Post { author: "bruno".into(), text: "same here".into() },
        ];
        slack_run(&slack_connector(), "C0123ABCD", "support", &mention, &posts, at(picked_up), Path::new("/bin/pinned"))
    }

    #[test]
    fn slack_run_answers_the_thread_it_was_mentioned_in() {
        let run = thread_run("2026-10-19T10:00:00Z", "2026-10-19T10:01:00Z");
        assert_eq!(run.subject, Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() });
        assert_eq!(
            run.subject.env(),
            [("PINNED_SLACK_CHANNEL".to_owned(), "C0123ABCD".to_owned()), ("PINNED_SLACK_THREAD".to_owned(), "1792400050.000200".to_owned())]
        );
        assert_eq!(
            run.args,
            [
                "--effort",
                "high",
                "--permission-mode",
                "plan",
                "--name",
                "#support @pinned export 500s since today",
                "Triage the thread.\n\nAnswer in the Slack thread below by running `'/bin/pinned' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn.\n\nSlack thread in #support, oldest first:\nana: @pinned export 500s since today\nsee the log\nbruno: same here",
            ]
        );
    }

    #[test]
    fn a_mention_picked_up_late_opens_with_an_apology() {
        let offline = "\n\npinned was closed when this mention arrived and is only seeing it now. Open your first reply by saying so, in the language of the thread.";
        let prompt = |picked_up| thread_run("2026-10-19T10:00:00Z", picked_up).args.pop().unwrap();
        assert!(!prompt("2026-10-19T10:05:00Z").ends_with(offline));
        assert!(prompt("2026-10-19T10:05:01Z").ends_with(offline));
        assert!(prompt("2026-10-21T08:00:00Z").ends_with(offline));
    }

    #[test]
    fn a_store_saved_before_slack_still_loads() {
        let saved = r#"{
          "linear": { "name": "Felipe", "email": "f@x.com", "organization": "X" },
          "connectors": [{
            "id": "c1", "name": "Resolve", "enabled": true,
            "trigger": { "kind": "linear-label", "label": "claude" },
            "folder": "/repo", "model": "opus", "effort": "high", "permissionMode": "auto", "worktree": true,
            "instructions": "Fix it.", "columns": { "picked": "Analyzing", "coding": "", "review": "" },
            "since": "2026-10-01T00:00:00+00:00", "handled": ["uuid"]
          }]
        }"#;
        let store: Store = serde_json::from_str(saved).unwrap();
        assert!(store.slack.is_none());
        let json = serde_json::to_value(&store.connectors).unwrap();
        assert_eq!(json[0]["trigger"], serde_json::json!({ "kind": "linear-label", "label": "claude" }));
        assert_eq!(json[0]["handled"], serde_json::json!(["uuid"]));
    }
}
