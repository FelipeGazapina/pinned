//! Connectors make the harness proactive: each one watches an integration and opens a tab
//! with a claude session already working on what it found.

use crate::agent::{self, Stage, Subject};
use crate::wire::RunSpec;
use crate::{github, linear, slack};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Notify;

const INTERVAL: Duration = Duration::from_secs(60);
const LINEAR_KEY: &str = "linear-key";
const SLACK_KEY: &str = "slack-key";
const GITHUB_KEY: &str = "github-key";
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

/// How far a Slack connector's session goes with a bug it finds.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Fix {
    /// Analyze and reply; never file an issue or change code.
    #[default]
    Off,
    /// Analyze, then ask in the thread; file and fix only once someone answered it with a mention.
    Ask,
    /// Fix obvious bugs right away, and ask about the rest.
    Auto,
}

/// connectors.json from before `fix` stored `autoFix: bool`.
fn fix_or_auto_fix<'de, D: Deserializer<'de>>(d: D) -> Result<Fix, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Fix(Fix),
        AutoFix(bool),
    }
    Ok(match Stored::deserialize(d)? {
        Stored::Fix(fix) => fix,
        Stored::AutoFix(true) => Fix::Auto,
        Stored::AutoFix(false) => Fix::Off,
    })
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
    /// The Linear team a Slack connector's sessions file issues in, by name or key.
    #[serde(default)]
    pub team: String,
    /// The Linear label put on the issues a Slack connector's sessions file; empty puts none.
    #[serde(default)]
    pub label: String,
    /// Whether a Slack connector's sessions may file a Linear issue and fix the bug.
    #[serde(default, alias = "autoFix", deserialize_with = "fix_or_auto_fix")]
    pub fix: Fix,
    /// The issue each Slack thread's session filed, by `<channel id>:<thread ts>`.
    #[serde(default)]
    pub created: BTreeMap<String, Filed>,
    /// How far each handled Slack thread was passed on to its session, by `<channel id>:<thread ts>`.
    #[serde(default)]
    pub followed: BTreeMap<String, Followed>,
    /// Whether a session that opens a pull request must prove it works, with a Playwright video.
    #[serde(default = "on")]
    pub proof: bool,
    #[serde(default)]
    pub machine: String,
    #[serde(default)]
    pub repo: String,
}

fn on() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Followed {
    /// The newest message the session has seen, as a Slack ts.
    pub cursor: String,
    /// The newest message pinned read, when people wrote past `cursor` without mentioning the bot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<String>,
    /// Someone mentioned the bot in the thread after its session started, which `Fix::Ask` waits for.
    #[serde(default)]
    pub replied: bool,
}

/// A Linear issue a Slack thread's session filed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Filed {
    pub id: String,
    pub identifier: String,
    pub url: String,
}

impl Filed {
    /// What `pinned linear create` prints.
    fn answer(&self) -> String {
        format!("{} {}", self.identifier, self.url)
    }
}

/// What `pinned linear create` does for a thread.
#[derive(Debug, PartialEq)]
enum Filing {
    /// The thread already filed this issue, so the call returns it instead of filing another.
    Filed(Filed),
    New { team: String, label: String },
}

impl Connector {
    fn filing(&self, channel: &str, thread: &str) -> Result<Filing, String> {
        let key = thread_key(channel, thread);
        match self.fix {
            Fix::Off => {
                return Err(format!(
                    "workflow {:?} does not file Linear issues: it is set to only analyze and reply",
                    self.name
                ))
            }
            Fix::Ask if !self.followed.get(&key).is_some_and(|f| f.replied) => {
                return Err(format!(
                    "workflow {:?} files an issue only after someone answers you in the thread with a mention: ask there, and wait for the answer to reach you",
                    self.name
                ))
            }
            Fix::Ask | Fix::Auto => {}
        }
        if let Some(filed) = self.created.get(&key) {
            return Ok(Filing::Filed(filed.clone()));
        }
        if self.team.trim().is_empty() {
            return Err(format!("workflow {:?} has no Linear team to file issues in: set one in its settings", self.name));
        }
        Ok(Filing::New { team: self.team.trim().to_owned(), label: self.label.trim().to_owned() })
    }

    /// Marks `key` handled, or answers false when it already was.
    fn claim(&mut self, key: String) -> bool {
        if self.handled.contains(&key) {
            return false;
        }
        self.handled.push(key);
        true
    }

    fn repo(&self) -> Option<String> {
        Some(github::clone_url(&self.repo)).filter(|r| !r.is_empty())
    }

    /// Moves a Slack connector's watermark up to `watermark`, and answers whether it moved. A check
    /// that found a mention leaves it where it is: the mention's run may find no machine, and
    /// `unclaim` alone doesn't bring back a mention the watermark has passed. Once a session takes
    /// the mention it is handled, and the next check moves the watermark on.
    fn advance(&mut self, watermark: DateTime<Utc>, mentioned: bool) -> bool {
        let behind = DateTime::parse_from_rfc3339(&self.since).map_or(true, |since| since < watermark);
        if !matches!(self.trigger, Trigger::SlackMention { .. }) || !behind || mentioned {
            return false;
        }
        self.since = watermark.to_rfc3339();
        true
    }

    fn unclaim(&mut self, subject: &Subject) {
        let key = match subject {
            Subject::LinearIssue { issue } => issue.clone(),
            Subject::SlackThread { channel, thread } => thread_key(channel, thread),
        };
        self.handled.retain(|k| *k != key);
        self.followed.remove(&key);
    }

    /// The Linear issue `pinned stage` moves for a session on `subject`.
    fn issue(&self, subject: &Subject) -> Result<String, String> {
        match subject {
            Subject::LinearIssue { issue } => Ok(issue.clone()),
            Subject::SlackThread { channel, thread } => self
                .created
                .get(&thread_key(channel, thread))
                .map(|f| f.id.clone())
                .ok_or_else(|| "this Slack thread has no Linear issue yet: file one with `pinned linear create` first".into()),
        }
    }
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
    #[serde(default)]
    github: Option<github::Account>,
    connectors: Vec<Connector>,
}

impl Store {
    fn connector(&self, id: &str) -> Result<&Connector, String> {
        self.connectors.iter().find(|c| c.id == id).ok_or_else(|| "the workflow was deleted".into())
    }

    /// The connector whose session works on the Linear issue `id`, with the thread it answers when a
    /// Slack thread's session filed or took the issue. An issue gets one session across connectors.
    fn working_on(&self, id: &str) -> Option<(&Connector, Option<&str>)> {
        self.connectors.iter().find_map(|c| match c.created.iter().find(|(_, filed)| filed.id == id) {
            Some((thread, _)) => Some((c, Some(thread.as_str()))),
            None => c.handled.iter().any(|h| h == id).then_some((c, None)),
        })
    }

