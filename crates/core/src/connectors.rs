//! Connectors make the harness proactive: each one watches an integration and opens a tab
//! with a claude session already working on what it found.

use crate::agent::{self, AgentStatus, Stage, Subject};
use crate::graph::migrate::migrate;
use crate::graph::project::project;
use crate::graph::runner::{self, Drive, Effect, Session, Status, Supplied, Wake};
use crate::graph::{Answers, ChosenOn, Gate, Graph, NodeKind, Watch};
use crate::wire::{MachineId, RunSpec, TabView};
use crate::{github, linear, slack, upstash, vercel};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
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
const VERCEL_KEY: &str = "vercel-key";
const UPSTASH_KEY: &str = "upstash-box-key";
const CLAUDE_TOKEN: &str = "claude-oauth-token";
/// A Slack watermark trails the check that moved it, so a message Slack shows a little late
/// is still after it. Threads already handled keep the overlap from starting a session twice.
const SLACK_OVERLAP: chrono::Duration = chrono::Duration::seconds(60);
/// A mention older than this when pinned picks it up arrived while pinned was closed.
const OFFLINE_AFTER: chrono::Duration = chrono::Duration::minutes(5);
pub(crate) const MIN_CONFIDENCE_TO_PAGE: f64 = 0.85;
pub(crate) const UNSURE_BELOW: f64 = 0.6;
const PAGE_COOLDOWN: chrono::Duration = chrono::Duration::minutes(30);

#[derive(Serialize, Deserialize, Clone, Copy, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// Serialized names match `claude --permission-mode`.
#[derive(Serialize, Deserialize, Clone, Copy, schemars::JsonSchema)]
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
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, schemars::JsonSchema)]
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Trigger {
    /// A Linear issue created with this label.
    LinearLabel { label: String },
    /// A Slack message in this channel, by name or id, that mentions the bot.
    SlackMention { channel: String },
    /// Every new message people start in this channel, by name or id, which a decision model sorts
    /// into one of `decide`'s categories. The workflow has no session of its own.
    SlackMessage { channel: String, decide: Decide },
    /// A Slack thread a message workflow's category hands it, which its session answers as a
    /// mention's session would. It watches no channel of its own.
    Called,
}

impl Trigger {
    fn integration(&self) -> &'static str {
        match self {
            Trigger::LinearLabel { .. } => "Linear",
            Trigger::SlackMention { .. } | Trigger::SlackMessage { .. } | Trigger::Called => "Slack",
        }
    }

    fn answers_threads(&self) -> bool {
        match self {
            Trigger::SlackMention { .. } | Trigger::Called => true,
            Trigger::LinearLabel { .. } | Trigger::SlackMessage { .. } => false,
        }
    }
}

/// How a message workflow sorts each message, and where each category sends it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Decide {
    /// A decision model on the Vercel AI Gateway, such as `typesafe-ai/jev`.
    pub model: String,
    pub instructions: String,
    pub categories: Vec<Category>,
    /// The category, by name, that takes a message the model is unsure about. None keeps its guess.
    #[serde(default)]
    pub fallback: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Category {
    pub name: String,
    /// What the decision model reads to tell this category from the others.
    pub description: String,
    /// The id of the `slack-mention` or `called` workflow that then owns the thread. None stops
    /// at the category.
    pub workflow: Option<String>,
    /// A Slack user group, by handle or id, paged in the thread of each message in the category.
    #[serde(default)]
    pub mention: Option<String>,
}

impl Decide {
    fn choices(&self, workflow: &str) -> Result<Vec<(&str, &str)>, String> {
        if self.model.trim().is_empty() {
            return Err(format!("workflow {workflow:?} has no decision model: pick one in its settings"));
        }
        if self.categories.is_empty() {
            return Err(format!("workflow {workflow:?} has no categories to sort messages into: add one in its settings"));
        }
        if let Some(name) = &self.fallback {
            let unsure = format!("workflow {workflow:?} sends the messages its decision model is unsure about to {name:?}");
            match self.fallback() {
                None => return Err(format!("{unsure}, which is not one of its categories: pick another one in its settings")),
                Some(i) if self.categories[i].group().is_some() => {
                    return Err(format!(
                        "{unsure}, which pings a group: the fallback can't ping a group, so pick a category without a mention in its settings"
                    ))
                }
                Some(_) => {}
            }
        }
        Ok(self.categories.iter().map(|c| (c.name.as_str(), c.description.as_str())).collect())
    }

    fn fallback(&self) -> Option<usize> {
        let name = self.fallback.as_deref()?;
        self.categories.iter().position(|c| c.name == name)
    }

    fn pick(&self, chosen: &vercel::Chosen) -> Pick {
        let quiet = |i: &usize| self.categories[*i].group().is_none();
        let (guess, likelihood) = (chosen.index, chosen.probabilities[chosen.index]);
        if let Some(fallback) = self.fallback().filter(quiet) {
            if likelihood < UNSURE_BELOW {
                return Pick::Unsure { guess, likelihood, fallback };
            }
        }
        if quiet(&guess) || likelihood >= MIN_CONFIDENCE_TO_PAGE {
            return Pick::Sure(guess);
        }
        match (0..self.categories.len()).filter(quiet).reduce(|best, i| if chosen.probabilities[i] > chosen.probabilities[best] { i } else { best }) {
            Some(quiet) => Pick::Downgraded(quiet),
            None => Pick::Nowhere,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Pick {
    Sure(usize),
    Unsure { guess: usize, likelihood: f64, fallback: usize },
    Downgraded(usize),
    Nowhere,
}

impl Pick {
    fn category(&self) -> Option<usize> {
        match *self {
            Pick::Sure(i) | Pick::Downgraded(i) | Pick::Unsure { fallback: i, .. } => Some(i),
            Pick::Nowhere => None,
        }
    }

    fn note(&self, categories: &[Category], router: &str) -> String {
        match *self {
            Pick::Sure(i) | Pick::Downgraded(i) => routed_note(router, &categories[i].name),
            Pick::Unsure { guess, likelihood, fallback } => format!(
                "\n\nThe decision model was unsure (it guessed {:?} at {:.0}%), so workflow {router:?} handed this thread over as {:?}.",
                categories[guess].name,
                likelihood * 100.0,
                categories[fallback].name
            ),
            Pick::Nowhere => String::new(),
        }
    }
}

impl Category {
    fn group(&self) -> Option<&str> {
        self.mention.as_deref().map(str::trim).filter(|g| !g.is_empty())
    }
}

/// A model on Vercel's AI Gateway that reads what the trigger found before the session starts.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    /// A gateway model id, as `typesafe-ai/jev`. The id picks chat or decision when the step runs.
    pub model: String,
    pub instructions: String,
    pub questions: Vec<Question>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    /// The key the answer goes under, as `kind`.
    pub name: String,
    pub ask: String,
    /// The answers the model must pick from; empty takes free text.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Choices that end the workflow without a session.
    #[serde(default)]
    pub stop_on: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, schemars::JsonSchema)]
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
    #[serde(default)]
    pub paged: BTreeMap<String, Paged>,
    /// Whether a session that opens a pull request must prove it works, with a Playwright video.
    #[serde(default = "on")]
    pub proof: bool,
    /// Asks a gateway model about the trigger's find before the session starts. None skips it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
    #[serde(default, rename = "machine")]
    pub runs_on: RunsOn,
    #[serde(default)]
    pub repo: String,
    /// The account its sessions run under; empty runs them under their project's.
    #[serde(default)]
    pub account: String,
    /// The workflow drawn as a graph. None runs the fixed pipeline the fields above describe.
    /// Older apps neither send nor read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<crate::graph::Graph>,
}

fn on() -> bool {
    true
}

/// What an app reads. A connector that has no stored graph still sends the graph `migrate` builds,
/// so a new app always sees one. The store keeps `None` until someone edits the graph.
fn presented(mut connector: Connector) -> Connector {
    if connector.graph.is_none() {
        connector.graph = Some(migrate(&connector));
    }
    connector
}

fn refresh_legacy(connector: &mut Connector, graph: &Graph) {
    let projected = project(graph);
    connector.trigger = projected.trigger;
    connector.ask = projected.ask;
}

/// A migrated router calls the slack-mention workflow that then owns the thread, not only a
/// `Watch::Called` one. Both answer a thread, which is what `Call` walks.
fn call_target(store: &Store, saving: &str, graph: &Graph, id: &str) -> bool {
    if !saving.is_empty() && id == saving {
        return answers_threads_graph(graph);
    }
    store.connectors.iter().find(|connector| connector.id == id).is_some_and(answers_threads)
}

fn answers_threads(connector: &Connector) -> bool {
    match &connector.graph {
        Some(graph) => answers_threads_graph(graph),
        None => matches!(connector.trigger, Trigger::SlackMention { .. } | Trigger::Called),
    }
}

fn answers_threads_graph(graph: &Graph) -> bool {
    graph.nodes.iter().any(|node| matches!(node.kind, NodeKind::Trigger { watch: Watch::Called | Watch::SlackMention { .. } }))
}

fn reject_call_cycle(origin: &str, origin_graph: &Graph, store: &Store) -> Result<(), String> {
    let mut stack = Vec::new();
    if !origin.is_empty() {
        stack.push(origin.to_owned());
    }
    walk_calls(origin_graph, &mut stack, origin, origin_graph, store)
}

fn graph_of(store: &Store, workflow: &str) -> Option<Graph> {
    let connector = store.connectors.iter().find(|connector| connector.id == workflow)?;
    Some(connector.graph.clone().unwrap_or_else(|| migrate(connector)))
}

fn walk_calls(graph: &Graph, stack: &mut Vec<String>, origin: &str, origin_graph: &Graph, store: &Store) -> Result<(), String> {
    for node in &graph.nodes {
        let NodeKind::Call { workflow } = &node.kind else { continue };
        if stack.iter().any(|seen| seen == workflow) {
            return Err(format!("\"{}\" calls \"{workflow}\", which calls back", node.name));
        }
        stack.push(workflow.clone());
        let result = if workflow == origin {
            walk_calls(origin_graph, stack, origin, origin_graph, store)
        } else if let Some(owned) = graph_of(store, workflow) {
            walk_calls(&owned, stack, origin, origin_graph, store)
        } else {
            Ok(())
        };
        stack.pop();
        result?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Paged {
    pub at: String,
    pub ping: bool,
}

/// Where a workflow's sessions run. Kept under the connector's original `machine` key: a machine
/// id string ("" = any), or `{"kind":"upstash-box",…}`. Old stores load as before, and an old UI
/// that never touches the select saves the object back untouched.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(from = "StoredRunsOn", into = "StoredRunsOn")]
pub enum RunsOn {
    /// Whichever of the account's machines is online.
    #[default]
    Any,
    /// This machine first, else whichever is online.
    Machine(MachineId),
    /// A new Upstash Box for each issue or thread, which the hub deletes once its session ends.
    FreshBox(BoxStep),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BoxStep {
    #[serde(default)]
    pub size: BoxSize,
}

/// Serialized names match Upstash's `size`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BoxSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum StoredRunsOn {
    Machine(String),
    Box {
        kind: UpstashBoxKind,
        #[serde(default)]
        size: BoxSize,
    },
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
enum UpstashBoxKind {
    #[serde(rename = "upstash-box")]
    UpstashBox,
}

impl From<StoredRunsOn> for RunsOn {
    fn from(stored: StoredRunsOn) -> Self {
        match stored {
            StoredRunsOn::Machine(id) if id.is_empty() => RunsOn::Any,
            StoredRunsOn::Machine(id) => RunsOn::Machine(id),
            StoredRunsOn::Box { size, .. } => RunsOn::FreshBox(BoxStep { size }),
        }
    }
}

impl From<RunsOn> for StoredRunsOn {
    fn from(runs_on: RunsOn) -> Self {
        match runs_on {
            RunsOn::Any => StoredRunsOn::Machine(String::new()),
            RunsOn::Machine(id) => StoredRunsOn::Machine(id),
            RunsOn::FreshBox(step) => StoredRunsOn::Box { kind: UpstashBoxKind::UpstashBox, size: step.size },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
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

    fn advance(&mut self, watermark: DateTime<Utc>, opening: bool) -> bool {
        let watched = match self.trigger {
            Trigger::SlackMention { .. } | Trigger::SlackMessage { .. } => true,
            Trigger::LinearLabel { .. } | Trigger::Called => false,
        };
        let behind = DateTime::parse_from_rfc3339(&self.since).map_or(true, |since| since < watermark);
        if !watched || !behind || opening {
            return false;
        }
        self.since = watermark.to_rfc3339();
        true
    }

    fn escalation_reply(&self, channel: &str, thread: &str, paging: Paging, now: DateTime<Utc>) -> Option<Reply> {
        if self.paged.contains_key(&thread_key(channel, thread)) {
            return None;
        }
        Some(match self.recent_page(channel, now) {
            Some(paged) => Reply::Already { handle: paging.handle, thread: paged },
            None => Reply::Ping(paging.ping),
        })
    }

    fn record_escalation(&mut self, channel: &str, thread: &str, reply: &Reply, now: DateTime<Utc>) {
        let ping = matches!(reply, Reply::Ping(_));
        self.paged.insert(thread_key(channel, thread), Paged { at: now.to_rfc3339(), ping });
    }

    fn recent_page(&self, channel: &str, now: DateTime<Utc>) -> Option<String> {
        let prefix = thread_key(channel, "");
        self.paged
            .iter()
            .filter(|(_, paged)| paged.ping)
            .filter_map(|(key, paged)| Some((key.strip_prefix(&prefix)?, DateTime::parse_from_rfc3339(&paged.at).ok()?)))
            .filter(|(_, at)| now.signed_duration_since(*at) < PAGE_COOLDOWN)
            .max_by_key(|(_, at)| *at)
            .map(|(thread, _)| thread.to_owned())
    }

    /// A box has only the account its Claude token signs in.
    fn account(&self) -> Option<String> {
        Some(self.account.trim().to_owned()).filter(|a| !a.is_empty() && !matches!(self.runs_on, RunsOn::FreshBox(_)))
    }

    fn unclaim(&mut self, subject: &Subject) {
        let key = match subject {
            Subject::LinearIssue { issue } => issue.clone(),
            Subject::SlackThread { channel, thread } => thread_key(channel, thread),
        };
        self.handled.retain(|k| *k != key);
        self.followed.remove(&key);
    }

    fn box_note(&self) -> &'static str {
        match self.runs_on {
            RunsOn::FreshBox(_) => BOX_NOTE,
            RunsOn::Any | RunsOn::Machine(_) => "",
        }
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
#[derive(Serialize, Deserialize, Clone, Default, schemars::JsonSchema)]
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

#[derive(Serialize, Deserialize, Default, schemars::JsonSchema)]
struct Store {
    linear: Option<linear::Account>,
    #[serde(default)]
    slack: Option<slack::Account>,
    #[serde(default)]
    github: Option<github::Account>,
    #[serde(default)]
    vercel: Option<vercel::Account>,
    #[serde(default)]
    upstash: Option<upstash::Account>,
    connectors: Vec<Connector>,
}

impl Store {
    fn connector(&self, id: &str) -> Result<&Connector, String> {
        self.connectors.iter().find(|c| c.id == id).ok_or_else(|| "the workflow was deleted".into())
    }

    fn handled(&self, key: &str) -> bool {
        self.connectors.iter().any(|c| c.handled.iter().any(|h| h == key))
    }

    fn claim_thread(&mut self, connector: &str, key: &str) -> Option<&mut Connector> {
        if self.handled(key) {
            return None;
        }
        let c = self.connectors.iter_mut().find(|c| c.id == connector)?;
        c.handled.push(key.to_owned());
        Some(c)
    }

    fn check_route(&self, router: &str, category: &str, workflow: &str) -> Result<(), String> {
        let Some(target) = self.connectors.iter().find(|c| c.id == workflow) else {
            return Err(format!(
                "workflow {router:?} hands {category:?} messages to a workflow that was deleted: pick another one for the category in its settings"
            ));
        };
        if !target.enabled {
            return Err(format!(
                "workflow {router:?} hands {category:?} messages to workflow {:?}, which is off: turn it on, or pick another one for the category",
                target.name
            ));
        }
        if !target.trigger.answers_threads() {
            return Err(format!(
                "workflow {router:?} hands {category:?} messages to workflow {:?}, which does not answer Slack threads: pick a Slack mention or called workflow for the category",
                target.name
            ));
        }
        Ok(())
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

    fn page(&mut self, router: &str, channel: String, thread: String, paging: Option<Paging>, now: DateTime<Utc>, react: bool) -> Option<Page> {
        let c = self.connectors.iter_mut().find(|c| c.id == router)?;
        let reply = c.escalation_reply(&channel, &thread, paging?, now)?;
        c.record_escalation(&channel, &thread, &reply, now);
        Some(Page { channel, thread, reply, react })
    }

    fn settle(&mut self, checked: &[String], found: Vec<(String, Found)>, now: DateTime<Utc>, exe: &str, runs: &[runner::Run]) -> Settled {
        let mut settled = Settled::default();
        let watermark = now - SLACK_OVERLAP;
        for connector in self.connectors.iter_mut().filter(|c| checked.contains(&c.id)) {
            let opening = found.iter().any(|(id, f)| *id == connector.id && f.opens_session());
            settled.needs_save |= connector.advance(watermark, opening);
        }
        for (id, found) in found {
            if let Found::Issue(issue) = &found {
                if self.working_on(&issue.id).is_some_and(|(owner, _)| owner.id != id) {
                    continue;
                }
            }
            let Some(connector) = self.connectors.iter_mut().find(|c| c.id == id && c.enabled) else { continue };
            let key = found.key();
            let subject = found.subject();
            // An edited graph walks the runner. The stored graph is None when it still equals migrate.
            if connector.graph.is_some() && matches!(found, Found::Issue(_) | Found::Mention { .. }) {
                if subject_taken(runs, &key) || self.handled(&key) || settled.starts.iter().any(|start| subject_key(&start.subject) == key) {
                    continue;
                }
                if let Some(input) = found.trigger(now) {
                    settled.starts.push(GraphStart { connector: id, subject, input });
                }
                continue;
            }
            match found {
                Found::Issue(issue) => {
                    if connector.claim(key) {
                        settled.work.push(Work::Run(run(connector, &issue, exe)));
                        settled.needs_save = true;
                    }
                }
                Found::Mention { channel, channel_name, mention, posts } => {
                    let Some(connector) = self.claim_thread(&id, &key) else { continue };
                    let cursor = posts.last().map_or_else(|| mention.thread.clone(), |p| p.ts.clone());
                    connector.followed.insert(key, Followed { cursor, seen: None, replied: false });
                    let note = mention_note(&mention, now);
                    let run = slack_run(connector, &channel, &channel_name, &mention.thread, &posts, note, exe);
                    settled.work.push(Work::Run(Run { mentioned: vec![mention.ts], ..run }));
                    settled.needs_save = true;
                }
                Found::Stopped { channel, thread, page } => {
                    if self.claim_thread(&id, &key).is_some() {
                        settled.pages.extend(self.page(&id, channel, thread, page, now, true));
                        settled.needs_save = true;
                    }
                }
                Found::Routed { channel, channel_name, thread, category, workflow, note, posts, page } => {
                    let router = connector.name.clone();
                    if let Err(e) = self.check_route(&router, &category, &workflow) {
                        if self.claim_thread(&id, &key).is_some() {
                            settled.pages.extend(self.page(&id, channel, thread, page, now, true));
                            settled.errors.push(e);
                            settled.needs_save = true;
                        }
                        continue;
                    }
                    if self.connectors.iter().any(|c| c.id == workflow && c.graph.is_some()) {
                        let subject = Subject::SlackThread { channel: channel.clone(), thread: thread.clone() };
                        let taken = subject_taken(runs, &key) || self.handled(&key) || settled.starts.iter().any(|start| subject_key(&start.subject) == key);
                        if !taken {
                            settled.starts.push(GraphStart {
                                connector: workflow.clone(),
                                subject,
                                input: TriggerInput::Slack {
                                    channel: channel.clone(),
                                    channel_name: channel_name.clone(),
                                    thread: thread.clone(),
                                    posts: trigger_posts(&posts),
                                    note: Some(note),
                                    mentioned: vec![thread.clone()],
                                },
                            });
                        }
                        settled.pages.extend(self.page(&id, channel, thread, page, now, false));
                        settled.needs_save = true;
                        continue;
                    }
                    let Some(target) = self.claim_thread(&workflow, &key) else { continue };
                    let cursor = posts.last().map_or_else(|| thread.clone(), |p| p.ts.clone());
                    target.followed.insert(key, Followed { cursor, seen: None, replied: false });
                    let run = slack_run(target, &channel, &channel_name, &thread, &posts, Some(&note), exe);
                    settled.work.push(Work::Run(Run { mentioned: vec![thread.clone()], ..run }));
                    settled.pages.extend(self.page(&id, channel, thread, page, now, false));
                    settled.needs_save = true;
                }
                Found::Replies { replies, .. } if replies.posts.is_empty() => {
                    if let Some(followed) = connector.followed.get_mut(&key) {
                        followed.seen = Some(replies.newest);
                        settled.needs_save = true;
                    }
                }
                Found::Replies { channel, replies } => {
                    let subject = Subject::SlackThread { channel: channel.clone(), thread: replies.thread.clone() };
                    if run_owns(runs, &subject) {
                        settled.inbox.push(GraphInbox {
                            channel,
                            thread: replies.thread,
                            messages: replies.posts.iter().map(reply_block).collect(),
                            cursor: replies.newest,
                            mentioned: replies.mentioned,
                        });
                        continue;
                    }
                    settled.work.push(Work::Delivery(Delivery {
                        connector: id,
                        channel,
                        thread: replies.thread,
                        messages: replies.posts.iter().map(reply_block).collect(),
                        cursor: replies.newest,
                        mentioned: replies.mentioned,
                    }))
                }
            }
        }
        settled
    }
}

/// A subject whose edited graph starts a run instead of today's session.
struct GraphStart {
    connector: String,
    subject: Subject,
    input: TriggerInput,
}

/// Replies for a graph run's thread, typed into its session rather than today's delivery.
struct GraphInbox {
    channel: String,
    thread: String,
    messages: Vec<String>,
    cursor: String,
    mentioned: Vec<String>,
}

enum InboxDone {
    Sent,
    Later,
}

#[derive(Default)]
struct Settled {
    work: Vec<Work>,
    pages: Vec<Page>,
    starts: Vec<GraphStart>,
    inbox: Vec<GraphInbox>,
    needs_save: bool,
    errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, schemars::JsonSchema)]
pub struct Check {
    pub at: String,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, schemars::JsonSchema)]
pub struct Settings {
    pub linear: Option<linear::Account>,
    pub slack: Option<slack::Account>,
    #[serde(default)]
    pub github: Option<github::Account>,
    #[serde(default)]
    pub vercel: Option<vercel::Account>,
    #[serde(default)]
    pub upstash: Option<upstash::Account>,
    pub connectors: Vec<Connector>,
    pub checked: Option<Check>,
}

/// The integrations' keys, which never leave a store but by `export`.
#[derive(Default, Debug, PartialEq)]
pub struct Keys {
    pub linear: Option<String>,
    pub slack: Option<String>,
    pub github: Option<String>,
    pub vercel: Option<String>,
}

/// A session a connector wants opened.
#[derive(Clone)]
pub struct Run {
    pub cwd: PathBuf,
    pub repo: Option<String>,
    pub args: Vec<String>,
    pub connector: String,
    pub subject: Subject,
    /// The Slack messages the bot reacts to once a tab takes the run, by ts.
    pub mentioned: Vec<String>,
    /// The account the connector names, or none for its project's.
    pub account: Option<String>,
    pub runs_on: RunsOn,
    /// The subject text an Ask step reads: the issue, or the Slack thread so far.
    pub input: String,
}


impl Run {
    pub fn spec(&self) -> RunSpec {
        RunSpec {
            folder: self.cwd.display().to_string(),
            repo: self.repo.clone(),
            args: self.args.clone(),
            env: agent::session_env(&self.connector, &self.subject),
            token: None,
            account: self.account.clone(),
        }
    }
}

/// What a check hands to the tabs.
pub enum Work {
    /// A session to open.
    Run(Run),
    /// New messages for the session that answers a Slack thread.
    Delivery(Delivery),
    /// Types `text` into the live session `env` names. Linear and Slack share this path.
    Prompt { env: Vec<(String, String)>, text: String },
}

/// Where a machine put a session. The desktop app reports machine `local` and the tab it opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub machine: String,
    pub tab: String,
}

/// What `open` answered. A hub returns the tab it used. The desktop app returns `()`.
pub trait Taken {
    fn taken(self) -> Result<Option<Placement>, String>;
}

impl Taken for Result<(), String> {
    fn taken(self) -> Result<Option<Placement>, String> {
        self.map(|()| None)
    }
}

impl Taken for Result<Option<Placement>, String> {
    fn taken(self) -> Result<Option<Placement>, String> {
        self
    }
}

/// Runs `work` off the async worker. Resume detection sleeps, and a desktop watcher must not block on it.
pub async fn off_worker<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) => Err(format!("workflow work could not run off the async worker: {error}")),
    }
}

/// The trigger text a graph run keeps, so a later `talk` can rebuild today's session prompt.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TriggerInput {
    Issue {
        id: String,
        identifier: String,
        title: String,
        description: Option<String>,
        url: String,
    },
    Slack {
        channel: String,
        #[serde(rename = "channelName")]
        channel_name: String,
        thread: String,
        posts: Vec<TriggerPost>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default)]
        mentioned: Vec<String>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TriggerPost {
    pub ts: String,
    pub author: String,
    pub text: String,
}