    /// Gives the Slack thread's session the existing `issue` to work on, unless another session
    /// already works on it. The thread asked for the issue, so `Fix::Ask` doesn't wait for a reply.
    fn take(&mut self, connector: &str, channel: &str, thread: &str, issue: &linear::Issue) -> Result<Filed, String> {
        let key = thread_key(channel, thread);
        let c = self.connector(connector)?;
        if c.fix == Fix::Off {
            return Err(format!("workflow {:?} does not work on Linear issues: it is set to only analyze and reply", c.name));
        }
        match c.created.get(&key) {
            Some(filed) if filed.id == issue.id => return Ok(filed.clone()),
            Some(filed) => return Err(format!("this thread already works on {}: one thread works on one issue", filed.identifier)),
            None => {}
        }
        if let Some((owner, thread)) = self.working_on(&issue.id) {
            let by = match thread {
                Some(_) => format!("the session of workflow {:?} that answers another Slack thread", owner.name),
                None => format!("the session workflow {:?} started for it", owner.name),
            };
            return Err(format!("{} is already being worked on by {by}. No second session starts on it.", issue.identifier));
        }
        let filed = Filed { id: issue.id.clone(), identifier: issue.identifier.clone(), url: issue.url.clone() };
        let c = self.connectors.iter_mut().find(|c| c.id == connector).expect("looked up above");
        c.created.insert(key, filed.clone());
        Ok(filed)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Check {
    pub at: String,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub linear: Option<linear::Account>,
    pub slack: Option<slack::Account>,
    #[serde(default)]
    pub github: Option<github::Account>,
    pub connectors: Vec<Connector>,
    pub checked: Option<Check>,
}

/// The integrations' keys, which never leave a store but by `export`.
#[derive(Default, Debug, PartialEq)]
pub struct Keys {
    pub linear: Option<String>,
    pub slack: Option<String>,
    pub github: Option<String>,
}

/// A session a connector wants opened.
#[derive(Clone)]
pub struct Run {
    pub cwd: PathBuf,
    pub repo: Option<String>,
    pub args: Vec<String>,
    pub connector: String,
    pub subject: Subject,
}

impl Run {
    pub fn spec(&self) -> RunSpec {
        RunSpec {
            folder: self.cwd.display().to_string(),
            repo: self.repo.clone(),
            args: self.args.clone(),
            env: agent::session_env(&self.connector, &self.subject),
            token: None,
        }
    }
}

/// What a check hands to the tabs.
pub enum Work {
    /// A session to open.
    Run(Run),
    /// New messages for the session that answers a Slack thread.
    Delivery(Delivery),
}

#[derive(Clone)]
pub struct Delivery {
    pub connector: String,
    pub channel: String,
    pub thread: String,
    /// One `[Slack reply from …]` block per new message, oldest first.
    pub messages: Vec<String>,
    /// The newest message's ts, which the thread's cursor moves to once a session has the messages.
    pub cursor: String,
}

impl Delivery {
    pub fn env(&self) -> Vec<(String, String)> {
        agent::session_env(&self.connector, &Subject::SlackThread { channel: self.channel.clone(), thread: self.thread.clone() })
    }

    pub fn text(&self) -> String {
        self.messages.join("\n\n")
    }
}

/// Something a connector's check turned up.
enum Found {
    /// Starts a session unless the issue already did.
    Issue(linear::Issue),
    /// Starts a session unless the thread already did.
    Mention { channel: String, channel_name: String, mention: slack::Mention, posts: Vec<slack::Post> },
    /// New messages in a thread that already started a session.
    Replies { channel: String, replies: slack::Replies },
}

impl Found {
    /// What `Connector::handled` and `Connector::followed` record it under.
    fn key(&self) -> String {
        match self {
            Found::Issue(issue) => issue.id.clone(),
            Found::Mention { channel, mention, .. } => thread_key(channel, &mention.thread),
            Found::Replies { channel, replies } => thread_key(channel, &replies.thread),
        }
    }
}

fn thread_key(channel: &str, thread: &str) -> String {
    format!("{channel}:{thread}")
}

/// A Slack connector's channel as the user typed it, without the `#`.
fn channel_name(name: &str) -> &str {
    name.trim().trim_start_matches('#')
}

#[derive(Serialize)]
pub struct Suggestions {
    labels: Vec<String>,
    columns: Vec<String>,
    teams: Vec<String>,
}

pub trait Storage: Send + Sync {
    /// The store's JSON, as `save` last wrote it.
    fn load(&self) -> Option<Vec<u8>>;
    fn save(&self, store: &[u8]) -> Result<(), String>;
    /// A key `write_key` kept under `name`, never empty.
    fn key(&self, name: &str) -> Option<String>;
    fn write_key(&self, name: &str, key: &str) -> Result<(), String>;
    fn remove_key(&self, name: &str);
}

pub struct FileStorage {
    dir: PathBuf,
}

impl FileStorage {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl Storage for FileStorage {
    fn load(&self) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join("connectors.json")).ok()
    }

    fn save(&self, store: &[u8]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join("connectors.json"), store).map_err(|e| e.to_string())
    }

    fn key(&self, name: &str) -> Option<String> {
        let key = std::fs::read_to_string(self.dir.join(name)).ok()?;
        Some(key.trim().to_owned()).filter(|k| !k.is_empty())
    }

    fn write_key(&self, name: &str, key: &str) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(self.dir.join(name))
            .and_then(|mut f| f.write_all(key.as_bytes()))
            .map_err(|e| e.to_string())
    }

    fn remove_key(&self, name: &str) {
        let _ = std::fs::remove_file(self.dir.join(name));
    }
}

pub struct Connectors {
    storage: Box<dyn Storage>,
    exe: String,
    store: Mutex<Store>,
    checked: Mutex<Option<Check>>,
    wake: Notify,
    /// Slack channel ids by the name a connector gives, so a check doesn't list every channel.
    channels: Mutex<HashMap<String, String>>,
    /// Held while a session files or takes a Linear issue, and while a check hands out issues, so
    /// two calls from one thread can't both file one and a check never starts an issue a thread holds.
    filing: tokio::sync::Mutex<()>,
}

impl Connectors {
    pub fn new(storage: impl Storage + 'static, exe: String) -> Self {
        let store = storage.load().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self {
            storage: Box::new(storage),
            exe,
            store: Mutex::new(store),
            checked: Mutex::new(None),
            wake: Notify::new(),
            channels: Mutex::new(HashMap::new()),
            filing: tokio::sync::Mutex::new(()),
        }
    }

    fn save(&self, store: &Store) -> Result<(), String> {
        self.storage.save(&serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?)
    }

    fn key(&self, name: &str) -> Option<String> {
        self.storage.key(name)
    }

    /// Starts a session for everything new the enabled connectors match, and passes new thread
    /// messages on. A failing connector doesn't hold up the others, but a rate limit skips the
    /// rest of that integration until the next check instead of hammering it.
    async fn poll(&self) -> (Vec<Work>, Option<String>) {
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

        let _filing = self.filing.lock().await;
        let mut store = self.store.lock().unwrap();
        let mut changed = false;
        let watermark = now - SLACK_OVERLAP;
        for connector in store.connectors.iter_mut().filter(|c| checked.contains(&c.id)) {
            let mentioned = found.iter().any(|(id, f)| *id == connector.id && matches!(f, Found::Mention { .. }));
            changed |= connector.advance(watermark, mentioned);
        }
        let mut work = vec![];
        for (id, found) in found {
            // A Slack thread's session filed or took the issue, or another connector started it.
            if let Found::Issue(issue) = &found {
                if store.working_on(&issue.id).is_some_and(|(owner, _)| owner.id != id) {
                    continue;
                }
            }
            // The connector may have been edited or deleted while the integration answered.
            let Some(connector) = store.connectors.iter_mut().find(|c| c.id == id && c.enabled) else { continue };
            let key = found.key();
            match found {
                Found::Issue(issue) => {
                    if connector.claim(key) {
                        work.push(Work::Run(run(connector, &issue, &self.exe)));
                        changed = true;
                    }
                }
                Found::Mention { channel, channel_name, mention, posts } => {
                    if !connector.claim(key.clone()) {
                        continue;
                    }
                    // The session starts with these messages, so only later ones are passed on.
                    let cursor = posts.last().map_or_else(|| mention.thread.clone(), |p| p.ts.clone());
                    connector.followed.insert(key, Followed { cursor, seen: None, replied: false });
                    let note = mention_note(&mention, now);
                    work.push(Work::Run(slack_run(connector, &channel, &channel_name, &mention.thread, &posts, note, &self.exe)));
                    changed = true;
                }
                Found::Replies { replies, .. } if replies.posts.is_empty() => {
                    // Nobody mentioned the bot, so nothing is passed on yet. The session gets these
                    // messages with the next mention, and nothing reads them again until then.
                    if let Some(followed) = connector.followed.get_mut(&key) {
                        followed.seen = Some(replies.newest);
                        changed = true;
                    }
                }
                Found::Replies { channel, replies } => work.push(Work::Delivery(Delivery {
                    connector: id,
                    channel,
                    thread: replies.thread,
                    messages: replies.posts.iter().map(reply_block).collect(),
                    cursor: replies.newest,
                })),
            }
        }
        if changed {
            if let Err(e) = self.save(&store) {
                errors.push(e);
            }
        }
        (work, (!errors.is_empty()).then(|| errors.join("; ")))
    }

    /// Gives `work` to the tabs. A thread's cursor moves only once its messages reached a session,
    /// its own or a new one, so messages that didn't are passed on again at the next check. A run
    /// no tab took gives its subject back, so the next check tries it again.
    async fn hand_over<F, Fut>(&self, work: Work, open: &F) -> Result<(), String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = bool>,
    {
        match work {
            Work::Run(run) => {
                let (connector, subject, folder) = (run.connector.clone(), run.subject.clone(), run.cwd.clone());
                if !open(Work::Run(run)).await {
                    return Err(self.unclaim(&connector, &subject, &folder));
                }
                // The session starts even when Linear won't move the issue; the check reports why.
                // A Slack thread has no issue to pick until its session files one.
                match &subject {
                    Subject::LinearIssue { .. } => self.stage(&connector, &subject, Stage::Picked).await.map(drop),
                    Subject::SlackThread { .. } => Ok(()),
                }
            }
            Work::Delivery(d) => {
                let cursor = if open(Work::Delivery(d.clone())).await {
                    d.cursor.clone()
                } else {
                    let (run, newest) = self.fallback(&d).await?;
                    let folder = run.cwd.clone();
                    if !open(Work::Run(run)).await {
                        return Err(format!("could not open a session in {} for a Slack thread; the next check tries again", folder.display()));
                    }
                    newest
                };
                self.follow(&d.connector, &d.channel, &d.thread, cursor)
            }
        }
    }

    fn unclaim(&self, connector: &str, subject: &Subject, folder: &Path) -> String {
        let error = format!("could not open a session in {}; the next check tries again", folder.display());
        let mut store = self.store.lock().unwrap();
        let Some(c) = store.connectors.iter_mut().find(|c| c.id == connector) else { return error };
        c.unclaim(subject);
        match self.save(&store) {
            Ok(()) => error,
            Err(e) => format!("{error}; {e}"),
        }
    }

    /// A new session for a thread no live tab answers, given the whole thread so far, and the
    /// newest message it was given.
    async fn fallback(&self, d: &Delivery) -> Result<(Run, String), String> {
        let c = self.store.lock().unwrap().connector(&d.connector)?.clone();
        let Trigger::SlackMention { channel: name } = &c.trigger else { return Err("the workflow no longer watches Slack".into()) };
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected")?;
        let posts = slack::thread(&token, &d.channel, &d.thread).await?;
        let newest = posts.last().map_or_else(|| d.cursor.clone(), |p| p.ts.clone());
        let note = c.created.get(&thread_key(&d.channel, &d.thread)).map(filed_note);
        Ok((slack_run(&c, &d.channel, channel_name(name), &d.thread, &posts, note.as_deref(), &self.exe), newest))
    }

    /// Records that the thread's session has every message up to `cursor`, someone else's among them.
    fn follow(&self, connector: &str, channel: &str, thread: &str, cursor: String) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        let Some(c) = store.connectors.iter_mut().find(|c| c.id == connector) else { return Ok(()) };
        c.followed.insert(thread_key(channel, thread), Followed { cursor, seen: None, replied: true });
        self.save(&store)
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
                let prefix = thread_key(&channel, "");
                let cursors = c
                    .followed
                    .iter()
                    .filter_map(|(key, f)| {
                        let read = f.seen.clone().unwrap_or_else(|| f.cursor.clone());
                        Some((key.strip_prefix(&prefix)?.to_owned(), slack::Cursor { given: f.cursor.clone(), read }))
                    })
                    .collect();
                let activity = slack::activity(&token, &channel, &bot, since, &cursors).await?;
                let mut found: Vec<Found> = vec![];
                // Oldest first, so a thread's first new mention is the one its session answers.
                for mention in activity.mentions {
                    let key = thread_key(&channel, &mention.thread);
                    if c.handled.contains(&key) || found.iter().any(|f| f.key() == key) {
                        continue;
                    }
                    let posts = slack::thread(&token, &channel, &mention.thread).await?;
                    let channel_name = channel_name(name).to_owned();
                    found.push(Found::Mention { channel: channel.clone(), channel_name, mention, posts });
                }
                found.extend(activity.replies.into_iter().map(|replies| Found::Replies { channel: channel.clone(), replies }));
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

    /// Moves the session's issue to the column the connector maps `stage` to.
    pub async fn stage(&self, connector: &str, subject: &Subject, stage: Stage) -> Result<String, String> {
        let (issue, column) = {
            let store = self.store.lock().unwrap();
            let c = store.connector(connector)?;
            (c.issue(subject)?, c.columns.get(stage).to_owned())
        };
        if column.is_empty() {
            return Ok("this workflow has no column for that stage, so the issue stays where it is".into());
        }
        let key = self.key(LINEAR_KEY).ok_or("Linear is not connected")?;
        linear::move_issue(&key, &issue, &column).await?;
        Ok(format!("moved the issue to {column}"))
    }

    /// Files a Linear issue for the Slack thread, linking back to it, unless the thread already
    /// filed one. Answers `<identifier> <url>` either way.
    pub async fn linear_create(
        &self,
        connector: &str,
        channel: &str,
        thread: &str,
        title: &str,
        description: &str,
        assignee: &str,
    ) -> Result<String, String> {
        let _filing = self.filing.lock().await;
        let filing = self.store.lock().unwrap().connector(connector)?.filing(channel, thread)?;
        let (team, label) = match filing {
            Filing::Filed(filed) => return Ok(filed.answer()),
            Filing::New { team, label } => (team, label),
        };
        let key = self.key(LINEAR_KEY).ok_or("Linear is not connected: connect it in pinned's Connectors settings")?;
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected: connect it in pinned's Connectors settings")?;
        let link = slack::permalink(&token, channel, thread).await?;
        let description = format!("{}\n\nSlack thread: {link}", description.trim());
        let issue = linear::create_issue(&key, &team, title.trim(), &description, &label, assignee).await?;
        let filed = Filed { id: issue.id, identifier: issue.identifier, url: issue.url };
        let mut store = self.store.lock().unwrap();
        // The connector may have been deleted while Linear answered; the issue exists all the same.
        if let Some(c) = store.connectors.iter_mut().find(|c| c.id == connector) {
            c.created.insert(thread_key(channel, thread), filed.clone());
            self.save(&store)?;
        }
        Ok(filed.answer())
    }

    /// Lets the Slack thread's session work on the existing issue `issue`, an identifier such as
    /// ENG-123, unless another session already works on it. Answers the issue's identifier, link,
    /// title and description.
    pub async fn linear_take(&self, connector: &str, channel: &str, thread: &str, issue: &str) -> Result<String, String> {
        let _filing = self.filing.lock().await;
        let key = self.key(LINEAR_KEY).ok_or("Linear is not connected: connect it in pinned's Connectors settings")?;
        let issue = linear::issue(&key, issue.trim()).await?;
        let mut store = self.store.lock().unwrap();
        let filed = store.take(connector, channel, thread, &issue)?;
        self.save(&store)?;
        let description = issue.description.as_deref().unwrap_or_default().trim();
        Ok(format!("{}\n\n{}\n\n{description}", filed.answer(), issue.title).trim_end().to_owned())
    }