/// Direct answers from one classify model, in question order. A decide's pick is not one of these.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassifyAnswers {
    pub model: String,
    pub answers: Vec<(String, String)>,
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
    /// The messages that mention the bot, which it reacts to once a session has them, by ts.
    pub mentioned: Vec<String>,
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
    Stopped { channel: String, thread: String, page: Option<Paging> },
    Routed {
        channel: String,
        channel_name: String,
        thread: String,
        category: String,
        workflow: String,
        note: String,
        posts: Vec<slack::Post>,
        page: Option<Paging>,
    },
}

impl Found {
    /// What `Connector::handled` and `Connector::followed` record it under.
    fn key(&self) -> String {
        match self {
            Found::Issue(issue) => issue.id.clone(),
            Found::Mention { channel, mention, .. } => thread_key(channel, &mention.thread),
            Found::Replies { channel, replies } => thread_key(channel, &replies.thread),
            Found::Stopped { channel, thread, .. } | Found::Routed { channel, thread, .. } => thread_key(channel, thread),
        }
    }

    fn opens_session(&self) -> bool {
        match self {
            Found::Issue(_) | Found::Mention { .. } | Found::Routed { .. } => true,
            Found::Replies { .. } | Found::Stopped { .. } => false,
        }
    }

    fn subject(&self) -> Subject {
        match self {
            Found::Issue(issue) => Subject::LinearIssue { issue: issue.id.clone() },
            Found::Mention { channel, mention, .. } => Subject::SlackThread { channel: channel.clone(), thread: mention.thread.clone() },
            Found::Replies { channel, replies } => Subject::SlackThread { channel: channel.clone(), thread: replies.thread.clone() },
            Found::Stopped { channel, thread, .. } | Found::Routed { channel, thread, .. } => Subject::SlackThread { channel: channel.clone(), thread: thread.clone() },
        }
    }

    /// The trigger a graph run stores. Stopped and routed finds carry a note of their own.
    fn trigger(&self, now: DateTime<Utc>) -> Option<TriggerInput> {
        match self {
            Found::Issue(issue) => Some(TriggerInput::Issue {
                id: issue.id.clone(),
                identifier: issue.identifier.clone(),
                title: issue.title.clone(),
                description: issue.description.clone(),
                url: issue.url.clone(),
            }),
            Found::Mention { channel, channel_name, mention, posts } => Some(TriggerInput::Slack {
                channel: channel.clone(),
                channel_name: channel_name.clone(),
                thread: mention.thread.clone(),
                posts: trigger_posts(posts),
                note: mention_note(mention, now).map(str::to_owned),
                mentioned: vec![mention.ts.clone()],
            }),
            Found::Replies { .. } | Found::Stopped { .. } | Found::Routed { .. } => None,
        }
    }
}

fn thread_key(channel: &str, thread: &str) -> String {
    format!("{channel}:{thread}")
}

#[derive(Clone, Debug, PartialEq)]
struct Paging {
    ping: String,
    handle: String,
}

#[derive(Debug, PartialEq)]
struct Page {
    channel: String,
    thread: String,
    reply: Reply,
    react: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Reply {
    Ping(String),
    Already { handle: String, thread: String },
}

fn page_text(group: &str, category: &Category) -> String {
    format!("{} {}: {}", slack::group_mention(group), category.name, category.description.trim())
}

/// What an escalation says when the group was paged for another thread a moment ago. It names the
/// group in plain text, which notifies no one.
fn already_text(handle: &str, permalink: &str) -> String {
    format!("Already escalated to @{}: {permalink}", handle.trim_start_matches('@'))
}

fn cursors(followed: &BTreeMap<String, Followed>, channel: &str) -> BTreeMap<String, slack::Cursor> {
    let prefix = thread_key(channel, "");
    followed
        .iter()
        .filter_map(|(key, f)| {
            let read = f.seen.clone().unwrap_or_else(|| f.cursor.clone());
            Some((key.strip_prefix(&prefix)?.to_owned(), slack::Cursor { given: f.cursor.clone(), read }))
        })
        .collect()
}

fn followed_channels(followed: &BTreeMap<String, Followed>, own: Option<&str>) -> Vec<String> {
    let mut channels: Vec<String> =
        followed.keys().filter_map(|key| key.split_once(':')).map(|(channel, _)| channel).filter(|c| Some(*c) != own).map(str::to_owned).collect();
    channels.dedup();
    channels
}

/// A Slack connector's channel as the user typed it, without the `#`.
fn channel_name(name: &str) -> &str {
    name.trim().trim_start_matches('#')
}

#[derive(Serialize, schemars::JsonSchema)]
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
    /// Graph runs, kept beside the store and never sent to apps.
    fn load_runs(&self) -> Option<Vec<u8>>;
    fn save_runs(&self, bytes: &[u8]) -> Result<(), String>;
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
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        crate::private::file()
            .create(true)
            .truncate(true)
            .open(self.dir.join(name))
            .and_then(|mut f| f.write_all(key.as_bytes()))
            .map_err(|e| e.to_string())
    }

    fn remove_key(&self, name: &str) {
        let _ = std::fs::remove_file(self.dir.join(name));
    }

    fn load_runs(&self) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join("runs.json")).ok()
    }

    fn save_runs(&self, bytes: &[u8]) -> Result<(), String> {
        replace_file(&self.dir, "runs.json", bytes)
    }
}

/// Writes `bytes` to `dir/name` by renaming a temp file over it, so a crash mid-write leaves the previous file.
fn replace_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    crate::private::dir(dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    let tmp = dir.join(format!(".{name}.tmp"));
    {
        use std::io::Write;
        let mut file = crate::private::file().create(true).truncate(true).open(&tmp).map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&path);
        if let Err(e) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
    }
    Ok(())
}

fn read_runs(bytes: Option<Vec<u8>>) -> (Vec<runner::Run>, Option<String>) {
    let Some(bytes) = bytes else { return (Vec::new(), None) };
    match serde_json::from_slice(&bytes) {
        Ok(runs) => (runs, None),
        Err(e) => (Vec::new(), Some(format!("could not read workflow runs: {e}"))),
    }
}

/// Unix millis now. A machine's tab list is stamped with this when it arrives.
pub fn unix_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn merge_error(error: Option<String>, extra: Option<String>) -> Option<String> {
    match (error, extra) {
        (Some(error), Some(extra)) if !error.contains(&extra) => Some(format!("{error}; {extra}")),
        (Some(error), _) => Some(error),
        (None, extra) => extra,
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
    /// Graph runs. A restart reads them back and `drive` continues each one.
    runs: Mutex<Vec<runner::Run>>,
    /// Slack replies waiting to be typed into a graph run's session.
    inbox: Mutex<Vec<GraphInbox>>,
    /// One drive at a time, so a tab wake cannot perform the effect a check is already performing.
    driving: tokio::sync::Mutex<()>,
    /// `runs.json` did not parse. Saves stay off so a later write does not replace the file with nothing.
    runs_broken: Mutex<Option<String>>,
    /// The latest tab list each machine has sent, with the unix millis it was observed.
    known_tabs: Mutex<HashMap<String, (u64, Vec<TabView>)>>,
}

impl Connectors {
    pub fn new(storage: impl Storage + 'static, exe: String) -> Self {
        let store = storage.load().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let (runs, runs_broken) = read_runs(storage.load_runs());
        let checked = runs_broken.as_ref().map(|why| Check { at: Utc::now().to_rfc3339(), error: Some(why.clone()) });
        Self {
            storage: Box::new(storage),
            exe,
            store: Mutex::new(store),
            checked: Mutex::new(checked),
            wake: Notify::new(),
            channels: Mutex::new(HashMap::new()),
            filing: tokio::sync::Mutex::new(()),
            runs: Mutex::new(runs),
            inbox: Mutex::new(vec![]),
            driving: tokio::sync::Mutex::new(()),
            runs_broken: Mutex::new(runs_broken),
            known_tabs: Mutex::new(HashMap::new()),
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

        let settled = {
            let runs = self.runs.lock().unwrap().clone();
            let _filing = self.filing.lock().await;
            let mut store = self.store.lock().unwrap();
            let settled = store.settle(&checked, found, now, &self.exe, &runs);
            if settled.needs_save {
                if let Err(e) = self.save(&store) {
                    errors.push(e);
                }
            }
            settled
        };
        for start in settled.starts {
            if let Err(e) = self.begin(start.connector, start.subject, start.input, &now.to_rfc3339()) {
                errors.push(e);
            }
        }
        for item in settled.inbox {
            self.enqueue(item);
        }
        errors.extend(settled.errors);
        for page in &settled.pages {
            if let Err(e) = self.page(page).await {
                errors.push(e);
            }
        }
        (settled.work, (!errors.is_empty()).then(|| errors.join("; ")))
    }

    async fn page(&self, page: &Page) -> Result<(), String> {
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected: connect it in pinned's Connectors settings")?;
        let text = match &page.reply {
            Reply::Ping(text) => text.clone(),
            Reply::Already { handle, thread } => already_text(handle, &slack::permalink(&token, &page.channel, thread).await?),
        };
        slack::post(&token, &page.channel, &page.thread, &text).await?;
        if page.react {
            slack::react(&token, &page.channel, &page.thread, slack::WORKING).await?;
        }
        Ok(())
    }

    /// Gives `work` to the tabs. A thread's cursor moves only once its messages reached a session,
    /// its own or a new one, so messages that didn't are passed on again at the next check. A run
    /// no tab took gives its subject back, so the next check tries it again.
    async fn hand_over<F, Fut, R>(&self, work: Work, open: &F) -> Result<(), String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        match work {
            Work::Run(mut run) => {
                let (connector, subject, folder, mentioned) = (run.connector.clone(), run.subject.clone(), run.cwd.clone(), run.mentioned.clone());
                match self.consult(&connector, &run.input).await {
                    Ok(None) => {}
                    Ok(Some((ask, answers))) => {
                        if stops(&ask, &answers).is_some() {
                            self.keep_stopped(&connector, &subject)?;
                            return Ok(());
                        }
                        append_answers(&mut run, &ask, &answers);
                    }
                    Err(why) => return Err(self.unclaim(&connector, &subject, &folder, &why)),
                }
                if let Err(why) = open(Work::Run(run)).await.taken() {
                    return Err(self.unclaim(&connector, &subject, &folder, &why));
                }
                // The session starts even when Linear won't move the issue or Slack won't take the
                // reaction; the check reports why. A Slack thread has no issue to pick until its
                // session files one.
                match &subject {
                    Subject::LinearIssue { .. } => self.stage(&connector, &subject, Stage::Picked).await.map(drop),
                    Subject::SlackThread { channel, .. } => self.acknowledge(channel, &mentioned).await,
                }
            }
            Work::Delivery(d) => {
                let cursor = if open(Work::Delivery(d.clone())).await.taken().is_ok() {
                    d.cursor.clone()
                } else {
                    let (run, newest) = self.fallback(&d).await?;
                    let folder = run.cwd.clone();
                    if let Err(why) = open(Work::Run(run)).await.taken() {
                        return Err(format!("could not open a session in {} for a Slack thread: {why}; the next check tries again", folder.display()));
                    }
                    newest
                };
                self.follow(&d.connector, &d.channel, &d.thread, cursor)?;
                self.acknowledge(&d.channel, &d.mentioned).await
            }
            Work::Prompt { env, text } => open(Work::Prompt { env, text }).await.taken().map(drop),
        }
    }

    /// Reacts to the messages a session took, so people can tell the bot didn't ignore them.
    /// Tries every message even when one fails, and answers the first failure.
    async fn acknowledge(&self, channel: &str, mentioned: &[String]) -> Result<(), String> {
        if mentioned.is_empty() {
            return Ok(());
        }
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected: connect it in pinned's Connectors settings")?;
        let mut result = Ok(());
        for ts in mentioned {
            let reacted = slack::react(&token, channel, ts, slack::WORKING).await;
            result = result.and(reacted);
        }
        result
    }

    /// The connector's Ask step and its answers, or None when it has no such step.
    async fn consult(&self, connector: &str, input: &str) -> Result<Option<(Ask, Vec<(String, String)>)>, String> {
        let ask = self.store.lock().unwrap().connectors.iter().find(|c| c.id == connector).and_then(|c| c.ask.clone());
        let Some(ask) = ask else { return Ok(None) };
        let key = self.key(VERCEL_KEY).ok_or_else(|| "Vercel AI Gateway is not connected: connect it in pinned's Connectors settings".to_owned())?;
        let answers = vercel::ask(&key, &ask, input).await?;
        Ok(Some((ask, answers)))
    }

    /// Leaves the subject handled, and drops a Slack thread's cursor so later replies are not delivered.
    fn keep_stopped(&self, connector: &str, subject: &Subject) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        if let Some(c) = store.connectors.iter_mut().find(|c| c.id == connector) {
            if let Subject::SlackThread { channel, thread } = subject {
                c.followed.remove(&thread_key(channel, thread));
            }
        }
        self.save(&store)
    }

    fn unclaim(&self, connector: &str, subject: &Subject, folder: &Path, why: &str) -> String {
        let error = format!("could not open a session in {}: {why}; the next check tries again", folder.display());
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
        let watched = match &c.trigger {
            Trigger::SlackMention { channel } => Some(channel.as_str()),
            Trigger::Called => None,
            Trigger::LinearLabel { .. } | Trigger::SlackMessage { .. } => return Err("the workflow no longer answers Slack threads".into()),
        };
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected")?;
        let own = watched.filter(|name| self.channels.lock().unwrap().get(*name) == Some(&d.channel));
        let name = match own {
            Some(name) => channel_name(name).to_owned(),
            None => slack::channel_name(&token, &d.channel).await?,
        };
        let posts = slack::thread(&token, &d.channel, &d.thread).await?;
        let newest = posts.last().map_or_else(|| d.cursor.clone(), |p| p.ts.clone());
        let note = c.created.get(&thread_key(&d.channel, &d.thread)).map(filed_note);
        let mut run = slack_run(&c, &d.channel, &name, &d.thread, &posts, note.as_deref(), &self.exe);
        run.mentioned = d.mentioned.clone();
        // The thread already passed the step, so a stop answer does not end this new session.
        if let Some((ask, answers)) = self.consult(&run.connector, &run.input).await? {
            append_answers(&mut run, &ask, &answers);
        }
        Ok((run, newest))
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
                let activity = slack::activity(&token, &channel, &bot, since, &cursors(&c.followed, &channel)).await?;
                let mut found: Vec<Found> = vec![];
                // Oldest first, so a thread's first new mention is the one its session answers.
                for mention in activity.mentions {
                    let key = thread_key(&channel, &mention.thread);
                    if self.busy(&key) || found.iter().any(|f| f.key() == key) {
                        continue;
                    }
                    let posts = slack::thread(&token, &channel, &mention.thread).await?;
                    let channel_name = channel_name(name).to_owned();
                    found.push(Found::Mention { channel: channel.clone(), channel_name, mention, posts });
                }
                found.extend(activity.replies.into_iter().map(|replies| Found::Replies { channel: channel.clone(), replies }));
                found.extend(self.follow_ups(&token, &bot, c, Some(&channel), now).await?);
                Ok(Some(found))
            }
            Trigger::SlackMessage { channel: name, .. } if c.graph.is_some() => self.scan_graph_channel(c, name, now).await,
            Trigger::SlackMessage { channel: name, decide } => {
                let bot = self.store.lock().unwrap().slack.as_ref().map(|a| a.user_id.clone());
                let (Some(token), Some(bot)) = (self.key(SLACK_KEY), bot) else { return Ok(None) };
                let channel = self.channel_id(&token, name).await?;
                let since = DateTime::parse_from_rfc3339(&c.since).map_or(now, Into::into);
                let mut messages = slack::messages(&token, &channel, &bot, since).await?;
                messages.retain(|m| !self.busy(&thread_key(&channel, &m.ts)));
                if messages.is_empty() {
                    return Ok(Some(vec![]));
                }
                let key = self.key(VERCEL_KEY).ok_or("the Vercel AI Gateway is not connected: connect it in pinned's Connectors settings")?;
                let choices = decide.choices(&c.name)?;
                let pages = self.pages(&token, c, decide).await?;
                let channel_name = channel_name(name);
                let mut found = vec![];
                for message in messages {
                    let state = json!({ "channel": format!("#{channel_name}"), "author": message.author, "message": message.text });
                    let chosen = vercel::choose(&key, decide.model.trim(), &decide.instructions, &choices, &state).await?;
                    let pick = decide.pick(&chosen);
                    let Some(chosen) = pick.category() else {
                        found.push(Found::Stopped { channel: channel.clone(), thread: message.ts, page: None });
                        continue;
                    };
                    let (category, page) = (&decide.categories[chosen], pages[chosen].clone());
                    found.push(match &category.workflow {
                        None => Found::Stopped { channel: channel.clone(), thread: message.ts, page },
                        Some(workflow) => Found::Routed {
                            posts: slack::thread(&token, &channel, &message.ts).await?,
                            channel: channel.clone(),
                            channel_name: channel_name.to_owned(),
                            thread: message.ts,
                            category: category.name.clone(),
                            workflow: workflow.clone(),
                            note: pick.note(&decide.categories, &c.name),
                            page,
                        },
                    });
                }
                Ok(Some(found))
            }
            Trigger::Called => {
                let bot = self.store.lock().unwrap().slack.as_ref().map(|a| a.user_id.clone());
                let (Some(token), Some(bot)) = (self.key(SLACK_KEY), bot) else { return Ok(None) };
                Ok(Some(self.follow_ups(&token, &bot, c, None, now).await?))
            }
        }
    }

    /// A graph watches the channel's new threads and lets the runner classify them.
    async fn scan_graph_channel(&self, c: &Connector, name: &str, now: DateTime<Utc>) -> Result<Option<Vec<Found>>, String> {
        let bot = self.store.lock().unwrap().slack.as_ref().map(|a| a.user_id.clone());
        let (Some(token), Some(bot)) = (self.key(SLACK_KEY), bot) else { return Ok(None) };
        let channel = self.channel_id(&token, name).await?;
        let since = DateTime::parse_from_rfc3339(&c.since).map_or(now, Into::into);
        let mut messages = slack::messages(&token, &channel, &bot, since).await?;
        messages.retain(|m| !self.busy(&thread_key(&channel, &m.ts)));
        let channel_name = channel_name(name).to_owned();
        let mut found = vec![];
        for message in messages {
            let posts = slack::thread(&token, &channel, &message.ts).await?;
            found.push(Found::Mention {
                channel: channel.clone(),
                channel_name: channel_name.clone(),
                mention: slack::Mention { thread: message.ts.clone(), ts: message.ts, at: now },
                posts,
            });
        }
        let activity = slack::activity(&token, &channel, &bot, now, &cursors(&c.followed, &channel)).await?;
        found.extend(activity.replies.into_iter().map(|replies| Found::Replies { channel: channel.clone(), replies }));
        found.extend(self.follow_ups(&token, &bot, c, Some(&channel), now).await?);
        Ok(Some(found))
    }

    /// Handled by any connector, or already inside a run that has not failed.
    fn busy(&self, key: &str) -> bool {
        if self.store.lock().unwrap().handled(key) {
            return true;
        }
        self.runs.lock().unwrap().iter().any(|run| !matches!(run.status, Status::Failed { .. } | Status::Done) && subject_key(&run.subject) == key)
    }

    async fn follow_ups(&self, token: &str, bot: &str, c: &Connector, own: Option<&str>, now: DateTime<Utc>) -> Result<Vec<Found>, String> {
        let mut found = vec![];
        for channel in followed_channels(&c.followed, own) {
            let activity = slack::activity(token, &channel, bot, now, &cursors(&c.followed, &channel)).await?;
            found.extend(activity.replies.into_iter().map(|replies| Found::Replies { channel: channel.clone(), replies }));
        }
        Ok(found)
    }

    async fn pages(&self, token: &str, c: &Connector, decide: &Decide) -> Result<Vec<Option<Paging>>, String> {
        let mut groups: HashMap<String, String> = HashMap::new();
        let mut pages = vec![];
        for category in &decide.categories {
            let Some(group) = category.group() else {
                pages.push(None);
                continue;
            };
            let id = match groups.get(group) {
                Some(id) => id.clone(),
                None => {
                    let id = slack::group_id(token, group)
                        .await
                        .map_err(|e| format!("workflow {:?} could not page category {:?}'s group: {e}", c.name, category.name))?;
                    groups.insert(group.to_owned(), id.clone());
                    id
                }
            };
            pages.push(Some(Paging { ping: page_text(&id, category), handle: group.to_owned() }));
        }
        Ok(pages)
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
    prompt += c.box_note();
    let input = format!(
        "{}: {}\n{}\n\n{}",
        issue.identifier,
        issue.title,
        issue.url,
        issue.description.as_deref().unwrap_or("(no description)"),
    );
    prompt += &format!("\n\nLinear issue {input}");
    args.push(prompt);
    Run { cwd: c.folder.clone(), repo: c.repo(), args, connector: c.id.clone(), subject: Subject::LinearIssue { issue: issue.id.clone() }, mentioned: vec![], account: c.account(), runs_on: c.runs_on.clone(), input }
}