    /// Posts `text` in the Slack thread the connector's session answers.
    pub async fn slack_reply(&self, connector: &str, channel: &str, thread: &str, text: &str) -> Result<String, String> {
        if !self.store.lock().unwrap().connectors.iter().any(|c| c.id == connector) {
            return Err("the workflow was deleted".into());
        }
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected: connect it in pinned's Connectors settings")?;
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

fn run(c: &Connector, issue: &linear::Issue, exe: &str) -> Run {
    let mut args = claude_args(c, &issue.identifier.to_lowercase(), format!("{} {}", issue.identifier, issue.title));
    let mut prompt = c.instructions.trim().to_owned();
    let report = [
        (Stage::Coding, "coding", "right before you start changing code"),
        (Stage::Review, "review", "right after the pull request is open"),
    ];
    let steps: Vec<String> = report
        .iter()
        .filter(|(stage, ..)| !c.columns.get(*stage).is_empty())
        .map(|(_, arg, when)| format!("- Run `'{}' stage {arg}` {when}.", exe))
        .collect();
    if !steps.is_empty() {
        prompt += "\n\nReport your progress to pinned, the app that started this session. Each command moves the Linear issue to its next column. Run each one once:\n";
        prompt += &steps.join("\n");
    }
    if c.proof {
        prompt += &proof_of_work(exe);
    }
    prompt += &format!(
        "\n\nLinear issue {}: {}\n{}\n\n{}",
        issue.identifier,
        issue.title,
        issue.url,
        issue.description.as_deref().unwrap_or("(no description)"),
    );
    args.push(prompt);
    Run { cwd: c.folder.clone(), repo: c.repo(), args, connector: c.id.clone(), subject: Subject::LinearIssue { issue: issue.id.clone() } }
}

/// What every connector session that may open a pull request must do to prove the change works.
/// The proof goes on the pull request as a comment, never into the repository.
fn proof_of_work(exe: &str) -> String {
    format!(
        "\n\nProof of work. Prove on the pull request that the change works. Never `git add` the video or the script: nothing from the recording is committed.\n\
         - If the change has behavior a user can see in a running web UI, record it with Playwright. Write the script in a new temporary directory outside the repository. \
         If the project has Playwright installed, load it from there (`createRequire('<repo>/')`); otherwise run `npm init -y && npm i -D playwright && npx playwright install chromium` in the temporary directory.\n\
         - Invoke the `pinned:tutorial-video` skill and record with its helper. The video must teach: start the app the way the project documents, then reach the feature by clicking the sidebar, nav bar and tabs on camera, never by pasting a URL, so the viewer sees where each link lives. The helper fails the run if the page changes without a click. Drive exactly the flow the change affects and hold the final state for a second.\n\
        - Run the script for real, then look at frames from the video (`ffmpeg`) and check that every page change follows a ringed click and that the last frame shows the fixed behavior. If the run fails because of the change, fix the change. Never hide the failure.\n\
        - Once the pull request is open, run `'{}' evidence <video> --caption \"<two lines on what the video shows>\"`. It uploads the video to the repository's `pinned-evidence` prerelease, posts a `## Proof` comment that links it on the pull request, and prints the comment link. \
         Running it again with the same file posts nothing new. Explain the bug path, before and after, in the pull request body.\n\
         - If a browser cannot show the change (a library, a CLI, backend only, no runnable UI), or the app cannot start here, do not fake a video. \
         Post the proof with `gh pr comment <number> --body-file <file>`: a `## Proof` heading, \"No video: <one-line reason>\", then the exact test or command output that proves the change.\n\
         - Never show secrets or real user data in the recording.",
        exe
    )
}

const SLACK_PROOF: &str = "\nPut the link to the Proof comment in your last reply in the thread too.";

const TAB_NAME_CHARS: usize = 50;
const TRIAGE_ONLY: &str = "\n\nThis workflow only triages: never change code, and never file or take a Linear issue. `pinned linear create` and `pinned linear take` are turned off for it.";
const OFFLINE: &str = "\n\npinned was closed when this mention arrived and is only seeing it now. Open your first reply by saying so, in the language of the thread.";

/// The numbered steps that file the issue and fix the bug, starting with the reply `first`.
fn fix_steps(c: &Connector, exe: &str, first: &str) -> String {
    let mut steps = vec![
        first.to_owned(),
        format!(
            "Run `'{exe}' linear create \"<title>\" \"<description>\"` to file the issue. It is assigned to the owner of the connected Linear account; \
             if the thread names who should own it, add `--assignee \"<their name or email>\"` right after `create`. It prints the issue identifier and link; reply with them."
        ),
    ];
    if !c.columns.get(Stage::Coding).is_empty() {
        steps.push(format!("Run `'{exe}' stage coding` right before you start changing code."));
    }
    steps.push("Implement the fix, run the project's tests and checks, and open the pull request.".into());
    if !c.columns.get(Stage::Review).is_empty() {
        steps.push(format!("Run `'{exe}' stage review` right after the pull request is open."));
    }
    steps.push("Reply with the pull request link.".into());
    let steps: Vec<String> = steps.iter().enumerate().map(|(i, s)| format!("   {}. {s}", i + 1)).collect();
    steps.join("\n")
}

/// What a Slack connector's session may do about a bug, with the commands to run.
fn fix_rule(c: &Connector, exe: &str) -> String {
    let answer = "- Not a bug, or existing behavior or a known fix already answers it: reply with the answer and stop.";
    let ask = "reply with your analysis and what you would change, ask in the thread whether to file a Linear issue and fix it, telling them to mention you in their answer, and stop without filing an issue or changing code";
    let failed = "If a pinned command fails, reply in the thread with its error and stop.";
    let take = format!(
        "- It asks you to work on an existing Linear issue, such as ENG-123: run `'{exe}' linear take <identifier>` before anything else. \
         If it says another session already works on the issue, reply that the issue is already being worked on and stop. \
         Otherwise it prints the issue: reply that you are taking it, then fix it in the order above, with that issue in place of filing one."
    );
    match c.fix {
        Fix::Off => TRIAGE_ONLY.to_owned(),
        Fix::Ask => format!(
            "\n\nDecide what the thread needs, then act:\n{answer}\n- A bug: {ask}.\n\
             When a reply in the thread says yes, fix it, in this order:\n{}\n\
             If the reply says no, acknowledge it and stop. `pinned linear create` fails until someone has mentioned you in the thread.\n{take}\n{failed}",
            fix_steps(c, exe, "Reply that you are opening a Linear issue and fixing it.")
        ),
        Fix::Auto => format!(
            "\n\nDecide what the thread needs, then act:\n{answer}\n\
             - A bug that is obvious, because you reproduced it or found it in the code, and whose fix is small and low-risk: fix it, in this order:\n{}\n\
             - A bug that is not obvious, or whose fix is big or risky: {ask}. When a reply in the thread says yes, fix it in the order above; if it says no, acknowledge it and stop.\n{take}\n{failed}",
            fix_steps(c, exe, "Reply with what you found, and that you are opening a Linear issue and fixing it.")
        ),
    }
}

/// The note a mention's session opens with when pinned picked the mention up late.
fn mention_note(mention: &slack::Mention, now: DateTime<Utc>) -> Option<&'static str> {
    (now - mention.at > OFFLINE_AFTER).then_some(OFFLINE)
}

/// A session on the Slack thread, given its messages so far. `note` ends the prompt.
fn slack_run(
    c: &Connector,
    channel: &str,
    channel_name: &str,
    thread: &str,
    posts: &[slack::Post],
    note: Option<&str>,
    exe: &str,
) -> Run {
    let opening = posts.first().and_then(|p| p.text.lines().next()).unwrap_or_default();
    let opening: String = opening.chars().take(TAB_NAME_CHARS).collect();
    let mut args = claude_args(c, &format!("slack-{}", thread.replace('.', "-")), format!("#{channel_name} {opening}"));
    let mut prompt = c.instructions.trim().to_owned();
    prompt += &format!(
        "\n\nAnswer in the Slack thread below by running `'{}' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. \
         Later messages in the thread reach you only when one of them mentions you, as `{}` blocks with every message written since the last one you got; answer them in the thread the same way.",
        exe,
        reply_header("<name>")
    );
    prompt += &fix_rule(c, exe);
    if c.proof && c.fix != Fix::Off {
        prompt += &proof_of_work(exe);
        prompt += SLACK_PROOF;
    }
    prompt += &format!("\n\nSlack thread in #{channel_name}, oldest first:\n");
    let transcript: Vec<String> = posts.iter().map(|p| format!("{}: {}", p.author, p.text)).collect();
    prompt += &transcript.join("\n");
    prompt += note.unwrap_or_default();
    args.push(prompt);
    let subject = Subject::SlackThread { channel: channel.to_owned(), thread: thread.to_owned() };
    Run { cwd: c.folder.clone(), repo: c.repo(), args, connector: c.id.clone(), subject }
}

fn reply_header(author: &str) -> String {
    format!("[Slack reply from {author} in the thread]")
}

/// A later message of the thread, as its session receives it.
fn reply_block(post: &slack::Post) -> String {
    format!("{}\n{}", reply_header(&post.author), post.text)
}

/// The note a thread's new session gets when an earlier session of it filed an issue.
fn filed_note(filed: &Filed) -> String {
    format!(
        "\n\nThis thread already has Linear issue {} ({}). Do not file another one: `pinned linear create` returns that issue.",
        filed.identifier, filed.url
    )
}

impl Connectors {
    /// Polls forever. `open` receives what each check found and answers whether a tab took it,
    /// and `on_check` hears how each check went. A settings change wakes it early.
    pub async fn watch<F, Fut>(&self, open: F, on_check: impl Fn(&Check))
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = bool>,
    {
        loop {
            let (work, mut error) = self.poll().await;
            for work in work {
                if let Err(e) = self.hand_over(work, &open).await {
                    error.get_or_insert(e);
                }
            }
            let check = Check { at: chrono::Utc::now().to_rfc3339(), error };
            *self.checked.lock().unwrap() = Some(check.clone());
            on_check(&check);
            let _ = tokio::time::timeout(INTERVAL, self.wake.notified()).await;
        }
    }

    pub fn get_settings(&self) -> Settings {
        let store = self.store.lock().unwrap();
        Settings {
            linear: store.linear.clone(),
            slack: store.slack.clone(),
            github: store.github.clone(),
            connectors: store.connectors.clone(),
            checked: self.checked.lock().unwrap().clone(),
        }
    }

    /// Checks the key against Linear before keeping it.
    pub async fn connect_linear(&self, key: &str) -> Result<linear::Account, String> {
        let key = key.trim();
        let account = linear::account(key).await?;
        self.storage.write_key(LINEAR_KEY, key)?;
        let mut store = self.store.lock().unwrap();
        store.linear = Some(account.clone());
        self.save(&store)?;
        self.wake.notify_one();
        Ok(account)
    }

    pub fn disconnect_linear(&self) -> Result<(), String> {
        self.storage.remove_key(LINEAR_KEY);
        let mut store = self.store.lock().unwrap();
        store.linear = None;
        self.save(&store)
    }

    pub async fn linear_suggestions(&self) -> Result<Suggestions, String> {
        let key = self.key(LINEAR_KEY).ok_or("Linear is not connected")?;
        Ok(Suggestions {
            labels: linear::label_names(&key).await?,
            columns: linear::state_names(&key).await?,
            teams: linear::team_names(&key).await?,
        })
    }

    /// Checks the bot token against Slack before keeping it.
    pub async fn connect_slack(&self, token: &str) -> Result<slack::Account, String> {
        let token = token.trim();
        let account = slack::account(token).await?;
        self.storage.write_key(SLACK_KEY, token)?;
        self.channels.lock().unwrap().clear();
        let mut store = self.store.lock().unwrap();
        store.slack = Some(account.clone());
        self.save(&store)?;
        self.wake.notify_one();
        Ok(account)
    }

    pub fn disconnect_slack(&self) -> Result<(), String> {
        self.storage.remove_key(SLACK_KEY);
        self.channels.lock().unwrap().clear();
        let mut store = self.store.lock().unwrap();
        store.slack = None;
        self.save(&store)
    }

    /// Checks the token against GitHub before keeping it.
    pub async fn connect_github(&self, token: &str) -> Result<github::Account, String> {
        let token = token.trim();
        let account = github::account(token).await?;
        self.storage.write_key(GITHUB_KEY, token)?;
        let mut store = self.store.lock().unwrap();
        store.github = Some(account.clone());
        self.save(&store)?;
        Ok(account)
    }

    pub fn disconnect_github(&self) -> Result<(), String> {
        self.storage.remove_key(GITHUB_KEY);
        let mut store = self.store.lock().unwrap();
        store.github = None;
        self.save(&store)
    }

    pub async fn github_repos(&self) -> Result<Vec<github::Repo>, String> {
        github::repos(&self.github_token().ok_or("GitHub is not connected")?).await
    }

    /// What a machine clones the account's private GitHub repositories with.
    pub fn github_token(&self) -> Option<String> {
        self.key(GITHUB_KEY)
    }

    /// Channels the bot was invited to, which a Slack connector can watch.
    pub async fn slack_suggestions(&self) -> Result<Vec<String>, String> {
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected")?;
        slack::member_channels(&token).await
    }

    /// Adds or replaces a connector. Its watermark, handled issues, filed issues and thread cursors stay with the stored copy.
    pub fn save_connector(&self, mut connector: Connector) -> Result<Connector, String> {
        let mut store = self.store.lock().unwrap();
        match store.connectors.iter_mut().find(|c| c.id == connector.id && !c.id.is_empty()) {
            Some(existing) => {
                connector.since = existing.since.clone();
                connector.handled = std::mem::take(&mut existing.handled);
                connector.created = std::mem::take(&mut existing.created);
                connector.followed = std::mem::take(&mut existing.followed);
                *existing = connector.clone();
            }
            None => {
                connector.id = format!("c{}", chrono::Utc::now().timestamp_millis());
                connector.since = chrono::Utc::now().to_rfc3339();
                connector.handled.clear();
                connector.created.clear();
                connector.followed.clear();
                store.connectors.push(connector.clone());
            }
        }
        self.save(&store)?;
        self.wake.notify_one();
        Ok(connector)
    }

    pub fn delete_connector(&self, id: &str) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        store.connectors.retain(|c| c.id != id);
        self.save(&store)
    }

    /// Takes exported connectors and keys, as when a signed-out app's connectors move to its account,
    /// and false when there are some already, so a later machine can't replace them. An integration
    /// whose key didn't come along counts as not connected.
    pub fn import(&self, store: serde_json::Value, keys: Keys) -> Result<bool, String> {
        let mut imported: Store = serde_json::from_value(store).map_err(|e| format!("not a connectors store: {e}"))?;
        let mut current = self.store.lock().unwrap();
        if current.linear.is_some() || current.slack.is_some() || current.github.is_some() || !current.connectors.is_empty() {
            return Ok(false);
        }
        let present = |key: Option<String>| key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
        let (linear_key, slack_key, github_key) = (present(keys.linear), present(keys.slack), present(keys.github));
        if linear_key.is_none() {
            imported.linear = None;
        }
        if slack_key.is_none() {
            imported.slack = None;
        }
        if github_key.is_none() {
            imported.github = None;
        }
        for (name, key) in [(LINEAR_KEY, linear_key), (SLACK_KEY, slack_key), (GITHUB_KEY, github_key)] {
            match key {
                Some(key) => self.storage.write_key(name, &key)?,
                None => self.storage.remove_key(name),
            }
        }
        self.channels.lock().unwrap().clear();
        *current = imported;
        self.save(&current)?;
        self.wake.notify_one();
        Ok(true)
    }