/// The first question whose answer is one of its `stop_on` values, ignoring case.
fn stops(ask: &Ask, answers: &[(String, String)]) -> Option<(String, String)> {
    ask.questions.iter().find_map(|question| {
        let (_, answer) = answers.iter().find(|(name, _)| name == &question.name)?;
        question.stop_on.iter().any(|stop| stop.eq_ignore_ascii_case(answer)).then(|| (question.name.clone(), answer.clone()))
    })
}

fn read_as(subject: &Subject) -> &'static str {
    match subject {
        Subject::LinearIssue { .. } => "issue",
        Subject::SlackThread { .. } => "thread",
    }
}

fn answered(model: &str, kind: &str, answers: &[(String, String)]) -> String {
    let lines = answers.iter().map(|(name, answer)| format!("- {name}: {answer}")).collect::<Vec<_>>().join("\n");
    format!("\n\nBefore this session, {model} on Vercel's AI Gateway read the {kind} and answered:\n{lines}")
}

fn append_answers(run: &mut Run, ask: &Ask, answers: &[(String, String)]) {
    if let Some(prompt) = run.args.last_mut() {
        prompt.push_str(&answered(&ask.model, read_as(&run.subject), answers));
    }
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

/// Ends the instructions of a session that runs in a box, which nothing outlives but what it pushed.
const BOX_NOTE: &str = "\n\nYou run in a disposable Upstash Box (Linux arm64) that pinned deletes 10 minutes after you go quiet, \
    or 2 minutes after you stop once you reported the review stage. \
    Push your branch before you finish, because anything not pushed is lost. \
    For proof videos, launch Playwright with `executablePath: '/usr/bin/chromium'` instead of installing a browser, and never attach over CDP to port 9222. \
    If `command -v ffmpeg` finds nothing, check the recording with Playwright screenshots taken at the same steps instead of extracting frames.";

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
        "\n\nAnswer in the Slack thread below by running `'{}' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. Keep every answer short. Before each `slack reply`, invoke the `pinned:bro` skill on your draft and post the message it gives you, not the draft. \
         Later messages in the thread reach you only when one of them mentions you, as `{}` blocks with every message written since the last one you got; answer them in the thread the same way.",
        exe,
        reply_header("<name>")
    );
    prompt += &fix_rule(c, exe);
    if c.proof && c.fix != Fix::Off {
        prompt += &proof_of_work(exe);
        prompt += SLACK_PROOF;
    }
    prompt += c.box_note();
    let input = transcript(posts);
    prompt += &format!("\n\nSlack thread in #{channel_name}, oldest first:\n{input}");
    prompt += note.unwrap_or_default();
    args.push(prompt);
    let subject = Subject::SlackThread { channel: channel.to_owned(), thread: thread.to_owned() };
    Run { cwd: c.folder.clone(), repo: c.repo(), args, connector: c.id.clone(), subject, mentioned: vec![], account: c.account(), runs_on: c.runs_on.clone(), input }
}

fn transcript(posts: &[slack::Post]) -> String {
    posts.iter().map(|p| format!("{}: {}", p.author, p.text)).collect::<Vec<_>>().join("\n")
}

fn reply_header(author: &str) -> String {
    format!("[Slack reply from {author} in the thread]")
}

/// A later message of the thread, as its session receives it.
fn reply_block(post: &slack::Post) -> String {
    format!("{}\n{}", reply_header(&post.author), post.text)
}

fn routed_note(router: &str, category: &str) -> String {
    format!("\n\nWorkflow {router:?} handed you this thread: its decision model put the first message in the category {category:?}.")
}

/// The note a thread's new session gets when an earlier session of it filed an issue.
fn filed_note(filed: &Filed) -> String {
    format!(
        "\n\nThis thread already has Linear issue {} ({}). Do not file another one: `pinned linear create` returns that issue.",
        filed.identifier, filed.url
    )
}

fn trigger_posts(posts: &[slack::Post]) -> Vec<TriggerPost> {
    posts.iter().map(|post| TriggerPost { ts: post.ts.clone(), author: post.author.clone(), text: post.text.clone() }).collect()
}

fn subject_key(subject: &Subject) -> String {
    match subject {
        Subject::LinearIssue { issue } => issue.clone(),
        Subject::SlackThread { channel, thread } => thread_key(channel, thread),
    }
}

fn run_open(runs: &[runner::Run], connector: &str, subject: &Subject) -> bool {
    runs.iter().any(|run| run.connector == connector && &run.subject == subject && matches!(run.status, Status::Running | Status::Waiting))
}

/// A live run already has this subject, on any connector.
fn subject_taken(runs: &[runner::Run], key: &str) -> bool {
    runs.iter().any(|run| subject_key(&run.subject) == key && matches!(run.status, Status::Running | Status::Waiting))
}

/// A reply belongs to a graph run until that run has failed.
fn run_owns(runs: &[runner::Run], subject: &Subject) -> bool {
    runs.iter().any(|run| &run.subject == subject && !matches!(run.status, Status::Failed { .. }))
}

fn trigger_of(run: &runner::Run) -> Result<TriggerInput, String> {
    serde_json::from_value(run.context.get("trigger").cloned().unwrap_or(Value::Null)).map_err(|e| format!("the run has no trigger input: {e}"))
}

fn trigger_text(input: &TriggerInput) -> String {
    match input {
        TriggerInput::Issue { identifier, title, url, description, .. } => {
            format!("{identifier}: {title}\n{url}\n\n{}", description.as_deref().unwrap_or("(no description)"))
        }
        TriggerInput::Slack { posts, .. } => posts.iter().map(|post| format!("{}: {}", post.author, post.text)).collect::<Vec<_>>().join("\n"),
    }
}

fn set_note(trigger: &mut TriggerInput, note: String) {
    if let TriggerInput::Slack { note: slot, .. } = trigger {
        *slot = Some(note);
    }
}

fn join_text(left: &str, right: &str) -> String {
    match (left.is_empty(), right.is_empty()) {
        (true, _) => right.to_owned(),
        (_, true) => left.to_owned(),
        _ => format!("{left}\n\n{right}"),
    }
}

fn talk_text(run: &runner::Run, prompt: &str, seeded: bool) -> String {
    if !seeded {
        return prompt.to_owned();
    }
    let prefix = format!("{}\n\n", runner::history(run));
    prompt.strip_prefix(&prefix).unwrap_or(prompt).to_owned()
}

fn ask_from(classify: &crate::graph::Classify) -> Ask {
    Ask {
        model: classify.model.clone(),
        instructions: classify.instructions.clone(),
        questions: classify
            .questions
            .iter()
            .map(|question| Question {
                name: question.name.clone(),
                ask: question.ask.clone(),
                choices: question.choices.iter().map(|choice| choice.label.clone()).collect(),
                stop_on: vec![],
            })
            .collect(),
    }
}

fn decide_state(trigger: &TriggerInput) -> Value {
    match trigger {
        TriggerInput::Slack { channel_name, posts, .. } => {
            let author = posts.first().map(|post| post.author.as_str()).unwrap_or("");
            let message = posts.first().map(|post| post.text.as_str()).unwrap_or("");
            json!({ "channel": format!("#{channel_name}"), "author": author, "message": message })
        }
        TriggerInput::Issue { .. } => json!({ "message": trigger_text(trigger) }),
    }
}

/// Direct classify answers, one group per model, in the order the run logged them.
fn direct_answers(run: &runner::Run, library: &runner::Library) -> Vec<ClassifyAnswers> {
    let mut grouped: Vec<ClassifyAnswers> = vec![];
    for entry in &run.log {
        let Some(object) = entry.what.get("answers").and_then(|value| value.as_object()) else { continue };
        let Some(graph) = library.get(&entry.connector) else { continue };
        let Some(node) = graph.node(&entry.node) else { continue };
        let NodeKind::Classify(classify) = &node.kind else { continue };
        if !matches!(classify.gate, Gate::Direct) {
            continue;
        }
        let mut pairs = vec![];
        for question in &classify.questions {
            if let Some(answer) = object.get(&question.name).and_then(|value| value.as_str()) {
                pairs.push((question.name.clone(), answer.to_owned()));
            }
        }
        if let Some(group) = grouped.iter_mut().find(|group| group.model == classify.model) {
            for pair in pairs {
                if !group.answers.iter().any(|(name, _)| name == &pair.0) {
                    group.answers.push(pair);
                }
            }
        } else if !pairs.is_empty() {
            grouped.push(ClassifyAnswers { model: classify.model.clone(), answers: pairs });
        }
    }
    grouped
}

/// The note today's routed session gets. A downgrade keeps the sure wording and names the category taken.
fn route_note(run: &runner::Run, library: &runner::Library, names: &BTreeMap<String, String>) -> Option<String> {
    let entry = run.log.iter().rev().find(|entry| entry.what.get("port").and_then(|port| port.as_str()).is_some())?;
    let port = entry.what["port"].as_str()?;
    if port.is_empty() || port == "nowhere" {
        return None;
    }
    let graph = library.get(&entry.connector)?;
    let node = graph.node(&entry.node)?;
    let NodeKind::Classify(classify) = &node.kind else { return None };
    if !matches!(classify.gate, Gate::Decide { .. }) {
        return None;
    }
    let name = classify.branch_on.as_deref()?;
    let question = classify.questions.iter().find(|question| question.name == name)?;
    let taken = question.choices.iter().position(|choice| choice.id.0 == port)?;
    let router = names.get(&entry.connector).map(String::as_str).unwrap_or(entry.connector.as_str());
    let taken_name = question.choices[taken].label.as_str();
    let Some(chosen) = run.answers(&entry.node).and_then(|answers| answers.chosen.clone()) else {
        return Some(routed_note(router, taken_name));
    };
    let likelihood = chosen.probabilities.get(chosen.index).copied().unwrap_or(1.0);
    if chosen.index != taken && likelihood < UNSURE_BELOW {
        let guess = question.choices.get(chosen.index).map(|choice| choice.label.as_str()).unwrap_or(taken_name);
        return Some(format!(
            "\n\nThe decision model was unsure (it guessed {guess:?} at {:.0}%), so workflow {router:?} handed this thread over as {taken_name:?}.",
            likelihood * 100.0
        ));
    }
    Some(routed_note(router, taken_name))
}

/// The session `Effect::Open` starts for `owner`: the trigger as today's builders read it, the
/// router's note when another workflow handed the thread over, every direct classify's answers, the
/// talk text, and the run's history when seeded.
fn run_opening(run: &runner::Run, owner: &Connector, prompt: &str, seeded: bool, library: &runner::Library, names: &BTreeMap<String, String>, exe: &str) -> Result<Run, String> {
    let mut trigger = trigger_of(run)?;
    if owner.id != run.connector {
        if let Some(note) = route_note(run, library, names) {
            set_note(&mut trigger, note);
        }
    }
    let talk = talk_text(run, prompt, seeded);
    let history = seeded.then(|| runner::history(run));
    graph_opening(owner, &trigger, &direct_answers(run, library), &talk, history.as_deref(), exe)
}

/// The session today's builders would open for `owner`, plus the run's direct answers, the talk, and,
/// when restarting a lost conversation, the history first. A migrated connector passes an empty talk
/// and no history, and the result equals `run` or `slack_run` plus `append_answers`.
pub fn graph_opening(owner: &Connector, trigger: &TriggerInput, answers: &[ClassifyAnswers], talk: &str, history: Option<&str>, exe: &str) -> Result<Run, String> {
    let mut opened = match trigger {
        TriggerInput::Issue { id, identifier, title, description, url } => {
            let issue = linear::Issue { id: id.clone(), identifier: identifier.clone(), title: title.clone(), description: description.clone(), url: url.clone() };
            run(owner, &issue, exe)
        }
        TriggerInput::Slack { channel, channel_name, thread, posts, note, mentioned } => {
            let posts: Vec<slack::Post> = posts.iter().map(|post| slack::Post { ts: post.ts.clone(), author: post.author.clone(), text: post.text.clone() }).collect();
            let mut opened = slack_run(owner, channel, channel_name, thread, &posts, note.as_deref(), exe);
            opened.mentioned = mentioned.clone();
            opened
        }
    };
    for group in answers {
        if group.answers.is_empty() {
            continue;
        }
        let ask = Ask { model: group.model.clone(), instructions: String::new(), questions: vec![] };
        append_answers(&mut opened, &ask, &group.answers);
    }
    if !talk.is_empty() {
        if let Some(prompt) = opened.args.last_mut() {
            prompt.push_str("\n\n");
            prompt.push_str(talk);
        }
    }
    if let Some(history) = history.filter(|history| !history.is_empty()) {
        if let Some(prompt) = opened.args.last_mut() {
            *prompt = format!("{history}\n\n{prompt}");
        }
    }
    Ok(opened)
}

fn note_followed(store: &mut Store, connector: &str, run: &runner::Run) {
    let Ok(TriggerInput::Slack { channel, thread, posts, .. }) = trigger_of(run) else { return };
    let Some(connector) = store.connectors.iter_mut().find(|c| c.id == connector) else { return };
    let key = thread_key(&channel, &thread);
    if connector.followed.contains_key(&key) {
        return;
    }
    let cursor = posts.last().map(|post| post.ts.clone()).unwrap_or(thread);
    connector.followed.insert(key, Followed { cursor, seen: None, replied: false });
}

/// The run's tab, by id. A different tab with the same subject is not this one.
fn tab_of<'a>(after: &'a [TabView], tab: &str) -> Option<&'a TabView> {
    after.iter().find(|view| view.id == tab)
}

/// Level-triggered wakes against one machine's current tab list.
/// A list captured before the wait started cannot say the tab left.
/// A terminal status ends the wait when it belongs to this turn: Working was seen after the wait
/// started, the status time is newer than the wait, or the wait is the session's first turn.
/// A wait after a typed message ignores a Done still showing from the previous turn.
fn reconcile_runs(machine: &str, after: &[TabView], observed: u64, runs: &mut [runner::Run], at: &str) -> bool {
    let notes: Vec<_> = runs
        .iter()
        .filter_map(|run| {
            let Session::Live { machine: live, tab, since, saw_working, await_working, .. } = &run.session else { return None };
            if live != machine {
                return None;
            }
            Some((run.id.0.clone(), tab.clone(), *since, *saw_working, *await_working, run.status.clone(), tab_of(after, tab).cloned()))
        })
        .collect();
    let mut changed = false;
    for (id, _tab, since, saw_working, await_working, status, view) in notes {
        let Some(run) = runs.iter_mut().find(|run| run.id.0 == id) else { continue };
        let Some(view) = view else {
            if observed < since {
                continue;
            }
            match status {
                Status::Waiting => {
                    runner::push(run, Wake::TabGone, at);
                    changed = true;
                }
                Status::Done => {
                    if let Session::Live { claude, .. } = &run.session {
                        let claude = claude.clone();
                        run.session = Session::Detached { claude };
                        changed = true;
                    }
                }
                Status::Running | Status::Failed { .. } => {}
            }
            continue;
        };
        if let Session::Live { claude, saw_working: flag, .. } = &mut run.session {
            if claude.is_none() && view.session.is_some() {
                *claude = view.session.clone();
                changed = true;
            }
            if matches!(view.status, Some(AgentStatus::Working)) && !*flag {
                *flag = true;
                changed = true;
            }
        }
        if !matches!(status, Status::Waiting) {
            continue;
        }
        let saw = match &run.session {
            Session::Live { saw_working, .. } => *saw_working,
            _ => saw_working,
        };
        let ended = matches!(view.status, Some(AgentStatus::Done | AgentStatus::Failed));
        let newer = view.updated.is_some_and(|updated| since > 0 && updated > since / 1000);
        if ended && (saw || newer || !await_working) {
            runner::push(run, Wake::TurnEnded { claude: view.session.clone() }, at);
            changed = true;
        }
    }
    changed
}

fn inbox_text(items: &[GraphInbox]) -> String {
    items.iter().flat_map(|item| item.messages.iter().cloned()).collect::<Vec<_>>().join("\n\n")
}

impl Connectors {
    /// Polls forever. `open` receives what each check found and answers whether a tab took it, or
    /// why none did, and `on_check` hears how each check went. A settings change wakes it early.
    pub async fn watch<F, Fut, R>(&self, open: F, on_check: impl Fn(&Check))
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        loop {
            let _guard = self.driving.lock().await;
            let (work, mut error) = self.poll().await;
            for work in work {
                if let Err(e) = self.hand_over(work, &open).await {
                    error.get_or_insert(e);
                }
            }
            if let Err(e) = self.drive_locked(&open).await {
                error.get_or_insert(e);
            }
            drop(_guard);
            let error = merge_error(error, self.runs_broken.lock().unwrap().clone());
            let check = Check { at: chrono::Utc::now().to_rfc3339(), error };
            *self.checked.lock().unwrap() = Some(check.clone());
            on_check(&check);
            let _ = tokio::time::timeout(INTERVAL, self.wake.notified()).await;
        }
    }

    /// True while a run of `subject` is still going and its session belongs to `connector`.
    /// A box stays up for that, and its idle timers wait until the run ends.
    pub fn holds_box(&self, connector: &str, subject: &Subject) -> bool {
        self.runs.lock().unwrap().iter().any(|run| {
            matches!(run.status, Status::Running | Status::Waiting)
                && &run.subject == subject
                && run.session_owner.as_deref().unwrap_or(run.connector.as_str()) == connector
        })
    }

    /// Tab statuses changed on `machine`. Waiting runs wake, then every running graph continues.
    /// `observed` is the unix millis the list arrived. Lists can queue behind a drive, so an older one never replaces a newer one.
    pub async fn on_tabs<F, Fut, R>(&self, machine: &str, observed: u64, after: &[TabView], open: F) -> Result<(), String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        let _guard = self.driving.lock().await;
        {
            let mut known = self.known_tabs.lock().unwrap();
            if known.get(machine).is_none_or(|(at, _)| *at <= observed) {
                known.insert(machine.to_owned(), (observed, after.to_vec()));
            }
        }
        self.drive_locked(&open).await
    }

    fn reconcile_known(&self) -> Result<(), String> {
        let known: Vec<_> = self.known_tabs.lock().unwrap().iter().map(|(machine, (at, tabs))| (machine.clone(), *at, tabs.clone())).collect();
        let at = Utc::now().to_rfc3339();
        let changed = {
            let mut runs = self.runs.lock().unwrap();
            known.into_iter().fold(false, |changed, (machine, observed, tabs)| reconcile_runs(&machine, &tabs, observed, &mut runs, &at) || changed)
        };
        if changed {
            self.save_runs()?;
        }
        Ok(())
    }

    /// Starts one graph run from an injected trigger. A poll calls this. The ignored live test does too,
    /// because it has no Linear or Slack network.
    #[doc(hidden)]
    pub fn begin(&self, connector: String, subject: Subject, input: TriggerInput, at: &str) -> Result<(), String> {
        if run_open(&self.runs.lock().unwrap(), &connector, &subject) {
            return Ok(());
        }
        let value = serde_json::to_value(&input).map_err(|e| e.to_string())?;
        let run = runner::start(&self.library(), &connector, subject, value, at);
        if let Status::Failed { why } = &run.status {
            return Err(why.clone());
        }
        self.runs.lock().unwrap().push(run);
        self.save_runs()
    }

    fn enqueue(&self, item: GraphInbox) {
        let mut inbox = self.inbox.lock().unwrap();
        if inbox.iter().any(|have| have.channel == item.channel && have.thread == item.thread && have.cursor == item.cursor) {
            return;
        }
        inbox.push(item);
    }

    fn library(&self) -> runner::Library {
        self.store.lock().unwrap().connectors.iter().map(|c| (c.id.clone(), c.graph.clone().unwrap_or_else(|| migrate(c)))).collect()
    }

    fn names(&self) -> BTreeMap<String, String> {
        self.store.lock().unwrap().connectors.iter().map(|c| (c.id.clone(), c.name.clone())).collect()
    }

    fn save_runs(&self) -> Result<(), String> {
        if let Some(why) = self.runs_broken.lock().unwrap().clone() {
            return Err(why);
        }
        let bytes = serde_json::to_vec(&*self.runs.lock().unwrap()).map_err(|e| e.to_string())?;
        self.storage.save_runs(&bytes)
    }

    fn persist_run(&self, run: &runner::Run) -> Result<(), String> {
        {
            let mut runs = self.runs.lock().unwrap();
            if let Some(slot) = runs.iter_mut().find(|have| have.id == run.id) {
                *slot = run.clone();
            }
        }
        self.save_runs()
    }

    /// Fills the wait clock once, when the run enters Waiting. Later parks leave it alone.
    fn stamp_wait(run: &mut runner::Run) {
        if !matches!(run.status, Status::Waiting) {
            return;
        }
        if let Session::Live { since, .. } = &mut run.session {
            if *since == 0 {
                *since = unix_millis();
            }
        }
    }

    fn replace_run(&self, run: runner::Run) -> Result<(), String> {
        {
            let mut runs = self.runs.lock().unwrap();
            if let Some(slot) = runs.iter_mut().find(|have| have.id == run.id) {
                *slot = run;
            } else {
                runs.push(run);
            }
        }
        self.save_runs()
    }

    fn mark_live(&self, id: &str, place: Placement, claude: Option<String>) -> Result<(), String> {
        {
            let mut runs = self.runs.lock().unwrap();
            if let Some(run) = runs.iter_mut().find(|run| run.id.0 == id) {
                run.session = Session::Live {
                    machine: place.machine,
                    tab: place.tab,
                    claude,
                    since: unix_millis(),
                    saw_working: false,
                    await_working: true,
                };
            }
        }
        self.save_runs()
    }

    /// Drives every running graph once. `watch` and `on_tabs` call the same loop. The ignored live
    /// test calls it between tab snapshots, the way `machine_frame` does.
    #[doc(hidden)]
    pub async fn drive<F, Fut, R>(&self, open: &F) -> Result<(), String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        let _guard = self.driving.lock().await;
        self.drive_locked(open).await
    }

    /// The runs on disk, for the ignored live test.
    #[doc(hidden)]
    pub fn graph_runs(&self) -> Vec<runner::Run> {
        self.runs.lock().unwrap().clone()
    }

    /// Points the live session at `claude` and stores the run. The ignored live test uses a missing
    /// id so the next resume fails and the run opens a history-seeded session.
    #[doc(hidden)]
    pub fn set_live_claude(&self, claude: &str) -> Result<(), String> {
        {
            let mut runs = self.runs.lock().unwrap();
            let run = runs
                .iter_mut()
                .find(|run| matches!(run.session, runner::Session::Live { .. }))
                .ok_or_else(|| "no live graph session".to_owned())?;
            if let runner::Session::Live { claude: slot, .. } = &mut run.session {
                *slot = Some(claude.to_owned());
            }
        }
        self.save_runs()
    }

    async fn drive_locked<F, Fut, R>(&self, open: &F) -> Result<(), String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        self.reconcile_known()?;
        let library = self.library();
        let mut failed = vec![];
        for _ in 0..256 {
            let index = self.runs.lock().unwrap().iter().position(|run| matches!(run.status, Status::Running));
            let Some(index) = index else { break };
            let mut run = self.runs.lock().unwrap()[index].clone();
            match runner::drive(&mut run, &library) {
                Drive::Act(effect) => {
                    let supplied = match self.perform(&mut run, effect, &library, open).await {
                        Ok(supplied) => supplied,
                        Err(why) => Supplied::Failed(why),
                    };
                    runner::supply(&mut run, supplied, &Utc::now().to_rfc3339());
                    Self::stamp_wait(&mut run);
                    if let Status::Failed { why } = &run.status {
                        failed.push(why.clone());
                    }
                    self.replace_run(run)?;
                }
                Drive::Park | Drive::Done => {
                    Self::stamp_wait(&mut run);
                    self.replace_run(run)?;
                }
            }
        }
        self.flush_locked(open).await?;
        let stuck = self.runs.lock().unwrap().iter().any(|run| matches!(run.status, Status::Running));
        if stuck {
            failed.push("a workflow run did not settle".into());
        }
        match failed.is_empty() {
            true => Ok(()),
            false => Err(failed.join("; ")),
        }
    }

    async fn flush_locked<F, Fut, R>(&self, open: &F) -> Result<(), String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        let pending = std::mem::take(&mut *self.inbox.lock().unwrap());
        let mut later = vec![];
        let mut errors = vec![];
        for item in pending {
            match self.deliver_inbox(&item, open).await {
                Ok(InboxDone::Sent) => {}
                Ok(InboxDone::Later) => later.push(item),
                Err(e) => {
                    later.push(item);
                    errors.push(e);
                }
            }
        }
        self.inbox.lock().unwrap().extend(later);
        match errors.is_empty() {
            true => Ok(()),
            false => Err(errors.join("; ")),
        }
    }

    async fn deliver_inbox<F, Fut, R>(&self, item: &GraphInbox, open: &F) -> Result<InboxDone, String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        let run = {
            let runs = self.runs.lock().unwrap();
            runs.iter()
                .rev()
                .find(|run| subject_key(&run.subject) == thread_key(&item.channel, &item.thread) && !matches!(run.status, Status::Failed { .. }))
                .cloned()
        };
        let Some(run) = run else { return Ok(InboxDone::Later) };
        let owner = run.session_owner.clone().unwrap_or_else(|| run.connector.clone());
        let text = item.messages.join("\n\n");
        match &run.session {
            Session::Live { .. } => {
                let env = agent::session_env(&owner, &run.subject);
                open(Work::Prompt { env, text }).await.taken()?;
                let _ = self.follow(&owner, &item.channel, &item.thread, item.cursor.clone());
                let _ = self.acknowledge(&item.channel, &item.mentioned).await;
                Ok(InboxDone::Sent)
            }
            Session::Detached { .. } if matches!(run.status, Status::Done) => {
                self.resume_finished(&run, &owner, &text, open).await?;
                let _ = self.follow(&owner, &item.channel, &item.thread, item.cursor.clone());
                let _ = self.acknowledge(&item.channel, &item.mentioned).await;
                Ok(InboxDone::Sent)
            }
            Session::Detached { .. } | Session::None => Ok(InboxDone::Later),
        }
    }

    /// A finished run's thread got another reply. Resume its conversation, or seed a new one.
    async fn resume_finished<F, Fut, R>(&self, run: &runner::Run, owner: &str, text: &str, open: &F) -> Result<(), String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        let owner_c = self.store.lock().unwrap().connector(owner).cloned()?;
        let trigger = trigger_of(run)?;
        let claude = match &run.session {
            Session::Detached { claude } => claude.clone(),
            Session::Live { .. } | Session::None => None,
        };
        if let Some(claude) = claude {
            let mut spec = graph_opening(&owner_c, &trigger, &[], "", None, &self.exe)?;
            spec.args.pop();
            spec.args.extend(["--resume".into(), claude.clone()]);
            match open(Work::Run(spec)).await.taken() {
                Ok(place) => {
                    let place = place.unwrap_or(Placement { machine: "local".into(), tab: "local".into() });
                    if !text.is_empty() {
                        let env = agent::session_env(owner, &run.subject);
                        open(Work::Prompt { env, text: text.to_owned() }).await.taken()?;
                    }
                    return self.mark_live(&run.id.0, place, Some(claude));
                }
                Err(why) if why.contains("No conversation found") => {}
                Err(why) => return Err(why),
            }
        }
        let spec = graph_opening(&owner_c, &trigger, &direct_answers(run, &self.library()), text, Some(&runner::history(run)), &self.exe)?;
        let place = open(Work::Run(spec)).await.taken()?.unwrap_or(Placement { machine: "local".into(), tab: "local".into() });
        self.mark_live(&run.id.0, place, None)
    }

    fn take_inbox(&self, subject: &Subject) -> Vec<GraphInbox> {
        let key = subject_key(subject);
        let mut inbox = self.inbox.lock().unwrap();
        let mut kept = vec![];
        let mut mine = vec![];
        for item in inbox.drain(..) {
            if thread_key(&item.channel, &item.thread) == key {
                mine.push(item);
            } else {
                kept.push(item);
            }
        }
        *inbox = kept;
        mine
    }

    fn restore_inbox(&self, items: Vec<GraphInbox>) {
        self.inbox.lock().unwrap().splice(0..0, items);
    }

    async fn perform<F, Fut, R>(&self, run: &mut runner::Run, effect: Effect, library: &runner::Library, open: &F) -> Result<Supplied, String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        match effect {
            Effect::Claim { connector, key } => {
                let mut store = self.store.lock().unwrap();
                if let Some(name) = store
                    .connectors
                    .iter()
                    .find(|c| c.id != connector && c.handled.iter().any(|handled| handled == &key))
                    .map(|c| c.name.clone())
                {
                    return Ok(Supplied::Held { why: format!("{key} is already handled by workflow {name:?}") });
                }
                let already = store.handled(&key);
                if !already && store.claim_thread(&connector, &key).is_none() {
                    return Err(format!("workflow {connector:?} was deleted"));
                }
                note_followed(&mut store, &connector, run);
                self.save(&store)?;
                Ok(Supplied::Done)
            }
            Effect::Classify { connector, node, .. } => {
                let classify = {
                    let graph = library.get(&connector).ok_or_else(|| format!("workflow {connector:?} has no graph"))?;
                    let found = graph.node(&node).ok_or_else(|| format!("node {} is not in workflow {connector:?}", node.0))?;
                    match &found.kind {
                        NodeKind::Classify(classify) => classify.clone(),
                        _ => return Err(format!("node {} is not a classify", node.0)),
                    }
                };
                let key = self.key(VERCEL_KEY).ok_or_else(|| "Vercel AI Gateway is not connected: connect it in pinned's Connectors settings".to_owned())?;
                let trigger = trigger_of(run)?;
                let text = trigger_text(&trigger);
                let answers = match &classify.gate {
                    Gate::Direct => Answers { answers: vercel::ask(&key, &ask_from(&classify), &text).await?, chosen: None },
                    Gate::Decide { .. } => {
                        let name = classify.branch_on.as_deref().ok_or_else(|| "the classify decides a port but branches on no question".to_owned())?;
                        let question = classify
                            .questions
                            .iter()
                            .find(|question| question.name == name)
                            .ok_or_else(|| format!("the classify branches on {name:?}, which is not one of its questions"))?;
                        let choices: Vec<(&str, &str)> = question.choices.iter().map(|choice| (choice.label.as_str(), choice.description.as_str())).collect();
                        let chosen = vercel::choose(&key, classify.model.trim(), &classify.instructions, &choices, &decide_state(&trigger)).await?;
                        let label = question.choices.get(chosen.index).map(|choice| choice.label.clone()).unwrap_or_default();
                        Answers { answers: vec![(question.name.clone(), label)], chosen: Some(ChosenOn::from(&chosen)) }
                    }
                };
                Ok(Supplied::Classified(answers))
            }
            Effect::Page { connector, group } => {
                let Subject::SlackThread { channel, thread } = &run.subject else {
                    return Err("a page node needs the run's Slack thread".into());
                };
                let (channel, thread) = (channel.clone(), thread.clone());
                let paging = self.paging_for(&connector, &group).await?;
                let now = Utc::now();
                let reply = {
                    let store = self.store.lock().unwrap();
                    let c = store.connectors.iter().find(|c| c.id == connector).ok_or_else(|| "the workflow was deleted".to_owned())?;
                    c.escalation_reply(&channel, &thread, paging, now)
                };
                if let Some(reply) = reply {
                    // Post first. A missed page is the failure a retry has to repair, so a crash after a successful post may page twice.
                    self.page(&Page { channel: channel.clone(), thread: thread.clone(), reply: reply.clone(), react: true }).await?;
                    let mut store = self.store.lock().unwrap();
                    if let Some(c) = store.connectors.iter_mut().find(|c| c.id == connector) {
                        c.record_escalation(&channel, &thread, &reply, now);
                    }
                    self.save(&store)?;
                }
                Ok(Supplied::Done)
            }
            Effect::StagePicked { connector } => {
                self.stage(&connector, &run.subject, Stage::Picked).await?;
                Ok(Supplied::Done)
            }
            Effect::Open { owner, prompt, seeded } => self.perform_open(run, &owner, &prompt, seeded, library, open).await,
            Effect::Resume { owner, claude, text } => self.perform_resume(run, &owner, &claude, &text, open).await,
            Effect::Type { text } => self.perform_type(run, &text, open).await,
        }
    }

    async fn paging_for(&self, connector: &str, group: &str) -> Result<Paging, String> {
        let (name, category) = {
            let store = self.store.lock().unwrap();
            let c = store.connector(connector)?;
            let category = match &c.trigger {
                Trigger::SlackMessage { decide, .. } => decide.categories.iter().find(|category| category.group() == Some(group.trim().trim_start_matches('@'))).cloned(),
                _ => None,
            };
            (c.name.clone(), category)
        };
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected: connect it in pinned's Connectors settings")?;
        let id = slack::group_id(&token, group).await.map_err(|e| format!("workflow {name:?} could not page {group:?}: {e}"))?;
        let ping = match &category {
            Some(category) => page_text(&id, category),
            None => format!("{} {}", slack::group_mention(&id), group.trim().trim_start_matches('@')),
        };
        Ok(Paging { ping, handle: group.trim().trim_start_matches('@').to_owned() })
    }

    async fn perform_open<F, Fut, R>(&self, run: &runner::Run, owner: &str, prompt: &str, seeded: bool, library: &runner::Library, open: &F) -> Result<Supplied, String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        let owner_c = self.store.lock().unwrap().connector(owner).cloned()?;
        let inbox = self.take_inbox(&run.subject);
        let spec = match run_opening(run, &owner_c, prompt, seeded, library, &self.names(), &self.exe) {
            Ok(mut spec) => {
                let waiting = inbox_text(&inbox);
                if let Some(last) = spec.args.last_mut().filter(|_| !waiting.is_empty()) {
                    *last = join_text(last, &waiting);
                }
                spec
            }
            Err(why) => {
                self.restore_inbox(inbox);
                return Err(why);
            }
        };
        let folder = spec.cwd.clone();
        let mentioned = spec.mentioned.clone();
        match open(Work::Run(spec)).await.taken() {
            Ok(place) => {
                if let Subject::SlackThread { channel, .. } = &run.subject {
                    let _ = self.acknowledge(channel, &mentioned).await;
                }
                let _ = self.ack_inbox(owner, &inbox).await;
                let place = place.unwrap_or(Placement { machine: "local".into(), tab: "local".into() });
                Ok(Supplied::Opened { machine: place.machine, tab: place.tab })
            }
            Err(why) => {
                self.restore_inbox(inbox);
                Err(self.unclaim(owner, &run.subject, &folder, &why))
            }
        }
    }

    async fn perform_resume<F, Fut, R>(&self, run: &mut runner::Run, owner: &str, claude: &str, text: &str, open: &F) -> Result<Supplied, String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        let owner_c = self.store.lock().unwrap().connector(owner).cloned()?;
        let trigger = trigger_of(run)?;
        let mut spec = graph_opening(&owner_c, &trigger, &[], "", None, &self.exe)?;
        spec.args.pop();
        spec.args.extend(["--resume".into(), claude.to_owned()]);
        let inbox = self.take_inbox(&run.subject);
        let text = join_text(text, &inbox_text(&inbox));
        let place = match open(Work::Run(spec)).await.taken() {
            Err(why) if why.contains("No conversation found") => {
                self.restore_inbox(inbox);
                return Ok(Supplied::NoConversation);
            }
            Err(why) => {
                self.restore_inbox(inbox);
                return Err(why);
            }
            Ok(place) => place.unwrap_or(Placement { machine: "local".into(), tab: "local".into() }),
        };
        if let Err(why) = self.type_into(run, owner, &text, open).await {
            self.restore_inbox(inbox);
            return Err(why);
        }
        let _ = self.ack_inbox(owner, &inbox).await;
        Ok(Supplied::Opened { machine: place.machine, tab: place.tab })
    }

    async fn perform_type<F, Fut, R>(&self, run: &mut runner::Run, text: &str, open: &F) -> Result<Supplied, String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        let owner = run.session_owner.clone().unwrap_or_else(|| run.connector.clone());
        let inbox = self.take_inbox(&run.subject);
        let text = join_text(text, &inbox_text(&inbox));
        if let Err(why) = self.type_into(run, &owner, &text, open).await {
            self.restore_inbox(inbox);
            return Err(why);
        }
        let _ = self.ack_inbox(&owner, &inbox).await;
        Ok(Supplied::Done)
    }

    /// Records `text` and saves the run before the prompt, so a crash mid-send does not type it twice.
    /// The window between the save and the machine accepting the prompt can still drop one send.
    async fn type_into<F, Fut, R>(&self, run: &mut runner::Run, owner: &str, text: &str, open: &F) -> Result<(), String>
    where
        F: Fn(Work) -> Fut,
        Fut: Future<Output = R>,
        R: Taken,
    {
        if text.is_empty() || runner::already_sent(run, text) {
            return Ok(());
        }
        runner::note_sent(run, text);
        self.persist_run(run)?;
        let env = agent::session_env(owner, &run.subject);
        if let Err(why) = open(Work::Prompt { env, text: text.to_owned() }).await.taken() {
            runner::forget_sent(run, text);
            let _ = self.persist_run(run);
            return Err(why);
        }
        Ok(())
    }

    async fn ack_inbox(&self, owner: &str, items: &[GraphInbox]) -> Result<(), String> {
        for item in items {
            self.follow(owner, &item.channel, &item.thread, item.cursor.clone())?;
            self.acknowledge(&item.channel, &item.mentioned).await?;
        }
        Ok(())
    }

    pub fn get_settings(&self) -> Settings {
        let store = self.store.lock().unwrap();
        Settings {
            linear: store.linear.clone(),
            slack: store.slack.clone(),
            github: store.github.clone(),
            vercel: store.vercel.clone(),
            upstash: store.upstash.clone(),
            connectors: store.connectors.iter().cloned().map(presented).collect(),
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

    /// Checks the key against the gateway before keeping it.
    pub async fn connect_vercel(&self, key: &str) -> Result<vercel::Account, String> {
        let key = key.trim();
        let account = vercel::account(key).await?;
        self.storage.write_key(VERCEL_KEY, key)?;
        let mut store = self.store.lock().unwrap();
        store.vercel = Some(account.clone());
        self.save(&store)?;
        self.wake.notify_one();
        Ok(account)
    }

    pub fn disconnect_vercel(&self) -> Result<(), String> {
        self.storage.remove_key(VERCEL_KEY);
        let mut store = self.store.lock().unwrap();
        store.vercel = None;
        self.save(&store)
    }

    /// Chat models tagged `structured-output`, and every decision model. The list needs no key.
    pub async fn vercel_models(&self) -> Result<Vec<vercel::Model>, String> {
        vercel::models().await
    }

    pub async fn github_repos(&self) -> Result<Vec<github::Repo>, String> {
        github::repos(&self.github_token().ok_or("GitHub is not connected")?).await
    }

    /// What a machine clones the account's private GitHub repositories with.
    pub fn github_token(&self) -> Option<String> {
        self.key(GITHUB_KEY)
    }

    /// Keeps the Box key and the Claude token, which the hub checked with Upstash first.
    pub fn connect_upstash(&self, keys: &upstash::Keys) -> Result<upstash::Account, String> {
        self.storage.write_key(UPSTASH_KEY, &keys.api_key)?;
        self.storage.write_key(CLAUDE_TOKEN, &keys.claude_token)?;
        let account = keys.account();
        let mut store = self.store.lock().unwrap();
        store.upstash = Some(account.clone());
        self.save(&store)?;
        Ok(account)
    }

    pub fn disconnect_upstash(&self) -> Result<(), String> {
        self.storage.remove_key(UPSTASH_KEY);
        self.storage.remove_key(CLAUDE_TOKEN);
        let mut store = self.store.lock().unwrap();
        store.upstash = None;
        self.save(&store)
    }

    /// What the hub creates and boots boxes with. Never serialized, never sent to a client.
    pub fn box_keys(&self) -> Option<upstash::Keys> {
        Some(upstash::Keys { api_key: self.key(UPSTASH_KEY)?, claude_token: self.key(CLAUDE_TOKEN)? })
    }

    /// Channels the bot was invited to, which a Slack connector can watch.
    pub async fn slack_suggestions(&self) -> Result<Vec<String>, String> {
        let token = self.key(SLACK_KEY).ok_or("Slack is not connected")?;
        slack::member_channels(&token).await
    }

    /// Adds or replaces a connector. Its watermark, handled issues, filed issues, thread cursors and pages stay with the stored copy.
    ///
    /// An older app omits `graph`. When the stored graph is exactly `migrate` of the stored
    /// connector, that graph was never edited, so the save replaces it. An edited graph stays,
    /// and `project` rewrites the legacy trigger and ask so the next old-app read matches it.
    /// A graph equal to `migrate` of the connector being stored is stored as `None`, so the
    /// untouched case stays derivable. What this returns still carries a graph, as `get_settings` does.
    pub fn save_connector(&self, mut connector: Connector) -> Result<Connector, String> {
        let mut store = self.store.lock().unwrap();
        if let Some(graph) = connector.graph.take() {
            {
                let called = |id: &str| call_target(&store, connector.id.as_str(), &graph, id);
                graph.validate(&called)?;
            }
            reject_call_cycle(connector.id.as_str(), &graph, &store)?;
            refresh_legacy(&mut connector, &graph);
            connector.graph = (graph != migrate(&connector)).then_some(graph);
        } else if let Some(existing) = store.connectors.iter().find(|existing| existing.id == connector.id && !connector.id.is_empty()) {
            if let Some(stored) = existing.graph.clone() {
                if stored == migrate(existing) {
                    connector.graph = None;
                } else {
                    refresh_legacy(&mut connector, &stored);
                    connector.graph = Some(stored);
                }
            }
        }
        match store.connectors.iter_mut().find(|c| c.id == connector.id && !c.id.is_empty()) {
            Some(existing) => {
                connector.since = existing.since.clone();
                connector.handled = std::mem::take(&mut existing.handled);
                connector.created = std::mem::take(&mut existing.created);
                connector.followed = std::mem::take(&mut existing.followed);
                connector.paged = std::mem::take(&mut existing.paged);
                *existing = connector.clone();
            }
            None => {
                connector.id = format!("c{}", chrono::Utc::now().timestamp_millis());
                connector.since = chrono::Utc::now().to_rfc3339();
                connector.handled.clear();
                connector.created.clear();
                connector.followed.clear();
                connector.paged.clear();
                store.connectors.push(connector.clone());
            }
        }
        self.save(&store)?;
        self.wake.notify_one();
        Ok(presented(connector))
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
        if current.linear.is_some() || current.slack.is_some() || current.github.is_some() || current.vercel.is_some() || !current.connectors.is_empty() {
            return Ok(false);
        }
        let present = |key: Option<String>| key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
        let (linear_key, slack_key, github_key, vercel_key) = (present(keys.linear), present(keys.slack), present(keys.github), present(keys.vercel));
        if linear_key.is_none() {
            imported.linear = None;
        }
        if slack_key.is_none() {
            imported.slack = None;
        }
        if github_key.is_none() {
            imported.github = None;
        }
        if vercel_key.is_none() {
            imported.vercel = None;
        }
        for (name, key) in [(LINEAR_KEY, linear_key), (SLACK_KEY, slack_key), (GITHUB_KEY, github_key), (VERCEL_KEY, vercel_key)] {
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
        (store, Keys { linear: self.key(LINEAR_KEY), slack: self.key(SLACK_KEY), github: self.key(GITHUB_KEY), vercel: self.key(VERCEL_KEY) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentStatus;
    use crate::graph::{Edge, Node, NodeId, PortId};
    use crate::wire::{Program, TabView};

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
    fn a_connector_from_v0_7_0_runs_under_its_projects_account_and_a_named_one_under_that() {
        let saved = serde_json::json!({
            "id": "c1", "name": "Bugs", "enabled": true, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": "/repo", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": "m1", "repo": "",
        });
        let old: Connector = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(run(&old, &issue(), "/bin/pinned").spec().account, None);
        let mut named = saved;
        named["account"] = " work ".into();
        let named: Connector = serde_json::from_value(named).unwrap();
        assert_eq!(run(&named, &issue(), "/bin/pinned").spec().account.as_deref(), Some("work"));
        assert_eq!(serde_json::to_value(&named).unwrap()["account"], " work ");
        assert_eq!(old.ask, None, "a connector stored before the ask step has none");
        assert!(serde_json::to_value(&old).unwrap().get("ask").is_none());
    }

    #[test]
    fn a_run_no_tab_took_is_found_again_by_the_next_check() {
        let mut linear = linear_connector();
        linear.handled = vec!["uuid".into(), "older".into()];
        let mut slack = Connector { id: "c2".into(), ..slack_connector() };
        slack.handled = vec!["C0123ABCD:1792400050.000200".into()];
        slack.followed.insert("C0123ABCD:1792400050.000200".into(), Followed { cursor: "1792400060.000000".into(), seen: None, replied: false });
        let (connectors, dir) = stored("unclaim", vec![linear, slack]);
        let refuse = |_: Work| async { Err::<(), String>("Upstash Box: the bootstrap failed: curl: (22) 404".to_owned()) };

        let issue_run = run(&linear_connector(), &issue(), "/bin/pinned");
        assert_eq!(
            block_on(connectors.hand_over(Work::Run(issue_run), &refuse)),
            Err("could not open a session in /repo: Upstash Box: the bootstrap failed: curl: (22) 404; the next check tries again".into()),
            "the check line says why no machine took the run"
        );
        let thread_run = slack_run(&Connector { id: "c2".into(), ..slack_connector() }, "C0123ABCD", "support", "1792400050.000200", &posts(), None, "/bin/pinned");
        let thread_run = Run { mentioned: vec!["1792400050.000200".into()], ..thread_run };
        assert_eq!(
            block_on(connectors.hand_over(Work::Run(thread_run), &refuse)),
            Err("could not open a session in /repo: Upstash Box: the bootstrap failed: curl: (22) 404; the next check tries again".into()),
            "nothing reacts to a mention no session took"
        );

        let reloaded = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into()).get_settings().connectors;
        assert_eq!(reloaded[0].handled, ["older"]);
        assert_eq!((reloaded[1].handled.len(), reloaded[1].followed.len()), (0, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stop_answer_keeps_the_subject_handled_and_forgets_the_thread() {
        let ask = Ask {
            model: "google/gemini-3.5-flash".into(),
            instructions: "Classify.".into(),
            questions: vec![
                Question { name: "kind".into(), ask: "What does it ask for?".into(), choices: vec!["bug".into(), "question".into()], stop_on: vec![] },
                Question { name: "agent".into(), ask: "Can a coding agent handle it?".into(), choices: vec!["yes".into(), "no".into()], stop_on: vec!["No".into()] },
            ],
        };
        let stopping = [("kind".into(), "bug".into()), ("agent".into(), "no".into())];
        assert_eq!(stops(&ask, &stopping), Some(("agent".into(), "no".into())));
        assert_eq!(stops(&ask, &[("kind".into(), "bug".into()), ("agent".into(), "yes".into())]), None);
        assert_eq!(
            answered("google/gemini-3.5-flash", "thread", &stopping),
            "\n\nBefore this session, google/gemini-3.5-flash on Vercel's AI Gateway read the thread and answered:\n- kind: bug\n- agent: no"
        );
        assert_eq!(
            answered("google/gemini-3.5-flash", "issue", &[("kind".into(), "question".into())]),
            "\n\nBefore this session, google/gemini-3.5-flash on Vercel's AI Gateway read the issue and answered:\n- kind: question"
        );

        let mut slack = Connector { id: "c2".into(), ask: Some(ask), ..slack_connector() };
        slack.handled = vec!["C0123ABCD:1792400050.000200".into()];
        slack.followed.insert("C0123ABCD:1792400050.000200".into(), Followed { cursor: "1792400060.000000".into(), seen: None, replied: false });
        let (connectors, dir) = stored("ask-stop", vec![slack]);
        std::fs::write(dir.join("vercel-key"), "vck_testa1b2").unwrap();
        let _reply = TestReply::set(r#"{"kind":"bug","agent":"No"}"#.into());
        let opened = std::sync::atomic::AtomicBool::new(false);
        let open = |_: Work| {
            opened.store(true, std::sync::atomic::Ordering::Relaxed);
            async { Ok(()) }
        };
        let thread_run = slack_run(&Connector { id: "c2".into(), ..slack_connector() }, "C0123ABCD", "support", "1792400050.000200", &posts(), None, "/bin/pinned");
        assert_eq!(block_on(connectors.hand_over(Work::Run(thread_run), &open)), Ok(()));
        assert!(!opened.load(std::sync::atomic::Ordering::Relaxed), "a stop answer starts no session");
        let reloaded = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into()).get_settings().connectors;
        assert_eq!(reloaded[0].handled, ["C0123ABCD:1792400050.000200"]);
        assert!(reloaded[0].followed.is_empty(), "later replies are not delivered, and fallback never opens a session");
        let _ = std::fs::remove_dir_all(dir);
    }

    static ASK_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TestReply {
        _gate: std::sync::MutexGuard<'static, ()>,
    }

    impl TestReply {
        fn set(content: String) -> Self {
            let gate = ASK_TEST.lock().unwrap();
            crate::vercel::set_test_content(Some(content));
            Self { _gate: gate }
        }
    }

    impl Drop for TestReply {
        fn drop(&mut self) {
            crate::vercel::set_test_content(None);
        }
    }

    fn classifying() -> Ask {
        Ask {
            model: "google/gemini-3.5-flash".into(),
            instructions: "Classify.".into(),
            questions: vec![
                Question { name: "kind".into(), ask: "What does it ask for?".into(), choices: vec!["bug".into(), "question".into()], stop_on: vec![] },
                Question { name: "agent".into(), ask: "Can a coding agent handle it?".into(), choices: vec!["yes".into(), "no".into()], stop_on: vec!["no".into()] },
            ],
        }
    }

    fn claimed_thread(name: &str, ask: Option<Ask>) -> (Connectors, PathBuf) {
        let mut slack = Connector { id: "c2".into(), ask, ..slack_connector() };
        slack.handled = vec!["C0123ABCD:1792400050.000200".into()];
        slack.followed.insert("C0123ABCD:1792400050.000200".into(), Followed { cursor: "1792400060.000000".into(), seen: None, replied: false });
        stored(name, vec![slack])
    }

    #[test]
    fn an_answer_that_continues_is_appended_to_the_prompt() {
        let (connectors, dir) = claimed_thread("ask-go", Some(classifying()));
        std::fs::write(dir.join("vercel-key"), "vck_testa1b2").unwrap();
        let _reply = TestReply::set(r#"{"kind":"bug","agent":"yes"}"#.into());
        let prompt = std::sync::Mutex::new(String::new());
        let open = |work: Work| {
            if let Work::Run(run) = work {
                *prompt.lock().unwrap() = run.args.last().cloned().unwrap_or_default();
            }
            async { Ok(()) }
        };
        let thread_run = slack_run(&Connector { id: "c2".into(), ..slack_connector() }, "C0123ABCD", "support", "1792400050.000200", &posts(), None, "/bin/pinned");
        assert_eq!(block_on(connectors.hand_over(Work::Run(thread_run), &open)), Ok(()));
        assert_eq!(
            prompt.lock().unwrap().as_str().rsplit_once("\n\n").unwrap().1,
            "Before this session, google/gemini-3.5-flash on Vercel's AI Gateway read the thread and answered:\n- kind: bug\n- agent: yes"
        );
        let reloaded = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into()).get_settings().connectors;
        assert_eq!(reloaded[0].handled, ["C0123ABCD:1792400050.000200"]);
        assert_eq!(reloaded[0].followed.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_gateway_key_gives_the_subject_back() {
        let (connectors, dir) = claimed_thread("ask-nokey", Some(classifying()));
        let thread_run = slack_run(&Connector { id: "c2".into(), ..slack_connector() }, "C0123ABCD", "support", "1792400050.000200", &posts(), None, "/bin/pinned");
        assert_eq!(
            block_on(connectors.hand_over(Work::Run(thread_run), &|_| async { Ok(()) })),
            Err("could not open a session in /repo: Vercel AI Gateway is not connected: connect it in pinned's Connectors settings; the next check tries again".into())
        );
        let reloaded = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into()).get_settings().connectors;
        assert_eq!((reloaded[0].handled.len(), reloaded[0].followed.len()), (0, 0));
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
        assert_eq!(block_on(connectors.hand_over(Work::Run(thread_run), &|_| async { Ok(()) })), Ok(()));
        assert_eq!(connectors.get_settings().connectors[0].handled, ["C0123ABCD:1792400050.000200"]);
        assert_eq!(block_on(connectors.acknowledge("C0123ABCD", &[])), Ok(()), "nothing to react to needs no Slack");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_reaction_that_fails_leaves_the_session_and_cursor_in_place() {
        let slack = Connector { id: "c2".into(), handled: vec!["C0123ABCD:1792400050.000200".into()], ..slack_connector() };
        let (connectors, dir) = stored("unreacted", vec![slack]);
        let not_connected = Err("Slack is not connected: connect it in pinned's Connectors settings".to_owned());
        let thread_run = slack_run(&Connector { id: "c2".into(), ..slack_connector() }, "C0123ABCD", "support", "1792400050.000200", &posts(), None, "/bin/pinned");
        let thread_run = Run { mentioned: vec!["1792400050.000200".into()], ..thread_run };
        assert_eq!(block_on(connectors.hand_over(Work::Run(thread_run), &|_| async { Ok(()) })), not_connected);
        assert_eq!(connectors.get_settings().connectors[0].handled, ["C0123ABCD:1792400050.000200"], "the run stays claimed");

        let delivery = Delivery {
            connector: "c2".into(),
            channel: "C0123ABCD".into(),
            thread: "1792400050.000200".into(),
            messages: vec!["yes, file it".into()],
            cursor: "1792400090.000000".into(),
            mentioned: vec!["1792400090.000000".into()],
        };
        assert_eq!(block_on(connectors.hand_over(Work::Delivery(delivery), &|_| async { Ok(()) })), not_connected);
        let reloaded = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into()).get_settings().connectors;
        assert_eq!(reloaded[0].followed["C0123ABCD:1792400050.000200"].cursor, "1792400090.000000", "the cursor moved all the same");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_import_fills_an_empty_store_and_keys_and_drops_integrations_without_a_key() {
        let (connectors, dir) = stored("import", vec![]);
        let exported = serde_json::json!({
            "linear": { "name": "Ana", "email": "ana@acme.dev", "organization": "Acme" },
            "slack": { "team": "Acme", "userId": "UBOT", "name": "pinned" },
            "vercel": { "key_hint": "…ck_1" },
            "connectors": [serde_json::to_value(Connector { id: "c9".into(), ..slack_connector() }).unwrap()],
        });
        let keys = Keys { linear: Some(" lin_api_1 ".into()), github: Some("ghp_1".into()), vercel: Some(" vck_1 ".into()), ..Keys::default() };
        assert_eq!(connectors.import(exported.clone(), keys), Ok(true));
        let (store, keys) = connectors.export();
        assert_eq!(keys, Keys { linear: Some("lin_api_1".into()), slack: None, github: Some("ghp_1".into()), vercel: Some("vck_1".into()) });
        assert_eq!(store["linear"]["email"], "ana@acme.dev");
        assert_eq!(store["vercel"], serde_json::json!({ "key_hint": "…ck_1" }));
        assert_eq!(std::fs::read_to_string(dir.join("vercel-key")).unwrap(), "vck_1");
        assert_eq!(store["slack"], serde_json::Value::Null);
        assert_eq!(store["connectors"][0]["id"], "c9");
        assert_eq!(std::fs::read_to_string(dir.join("linear-key")).unwrap(), "lin_api_1");
        assert_eq!(std::fs::read_to_string(dir.join("vercel-key")).unwrap(), "vck_1");
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
            paged: BTreeMap::new(),
            proof: false,
            ask: None,
            runs_on: RunsOn::Any,
            repo: String::new(),
            account: String::new(),
            graph: None,
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
        assert_eq!(run.input, "ENG-7: Broken button\nhttps://linear.app/x/issue/ENG-7\n\n(no description)");
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
            paged: BTreeMap::new(),
            proof: false,
            ask: None,
            runs_on: RunsOn::Any,
            repo: String::new(),
            account: String::new(),
            graph: None,
        }
    }

    fn thread_run(mentioned: &str, picked_up: &str) -> Run {
        connector_thread_run(&slack_connector(), mentioned, picked_up)
    }

    fn connector_thread_run(c: &Connector, mentioned: &str, picked_up: &str) -> Run {
        let at = |t: &str| DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc);
        let mention = slack::Mention { thread: "1792400050.000200".into(), ts: "1792400050.000200".into(), at: at(mentioned) };
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
                "Triage the thread.\n\nAnswer in the Slack thread below by running `'/bin/pinned' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. Keep every answer short. Before each `slack reply`, invoke the `pinned:bro` skill on your draft and post the message it gives you, not the draft. Later messages in the thread reach you only when one of them mentions you, as `[Slack reply from <name> in the thread]` blocks with every message written since the last one you got; answer them in the thread the same way.\n\nThis workflow only triages: never change code, and never file or take a Linear issue. `pinned linear create` and `pinned linear take` are turned off for it.\n\nSlack thread in #support, oldest first:\nana: @pinned export 500s since today\nsee the log\nbruno: same here",
            ]
        );
        assert_eq!(run.input, "ana: @pinned export 500s since today\nsee the log\nbruno: same here");
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
            "Fix what you can.\n\nAnswer in the Slack thread below by running `'/bin/pinned' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. Keep every answer short. Before each `slack reply`, invoke the `pinned:bro` skill on your draft and post the message it gives you, not the draft. Later messages in the thread reach you only when one of them mentions you, as `[Slack reply from <name> in the thread]` blocks with every message written since the last one you got; answer them in the thread the same way.\n\n\
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
            "Fix what you can.\n\nAnswer in the Slack thread below by running `'/bin/pinned' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. Keep every answer short. Before each `slack reply`, invoke the `pinned:bro` skill on your draft and post the message it gives you, not the draft. Later messages in the thread reach you only when one of them mentions you, as `[Slack reply from <name> in the thread]` blocks with every message written since the last one you got; answer them in the thread the same way.\n\n\
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
        assert!(store.vercel.is_none(), "a store saved before the Vercel AI Gateway has no account for it");
        assert_eq!(store.connectors[0].paged, BTreeMap::new(), "a store saved before pages loads with none");
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

    const THREAD: &str = "C0123ABCD:1792400050.000200";
    const ROUTED: &str = "Triage the thread.\n\nAnswer in the Slack thread below by running `'/bin/pinned' slack reply \"<message>\"`. Run it once per answer, and write the message as plain Slack mrkdwn. Keep every answer short. Before each `slack reply`, invoke the `pinned:bro` skill on your draft and post the message it gives you, not the draft. Later messages in the thread reach you only when one of them mentions you, as `[Slack reply from <name> in the thread]` blocks with every message written since the last one you got; answer them in the thread the same way.\n\nThis workflow only triages: never change code, and never file or take a Linear issue. `pinned linear create` and `pinned linear take` are turned off for it.\n\nSlack thread in #support, oldest first:\nana: @pinned export 500s since today\nsee the log\nbruno: same here\n\nWorkflow \"Support router\" handed you this thread: its decision model put the first message in the category \"error\".";

    fn category(name: &str, description: &str, workflow: Option<&str>, mention: Option<&str>) -> Category {
        Category { name: name.into(), description: description.into(), workflow: workflow.map(Into::into), mention: mention.map(Into::into) }
    }

    fn router() -> Connector {
        let decide = Decide {
            model: "typesafe-ai/jev".into(),
            instructions: "Categorize this Slack message.".into(),
            categories: vec![
                category("error", "Someone reports a bug or an error", Some("c2"), None),
                category("doubt", "Someone asks a question", Some("c6"), None),
                category("escalation", "Someone needs an engineer on support now", None, Some("prod-engg")),
                category("ignore", "Anything else", None, None),
            ],
            fallback: None,
        };
        Connector {
            id: "c5".into(),
            name: "Support router".into(),
            trigger: Trigger::SlackMessage { channel: "#support".into(), decide },
            since: "2026-10-19T10:00:00+00:00".into(),
            ..slack_connector()
        }
    }

    fn called() -> Connector {
        Connector { id: "c6".into(), name: "Answer".into(), trigger: Trigger::Called, ..slack_connector() }
    }

    fn routed(category: &str, workflow: &str, page: Option<Paging>) -> Found {
        Found::Routed {
            channel: "C0123ABCD".into(),
            channel_name: "support".into(),
            thread: "1792400050.000200".into(),
            category: category.into(),
            workflow: workflow.into(),
            note: routed_note("Support router", category),
            posts: posts().into(),
            page,
        }
    }

    fn stopped(page: Option<Paging>) -> Found {
        Found::Stopped { channel: "C0123ABCD".into(), thread: "1792400050.000200".into(), page }
    }

    fn paging() -> Paging {
        let escalation = category("escalation", " Someone needs an engineer on support now ", None, Some("prod-engg"));
        Paging { ping: page_text("S0123ABC", &escalation), handle: "prod-engg".into() }
    }

    fn escalated(channel: &str, thread: &str) -> Found {
        Found::Stopped { channel: channel.into(), thread: thread.into(), page: Some(paging()) }
    }

    fn mentioned() -> Found {
        let at = DateTime::parse_from_rfc3339("2026-10-19T10:00:00Z").unwrap().with_timezone(&Utc);
        let mention = slack::Mention { thread: "1792400050.000200".into(), ts: "1792400050.000200".into(), at };
        Found::Mention { channel: "C0123ABCD".into(), channel_name: "support".into(), mention, posts: posts().into() }
    }

    fn checked_at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-19T10:01:00Z").unwrap().with_timezone(&Utc)
    }

    fn settle(store: &mut Store, found: Vec<(&str, Found)>) -> Settled {
        settle_at(store, found, checked_at())
    }

    fn settle_at(store: &mut Store, found: Vec<(&str, Found)>, now: DateTime<Utc>) -> Settled {
        let checked: Vec<String> = found.iter().map(|(id, _)| id.to_string()).collect();
        store.settle(&checked, found.into_iter().map(|(id, f)| (id.to_owned(), f)).collect(), now, "/bin/pinned", &[])
    }

    fn runs(settled: &Settled) -> (Vec<(String, Subject, Vec<String>)>, String) {
        let mut prompt = String::new();
        let runs = settled
            .work
            .iter()
            .map(|w| match w {
                Work::Run(run) => {
                    prompt = run.args.last().unwrap().clone();
                    (run.connector.clone(), run.subject.clone(), run.mentioned.clone())
                }
                Work::Delivery(d) => panic!("a delivery for {}", d.connector),
                Work::Prompt { .. } => panic!("a graph prompt is not a settled run"),
            })
            .collect();
        (runs, prompt)
    }

    fn thread_subject() -> Subject {
        Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() }
    }

    #[test]
    fn a_routed_message_starts_a_session_of_the_workflow_its_category_names() {
        let mut store = Store { connectors: vec![router(), slack_connector(), called()], ..Store::default() };
        let settled = settle(&mut store, vec![("c5", routed("error", "c2", None))]);
        let (started, prompt) = runs(&settled);
        assert_eq!(started, [("c2".to_owned(), thread_subject(), vec!["1792400050.000200".to_owned()])]);
        assert_eq!(prompt, ROUTED);
        assert_eq!((settled.errors.len(), settled.pages.len(), settled.needs_save), (0, 0, true));
        let [kept, target, _] = &store.connectors[..] else { unreachable!() };
        assert_eq!(target.handled, [THREAD], "the target owns the thread");
        assert_eq!(target.followed[THREAD], Followed { cursor: "1792400060.000000".into(), seen: None, replied: false });
        assert_eq!((kept.handled.len(), kept.followed.len()), (0, 0), "the router keeps nothing it handed on");
        assert_eq!(kept.since, "2026-10-19T10:00:00+00:00", "the watermark waits until a session takes the message");

        let mut store = Store { connectors: vec![router(), slack_connector(), called()], ..Store::default() };
        let (started, _) = runs(&settle(&mut store, vec![("c5", routed("doubt", "c6", None))]));
        assert_eq!(started, [("c6".to_owned(), thread_subject(), vec!["1792400050.000200".to_owned()])], "a called workflow takes it too");
        assert_eq!(store.connectors[2].handled, [THREAD]);
    }

    #[test]
    fn a_category_without_a_workflow_stops_on_the_router() {
        let mut store = Store { connectors: vec![router(), slack_connector()], ..Store::default() };
        let settled = settle(&mut store, vec![("c5", stopped(None))]);
        assert_eq!((settled.work.len(), settled.pages, settled.errors.len()), (0, vec![], 0));
        assert_eq!(store.connectors[0].handled, [THREAD]);
        assert!(!settle(&mut store, vec![("c5", stopped(None))]).needs_save, "a stopped thread is recorded once");
    }

    #[test]
    fn a_category_that_mentions_a_group_pages_it_once() {
        let ping = "<!subteam^S0123ABC> escalation: Someone needs an engineer on support now";
        assert_eq!(paging().ping, ping);

        let mut store = Store { connectors: vec![router(), slack_connector()], ..Store::default() };
        let settled = settle(&mut store, vec![("c5", stopped(Some(paging())))]);
        let posted = Page { channel: "C0123ABCD".into(), thread: "1792400050.000200".into(), reply: Reply::Ping(ping.into()), react: true };
        assert_eq!((settled.work.len(), settled.pages, settled.errors.len()), (0, vec![posted], 0), "it stops, pages and reacts");
        assert_eq!(store.connectors[0].handled, [THREAD]);
        let pinged = Paged { at: "2026-10-19T10:01:00+00:00".into(), ping: true };
        assert_eq!(store.connectors[0].paged, BTreeMap::from([(THREAD.to_owned(), pinged)]));
        assert_eq!(settle(&mut store, vec![("c5", stopped(Some(paging())))]).pages, [], "the next check pages no one again");

        let mut store = Store { connectors: vec![router(), slack_connector()], ..Store::default() };
        let settled = settle(&mut store, vec![("c5", routed("error", "c2", Some(paging())))]);
        let posted = Page { channel: "C0123ABCD".into(), thread: "1792400050.000200".into(), reply: Reply::Ping(ping.into()), react: false };
        assert_eq!((runs(&settled).0.len(), settled.pages), (1, vec![posted]), "the session reacts once a tab takes it");
        assert_eq!(store.connectors[0].paged.len(), 1, "the router records the page of a thread it handed on");
    }

    #[test]
    fn a_thread_sorted_again_is_not_paged_again() {
        let (connectors, dir) = stored("paged-once", vec![router(), slack_connector()]);
        let settled = settle(&mut connectors.store.lock().unwrap(), vec![("c5", routed("escalation", "c2", Some(paging())))]);
        assert_eq!(settled.pages.len(), 1);
        let work = settled.work.into_iter().next().unwrap();
        assert!(block_on(connectors.hand_over(work, &|_| async { Err::<(), String>("no tab took it".to_owned()) })).is_err());
        let mut store = connectors.store.lock().unwrap();
        let later = checked_at() + chrono::Duration::hours(2);
        let settled = settle_at(&mut store, vec![("c5", routed("escalation", "c2", Some(paging())))], later);
        assert_eq!((runs(&settled).0.len(), settled.pages), (1, vec![]), "the thread starts a session again but pages no one");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_category_that_pages_wins_only_when_the_model_is_sure() {
        let Trigger::SlackMessage { decide, .. } = router().trigger else { unreachable!() };
        let picked = |index, probabilities: [f64; 4]| {
            decide.pick(&vercel::Chosen { index, probabilities: probabilities.into() }).category().map(|i| decide.categories[i].name.clone())
        };
        assert_eq!(picked(2, [0.03, 0.10, 0.84, 0.03]), Some("doubt".into()), "the likeliest category that pages no one");
        assert_eq!(picked(2, [0.04, 0.03, 0.90, 0.03]), Some("escalation".into()));
        assert_eq!(picked(2, [0.05, 0.05, 0.85, 0.05]), Some("escalation".into()));
        assert_eq!(picked(0, [0.40, 0.30, 0.20, 0.10]), Some("error".into()), "a category that pages no one needs no certainty");
        assert_eq!(picked(2, [0.0, 0.0, 0.0, 0.0]), Some("error".into()), "an answer without probabilities pages no one");

        let paging_only = Decide { categories: vec![category("escalation", "Production is down", None, Some("prod-engg"))], ..decide.clone() };
        assert_eq!(paging_only.pick(&vercel::Chosen { index: 0, probabilities: vec![0.6] }), Pick::Nowhere);
        assert_eq!(paging_only.pick(&vercel::Chosen { index: 0, probabilities: vec![0.9] }), Pick::Sure(0));
    }

    #[test]
    fn escalations_in_a_channel_page_once_per_cooldown() {
        let mut store = Store { connectors: vec![router(), slack_connector()], ..Store::default() };
        let at = |minutes| checked_at() + chrono::Duration::minutes(minutes);
        let page = |channel: &str, thread: &str, reply, react| Page { channel: channel.into(), thread: thread.into(), reply, react };
        let ping = || Reply::Ping("<!subteam^S0123ABC> escalation: Someone needs an engineer on support now".into());
        let already = || Reply::Already { handle: "prod-engg".into(), thread: "1792399000.000000".into() };

        let settled = settle_at(&mut store, vec![("c5", escalated("C0123ABCD", "1792399000.000000"))], at(0));
        assert_eq!(settled.pages, [page("C0123ABCD", "1792399000.000000", ping(), true)]);

        let settled = settle_at(&mut store, vec![("c5", escalated("C0123ABCD", "1792399600.000000"))], at(10));
        assert_eq!(settled.pages, [page("C0123ABCD", "1792399600.000000", already(), true)], "it points at the page and reacts");
        let text = already_text("prod-engg", "https://acme.slack.com/archives/C0123ABCD/p1792399000000000");
        assert_eq!(text, "Already escalated to @prod-engg: https://acme.slack.com/archives/C0123ABCD/p1792399000000000");
        assert_eq!(already_text("@prod-engg", "https://x"), "Already escalated to @prod-engg: https://x");

        let settled = settle_at(&mut store, vec![("c5", routed("escalation", "c2", Some(paging())))], at(20));
        assert_eq!(runs(&settled).0.len(), 1, "a category with a workflow still hands the thread on");
        assert_eq!(settled.pages, [page("C0123ABCD", "1792400050.000200", already(), false)]);

        let settled = settle_at(&mut store, vec![("c5", escalated("C0999WXYZ", "1792400100.000000"))], at(21));
        assert_eq!(settled.pages, [page("C0999WXYZ", "1792400100.000000", ping(), true)], "another channel is another incident");

        let settled = settle_at(&mut store, vec![("c5", escalated("C0123ABCD", "1792400900.000000"))], at(31));
        assert_eq!(settled.pages, [page("C0123ABCD", "1792400900.000000", ping(), true)], "after the cooldown it pages again");

        let router = &store.connectors[0];
        assert_eq!(router.handled, ["C0123ABCD:1792399000.000000", "C0123ABCD:1792399600.000000", "C0999WXYZ:1792400100.000000", "C0123ABCD:1792400900.000000"]);
        assert_eq!(
            router.paged.iter().map(|(key, paged)| (key.as_str(), paged.ping)).collect::<Vec<_>>(),
            [
                ("C0123ABCD:1792399000.000000", true),
                ("C0123ABCD:1792399600.000000", false),
                ("C0123ABCD:1792400050.000200", false),
                ("C0123ABCD:1792400900.000000", true),
                ("C0999WXYZ:1792400100.000000", true),
            ],
            "every escalation is recorded, and the cooldown runs from the pings"
        );
    }

    #[test]
    fn a_thread_told_it_was_already_escalated_is_told_once() {
        let (connectors, dir) = stored("already-once", vec![router(), slack_connector()]);
        let at = |minutes| checked_at() + chrono::Duration::minutes(minutes);
        let already = |thread: &str| Reply::Already { handle: "prod-engg".into(), thread: thread.into() };
        let replies = |settled: &Settled| settled.pages.iter().map(|p| (p.thread.clone(), p.reply.clone())).collect::<Vec<_>>();

        let settled = settle_at(&mut connectors.store.lock().unwrap(), vec![("c5", escalated("C0123ABCD", "1792399000.000000"))], at(0));
        assert_eq!(settled.pages.len(), 1);
        let settled = settle_at(&mut connectors.store.lock().unwrap(), vec![("c5", routed("escalation", "c2", Some(paging())))], at(5));
        assert_eq!(replies(&settled), [("1792400050.000200".to_owned(), already("1792399000.000000"))]);
        let work = settled.work.into_iter().next().unwrap();
        assert!(block_on(connectors.hand_over(work, &|_| async { Err::<(), String>("no tab took it".to_owned()) })).is_err());

        let mut store = connectors.store.lock().unwrap();
        let settled = settle_at(&mut store, vec![("c5", routed("escalation", "c2", Some(paging())))], at(8));
        assert_eq!((runs(&settled).0.len(), replies(&settled)), (1, vec![]), "the thread sorted again starts a session and posts nothing");
        assert_eq!(store.connectors[0].paged[THREAD], Paged { at: "2026-10-19T10:06:00+00:00".into(), ping: false });

        let settled = settle_at(&mut store, vec![("c5", escalated("C0123ABCD", "1792400300.000000"))], at(12));
        assert_eq!(replies(&settled), [("1792400300.000000".to_owned(), already("1792399000.000000"))], "the link points at the thread that was pinged");
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_target_that_cannot_answer_the_thread_stops_it_on_the_router_with_an_error() {
        let off = Connector { enabled: false, ..slack_connector() };
        let linear = Connector { id: "c2".into(), name: "Resolve".into(), ..linear_connector() };
        let cases = [
            (vec![router()], "workflow \"Support router\" hands \"error\" messages to a workflow that was deleted: pick another one for the category in its settings"),
            (vec![router(), off], "workflow \"Support router\" hands \"error\" messages to workflow \"Triage\", which is off: turn it on, or pick another one for the category"),
            (vec![router(), linear], "workflow \"Support router\" hands \"error\" messages to workflow \"Resolve\", which does not answer Slack threads: pick a Slack mention or called workflow for the category"),
        ];
        for (connectors, error) in cases {
            let mut store = Store { connectors, ..Store::default() };
            let settled = settle(&mut store, vec![("c5", routed("error", "c2", None))]);
            assert_eq!((settled.work.len(), settled.errors), (0, vec![error.to_owned()]));
            assert_eq!(store.connectors[0].handled, [THREAD], "{error}");
            assert!(store.connectors[1..].iter().all(|c| c.handled.is_empty()), "{error}");
            assert_eq!(settle(&mut store, vec![("c5", routed("error", "c2", None))]).errors, Vec::<String>::new(), "reported once");
        }
        let mut store = Store { connectors: vec![router(), Connector { id: "c7".into(), ..router() }], ..Store::default() };
        let settled = settle(&mut store, vec![("c5", routed("error", "c7", None))]);
        assert_eq!(
            settled.errors,
            ["workflow \"Support router\" hands \"error\" messages to workflow \"Support router\", which does not answer Slack threads: pick a Slack mention or called workflow for the category"]
        );
    }

    #[test]
    fn a_message_that_mentions_the_bot_and_is_routed_starts_one_session() {
        let watching = Connector { id: "c3".into(), name: "Mentions".into(), ..slack_connector() };
        let store = || Store { connectors: vec![router(), slack_connector(), watching.clone()], ..Store::default() };

        let mut routed_first = store();
        let settled = settle(&mut routed_first, vec![("c5", routed("error", "c2", None)), ("c3", mentioned())]);
        assert_eq!(runs(&settled).0, [("c2".to_owned(), thread_subject(), vec!["1792400050.000200".to_owned()])]);
        assert_eq!(routed_first.connectors[2].handled.len(), 0);

        let mut mention_first = store();
        let settled = settle(&mut mention_first, vec![("c3", mentioned()), ("c5", routed("error", "c2", None))]);
        assert_eq!(runs(&settled).0, [("c3".to_owned(), thread_subject(), vec!["1792400050.000200".to_owned()])]);
        assert_eq!(mention_first.connectors[1].handled.len(), 0);
        assert_eq!(runs(&settle(&mut mention_first, vec![("c5", routed("error", "c2", None))])).0, [], "and later checks start none");
    }

    #[test]
    fn a_routed_run_no_tab_took_is_classified_again() {
        let (connectors, dir) = stored("routed-unclaim", vec![router(), slack_connector()]);
        let settled = settle(&mut connectors.store.lock().unwrap(), vec![("c5", routed("error", "c2", None))]);
        let work = settled.work.into_iter().next().unwrap();
        assert_eq!(
            block_on(connectors.hand_over(work, &|_| async { Err::<(), String>("no tab took it".to_owned()) })),
            Err("could not open a session in /repo: no tab took it; the next check tries again".into())
        );
        let mut store = connectors.store.lock().unwrap();
        assert!(!store.handled(THREAD), "the router's next check asks the decision model again");
        assert_eq!(store.connectors[0].since, "2026-10-19T10:00:00+00:00", "and still sees the message");
        let (started, _) = runs(&settle(&mut store, vec![("c5", routed("error", "c2", None))]));
        assert_eq!(started, [("c2".to_owned(), thread_subject(), vec!["1792400050.000200".to_owned()])]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_workflow_follows_up_on_threads_outside_the_channel_it_watches() {
        let at = |cursor: &str| Followed { cursor: cursor.into(), seen: None, replied: false };
        let followed = BTreeMap::from([
            ("C0123ABCD:1792400050.000200".to_owned(), at("1792400060.000000")),
            ("C0123ABCD:1792400070.000000".to_owned(), Followed { seen: Some("1792400090.000000".into()), ..at("1792400080.000000") }),
            ("C0999WXYZ:1792400100.000000".to_owned(), at("1792400100.000000")),
        ]);
        assert_eq!(followed_channels(&followed, Some("C0123ABCD")), ["C0999WXYZ"]);
        assert_eq!(followed_channels(&followed, None), ["C0123ABCD", "C0999WXYZ"], "a called workflow follows every thread it was handed");
        let read: Vec<(String, String, String)> =
            cursors(&followed, "C0123ABCD").into_iter().map(|(thread, c)| (thread, c.given, c.read)).collect();
        assert_eq!(
            read,
            [
                ("1792400050.000200".to_owned(), "1792400060.000000".to_owned(), "1792400060.000000".to_owned()),
                ("1792400070.000000".to_owned(), "1792400080.000000".to_owned(), "1792400090.000000".to_owned()),
            ]
        );
    }

    #[test]
    fn a_delivery_no_tab_took_needs_a_workflow_that_answers_threads() {
        let delivery = Delivery {
            connector: "c5".into(),
            channel: "C0123ABCD".into(),
            thread: "1792400050.000200".into(),
            messages: vec!["any news?".into()],
            cursor: "1792400090.000000".into(),
            mentioned: vec![],
        };
        let (connectors, dir) = stored("fallback", vec![router(), called()]);
        assert_eq!(block_on(connectors.fallback(&delivery)).err(), Some("the workflow no longer answers Slack threads".into()));
        let called = Delivery { connector: "c6".into(), ..delivery };
        assert_eq!(block_on(connectors.fallback(&called)).err(), Some("Slack is not connected".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn message_and_called_triggers_round_trip() {
        let message = serde_json::json!({
            "kind": "slack-message", "channel": "support",
            "decide": { "model": "typesafe-ai/jev", "instructions": "Categorize this Slack message.", "categories": [
                { "name": "error", "description": "Someone reports a bug or an error", "workflow": "c1793" },
                { "name": "escalation", "description": "Someone needs an engineer on support now", "workflow": null, "mention": "prod-engg" },
                { "name": "ignore", "description": "Anything else", "workflow": null },
            ] },
        });
        let trigger: Trigger = serde_json::from_value(message).unwrap();
        let decide = Decide {
            model: "typesafe-ai/jev".into(),
            instructions: "Categorize this Slack message.".into(),
            categories: vec![
                category("error", "Someone reports a bug or an error", Some("c1793"), None),
                category("escalation", "Someone needs an engineer on support now", None, Some("prod-engg")),
                category("ignore", "Anything else", None, None),
            ],
            fallback: None,
        };
        assert_eq!(trigger, Trigger::SlackMessage { channel: "support".into(), decide });
        assert_eq!(
            serde_json::to_value(&trigger).unwrap()["decide"]["categories"][2],
            serde_json::json!({ "name": "ignore", "description": "Anything else", "workflow": null, "mention": null })
        );
        let json = serde_json::to_value(&trigger).unwrap();
        assert_eq!(serde_json::from_value::<Trigger>(json).unwrap(), trigger);
        assert_eq!(serde_json::to_value(Trigger::Called).unwrap(), serde_json::json!({ "kind": "called" }));
        assert_eq!(serde_json::from_value::<Trigger>(serde_json::json!({ "kind": "called" })).unwrap(), Trigger::Called);
        assert!(serde_json::from_value::<Trigger>(serde_json::json!({ "kind": "slack-message", "channel": "support" })).is_err(), "a message trigger needs its decision");
    }

    #[test]
    fn a_fallback_round_trips_and_defaults_to_the_models_guess() {
        let Trigger::SlackMessage { decide, .. } = router().trigger else { unreachable!() };
        let json = serde_json::to_value(&decide).unwrap();
        assert_eq!(json["fallback"], serde_json::Value::Null);
        let mut absent = json;
        absent.as_object_mut().unwrap().remove("fallback");
        assert_eq!(serde_json::from_value::<Decide>(absent).unwrap(), decide, "a decision saved before the fallback keeps the model's guess");

        let with = Decide { fallback: Some("doubt".into()), ..decide };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["fallback"], serde_json::json!("doubt"));
        assert_eq!(serde_json::from_value::<Decide>(json).unwrap(), with);
    }

    #[test]
    fn an_unsure_answer_goes_to_the_fallback() {
        let Trigger::SlackMessage { decide, .. } = router().trigger else { unreachable!() };
        let routed = |decide: &Decide, index, probabilities: [f64; 4]| {
            let pick = decide.pick(&vercel::Chosen { index, probabilities: probabilities.into() });
            (pick.category().map(|i| decide.categories[i].name.clone()), pick.note(&decide.categories, "Support router"))
        };
        let sure = |category: &str| (Some(category.to_owned()), routed_note("Support router", category));
        let with = Decide { fallback: Some("doubt".into()), ..decide.clone() };

        assert_eq!(
            routed(&with, 0, [0.55, 0.30, 0.10, 0.05]),
            (
                Some("doubt".into()),
                "\n\nThe decision model was unsure (it guessed \"error\" at 55%), so workflow \"Support router\" handed this thread over as \"doubt\".".into()
            )
        );
        assert_eq!(routed(&with, 0, [0.70, 0.20, 0.05, 0.05]), sure("error"));
        assert_eq!(routed(&with, 0, [0.60, 0.30, 0.05, 0.05]), sure("error"), "60% is sure enough");
        assert_eq!(routed(&decide, 0, [0.55, 0.30, 0.10, 0.05]), sure("error"), "without a fallback the guess stands");
        assert_eq!(
            routed(&with, 2, [0.10, 0.20, 0.52, 0.18]).1,
            "\n\nThe decision model was unsure (it guessed \"escalation\" at 52%), so workflow \"Support router\" handed this thread over as \"doubt\".",
            "an unsure guess that pages goes to the fallback, not the likeliest quiet category"
        );
        let to_ignore = Decide { fallback: Some("ignore".into()), ..decide };
        assert_eq!(routed(&to_ignore, 2, [0.20, 0.25, 0.52, 0.03]).0, Some("ignore".into()));
        assert_eq!(routed(&to_ignore, 2, [0.03, 0.10, 0.84, 0.03]), sure("doubt"), "a sure but not certain page still goes to the likeliest quiet category");
    }

    #[test]
    fn a_fallback_must_be_a_category_that_pings_no_one() {
        let Trigger::SlackMessage { decide, .. } = router().trigger else { unreachable!() };
        let check = |fallback: &str| Decide { fallback: Some(fallback.into()), ..decide.clone() }.choices("Support router").map(|c| c.len());
        assert_eq!(check("doubt"), Ok(4));
        assert_eq!(
            check("escalation"),
            Err("workflow \"Support router\" sends the messages its decision model is unsure about to \"escalation\", which pings a group: the fallback can't ping a group, so pick a category without a mention in its settings".into())
        );
        assert_eq!(
            check("spam"),
            Err("workflow \"Support router\" sends the messages its decision model is unsure about to \"spam\", which is not one of its categories: pick another one in its settings".into())
        );
    }

    #[test]
    fn a_decision_needs_a_model_and_categories() {
        let Trigger::SlackMessage { decide, .. } = router().trigger else { unreachable!() };
        assert_eq!(decide.choices("Support router").unwrap()[2], ("escalation", "Someone needs an engineer on support now"));
        assert_eq!(
            Decide { model: " ".into(), ..decide.clone() }.choices("Support router"),
            Err("workflow \"Support router\" has no decision model: pick one in its settings".into())
        );
        assert_eq!(
            Decide { categories: vec![], ..decide }.choices("Support router"),
            Err("workflow \"Support router\" has no categories to sort messages into: add one in its settings".into())
        );
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

    #[test]
    fn run_on_loads_any_machine_or_box_from_the_machine_key_and_saves_it_back_as_it_was() {
        let base = serde_json::to_value(linear_connector()).unwrap();
        for (stored, runs_on) in [
            (serde_json::json!(""), RunsOn::Any),
            (serde_json::json!("m1"), RunsOn::Machine("m1".into())),
            (serde_json::json!({ "kind": "upstash-box", "size": "medium" }), RunsOn::FreshBox(BoxStep { size: BoxSize::Medium })),
            (serde_json::json!({ "kind": "upstash-box", "size": "large" }), RunsOn::FreshBox(BoxStep { size: BoxSize::Large })),
        ] {
            let mut saved = base.clone();
            saved["machine"] = stored.clone();
            let text = serde_json::to_string(&saved).unwrap();
            let loaded: Connector = serde_json::from_str(&text).unwrap();
            assert_eq!(loaded.runs_on, runs_on);
            assert_eq!(serde_json::to_string(&loaded).unwrap(), text, "{stored} saves back byte for byte");
        }
        let mut sizeless = base.clone();
        sizeless["machine"] = serde_json::json!({ "kind": "upstash-box" });
        assert_eq!(serde_json::from_value::<Connector>(sizeless).unwrap().runs_on, RunsOn::FreshBox(BoxStep { size: BoxSize::Medium }));
        let mut unknown = base;
        unknown["machine"] = serde_json::json!({ "kind": "lambda" });
        assert!(serde_json::from_value::<Connector>(unknown).is_err());
    }

    #[test]
    fn a_store_saved_by_v0_12_still_loads() {
        let saved = r#"{
          "linear": { "name": "Felipe", "email": "f@x.com", "organization": "X" },
          "github": { "login": "felipe", "name": "Felipe" },
          "connectors": [{
            "id": "c1", "name": "Resolve", "enabled": true,
            "trigger": { "kind": "linear-label", "label": "claude" },
            "folder": "/repo", "model": "opus", "effort": "high", "permissionMode": "auto", "worktree": true,
            "instructions": "Fix it.", "columns": { "picked": "Analyzing", "coding": "", "review": "" },
            "since": "2026-10-01T00:00:00+00:00", "handled": ["uuid"], "proof": true,
            "machine": "7c0d2a9e-1111-4222-8333-944455556666", "repo": "acme/app", "account": "work"
          }]
        }"#;
        let store: Store = serde_json::from_str(saved).unwrap();
        assert!(store.upstash.is_none());
        assert_eq!(store.connectors[0].runs_on, RunsOn::Machine("7c0d2a9e-1111-4222-8333-944455556666".into()));
        assert_eq!(serde_json::to_value(&store.connectors[0]).unwrap()["machine"], "7c0d2a9e-1111-4222-8333-944455556666");
    }

    #[test]
    fn a_box_workflows_run_goes_to_a_box_with_the_box_note_and_no_account() {
        let boxed = Connector { runs_on: RunsOn::FreshBox(BoxStep::default()), account: "work".into(), proof: true, ..linear_connector() };
        let box_run = run(&boxed, &issue(), "/bin/pinned");
        assert_eq!(box_run.runs_on, RunsOn::FreshBox(BoxStep { size: BoxSize::Medium }));
        assert_eq!(box_run.account, None, "a box signs in only with the Claude token");
        let prompt = box_run.args.last().unwrap();
        assert!(prompt.contains(&format!("{PROOF}{BOX_NOTE}\n\nLinear issue ENG-7")), "{prompt}");

        let on_machine = Connector { runs_on: RunsOn::Machine("m1".into()), proof: true, ..linear_connector() };
        let machine_run = run(&on_machine, &issue(), "/bin/pinned");
        assert_eq!((machine_run.runs_on, machine_run.account), (RunsOn::Machine("m1".into()), None));
        assert_eq!(machine_run.args.last().unwrap().replace(BOX_NOTE, ""), *machine_run.args.last().unwrap(), "a machine's prompt has no box note");
        assert_eq!(*machine_run.args.last().unwrap(), prompt.replace(BOX_NOTE, ""));

        let thread = |runs_on| connector_thread_run(&Connector { runs_on, ..fixing_connector() }, "2026-10-19T10:00:00Z", "2026-10-19T10:01:00Z").args.pop().unwrap();
        let in_box = thread(RunsOn::FreshBox(BoxStep::default()));
        assert!(in_box.contains(&format!("{BOX_NOTE}\n\nSlack thread in #support")), "{in_box}");
        assert_eq!(in_box.replace(BOX_NOTE, ""), thread(RunsOn::Any));
    }

    #[test]
    fn upstash_keys_stay_in_storage_and_only_their_hints_reach_settings() {
        let (connectors, dir) = stored("upstash", vec![]);
        assert!(connectors.box_keys().is_none());
        let keys = upstash::Keys::parse("box_abcdA1b2", "sk-ant-oat01-zz9f3c").unwrap();
        assert_eq!(connectors.connect_upstash(&keys).unwrap(), upstash::Account { key_hint: "…A1b2".into(), claude_hint: "…9f3c".into() });
        let kept = connectors.box_keys().unwrap();
        assert_eq!((kept.api_key.as_str(), kept.claude_token.as_str()), ("box_abcdA1b2", "sk-ant-oat01-zz9f3c"));
        let settings = serde_json::to_string(&connectors.get_settings()).unwrap();
        assert!(settings.contains("\"upstash\":{\"key_hint\":\"…A1b2\",\"claude_hint\":\"…9f3c\"}"), "{settings}");
        assert!(!settings.contains("box_abcd") && !settings.contains("sk-ant-oat01"), "{settings}");
        connectors.disconnect_upstash().unwrap();
        assert!(connectors.box_keys().is_none());
        assert!(connectors.get_settings().upstash.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn on_disk(dir: &std::path::Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(dir.join("connectors.json")).unwrap()).unwrap()
    }

    fn calling(workflow: &str) -> crate::graph::Graph {
        crate::graph::Graph {
            nodes: vec![
                Node { id: NodeId("trigger".into()), name: "Trigger".into(), position: [0, 0], kind: NodeKind::Trigger { watch: Watch::Called } },
                Node { id: NodeId("route".into()), name: "Route".into(), position: [240, 0], kind: NodeKind::Call { workflow: workflow.into() } },
            ],
            edges: vec![Edge { from: NodeId("trigger".into()), port: PortId("out".into()), to: NodeId("route".into()) }],
        }
    }

    #[test]
    fn two_new_workflows_saved_in_one_millisecond_keep_distinct_ids() {
        let start = chrono::Utc::now().timestamp_millis();
        let occupied = (0..500)
            .map(|offset| {
                let mut connector = slack_connector();
                connector.id = format!("c{}", start + offset);
                connector.name = format!("keep-{offset}");
                connector
            })
            .collect();
        let (connectors, dir) = stored("fresh-id", occupied);
        let mut incoming = slack_connector();
        incoming.id.clear();
        incoming.name = "New".into();
        let saved = connectors.save_connector(incoming).unwrap();
        let on_disk = on_disk(&dir)["connectors"].as_array().unwrap().clone();
        let same = on_disk.iter().filter(|connector| connector["id"] == saved.id).count();
        assert_eq!(same, 1, "id {} shared by {same} workflows", saved.id);
        assert!(on_disk.iter().any(|connector| connector["name"] == "New"));
        assert!(on_disk.iter().any(|connector| connector["name"] == "keep-0"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_read_sends_a_graph_and_leaves_the_store_without_one() {
        let (connectors, dir) = stored("send-graph", vec![slack_connector()]);
        let sent = connectors.get_settings().connectors;
        assert_eq!(sent[0].graph, Some(migrate(&slack_connector())));
        assert!(on_disk(&dir)["connectors"][0].get("graph").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_save_of_the_migrated_graph_stores_none_and_still_returns_one() {
        let (connectors, dir) = stored("collapse-graph", vec![router(), slack_connector(), called()]);
        let mut incoming = router();
        incoming.graph = Some(migrate(&router()));
        let saved = connectors.save_connector(incoming).unwrap();
        assert_eq!(saved.graph, Some(migrate(&router())));
        let stored = on_disk(&dir)["connectors"].as_array().unwrap().iter().find(|connector| connector["id"] == "c5").unwrap().clone();
        assert!(stored.get("graph").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_edited_graph_is_stored_and_its_legacy_fields_come_from_the_graph() {
        let (connectors, dir) = stored("refresh-graph", vec![]);
        let mut incoming = linear_connector();
        incoming.id.clear();
        let mut graph = migrate(&incoming);
        match &mut graph.nodes[0].kind {
            NodeKind::Trigger { watch: Watch::LinearLabel { label } } => *label = "bug".into(),
            _ => panic!("trigger"),
        }
        graph.nodes.iter_mut().for_each(|node| {
            if let NodeKind::Talk { prompt } = &mut node.kind {
                *prompt = "note".into();
            }
        });
        incoming.graph = Some(graph);
        connectors.save_connector(incoming).unwrap();
        let stored = &on_disk(&dir)["connectors"][0];
        assert_eq!(stored["trigger"]["label"], "bug");
        assert_eq!(stored["graph"]["nodes"][2]["prompt"], "note");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_untouched_graph_is_dropped_when_an_older_app_saves() {
        let mut stored_connector = slack_connector();
        stored_connector.graph = Some(migrate(&slack_connector()));
        let (connectors, dir) = stored("untouched-graph", vec![stored_connector]);
        let incoming = serde_json::json!({
            "id": "c2", "name": "Triage", "enabled": true,
            "trigger": { "kind": "slack-mention", "channel": "#bugs" },
            "folder": "/repo", "model": "", "effort": "high", "permissionMode": "plan", "worktree": false,
            "instructions": "Changed.",
        });
        let incoming: Connector = serde_json::from_value(incoming).unwrap();
        assert!(incoming.graph.is_none());
        connectors.save_connector(incoming).unwrap();
        let stored = &on_disk(&dir)["connectors"][0];
        assert!(stored.get("graph").is_none());
        assert_eq!(stored["instructions"], "Changed.");
        assert_eq!(stored["trigger"]["channel"], "#bugs");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_0_14_save_of_an_edited_graph_keeps_the_graph() {
        let mut incoming = slack_connector();
        let mut graph = migrate(&incoming);
        graph.nodes.push(Node { id: NodeId("extra".into()), name: "Extra".into(), position: [10, 10], kind: NodeKind::Stop });
        incoming.graph = Some(graph);
        let (connectors, dir) = stored("keep-edited", vec![]);
        let saved = connectors.save_connector(incoming).unwrap();
        let old = serde_json::json!({
            "id": saved.id, "name": "Renamed", "enabled": true,
            "trigger": { "kind": "slack-mention", "channel": "#other" },
            "folder": "/elsewhere", "model": "", "effort": "high", "permissionMode": "plan", "worktree": false,
            "instructions": "New instructions.",
        });
        let old: Connector = serde_json::from_value(old).unwrap();
        assert!(old.graph.is_none(), "0.14 json has no graph key");
        connectors.save_connector(old).unwrap();
        let stored = &on_disk(&dir)["connectors"][0];
        assert_eq!(stored["name"], "Renamed");
        assert_eq!(stored["instructions"], "New instructions.");
        assert_eq!(stored["folder"], "/elsewhere");
        assert_eq!(stored["trigger"]["channel"], "#support");
        assert!(stored["graph"]["nodes"].as_array().unwrap().iter().any(|node| node["id"] == "extra"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_save_rejects_an_invalid_graph_and_a_call_cycle() {
        let (connectors, dir) = stored("reject-graph", vec![slack_connector()]);
        let mut bad = slack_connector();
        bad.graph = Some(crate::graph::Graph { nodes: vec![], edges: vec![] });
        assert_eq!(connectors.save_connector(bad).err().as_deref(), Some("a workflow needs a trigger"));
        assert_eq!(on_disk(&dir)["connectors"][0]["name"], "Triage");
        assert!(on_disk(&dir)["connectors"][0].get("graph").is_none());

        let mut mention = slack_connector();
        mention.graph = Some(calling("c1"));
        assert_eq!(connectors.save_connector(mention).err().as_deref(), Some("\"Route\" calls \"c1\", which is not a called workflow"));
        let _ = std::fs::remove_dir_all(dir);

        let mut other = called();
        other.id = "c7".into();
        other.name = "Other".into();
        let (connectors, dir) = stored("cycle-graph", vec![called(), other]);
        let mut first = called();
        first.graph = Some(calling("c7"));
        connectors.save_connector(first).unwrap();
        let mut back = called();
        back.id = "c7".into();
        back.name = "Other".into();
        back.graph = Some(calling("c6"));
        assert_eq!(connectors.save_connector(back).err().as_deref(), Some("\"Route\" calls \"c7\", which calls back"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn talks(prompts: &[&str]) -> crate::graph::Graph {
        let mut nodes = vec![Node {
            id: NodeId("t".into()),
            name: "Trigger".into(),
            position: [0, 0],
            kind: NodeKind::Trigger { watch: Watch::LinearLabel { label: "claude".into() } },
        }];
        let mut edges = vec![];
        let mut prev = "t".to_owned();
        for (index, prompt) in prompts.iter().enumerate() {
            let id = format!("talk-{index}");
            nodes.push(Node {
                id: NodeId(id.clone()),
                name: format!("Talk {index}"),
                position: [(index + 1) as i32, 0],
                kind: NodeKind::Talk { prompt: (*prompt).into() },
            });
            edges.push(Edge { from: NodeId(prev), port: PortId("out".into()), to: NodeId(id.clone()) });
            prev = id;
        }
        crate::graph::Graph { nodes, edges }
    }

    fn issue_trigger() -> TriggerInput {
        let issue = issue();
        TriggerInput::Issue { id: issue.id, identifier: issue.identifier, title: issue.title, description: issue.description, url: issue.url }
    }

    fn slack_trigger(note: Option<String>) -> TriggerInput {
        TriggerInput::Slack {
            channel: "C0123ABCD".into(),
            channel_name: "support".into(),
            thread: "1792400050.000200".into(),
            posts: posts().into_iter().map(|post| TriggerPost { ts: post.ts, author: post.author, text: post.text }).collect(),
            note,
            mentioned: vec!["1792400050.000200".into()],
        }
    }

    #[test]
    fn graph_opening_matches_the_session_a_migrated_connector_opens_today() {
        let linear = linear_connector();
        let opened = graph_opening(&linear, &issue_trigger(), &[], "", None, "/bin/pinned").unwrap();
        let today = run(&linear, &issue(), "/bin/pinned");
        assert_eq!(opened.args, today.args);
        assert_eq!(opened.input, today.input);
        assert_eq!((opened.connector, opened.subject, opened.cwd), (today.connector, today.subject, today.cwd));

        let slack = slack_connector();
        let note = routed_note("Support router", "error");
        let opened = graph_opening(&slack, &slack_trigger(Some(note.clone())), &[], "", None, "/bin/pinned").unwrap();
        let mut today = slack_run(&slack, "C0123ABCD", "support", "1792400050.000200", &posts(), Some(&note), "/bin/pinned");
        today.mentioned = vec!["1792400050.000200".into()];
        assert_eq!(opened.args, today.args);
        assert_eq!(opened.args.last().unwrap(), ROUTED);
        assert_eq!(opened.input, today.input);

        let answers = [ClassifyAnswers { model: classifying().model.clone(), answers: vec![("kind".into(), "bug".into()), ("agent".into(), "yes".into())] }];
        let opened = graph_opening(&slack, &slack_trigger(None), &answers, "", None, "/bin/pinned").unwrap();
        let mut today = slack_run(&slack, "C0123ABCD", "support", "1792400050.000200", &posts(), None, "/bin/pinned");
        append_answers(&mut today, &classifying(), &[("kind".into(), "bug".into()), ("agent".into(), "yes".into())]);
        today.mentioned = vec!["1792400050.000200".into()];
        assert_eq!(opened.args, today.args);

        let opened = graph_opening(&linear, &issue_trigger(), &[], "look here", Some("HISTORY"), "/bin/pinned").unwrap();
        let prompt = opened.args.last().unwrap();
        assert!(prompt.starts_with("HISTORY\n\n"), "{prompt}");
        assert!(prompt.contains("ENG-7: Broken button"), "{prompt}");
        assert!(prompt.ends_with("\n\nlook here"), "{prompt}");
    }

    #[test]
    fn an_edited_graph_starts_a_run_and_a_fixed_pipeline_still_claims() {
        let mut graphed = linear_connector();
        graphed.graph = Some(talks(&[""]));
        let mut store = Store { connectors: vec![graphed], ..Store::default() };
        let settled = settle(&mut store, vec![("c1", Found::Issue(issue()))]);
        assert!(settled.work.is_empty());
        assert_eq!(settled.starts.len(), 1);
        assert_eq!(settled.starts[0].connector, "c1");
        assert!(store.connectors[0].handled.is_empty());

        let library = BTreeMap::from([("c1".into(), talks(&[""]))]);
        let open = runner::start(&library, "c1", Subject::LinearIssue { issue: "uuid".into() }, serde_json::json!({}), "t");
        let again = store.settle(&["c1".into()], vec![("c1".into(), Found::Issue(issue()))], checked_at(), "/bin/pinned", &[open]);
        assert!(again.starts.is_empty());

        let mut plain = Store { connectors: vec![linear_connector()], ..Store::default() };
        let settled = settle(&mut plain, vec![("c1", Found::Issue(issue()))]);
        assert_eq!(settled.work.len(), 1);
        assert!(settled.starts.is_empty());
        assert_eq!(plain.connectors[0].handled, ["uuid"]);
    }

    #[test]
    fn a_graph_run_is_stored_beside_the_connectors() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one"]));
        let (connectors, dir) = stored("graph-store", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        assert!(connectors.holds_box("c1", &Subject::LinearIssue { issue: "uuid".into() }));
        assert!(!connectors.holds_box("c2", &Subject::LinearIssue { issue: "uuid".into() }));
        drop(connectors);
        let store = std::fs::read_to_string(dir.join("connectors.json")).unwrap();
        assert!(!store.contains("2026-10-29T12:00:00Z"), "{store}");
        let saved: Vec<runner::Run> = serde_json::from_slice(&std::fs::read(dir.join("runs.json")).unwrap()).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].connector, "c1");
        assert!(matches!(saved[0].status, Status::Running));
        let loaded = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into());
        assert_eq!(loaded.runs.lock().unwrap().len(), 1);
        assert!(loaded.holds_box("c1", &Subject::LinearIssue { issue: "uuid".into() }));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn session_tab(status: Option<AgentStatus>, session: Option<&str>) -> TabView {
        TabView {
            id: "tab-1".into(),
            cwd: "/repo".into(),
            account: crate::accounts::default_account(),
            program: Program::Agent,
            bypass: false,
            running: true,
            status,
            title: None,
            message: None,
            resume_at: None,
            held: false,
            connector: Some("c1".into()),
            subject: Some(Subject::LinearIssue { issue: "uuid".into() }),
            opened: 1,
            updated: None,
            session: session.map(str::to_owned),
            stats: None,
            role: None,
        }
    }

    #[test]
    fn two_talks_share_one_session_and_a_missing_resume_is_seeded() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one", "two"]));
        let (connectors, dir) = stored("graph-talks", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let open = |work: Work| {
            let line = match &work {
                Work::Run(run) => {
                    let env = run.spec().env.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join("\n");
                    format!("run\n{env}\n{}", run.args.join("\n"))
                }
                Work::Prompt { env, text } => {
                    let env = env.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join("\n");
                    format!("prompt\n{env}\n{text}")
                }
                Work::Delivery(delivery) => format!("delivery {}", delivery.connector),
            };
            seen.lock().unwrap().push(line);
            let missing = matches!(&work, Work::Run(run) if run.args.windows(2).any(|pair| pair[0] == "--resume"));
            async move {
                if missing {
                    Err::<Option<Placement>, String>("No conversation found".into())
                } else {
                    Ok(Some(Placement { machine: "m1".into(), tab: "tab-1".into() }))
                }
            }
        };
        block_on(connectors.drive(&open)).unwrap();
        let done = session_tab(Some(AgentStatus::Done), Some("claude-1"));
        block_on(connectors.on_tabs("m1", unix_millis(), std::slice::from_ref(&done), &open)).unwrap();
        {
            let run = connectors.runs.lock().unwrap()[0].clone();
            assert!(matches!(run.session, Session::Live { ref machine, ref tab, ref claude, .. } if machine == "m1" && tab == "tab-1" && claude.as_deref() == Some("claude-1")));
        }
        block_on(connectors.on_tabs("m1", unix_millis(), &[], &open)).unwrap();
        let lines = seen.lock().unwrap().clone();
        assert_eq!(lines.len(), 4, "{lines:#?}");
        assert!(lines[0].contains("PINNED_CONNECTOR=c1\nPINNED_ISSUE=uuid"), "{}", lines[0]);
        assert!(lines[0].contains("ENG-7: Broken button"), "{}", lines[0]);
        assert!(lines[0].contains("\none\n") || lines[0].ends_with("\none"), "{}", lines[0]);
        assert!(!lines[0].contains("--resume"), "{}", lines[0]);
        assert!(lines[1].contains("PINNED_CONNECTOR=c1\nPINNED_ISSUE=uuid"), "{}", lines[1]);
        assert!(lines[1].ends_with("\ntwo") || lines[1].ends_with("two"), "{}", lines[1]);
        assert!(lines[2].contains("--resume\nclaude-1"), "{}", lines[2]);
        assert!(!lines[2].contains("Fix it."), "{}", lines[2]);
        assert!(lines[2].contains("PINNED_CONNECTOR=c1\nPINNED_ISSUE=uuid"), "{}", lines[2]);
        assert!(lines[3].contains("PINNED_CONNECTOR=c1\nPINNED_ISSUE=uuid"), "{}", lines[3]);
        assert!(lines[3].starts_with("run\n") && lines[3].contains("This session continues a run that already happened."), "{}", lines[3]);
        assert!(lines[3].contains("ENG-7: Broken button"), "{}", lines[3]);
        assert!(lines[3].contains("\n\ntwo"), "{}", lines[3]);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        let _ = std::fs::remove_dir_all(to);
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }

    fn placed(work: Work) -> impl Future<Output = Result<Option<Placement>, String>> {
        let _ = work;
        async { Ok(Some(Placement { machine: "m1".into(), tab: "tab-1".into() })) }
    }

    #[test]
    fn two_edited_graphs_do_not_both_start_one_issue() {
        let mut first = linear_connector();
        first.graph = Some(talks(&["one"]));
        let mut second = linear_connector();
        second.id = "c2".into();
        second.name = "Other".into();
        second.graph = Some(talks(&["one"]));
        let mut store = Store { connectors: vec![first, second], ..Store::default() };
        let settled = settle(&mut store, vec![("c1", Found::Issue(issue())), ("c2", Found::Issue(issue()))]);
        let started: Vec<_> = settled.starts.iter().map(|start| start.connector.clone()).collect();
        assert_eq!(started, vec!["c1".to_owned()]);
    }

    #[test]
    fn a_waiting_run_wakes_when_the_first_tab_list_lacks_its_tab() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one", "two"]));
        let (connectors, dir) = stored("tab-gone-first-list", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let opened = std::sync::Mutex::new(0u32);
        let open = |work: Work| {
            if matches!(work, Work::Run(_)) {
                *opened.lock().unwrap() += 1;
            }
            placed(work)
        };
        block_on(connectors.drive(&open)).unwrap();
        assert_eq!(*opened.lock().unwrap(), 1);
        block_on(connectors.on_tabs("m1", unix_millis(), &[], &open)).unwrap();
        assert_eq!(*opened.lock().unwrap(), 2, "the tab is already gone, so the run opens its next session");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_run_stores_the_session_from_its_own_tab_and_ignores_another_with_the_same_subject() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one", "two"]));
        let (connectors, dir) = stored("tab-id-only", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let open = |work: Work| placed(work);
        block_on(connectors.drive(&open)).unwrap();
        let working = session_tab(Some(AgentStatus::Working), Some("claude-1"));
        block_on(connectors.on_tabs("m1", unix_millis(), std::slice::from_ref(&working), &open)).unwrap();
        {
            let run = connectors.runs.lock().unwrap()[0].clone();
            assert!(matches!(run.status, Status::Waiting), "working is not the end of the turn");
            assert!(matches!(run.session, Session::Live { ref claude, .. } if claude.as_deref() == Some("claude-1")));
        }
        let mut other = session_tab(Some(AgentStatus::Done), Some("claude-other"));
        other.id = "tab-9".into();
        block_on(connectors.on_tabs("m1", unix_millis(), std::slice::from_ref(&other), &open)).unwrap();
        let run = connectors.runs.lock().unwrap()[0].clone();
        assert!(
            matches!(run.session, Session::Live { ref tab, ref claude, .. } if tab == "tab-1" && claude.as_deref() == Some("claude-1"))
                || matches!(run.session, Session::Detached { ref claude } if claude.is_none()),
            "a different tab is not this run's session: {:?}",
            run.session
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_runs_file_is_reported_and_left_in_place() {
        let (connectors, dir) = stored("broken-runs", vec![linear_connector()]);
        drop(connectors);
        let broken = b"{not json";
        std::fs::write(dir.join("runs.json"), broken).unwrap();
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into());
        let error = connectors.get_settings().checked.and_then(|check| check.error).unwrap_or_default();
        assert!(error.contains("could not read workflow runs"), "{error}");
        let _ = connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z");
        assert_eq!(std::fs::read(dir.join("runs.json")).unwrap(), broken);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_replayed_type_does_not_type_again() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one", "two"]));
        let (connectors, dir) = stored("replay-type", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let snap = dir.join("mid-type");
        let dir_for_copy = dir.clone();
        let snap_for_open = snap.clone();
        let open = move |work: Work| {
            if matches!(work, Work::Prompt { .. }) {
                copy_dir(&dir_for_copy, &snap_for_open);
            }
            placed(work)
        };
        block_on(connectors.drive(&open)).unwrap();
        let done = session_tab(Some(AgentStatus::Done), Some("claude-1"));
        block_on(connectors.on_tabs("m1", unix_millis(), std::slice::from_ref(&done), &open)).unwrap();
        let replay = Connectors::new(FileStorage::new(snap), "/bin/pinned".into());
        let again = std::sync::Mutex::new(0u32);
        let open = |work: Work| {
            if matches!(work, Work::Prompt { .. }) {
                *again.lock().unwrap() += 1;
            }
            placed(work)
        };
        block_on(replay.drive(&open)).unwrap();
        assert_eq!(*again.lock().unwrap(), 0, "the prompt was already handed to the tab");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_later_talk_with_the_same_text_still_types() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["again", "again", "again"]));
        let (connectors, dir) = stored("same-text", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let typed = std::sync::Mutex::new(0u32);
        let open = |work: Work| {
            if matches!(work, Work::Prompt { .. }) {
                *typed.lock().unwrap() += 1;
            }
            placed(work)
        };
        block_on(connectors.drive(&open)).unwrap();
        let working = session_tab(Some(AgentStatus::Working), Some("claude-1"));
        let done = session_tab(Some(AgentStatus::Done), Some("claude-1"));
        for _ in 0..3 {
            block_on(connectors.on_tabs("m1", unix_millis(), std::slice::from_ref(&working), &open)).unwrap();
            block_on(connectors.on_tabs("m1", unix_millis(), std::slice::from_ref(&done), &open)).unwrap();
        }
        assert_eq!(*typed.lock().unwrap(), 2, "the second and third talks each type");
        assert!(matches!(connectors.runs.lock().unwrap()[0].status, Status::Done));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_tab_list_that_arrived_earlier_does_not_replace_a_newer_one() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one", "two"]));
        let (connectors, dir) = stored("stale-list", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let opened = std::sync::Mutex::new(0u32);
        let open = |work: Work| {
            if matches!(work, Work::Run(_)) {
                *opened.lock().unwrap() += 1;
            }
            placed(work)
        };
        block_on(connectors.drive(&open)).unwrap();
        let fresh = unix_millis() + 10;
        let working = session_tab(Some(AgentStatus::Working), Some("claude-1"));
        block_on(connectors.on_tabs("m1", fresh, std::slice::from_ref(&working), &open)).unwrap();
        block_on(connectors.on_tabs("m1", fresh - 1, &[], &open)).unwrap();
        assert_eq!(*opened.lock().unwrap(), 1, "the list without the tab arrived first, so the tab is still there");
        assert!(matches!(connectors.runs.lock().unwrap()[0].status, Status::Waiting));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resume_detection_runs_off_the_async_worker() {
        let here = std::thread::current().id();
        let there = block_on(off_worker(|| Ok(std::thread::current().id()))).unwrap();
        assert_ne!(there, here);
    }

    #[test]
    fn a_claim_another_workflow_holds_ends_without_a_session() {
        let mut first = linear_connector();
        first.name = "Resolve".into();
        first.handled = vec!["uuid".into()];
        first.graph = Some(talks(&["one"]));
        let mut second = linear_connector();
        second.id = "c2".into();
        second.name = "Other".into();
        second.graph = Some(talks(&["one"]));
        let (connectors, dir) = stored("held-claim", vec![first, second]);
        let opened = std::sync::Mutex::new(0u32);
        let open = |work: Work| {
            if matches!(work, Work::Run(_)) {
                *opened.lock().unwrap() += 1;
            }
            placed(work)
        };
        connectors.begin("c2".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        block_on(connectors.drive(&open)).unwrap();
        assert_eq!(*opened.lock().unwrap(), 0);
        let run = connectors.runs.lock().unwrap()[0].clone();
        assert!(matches!(run.status, Status::Done));
        assert!(matches!(run.session, Session::None));
        let log = serde_json::to_string(&run.log).unwrap();
        assert!(log.contains("already handled"), "{log}");
        assert!(log.contains("Resolve"), "{log}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_later_drive_ends_a_turn_the_tab_list_already_showed() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one", "two"]));
        let (connectors, dir) = stored("known-done", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let prompts = std::sync::Mutex::new(0u32);
        let open = |work: Work| {
            if matches!(work, Work::Prompt { .. }) {
                *prompts.lock().unwrap() += 1;
            }
            placed(work)
        };
        let done = session_tab(Some(AgentStatus::Done), Some("claude-1"));
        block_on(connectors.on_tabs("m1", unix_millis(), std::slice::from_ref(&done), &open)).unwrap();
        assert_eq!(*prompts.lock().unwrap(), 0, "opening the session is not the end of a turn the list already showed");
        block_on(connectors.drive(&open)).unwrap();
        assert_eq!(*prompts.lock().unwrap(), 1, "the next check types the next talk");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_tab_list_from_before_the_wait_does_not_say_the_tab_is_gone() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one", "two"]));
        let (connectors, dir) = stored("old-list", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let opened = std::sync::Mutex::new(0u32);
        let open = |work: Work| {
            if matches!(work, Work::Run(_)) {
                *opened.lock().unwrap() += 1;
            }
            placed(work)
        };
        block_on(connectors.drive(&open)).unwrap();
        connectors.known_tabs.lock().unwrap().insert("m1".into(), (1, Vec::new()));
        block_on(connectors.drive(&open)).unwrap();
        assert_eq!(*opened.lock().unwrap(), 1, "a list from before the wait cannot drop the tab");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tab_at(status: Option<AgentStatus>, updated: Option<u64>) -> TabView {
        let mut tab = session_tab(status, Some("claude-1"));
        tab.updated = updated;
        tab
    }

    #[test]
    fn a_stale_done_does_not_end_the_turn_a_typed_message_is_waiting_on() {
        let mut connector = linear_connector();
        connector.graph = Some(talks(&["one", "two"]));
        let (connectors, dir) = stored("stale-done", vec![connector]);
        connectors.begin("c1".into(), Subject::LinearIssue { issue: "uuid".into() }, issue_trigger(), "2026-10-29T12:00:00Z").unwrap();
        let prompts = std::sync::Mutex::new(0u32);
        let open = |work: Work| {
            if matches!(work, Work::Prompt { .. }) {
                *prompts.lock().unwrap() += 1;
            }
            placed(work)
        };
        let show = |tab: &TabView| block_on(connectors.on_tabs("m1", unix_millis(), std::slice::from_ref(tab), &open)).unwrap();
        block_on(connectors.drive(&open)).unwrap();
        let working = tab_at(Some(AgentStatus::Working), Some(u64::MAX));
        show(&working);
        assert!(matches!(connectors.runs.lock().unwrap()[0].status, Status::Waiting));
        let done = tab_at(Some(AgentStatus::Done), Some(u64::MAX));
        show(&done);
        assert_eq!(*prompts.lock().unwrap(), 1, "working then done ends the first turn");
        assert!(matches!(connectors.runs.lock().unwrap()[0].status, Status::Waiting));
        let stale = tab_at(Some(AgentStatus::Done), Some(1));
        show(&stale);
        assert_eq!(*prompts.lock().unwrap(), 1, "a done from before this wait does not end it");
        assert!(matches!(connectors.runs.lock().unwrap()[0].status, Status::Waiting));
        show(&working);
        assert_eq!(*prompts.lock().unwrap(), 1);
        assert!(matches!(connectors.runs.lock().unwrap()[0].status, Status::Waiting));
        show(&done);
        assert_eq!(*prompts.lock().unwrap(), 1, "the second turn has no further talk");
        assert!(matches!(connectors.runs.lock().unwrap()[0].status, Status::Done));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn install_rustls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    struct RestoreSlack;

    impl Drop for RestoreSlack {
        fn drop(&mut self) {
            slack::set_api_root(None);
        }
    }

    fn serve_slack() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let mut served = 0u32;
            while served < 32 && started.elapsed() < std::time::Duration::from_secs(8) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        served += 1;
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                        let mut buf = Vec::new();
                        let mut tmp = [0u8; 1024];
                        loop {
                            match std::io::Read::read(&mut stream, &mut tmp) {
                                Ok(0) => break,
                                Ok(n) => {
                                    buf.extend_from_slice(&tmp[..n]);
                                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        let req = String::from_utf8_lossy(&buf);
                        let body = if req.contains("conversations.history") {
                            r#"{"ok":true,"messages":[{"type":"message","user":"U1","text":"start","ts":"1792400050.000200","thread_ts":"1792400050.000200","latest_reply":"1792400090.000000"}]}"#
                        } else if req.contains("conversations.replies") {
                            r#"{"ok":true,"messages":[{"type":"message","user":"U1","text":"start","ts":"1792400050.000200"},{"type":"message","user":"U3","text":"<@UBOT> here is the log","ts":"1792400090.000000"}]}"#
                        } else if req.contains("users.info") {
                            r#"{"ok":true,"user":{"name":"cara","profile":{"display_name":"Cara"}}}"#
                        } else {
                            r#"{"ok":true}"#
                        };
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(20)),
                    Err(_) => break,
                }
            }
        });
        format!("http://{address}/")
    }

    #[test]
    fn a_channel_graph_run_receives_a_later_reply_in_its_thread() {
        let _api = slack::lock_api();
        let _restore = RestoreSlack;
        install_rustls();
        slack::set_api_root(Some(serve_slack()));
        let mut connector = slack_connector();
        connector.trigger = Trigger::SlackMessage {
            channel: "C0123ABCD".into(),
            decide: Decide { model: String::new(), instructions: String::new(), categories: vec![], fallback: None },
        };
        let mut graph = talks(&["one"]);
        graph.nodes[0].kind = NodeKind::Trigger { watch: Watch::SlackMessage { channel: "C0123ABCD".into() } };
        connector.graph = Some(graph);
        let (connectors, dir) = stored("channel-replies", vec![connector]);
        std::fs::write(dir.join("slack-key"), "xoxb-test").unwrap();
        connectors.store.lock().unwrap().slack = Some(slack::Account { team: "T".into(), user_id: "UBOT".into(), name: "pinned".into() });
        connectors
            .begin("c2".into(), Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() }, slack_trigger(None), "2026-10-29T12:00:00Z")
            .unwrap();
        let open = |work: Work| placed(work);
        block_on(connectors.drive(&open)).unwrap();
        let (work, error) = block_on(connectors.poll());
        assert!(error.is_none(), "{error:?}");
        assert!(work.is_empty(), "the reply belongs to the run");
        let prompts = std::sync::Mutex::new(Vec::new());
        let open = |work: Work| {
            if let Work::Prompt { text, .. } = &work {
                prompts.lock().unwrap().push(text.clone());
            }
            placed(work)
        };
        block_on(connectors.drive(&open)).unwrap();
        let prompts = prompts.lock().unwrap().clone();
        assert!(prompts.iter().any(|text| text.contains("here is the log")), "{prompts:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_later_check_keeps_a_broken_runs_file_on_the_line() {
        let mut connector = linear_connector();
        connector.enabled = false;
        let (connectors, dir) = stored("broken-stays", vec![connector]);
        drop(connectors);
        std::fs::write(dir.join("runs.json"), b"{not json").unwrap();
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into());
        let before = connectors.get_settings().checked.unwrap().at;
        block_on(async {
            let watch = connectors.watch(|_| async { Ok::<(), String>(()) }, |_| {});
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), watch).await;
        });
        let check = connectors.get_settings().checked.expect("a check ran");
        assert_ne!(check.at, before, "the check did not finish");
        let error = check.error.unwrap_or_default();
        assert!(error.contains("could not read workflow runs"), "{error}");
        assert_eq!(std::fs::read(dir.join("runs.json")).unwrap(), b"{not json");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_page_that_fails_to_post_is_not_recorded() {
        let _api = slack::lock_api();
        let _restore = RestoreSlack;
        install_rustls();
        let mut connector = slack_connector();
        connector.trigger = Trigger::SlackMessage {
            channel: "C0123ABCD".into(),
            decide: Decide { model: "typesafe-ai/jev".into(), instructions: String::new(), categories: vec![], fallback: None },
        };
        connector.graph = Some(crate::graph::Graph {
            nodes: vec![
                Node { id: NodeId("t".into()), name: "Trigger".into(), position: [0, 0], kind: NodeKind::Trigger { watch: Watch::SlackMessage { channel: "C0123ABCD".into() } } },
                Node { id: NodeId("p".into()), name: "Page".into(), position: [1, 0], kind: NodeKind::Page { group: "S0123ABCD".into() } },
            ],
            edges: vec![Edge { from: NodeId("t".into()), port: PortId("out".into()), to: NodeId("p".into()) }],
        });
        let (connectors, dir) = stored("page-order", vec![connector]);
        std::fs::write(dir.join("slack-key"), "xoxb-test").unwrap();
        slack::set_api_root(Some("http://127.0.0.1:1/".into()));
        connectors
            .begin("c2".into(), Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() }, slack_trigger(None), "2026-10-29T12:00:00Z")
            .unwrap();
        let open = |work: Work| placed(work);
        let failed = block_on(connectors.drive(&open));
        slack::set_api_root(None);
        assert!(failed.is_err(), "the post did not reach Slack");
        assert!(connectors.store.lock().unwrap().connectors[0].paged.is_empty(), "the thread stays unpaged so the next attempt posts");
        let _ = std::fs::remove_dir_all(dir);
    }

    include!("legacy_outcome.rs");
}