    /// The store and the integrations' keys, as `import` takes them.
    pub fn export(&self) -> (serde_json::Value, Keys) {
        let store = serde_json::to_value(&*self.store.lock().unwrap()).unwrap();
        (store, Keys { linear: self.key(LINEAR_KEY), slack: self.key(SLACK_KEY), github: self.key(GITHUB_KEY) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-connectors-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn stored(name: &str, connectors: Vec<Connector>) -> (Connectors, PathBuf) {
        let dir = temp_dir(name);
        let storage = FileStorage::new(dir.clone());
        storage.save(&serde_json::to_vec(&Store { connectors, ..Store::default() }).unwrap()).unwrap();
        (Connectors::new(storage, "/bin/pinned".into()), dir)
    }

    #[test]
    fn a_run_built_in_the_hub_names_the_binary_by_placeholder() {
        let c = Connector { repo: " git@github.com:acme/app.git ".into(), ..linear_connector() };
        let spec = run(&c, &issue(), crate::wire::EXE_PLACEHOLDER).spec();
        assert_eq!((spec.folder.as_str(), spec.repo.as_deref()), ("/repo", Some("git@github.com:acme/app.git")));
        assert_eq!(spec.env, [("PINNED_CONNECTOR".to_owned(), "c1".to_owned()), ("PINNED_ISSUE".to_owned(), "uuid".to_owned())]);
        assert!(spec.args.last().unwrap().contains("- Run `'{{pinned}}' stage coding` right before you start changing code."));
        assert_eq!(run(&linear_connector(), &issue(), "/bin/pinned").spec().repo, None);
    }

    #[test]
    fn a_run_no_tab_took_is_found_again_by_the_next_check() {
        let mut linear = linear_connector();
        linear.handled = vec!["uuid".into(), "older".into()];
        let mut slack = Connector { id: "c2".into(), ..slack_connector() };
        slack.handled = vec!["C0123ABCD:1792400050.000200".into()];
        slack.followed.insert("C0123ABCD:1792400050.000200".into(), Followed { cursor: "1792400060.000000".into(), seen: None, replied: false });
        let (connectors, dir) = stored("unclaim", vec![linear, slack]);
        let refuse = |_: Work| async { false };

        let issue_run = run(&linear_connector(), &issue(), "/bin/pinned");
        assert_eq!(
            block_on(connectors.hand_over(Work::Run(issue_run), &refuse)),
            Err("could not open a session in /repo; the next check tries again".into())
        );
        let thread_run = slack_run(&Connector { id: "c2".into(), ..slack_connector() }, "C0123ABCD", "support", "1792400050.000200", &posts(), None, "/bin/pinned");
        assert!(block_on(connectors.hand_over(Work::Run(thread_run), &refuse)).is_err());

        let reloaded = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into()).get_settings().connectors;
        assert_eq!(reloaded[0].handled, ["older"]);
        assert_eq!((reloaded[1].handled.len(), reloaded[1].followed.len()), (0, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_slack_watermark_waits_for_a_mention_until_a_session_takes_it() {
        let at = |rfc3339: &str| DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc);
        let mut slack = Connector { since: "2026-10-22T16:18:15+00:00".into(), ..slack_connector() };
        assert!(!slack.advance(at("2026-10-22T16:19:17Z"), true), "a check that found a mention keeps the watermark");
        assert!(!slack.advance(at("2026-10-22T16:20:20Z"), true), "and so does the next one while no machine takes it");
        assert_eq!(slack.since, "2026-10-22T16:18:15+00:00");
        assert!(slack.advance(at("2026-10-22T16:21:22Z"), false), "a check that found nothing new moves it on");
        assert_eq!(slack.since, "2026-10-22T16:21:22+00:00");
        assert!(!slack.advance(at("2026-10-22T16:20:00Z"), false), "it never moves back");
        let mut linear = Connector { since: "2026-10-22T16:18:15+00:00".into(), ..linear_connector() };
        assert!(!linear.advance(at("2026-10-22T16:21:22Z"), false), "Linear asks by creation time, not this watermark");
    }

    #[test]
    fn a_run_a_tab_took_stays_claimed() {
        let slack = Connector { id: "c2".into(), handled: vec!["C0123ABCD:1792400050.000200".into()], ..slack_connector() };
        let (connectors, dir) = stored("claimed", vec![slack]);
        let thread_run = slack_run(&Connector { id: "c2".into(), ..slack_connector() }, "C0123ABCD", "support", "1792400050.000200", &posts(), None, "/bin/pinned");
        assert_eq!(block_on(connectors.hand_over(Work::Run(thread_run), &|_| async { true })), Ok(()));
        assert_eq!(connectors.get_settings().connectors[0].handled, ["C0123ABCD:1792400050.000200"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_import_fills_an_empty_store_and_keys_and_drops_integrations_without_a_key() {
        let (connectors, dir) = stored("import", vec![]);
        let exported = serde_json::json!({
            "linear": { "name": "Ana", "email": "ana@acme.dev", "organization": "Acme" },
            "slack": { "team": "Acme", "userId": "UBOT", "name": "pinned" },
            "connectors": [serde_json::to_value(Connector { id: "c9".into(), ..slack_connector() }).unwrap()],
        });
        let keys = Keys { linear: Some(" lin_api_1 ".into()), github: Some("ghp_1".into()), ..Keys::default() };
        assert_eq!(connectors.import(exported.clone(), keys), Ok(true));
        let (store, keys) = connectors.export();
        assert_eq!(keys, Keys { linear: Some("lin_api_1".into()), slack: None, github: Some("ghp_1".into()) });
        assert_eq!(store["linear"]["email"], "ana@acme.dev");
        assert_eq!(store["slack"], serde_json::Value::Null);
        assert_eq!(store["connectors"][0]["id"], "c9");
        assert_eq!(std::fs::read_to_string(dir.join("linear-key")).unwrap(), "lin_api_1");
        let other = Keys { slack: Some("xoxb-2".into()), ..Keys::default() };
        assert_eq!(connectors.import(serde_json::json!({ "connectors": [] }), other), Ok(false), "a store with connectors keeps them");
        assert_eq!(connectors.export().1.linear.as_deref(), Some("lin_api_1"));
        assert!(connectors.import(serde_json::json!({ "connectors": "no" }), Keys::default()).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn linear_connector() -> Connector {
        Connector {
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
            team: String::new(),
            label: String::new(),
            fix: Fix::Off,
            created: BTreeMap::new(),
            followed: BTreeMap::new(),
            proof: false,
            machine: String::new(),
            repo: String::new(),
        }
    }

    fn issue() -> linear::Issue {
        linear::Issue {
            id: "uuid".into(),
            identifier: "ENG-7".into(),
            title: "Broken button".into(),
            description: None,
            url: "https://linear.app/x/issue/ENG-7".into(),
        }
    }

    #[test]
    fn run_passes_connector_settings_to_claude() {
        let run = run(&linear_connector(), &issue(), "/Applications/pinned.app/Contents/MacOS/pinned");
        assert_eq!(run.cwd, PathBuf::from("/repo"));
        assert_eq!((run.connector.as_str(), &run.subject), ("c1", &Subject::LinearIssue { issue: "uuid".into() }));
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
            team: String::new(),
            label: String::new(),
            fix: Fix::Off,
            created: BTreeMap::new(),
            followed: BTreeMap::new(),
            proof: false,
            machine: String::new(),
            repo: String::new(),
        }
    }

    fn thread_run(mentioned: &str, picked_up: &str) -> Run {
        connector_thread_run(&slack_connector(), mentioned, picked_up)
    }

    fn connector_thread_run(c: &Connector, mentioned: &str, picked_up: &str) -> Run {
        let at = |t: &str| DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc);
        let mention = slack::Mention { thread: "1792400050.000200".into(), at: at(mentioned) };
        slack_run(c, "C0123ABCD", "support", &mention.thread, &posts(), mention_note(&mention, at(picked_up)), "/bin/pinned")
    }

    fn posts() -> [slack::Post; 2] {
        [
            slack::Post { ts: "1792400050.000200".into(), author: "ana".into(), text: "@pinned export 500s since today\nsee the log".into() },
            slack::Post { ts: "1792400060.000000".into(), author: "bruno".into(), text: "same here".into() },
        ]
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
                "Triage the thread.\n\nAnswer in the Slack thread below by running `'/bin/pinned' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. Later messages in the thread reach you only when one of them mentions you, as `[Slack reply from <name> in the thread]` blocks with every message written since the last one you got; answer them in the thread the same way.\n\nThis workflow only triages: never change code, and never file or take a Linear issue. `pinned linear create` and `pinned linear take` are turned off for it.\n\nSlack thread in #support, oldest first:\nana: @pinned export 500s since today\nsee the log\nbruno: same here",
            ]
        );
    }

    fn fixing_connector() -> Connector {
        Connector {
            fix: Fix::Auto,
            team: "ENG".into(),
            label: "from-slack".into(),
            permission_mode: PermissionMode::Auto,
            columns: Columns { picked: "Analyzing".into(), coding: "In Dev".into(), review: "Ready to Review".into() },
            instructions: "Fix what you can.".into(),
            ..slack_connector()
        }
    }

    #[test]
    fn a_slack_run_that_fixes_bugs_gets_the_decision_rule_and_its_commands() {
        let prompt = connector_thread_run(&fixing_connector(), "2026-10-19T10:00:00Z", "2026-10-19T10:01:00Z").args.pop().unwrap();
        assert_eq!(
            prompt,
            "Fix what you can.\n\nAnswer in the Slack thread below by running `'/bin/pinned' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. Later messages in the thread reach you only when one of them mentions you, as `[Slack reply from <name> in the thread]` blocks with every message written since the last one you got; answer them in the thread the same way.\n\n\
             Decide what the thread needs, then act:\n\
             - Not a bug, or existing behavior or a known fix already answers it: reply with the answer and stop.\n\
             - A bug that is obvious, because you reproduced it or found it in the code, and whose fix is small and low-risk: fix it, in this order:\n\
             \x20  1. Reply with what you found, and that you are opening a Linear issue and fixing it.\n\
             \x20  2. Run `'/bin/pinned' linear create \"<title>\" \"<description>\"` to file the issue. It is assigned to the owner of the connected Linear account; if the thread names who should own it, add `--assignee \"<their name or email>\"` right after `create`. It prints the issue identifier and link; reply with them.\n\
             \x20  3. Run `'/bin/pinned' stage coding` right before you start changing code.\n\
             \x20  4. Implement the fix, run the project's tests and checks, and open the pull request.\n\
             \x20  5. Run `'/bin/pinned' stage review` right after the pull request is open.\n\
             \x20  6. Reply with the pull request link.\n\
             - A bug that is not obvious, or whose fix is big or risky: reply with your analysis and what you would change, ask in the thread whether to file a Linear issue and fix it, telling them to mention you in their answer, and stop without filing an issue or changing code. When a reply in the thread says yes, fix it in the order above; if it says no, acknowledge it and stop.\n\
             - It asks you to work on an existing Linear issue, such as ENG-123: run `'/bin/pinned' linear take <identifier>` before anything else. If it says another session already works on the issue, reply that the issue is already being worked on and stop. Otherwise it prints the issue: reply that you are taking it, then fix it in the order above, with that issue in place of filing one.\n\
             If a pinned command fails, reply in the thread with its error and stop.\n\n\
             Slack thread in #support, oldest first:\nana: @pinned export 500s since today\nsee the log\nbruno: same here"
        );
        let asking = Connector { fix: Fix::Ask, ..fixing_connector() };
        let prompt = connector_thread_run(&asking, "2026-10-19T10:00:00Z", "2026-10-19T10:01:00Z").args.pop().unwrap();
        assert_eq!(
            prompt,
            "Fix what you can.\n\nAnswer in the Slack thread below by running `'/bin/pinned' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. Later messages in the thread reach you only when one of them mentions you, as `[Slack reply from <name> in the thread]` blocks with every message written since the last one you got; answer them in the thread the same way.\n\n\
             Decide what the thread needs, then act:\n\
             - Not a bug, or existing behavior or a known fix already answers it: reply with the answer and stop.\n\
             - A bug: reply with your analysis and what you would change, ask in the thread whether to file a Linear issue and fix it, telling them to mention you in their answer, and stop without filing an issue or changing code.\n\
             When a reply in the thread says yes, fix it, in this order:\n\
             \x20  1. Reply that you are opening a Linear issue and fixing it.\n\
             \x20  2. Run `'/bin/pinned' linear create \"<title>\" \"<description>\"` to file the issue. It is assigned to the owner of the connected Linear account; if the thread names who should own it, add `--assignee \"<their name or email>\"` right after `create`. It prints the issue identifier and link; reply with them.\n\
             \x20  3. Run `'/bin/pinned' stage coding` right before you start changing code.\n\
             \x20  4. Implement the fix, run the project's tests and checks, and open the pull request.\n\
             \x20  5. Run `'/bin/pinned' stage review` right after the pull request is open.\n\
             \x20  6. Reply with the pull request link.\n\
             If the reply says no, acknowledge it and stop. `pinned linear create` fails until someone has mentioned you in the thread.\n\
             - It asks you to work on an existing Linear issue, such as ENG-123: run `'/bin/pinned' linear take <identifier>` before anything else. If it says another session already works on the issue, reply that the issue is already being worked on and stop. Otherwise it prints the issue: reply that you are taking it, then fix it in the order above, with that issue in place of filing one.\n\
             If a pinned command fails, reply in the thread with its error and stop.\n\n\
             Slack thread in #support, oldest first:\nana: @pinned export 500s since today\nsee the log\nbruno: same here"
        );
        let no_columns = Connector { columns: Columns::default(), ..fixing_connector() };
        let prompt = connector_thread_run(&no_columns, "2026-10-19T10:00:00Z", "2026-10-19T10:01:00Z").args.pop().unwrap();
        assert!(!prompt.contains("stage"));
        assert!(prompt.contains("   3. Implement the fix"));
    }

    #[test]
    fn a_thread_files_one_issue_and_stages_move_it() {
        let thread = Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() };
        let mut c = fixing_connector();
        assert_eq!(c.issue(&thread), Err("this Slack thread has no Linear issue yet: file one with `pinned linear create` first".into()));
        assert_eq!(c.filing("C0123ABCD", "1792400050.000200"), Ok(Filing::New { team: "ENG".into(), label: "from-slack".into() }));
        let filed = Filed { id: "uuid".into(), identifier: "ENG-12".into(), url: "https://linear.app/x/issue/ENG-12".into() };
        c.created.insert("C0123ABCD:1792400050.000200".into(), filed.clone());
        assert_eq!(c.filing("C0123ABCD", "1792400050.000200"), Ok(Filing::Filed(filed)));
        assert_eq!(c.issue(&thread), Ok("uuid".into()));
        assert_eq!(c.filing("C0123ABCD", "1792499999.000100"), Ok(Filing::New { team: "ENG".into(), label: "from-slack".into() }));
        assert_eq!(c.issue(&Subject::LinearIssue { issue: "other".into() }), Ok("other".into()));
    }

    #[test]
    fn the_fix_setting_decides_when_a_thread_may_file_an_issue() {
        let new = Ok(Filing::New { team: "ENG".into(), label: "from-slack".into() });
        let ask_first = Err("workflow \"Triage\" files an issue only after someone answers you in the thread with a mention: ask there, and wait for the answer to reach you".to_owned());
        let with = |fix, followed: Option<bool>| {
            let mut c = Connector { fix, ..fixing_connector() };
            if let Some(replied) = followed {
                c.followed.insert("C0123ABCD:1792400050.000200".into(), Followed { cursor: "1792400060.000000".into(), seen: None, replied });
            }
            c.filing("C0123ABCD", "1792400050.000200")
        };
        assert_eq!(with(Fix::Off, Some(true)), Err("workflow \"Triage\" does not file Linear issues: it is set to only analyze and reply".into()));
        assert_eq!(with(Fix::Ask, None), ask_first);
        assert_eq!(with(Fix::Ask, Some(false)), ask_first);
        assert_eq!(with(Fix::Ask, Some(true)), new);
        assert_eq!(with(Fix::Auto, None), new);
        assert_eq!(with(Fix::Auto, Some(false)), new);
        assert_eq!(
            slack_connector().filing("C0123ABCD", "1792400050.000200"),
            Err("workflow \"Triage\" does not file Linear issues: it is set to only analyze and reply".into())
        );
        let teamless = Connector { team: " ".into(), ..fixing_connector() };
        assert_eq!(
            teamless.filing("C0123ABCD", "1792400050.000200"),
            Err("workflow \"Triage\" has no Linear team to file issues in: set one in its settings".into())
        );
    }

    #[test]
    fn an_issue_gets_one_session_across_connectors() {
        let mut store = Store { connectors: vec![linear_connector(), fixing_connector()], ..Store::default() };
        let filed = Filed { id: "uuid".into(), identifier: "ENG-7".into(), url: "https://linear.app/x/issue/ENG-7".into() };
        assert_eq!(store.take("c2", "C0123ABCD", "1792400050.000200", &issue()), Ok(filed.clone()));
        assert_eq!(store.take("c2", "C0123ABCD", "1792400050.000200", &issue()), Ok(filed));
        assert_eq!(
            store.working_on("uuid").map(|(c, thread)| (c.id.as_str(), thread)),
            Some(("c2", Some("C0123ABCD:1792400050.000200")))
        );
        assert_eq!(
            store.take("c2", "C0123ABCD", "1792499999.000100", &issue()),
            Err("ENG-7 is already being worked on by the session of workflow \"Triage\" that answers another Slack thread. No second session starts on it.".into())
        );

        store.connectors[0].handled.push("uuid-8".into());
        let picked = linear::Issue { id: "uuid-8".into(), identifier: "ENG-8".into(), ..issue() };
        assert_eq!(store.working_on("uuid-8").map(|(c, thread)| (c.id.as_str(), thread)), Some(("c1", None)));
        assert_eq!(
            store.take("c2", "C0123ABCD", "1792499999.000100", &picked),
            Err("ENG-8 is already being worked on by the session workflow \"Resolve\" started for it. No second session starts on it.".into())
        );

        let free = linear::Issue { id: "uuid-9".into(), identifier: "ENG-9".into(), ..issue() };
        assert_eq!(store.working_on("uuid-9").map(|(c, _)| c.id.clone()), None);
        assert_eq!(
            store.take("c2", "C0123ABCD", "1792400050.000200", &free),
            Err("this thread already works on ENG-7: one thread works on one issue".into())
        );
        store.connectors.push(Connector { id: "c3".into(), ..slack_connector() });
        assert_eq!(
            store.take("c3", "C0123ABCD", "1792499999.000100", &free),
            Err("workflow \"Triage\" does not work on Linear issues: it is set to only analyze and reply".into())
        );
        assert_eq!(store.take("gone", "C0123ABCD", "1792499999.000100", &free), Err("the workflow was deleted".into()));
    }

    /// Through the harness's entry points, on a store as connectors.json holds it. No call here
    /// reaches Linear or Slack: each answers from the store or fails before the network.
    #[test]
    fn the_harness_answers_a_second_create_and_a_stage_from_the_stored_issue() {
        let dir = std::env::temp_dir().join(format!("pinned-connectors-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = fixing_connector();
        c.created.insert(
            "C0123ABCD:1792400050.000200".into(),
            Filed { id: "uuid".into(), identifier: "ENG-12".into(), url: "https://linear.app/x/issue/ENG-12".into() },
        );
        let store = Store { connectors: vec![c, Connector { id: "c3".into(), ..slack_connector() }], ..Store::default() };
        std::fs::write(dir.join("connectors.json"), serde_json::to_vec(&store).unwrap()).unwrap();
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into());

        assert_eq!(
            block_on(connectors.linear_create("c2", "C0123ABCD", "1792400050.000200", "Export 500s again", "dup", "")),
            Ok("ENG-12 https://linear.app/x/issue/ENG-12".into())
        );
        let thread = |t: &str| Subject::SlackThread { channel: "C0123ABCD".into(), thread: t.into() };
        assert_eq!(block_on(connectors.stage("c2", &thread("1792400050.000200"), Stage::Coding)), Err("Linear is not connected".into()));
        assert_eq!(
            block_on(connectors.stage("c2", &thread("1792499999.000100"), Stage::Coding)),
            Err("this Slack thread has no Linear issue yet: file one with `pinned linear create` first".into())
        );
        assert_eq!(
            block_on(connectors.linear_create("c3", "C0123ABCD", "1792400050.000200", "Export 500s", "desc", "")),
            Err("workflow \"Triage\" does not file Linear issues: it is set to only analyze and reply".into())
        );
        assert_eq!(block_on(connectors.linear_create("gone", "C0123ABCD", "1", "t", "d", "")), Err("the workflow was deleted".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_store_saved_before_issue_filing_still_loads() {
        let saved = r##"{
          "linear": null,
          "slack": { "team": "Acme", "userId": "UBOT", "name": "pinned" },
          "connectors": [{
            "id": "c2", "name": "Triage", "enabled": true,
            "trigger": { "kind": "slack-mention", "channel": "#support" },
            "folder": "/repo", "model": "", "effort": "high", "permissionMode": "plan", "worktree": false,
            "instructions": "Triage it.", "columns": { "picked": "", "coding": "", "review": "" },
            "since": "2026-10-19T10:00:00+00:00", "handled": ["C0123ABCD:1792400050.000200"]
          }]
        }"##;
        let store: Store = serde_json::from_str(saved).unwrap();
        let c = &store.connectors[0];
        assert_eq!((c.team.as_str(), c.label.as_str(), c.fix, c.created.len(), c.followed.len()), ("", "", Fix::Off, 0, 0));
        assert_eq!(c.handled, ["C0123ABCD:1792400050.000200"]);
    }

    #[test]
    fn a_store_saved_with_auto_fix_loads_into_the_fix_setting() {
        let saved = |auto_fix: &str| {
            format!(
                r##"{{"linear": null, "connectors": [{{
                  "id": "c2", "name": "Triage", "enabled": true,
                  "trigger": {{ "kind": "slack-mention", "channel": "#support" }},
                  "folder": "/repo", "model": "", "effort": "high", "permissionMode": "auto", "worktree": false,
                  "instructions": "Fix it.", "columns": {{ "picked": "", "coding": "In Dev", "review": "" }},
                  "since": "2026-10-19T10:00:00+00:00", "handled": ["C0123ABCD:1792400050.000200"],
                  "team": "ENG", "label": "from-slack", {auto_fix},
                  "created": {{ "C0123ABCD:1792400050.000200": {{ "id": "uuid", "identifier": "ENG-12", "url": "https://linear.app/x/issue/ENG-12" }} }}
                }}]}}"##
            )
        };
        let fix = |field: &str| serde_json::from_str::<Store>(&saved(field)).unwrap().connectors[0].fix;
        assert_eq!(fix(r#""autoFix": true"#), Fix::Auto);
        assert_eq!(fix(r#""autoFix": false"#), Fix::Off);
        assert_eq!(fix(r#""fix": "ask""#), Fix::Ask);
        let store: Store = serde_json::from_str(&saved(r#""autoFix": true"#)).unwrap();
        assert_eq!(store.connectors[0].created["C0123ABCD:1792400050.000200"].identifier, "ENG-12");
        let json = serde_json::to_value(&store.connectors[0]).unwrap();
        assert_eq!((&json["fix"], json.get("autoFix")), (&serde_json::json!("auto"), None));
    }

    #[test]
    fn later_messages_reach_the_session_under_their_author() {
        let post = slack::Post { ts: "1792400090.000000".into(), author: "Ana Lima".into(), text: "yes, create the task\n@pinned thanks".into() };
        assert_eq!(reply_block(&post), "[Slack reply from Ana Lima in the thread]\nyes, create the task\n@pinned thanks");
    }

    #[test]
    fn a_thread_whose_session_is_gone_starts_over_knowing_its_issue() {
        let c = fixing_connector();
        let gone = |note: Option<&str>| slack_run(&c, "C0123ABCD", "support", "1792400050.000200", &posts(), note, "/bin/pinned");
        let first = connector_thread_run(&c, "2026-10-19T10:00:00Z", "2026-10-19T10:01:00Z");
        assert_eq!(gone(None).args, first.args);
        let filed = Filed { id: "uuid".into(), identifier: "ENG-12".into(), url: "https://linear.app/x/issue/ENG-12".into() };
        let prompt = gone(Some(&filed_note(&filed))).args.pop().unwrap();
        assert!(prompt.ends_with(
            "Slack thread in #support, oldest first:\nana: @pinned export 500s since today\nsee the log\nbruno: same here\n\n\
             This thread already has Linear issue ENG-12 (https://linear.app/x/issue/ENG-12). Do not file another one: `pinned linear create` returns that issue."
        ));
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
        assert!(store.connectors[0].proof, "a connector saved before the proof setting requires proof");
        assert_eq!(json[0]["proof"], serde_json::json!(true));
        let mut off = json[0].clone();
        off["proof"] = serde_json::json!(false);
        assert!(!serde_json::from_value::<Connector>(off).unwrap().proof);
    }

    const PROOF: &str = "\n\nProof of work. Prove on the pull request that the change works. Never `git add` the video or the script: nothing from the recording is committed.\n\
        - If the change has behavior a user can see in a running web UI, record it with Playwright. Write the script in a new temporary directory outside the repository. \
        If the project has Playwright installed, load it from there (`createRequire('<repo>/')`); otherwise run `npm init -y && npm i -D playwright && npx playwright install chromium` in the temporary directory.\n\
        - Invoke the `pinned:tutorial-video` skill and record with its helper. The video must teach: start the app the way the project documents, then reach the feature by clicking the sidebar, nav bar and tabs on camera, never by pasting a URL, so the viewer sees where each link lives. The helper fails the run if the page changes without a click. Drive exactly the flow the change affects and hold the final state for a second.\n\
        - Run the script for real, then look at frames from the video (`ffmpeg`) and check that every page change follows a ringed click and that the last frame shows the fixed behavior. If the run fails because of the change, fix the change. Never hide the failure.\n\
        - Once the pull request is open, run `'/bin/pinned' evidence <video> --caption \"<two lines on what the video shows>\"`. It uploads the video to the repository's `pinned-evidence` prerelease, posts a `## Proof` comment that links it on the pull request, and prints the comment link. \
        Running it again with the same file posts nothing new. Explain the bug path, before and after, in the pull request body.\n\
        - If a browser cannot show the change (a library, a CLI, backend only, no runnable UI), or the app cannot start here, do not fake a video. \
        Post the proof with `gh pr comment <number> --body-file <file>`: a `## Proof` heading, \"No video: <one-line reason>\", then the exact test or command output that proves the change.\n\
        - Never show secrets or real user data in the recording.";

    #[test]
    fn a_linear_run_must_prove_its_pull_request_unless_proof_is_off() {
        let prompt = |proof| run(&Connector { proof, ..linear_connector() }, &issue(), "/bin/pinned").args.pop().unwrap();
        assert_eq!(
            prompt(true),
            format!(
                "Fix it.\n\nReport your progress to pinned, the app that started this session. Each command moves the Linear issue to its next column. Run each one once:\n- Run `'/bin/pinned' stage coding` right before you start changing code.\
                 {PROOF}\n\nLinear issue ENG-7: Broken button\nhttps://linear.app/x/issue/ENG-7\n\n(no description)"
            )
        );
        assert!(!prompt(false).contains("Proof"));
    }

    #[test]
    fn a_slack_run_proves_its_pull_request_only_when_it_may_open_one() {
        let prompt = |fix, proof| {
            let c = Connector { fix, proof, ..fixing_connector() };
            connector_thread_run(&c, "2026-10-19T10:00:00Z", "2026-10-19T10:01:00Z").args.pop().unwrap()
        };
        let block = format!(
            "{PROOF}\nPut the link to the Proof comment in your last reply in the thread too.\n\nSlack thread in #support, oldest first:\n"
        );
        for fix in [Fix::Ask, Fix::Auto] {
            let with = prompt(fix, true);
            assert!(with.contains(&format!("If a pinned command fails, reply in the thread with its error and stop.{block}")), "{fix:?}");
            assert_eq!(with.replace(&block, "\n\nSlack thread in #support, oldest first:\n"), prompt(fix, false), "{fix:?}");
        }
        assert!(!prompt(Fix::Off, true).contains("Proof"));
        assert_eq!(prompt(Fix::Off, true), prompt(Fix::Off, false));
    }
}
