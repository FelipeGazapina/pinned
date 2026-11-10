//! The Upstash Boxes a workflow runs in. The account's box permission chooses a new box per issue
//! or thread, one box reused by later runs, a question before any box, or none. A box boots as a
//! node of the account, is given the run, and is deleted with its machine once the session ends.
//! The `boxes` table is the only source of truth. Every step checks it before acting, so the first
//! pass after a restart finishes whatever the last hub left halfway.

use crate::hub::{Account, Hub, RUN_WAIT};
use futures_util::future::BoxFuture;
use pinned_core::account::Credentials;
use pinned_core::accounts::{self, Provider};
use pinned_core::agent::{session_env, Subject};
use pinned_core::connectors::{unix_millis, BoxPermission, BoxSize, BoxStep, Connectors, FixedClaim, Placement, Run};
use pinned_core::delegate::{BoxTarget, Delegator, DelegatorKind, Origin, RemoteEnd, Request, RunTarget, TaskOutcome, TaskRecord, TaskUsage};
use pinned_core::surface::ActorRole;
use pinned_core::global_orchestrator::BusyPolicy;
use serde::{Deserialize, Serialize};
use pinned_core::upstash::{self, BoxId, Exec, Keys, Listed, NewBox};
use pinned_core::wire::{BoxStatus, Call, ProjectView, RunSpec, BOX_USD_PER_HOUR};
use pinned_core::github;
use pinned_core::machine;
use pinned_core::taste::TasteScope;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const DIR: &str = "/workspace/home/.pinned";
/// The connector interactive box sessions run under. The hub sees their tabs through it, and the
/// app treats their first prompt as the user's own rather than automation's.
pub const INTERACTIVE_CONNECTOR: &str = "session";
/// The connector a `delegate_task` worker's box runs under.
pub const DELEGATE_CONNECTOR: &str = "delegate";
/// The connector a Captain's box, opened by the global orchestrator, runs under.
pub const CAPTAIN_CONNECTOR: &str = "captain";
const BOOTSTRAP: &str = include_str!("box-bootstrap.sh");
const START: &str = include_str!("box-start.sh");
/// The bootstrap output a failure quotes, from its end.
const TAIL: usize = 600;

/// The Box API as the lifecycle uses it. `Upstash` forwards to `pinned_core::upstash`; tests pass a
/// fake whose exec of the bootstrap starts an in-process node.
pub trait BoxApi: Send + Sync {
    fn create<'a>(&'a self, key: &'a str, new: NewBox) -> BoxFuture<'a, Result<BoxId, String>>;
    fn list<'a>(&'a self, key: &'a str, label: &'a str) -> BoxFuture<'a, Result<Vec<Listed>, String>>;
    fn write<'a>(&'a self, key: &'a str, id: &'a BoxId, path: &'a str, content: String) -> BoxFuture<'a, Result<(), String>>;
    fn exec<'a>(&'a self, key: &'a str, id: &'a BoxId, script: String) -> BoxFuture<'a, Result<Exec, String>>;
    fn delete<'a>(&'a self, key: &'a str, id: &'a BoxId) -> BoxFuture<'a, Result<(), String>>;
}

pub struct Upstash;

impl BoxApi for Upstash {
    fn create<'a>(&'a self, key: &'a str, new: NewBox) -> BoxFuture<'a, Result<BoxId, String>> {
        Box::pin(async move { upstash::create(key, &new).await })
    }
    fn list<'a>(&'a self, key: &'a str, label: &'a str) -> BoxFuture<'a, Result<Vec<Listed>, String>> {
        Box::pin(upstash::list(key, label))
    }
    fn write<'a>(&'a self, key: &'a str, id: &'a BoxId, path: &'a str, content: String) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { upstash::write(key, id, path, &content).await })
    }
    fn exec<'a>(&'a self, key: &'a str, id: &'a BoxId, script: String) -> BoxFuture<'a, Result<Exec, String>> {
        Box::pin(async move { upstash::exec(key, id, &script).await })
    }
    fn delete<'a>(&'a self, key: &'a str, id: &'a BoxId) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(upstash::delete(key, id))
    }
}

/// When a box's session counts as ended, and how often the hub looks.
#[derive(Clone, Debug)]
pub struct Policy {
    /// Creating the box, booting its node and handing it the run.
    pub boot_wait: Duration,
    /// With the session not working.
    pub idle: Duration,
    /// With the session not working after it opened its pull request, which is its work done.
    pub done: Duration,
    /// With claude exited; restarting it from a client brings it back.
    pub exited: Duration,
    /// With no tab for the run, which covers the time from the run to the node's first tabs.
    pub tab_gone: Duration,
    pub offline: Duration,
    /// From the row's creation, whatever the session does.
    pub max_life: Duration,
    /// For a workflow after one of its boxes failed to boot, so a boot that always fails doesn't
    /// pay for a box every check.
    pub cooldown: Duration,
    pub tick: Duration,
    /// Between looks for boxes of this hub that no row names.
    pub sweep: Duration,
    /// How many of the account's boxes may exist at once. A run past this creates nothing.
    pub max_open: u32,
}

impl Default for Policy {
    fn default() -> Policy {
        let minutes = |m: u64| Duration::from_secs(m * 60);
        Policy {
            boot_wait: minutes(6),
            idle: minutes(10),
            done: minutes(2),
            exited: minutes(5),
            tab_gone: minutes(2),
            offline: minutes(10),
            max_life: minutes(4 * 60),
            cooldown: minutes(10),
            tick: Duration::from_secs(60),
            sweep: minutes(10),
            max_open: 8,
        }
    }
}

/// The hub's record that one run lives in one box. It exists before its box does and goes only
/// after the box and its machine are gone.
#[derive(Clone, Debug)]
pub struct BoxRow {
    /// 16 hex characters; the box carries the label `p:<id>`.
    pub id: String,
    pub account: Uuid,
    /// Chosen with the row, so a crash at any point knows which machine to remove.
    pub machine: Uuid,
    pub upstash: Option<BoxId>,
    pub connector: String,
    pub subject: Subject,
    /// Without the GitHub token, which dispatch adds back from the account's key.
    pub spec: RunSpec,
    pub size: BoxSize,
    pub phase: Phase,
    /// Since the row was written, by Postgres's clock.
    pub age: Duration,
    /// What the box does once its node is online.
    pub work: BoxWork,
}

/// What a box is for. Kept in the row's spec, so a hub restarted mid-boot still knows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoxWork {
    /// `spec` as `Call::Run`: a workflow's session or an interactive one.
    #[default]
    Run,
    /// A `delegate_task` worker for `origin`, signed in as the pinned agent account `claude`.
    Delegate { origin: Origin, request: Box<Request>, claude: Option<String> },
    /// The Captain of the project `spec.repo` clones, started on the global orchestrator's message.
    Captain {
        request_id: String,
        text: String,
        sender: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        busy: Option<BusyPolicy>,
    },
}

impl BoxRow {
    /// The pinned agent account whose Claude token the box signs in with, chosen before the row
    /// was written: a worker's in its work, any other box's in its spec. A row an older hub wrote
    /// names none and keeps the default account it was opened for.
    pub fn signs_in_as(&self) -> String {
        let named = match &self.work {
            BoxWork::Delegate { claude, .. } => claude.as_deref(),
            _ => self.spec.account.as_deref(),
        };
        named.filter(|name| !name.is_empty()).map_or_else(accounts::default_account, str::to_owned)
    }

    pub fn remains(&self) -> Remains {
        Remains { id: self.id.clone(), machine: self.machine, upstash: self.upstash.clone(), why: format!("{:?}", self.phase) }
    }
}

/// The agent session that opened a box through `act`, whose task the box counts as once it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct Opener {
    pub machine: Uuid,
    pub tab: String,
    pub role: ActorRole,
}

/// What deleting a row's box and machine takes, which even a row this hub can't read gives, as
/// one a hub with another run shape wrote.
#[derive(Clone, Debug)]
pub struct Remains {
    pub id: String,
    pub machine: Uuid,
    pub upstash: Option<BoxId>,
    /// Why it ends, for the log.
    pub why: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Creating,
    Booting,
    Running,
    Ending(End),
}

#[derive(Clone, Debug, PartialEq)]
pub enum End {
    TabClosed,
    Exited,
    Idle,
    /// The session opened its pull request and stopped.
    Done,
    Offline,
    Expired,
    /// Someone removed its machine.
    Removed,
    /// Someone deleted the box outside pinned.
    Vanished,
    Disconnected,
    Failed(String),
}

const ENDS: [(End, &str); 9] = [
    (End::TabClosed, "tab-closed"),
    (End::Exited, "exited"),
    (End::Idle, "idle"),
    (End::Done, "done"),
    (End::Offline, "offline"),
    (End::Expired, "expired"),
    (End::Removed, "removed"),
    (End::Vanished, "vanished"),
    (End::Disconnected, "disconnected"),
];
const FAILED: &str = "failed: ";

impl Phase {
    /// The `phase` and `reason` columns.
    pub fn stored(&self) -> (&'static str, Option<String>) {
        match self {
            Phase::Creating => ("creating", None),
            Phase::Booting => ("booting", None),
            Phase::Running => ("running", None),
            Phase::Ending(End::Failed(why)) => ("ending", Some(format!("{FAILED}{why}"))),
            Phase::Ending(end) => ("ending", ENDS.iter().find(|(e, _)| e == end).map(|(_, name)| name.to_string())),
        }
    }

    pub fn load(phase: &str, reason: Option<String>) -> Phase {
        match phase {
            "creating" => Phase::Creating,
            "booting" => Phase::Booting,
            "running" => Phase::Running,
            _ => {
                let reason = reason.unwrap_or_default();
                let end = match reason.strip_prefix(FAILED) {
                    Some(why) => End::Failed(why.to_owned()),
                    None => ENDS.iter().find(|(_, name)| *name == reason).map_or_else(|| End::Failed(reason.clone()), |(e, _)| e.clone()),
                };
                Phase::Ending(end)
            }
        }
    }
}

/// What the hub's live state shows of a running row's session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seen {
    Offline,
    NoTab,
    /// `quiet`: the session is not working.
    Tab { running: bool, quiet: bool },
}

/// When each condition of a running row began, by the hub's clock. A restart resets them, and
/// `max_life` still bounds what a box costs.
#[derive(Default, Debug)]
struct Timers {
    quiet_since: Option<Instant>,
    exited_since: Option<Instant>,
    gone_since: Option<Instant>,
    offline_since: Option<Instant>,
    /// The session reported its pull request opened.
    done: bool,
}

/// How far `boot` brought a row.
enum Boot {
    Online,
    /// Another hub process moved the row on, as during a deploy, so this one stops driving it.
    Moved,
}

/// Whether a running row's session has ended. Every end rule lives here.
/// What a box for `repo` may spend: `asked`, at most the Box budget of the account's tastes for
/// that project, which is also the default. A box with no repo, or one no project names, takes the
/// Captains' budget.
pub async fn box_budget(hub: &Hub, account: &Account, repo: Option<&str>, asked: Option<f64>) -> Result<Option<f64>, String> {
    let tastes = hub.db.tastes(account.id).await?.unwrap_or_default();
    let scope = repo.and_then(machine::repo_key).map_or(TasteScope::Captains, |repo| TasteScope::Project { repo });
    tastes.resolve(&scope).limits().box_budget_usd(asked)
}

/// The task record of a box an agent opened through `act`, which ended for `why` at unix ms
/// `ended`: the opener's task on its own machine, billed the box's life at the box rate.
pub fn agent_box_record(row: &BoxRow, opener: &Opener, started: u64, ended: u64, why: &End) -> TaskRecord {
    let box_ms = ended.saturating_sub(started);
    let (outcome, reason, limit) = match why {
        End::Failed(why) => (TaskOutcome::Failed, Some(why.clone()), None),
        End::Expired if over_budget(&row.spec, Duration::from_millis(box_ms)) => (TaskOutcome::Limit, None, Some("budget".to_owned())),
        End::Expired => (TaskOutcome::Limit, None, Some("time".to_owned())),
        End::Removed | End::Disconnected => (TaskOutcome::Cancelled, None, None),
        End::TabClosed | End::Exited | End::Idle | End::Done | End::Offline | End::Vanished => (TaskOutcome::Done, None, None),
    };
    let kind = match opener.role {
        ActorRole::GlobalOrchestrator => DelegatorKind::GlobalOrchestrator,
        ActorRole::Captain => DelegatorKind::Captain,
        ActorRole::Worker => DelegatorKind::Worker,
        ActorRole::TasteChat | ActorRole::Workflow | ActorRole::Session => DelegatorKind::Session,
    };
    let repo = row.spec.repo.as_deref();
    let delegator = Delegator { kind, tab: Some(opener.tab.clone()), project: repo.and_then(machine::repo_key), project_name: repo.and_then(machine::repo_name).map(str::to_owned) };
    let usage = TaskUsage { wall_ms: box_ms, box_ms: Some(box_ms), box_usd: Some(box_ms as f64 / 3_600_000.0 * BOX_USD_PER_HOUR), ..TaskUsage::default() };
    TaskRecord {
        id: row.id.clone(),
        machine: opener.machine.to_string(),
        machine_name: String::new(),
        title: machine_name(&row.spec),
        delegator,
        provider: Provider::Claude,
        account: accounts::DEFAULT.to_owned(),
        model: None,
        tier: None,
        effort: None,
        target: RunTarget::UpstashBox,
        started,
        ended,
        usage,
        outcome,
        reason,
        limit,
        asked_account: None,
        asked_model: None,
        moved_because: None,
        review: None,
        thread: None,
    }
}

/// Whether the spec's budget paid for `age` at the box hourly rate. None budgets run free.
fn over_budget(spec: &RunSpec, age: Duration) -> bool {
    spec.budget_usd.is_some_and(|b| b > 0.0 && age.as_secs_f64() * BOX_USD_PER_HOUR / 3600.0 >= b)
}

fn verdict(age: Duration, seen: Seen, t: &mut Timers, now: Instant, p: &Policy) -> Option<End> {
    // `None` is a condition this look can't see, as the session's while its node is offline: its
    // timer neither starts nor restarts, so a dropped link doesn't buy a quiet box more time.
    let (offline, gone, exited, quiet) = match seen {
        Seen::Offline => (Some(true), None, None, None),
        Seen::NoTab => (Some(false), Some(true), None, None),
        Seen::Tab { running, quiet } => (Some(false), Some(false), Some(!running), Some(quiet)),
    };
    let held = |since: &mut Option<Instant>, on: Option<bool>| match on {
        Some(false) => {
            *since = None;
            None
        }
        Some(true) => Some(now.duration_since(*since.get_or_insert(now))),
        None => since.map(|at| now.duration_since(at)),
    };
    let quiet = held(&mut t.quiet_since, quiet);
    let ended = [
        (held(&mut t.offline_since, offline).is_some_and(|h| h >= p.offline), End::Offline),
        (held(&mut t.gone_since, gone).is_some_and(|h| h >= p.tab_gone), End::TabClosed),
        (held(&mut t.exited_since, exited).is_some_and(|h| h >= p.exited), End::Exited),
        (t.done && quiet.is_some_and(|h| h >= p.done), End::Done),
        (quiet.is_some_and(|h| h >= p.idle), End::Idle),
    ];
    if age >= p.max_life {
        return Some(End::Expired);
    }
    ended.into_iter().find(|(over, _)| *over).map(|(_, end)| end)
}

const NOT_CONNECTED: &str = "Upstash Box is not connected: connect it in pinned's Connectors settings";

/// Whether the account's box permission for `who` lets a box open for `subject`. Ask files the
/// question on the Upstash card and refuses until it is answered.
async fn allowed(connectors: &Connectors, permission: BoxPermission, who: &str, connector: &str, subject: &Subject) -> Result<(), String> {
    match permission {
        BoxPermission::Fresh => Ok(()),
        BoxPermission::Off => Err(format!("Upstash Box is off for {who}: choose a box permission for {who} in Connectors settings")),
        BoxPermission::Fixed { .. } => Err(format!("Upstash Box: One box, reused is for workflows only; choose New box each time or Ask first for {who} in Connectors settings")),
        BoxPermission::Ask => match connectors.box_grant(connector, subject) {
            Some(true) => Ok(()),
            Some(false) => Err("Upstash Box: you said no to a box for this, so none was opened".into()),
            None => {
                connectors.ensure_box_ask(connector, subject).await?;
                Err("Upstash Box: waiting for a yes on the Upstash card in Connectors before it opens a box for this; ask again once it is answered".into())
            }
        },
    }
}

/// How long the box path waits for a machine to list its projects before it reads the list that
/// machine gave last.
const PROJECTS_WAIT: Duration = Duration::from_secs(10);

/// Where to add the Claude token a box needs.
const ADD_TOKEN: &str = "add it in Settings › Integrations › Upstash Box › Claude token per account";

/// Refuses `account` when no Claude token is kept for it, before any box is reserved or billed. A
/// box never signs in as another account instead.
fn token_kept(connectors: &Connectors, account: &str, project: Option<&str>) -> Result<(), String> {
    if connectors.box_claude_token(account).is_some() {
        return Ok(());
    }
    Err(match project {
        Some(project) => format!("No Claude token is kept for {account}, the account {project} runs on: {ADD_TOKEN}."),
        None => format!("No Claude token is kept for {account}: {ADD_TOKEN}."),
    })
}

/// A project's name as a refusal reads it: its repository's, else its folder's.
fn project_name(folder: Option<&str>, repo: Option<&str>) -> String {
    let named = repo.and_then(machine::repo_name).or_else(|| folder.and_then(|f| std::path::Path::new(f).file_name()?.to_str()));
    named.unwrap_or("this project").to_owned()
}

/// The pinned agent account the sessions of a project run under on the account's machines, as
/// each lists its projects: the one at `folder`, else any cloning `repo`. A machine that does not
/// answer counts with the projects it listed last. A box's own machine runs every project under
/// the default account, so it never counts. A project no machine has runs under the default
/// account, as it would on a machine that opened it first.
async fn project_account(hub: &Hub, account: &Arc<Account>, folder: Option<&str>, repo: Option<&str>) -> Result<String, String> {
    let machines = hub.machines(account).await?;
    let listed = futures_util::future::join_all(machines.iter().filter(|m| m.upstash_box.is_none()).map(|m| async move {
        let fresh = match m.online {
            true => hub.call_machine(account, &m.id, Call::ListProjects, PROJECTS_WAIT).await.ok().and_then(|v| serde_json::from_value::<Vec<ProjectView>>(v).ok()),
            false => None,
        };
        let projects = match fresh {
            Some(projects) => {
                account.listed(&m.id, projects.clone());
                projects
            }
            None => account.last_listed(&m.id),
        };
        (m.name.clone(), projects)
    }))
    .await;
    let key = repo.and_then(machine::repo_key);
    let at_folder = |p: &ProjectView| folder.is_some_and(|f| p.path == f);
    let cloning = |p: &ProjectView| key.is_some() && p.repo.as_deref().and_then(machine::repo_key) == key;
    let all = || listed.iter().flat_map(|(name, projects)| projects.iter().map(move |p| (name, p)));
    let mut matched: Vec<(&String, &ProjectView)> = all().filter(|(_, p)| at_folder(p)).collect();
    if matched.is_empty() {
        matched = all().filter(|(_, p)| cloning(p)).collect();
    }
    let accounts: BTreeSet<&str> = matched.iter().map(|(_, p)| p.account.as_str()).collect();
    match accounts.into_iter().collect::<Vec<_>>().as_slice() {
        [] => Ok(accounts::default_account()),
        [one] => Ok((*one).to_owned()),
        several => {
            let on: Vec<String> = matched.iter().map(|(name, p)| format!("{} on {name}", p.account)).collect::<BTreeSet<_>>().into_iter().collect();
            Err(format!(
                "{} runs on {} accounts ({}): give it one account on every machine, so a box knows whose Claude token to sign in with",
                project_name(folder, repo),
                several.len(),
                on.join(", ")
            ))
        }
    }
}

/// The label every box of this hub carries, so hubs sharing one Box key never sweep each other's.
pub fn hub_label(public_url: &str) -> String {
    let digest = Sha256::digest(public_url.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("hub:{}", &hex[..12])
}

fn row_label(row: &str) -> String {
    format!("p:{row}")
}

fn new_row_id() -> String {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn quoted(text: &str) -> String {
    text.replace('\'', "'\\''")
}

/// The bootstrap the hub runs in a new box. It carries no secret.
pub fn bootstrap_script(hub: &str) -> String {
    BOOTSTRAP.replace("{{hub}}", &quoted(hub))
}

/// The script that starts the box's node on every container start. It carries no secret.
/// `repo` is the run's repository, whose clone it marks trusted for claude.
pub fn start_script(login: &str, repo: Option<&str>) -> String {
    let clones = serde_json::to_string(&repo.and_then(machine::repo_name).into_iter().collect::<Vec<_>>()).unwrap();
    START.replace("{{login}}", &quoted(login)).replace("{{clones}}", &quoted(&clones))
}

/// The secrets the node and its sessions run with, as `start.sh` sources them.
fn box_env(keys: &Keys, github: Option<&str>) -> String {
    let mut env = format!("CLAUDE_CODE_OAUTH_TOKEN='{}'\n", quoted(&keys.claude_token));
    if let Some(token) = github {
        env += &format!("GH_TOKEN='{}'\n", quoted(token));
    }
    env
}

/// An `owner/repo` shorthand or GitHub clone URL as the https URL the box clones.
fn normalize_repo(repo: &str) -> Option<String> {
    let repo = repo.trim().trim_end_matches('/').trim_end_matches(".git");
    if repo.is_empty() {
        return None;
    }
    if let Some(rest) = repo.strip_prefix("https://github.com/").or_else(|| repo.strip_prefix("http://github.com/")) {
        let rest = rest.trim_matches('/');
        return (rest.contains('/') && machine::repo_name(rest).is_some()).then(|| format!("https://github.com/{rest}.git"));
    }
    if repo.contains("://") || repo.contains('@') || !repo.contains('/') {
        return None;
    }
    let parts: Vec<&str> = repo.split('/').collect();
    (parts.len() == 2 && parts.iter().all(|p| !p.is_empty())).then(|| format!("https://github.com/{repo}.git"))
}

/// A checkout's git origin as the https URL a box clones: an SSH GitHub origin too, since the
/// box clones with the account's token.
fn origin_repo(origin: &str) -> Option<String> {
    let origin = origin.trim();
    normalize_repo(origin.strip_prefix("git@github.com:").or_else(|| origin.strip_prefix("github.com/")).unwrap_or(origin))
}

/// `Box · <the session's name>`, as the run names its tab.
fn machine_name(spec: &RunSpec) -> String {
    let named = spec.args.iter().position(|a| a == "--name").and_then(|i| spec.args.get(i + 1));
    let name = format!("Box · {}", named.map_or("pinned", String::as_str));
    name.chars().take(100).collect()
}

fn tail(exec: &Exec) -> String {
    let text = format!("{}{}", exec.output, exec.error);
    let text = text.trim();
    let start = text.char_indices().rev().nth(TAIL - 1).map_or(0, |(i, _)| i);
    text[start..].to_owned()
}

/// Marks a row as driven while it lives, so an open and a reap never drive one row at once.
struct Driving<'a> {
    rows: &'a Mutex<HashSet<String>>,
    id: String,
}

impl Drop for Driving<'_> {
    fn drop(&mut self) {
        self.rows.lock().unwrap().remove(&self.id);
    }
}

pub struct Boxes {
    api: Arc<dyn BoxApi>,
    pub policy: Policy,
    /// Every box this hub creates carries it.
    pub label: String,
    driving: Mutex<HashSet<String>>,
    timers: Mutex<HashMap<String, Timers>>,
    cooling: Mutex<HashMap<(Uuid, String), Instant>>,
    swept: Mutex<HashMap<Uuid, Instant>>,
}

impl Boxes {
    pub fn new(api: Arc<dyn BoxApi>, policy: Policy, public_url: &str) -> Boxes {
        Boxes {
            api,
            policy,
            label: hub_label(public_url),
            driving: Mutex::default(),
            timers: Mutex::default(),
            cooling: Mutex::default(),
            swept: Mutex::default(),
        }
    }

    /// Checks the Box key with Upstash before the account keeps it.
    pub async fn check_key(&self, key: &str) -> Result<(), String> {
        self.api.list(key, &self.label).await.map(drop)
    }

    fn drive_guard(&self, id: &str) -> Option<Driving<'_>> {
        self.driving.lock().unwrap().insert(id.to_owned()).then(|| Driving { rows: &self.driving, id: id.to_owned() })
    }

    /// `Hub::open` for a box run. The account's workflow permission decides what happens.
    /// `Fresh` gives the issue or thread its own box. `Fixed` reuses one box for every run.
    /// `Ask` opens nothing until the user says yes; the user's own Run now counts as that yes. `Off`, and a disconnected Upstash, open nothing.
    /// A run whose issue or thread already has a box goes to it. On failure the row ends, and the
    /// answer says why, which reaches the workflow's check line.
    /// The box signs in as the account the workflow names, else as its project's.
    pub async fn open(&self, hub: &Hub, account: &Arc<Account>, mut run: Run, step: BoxStep) -> Result<Placement, String> {
        let connectors = hub.connectors(account).await?;
        let keys = connectors.box_keys().ok_or(NOT_CONNECTED)?;
        let permission = connectors.box_access().workflow;
        if permission == BoxPermission::Off {
            return Err("Upstash Box is off for workflows: choose a box permission in Connectors settings".into());
        }
        let folder = run.cwd.to_str().filter(|f| !f.is_empty());
        let signs_in = match run.account.as_deref().filter(|a| !a.is_empty()) {
            Some(named) => named.to_owned(),
            None => project_account(hub, account, folder, run.repo.as_deref()).await?,
        };
        token_kept(&connectors, &signs_in, Some(&project_name(folder, run.repo.as_deref())))?;
        run.account = Some(signs_in);
        match permission {
            // The owner's own Run now click is the yes: an agent's never opens a box.
            BoxPermission::Ask if pinned_core::connectors::is_run_now(&run.subject) => {}
            BoxPermission::Ask => match connectors.box_grant(&run.connector, &run.subject) {
                Some(true) => {}
                Some(false) => return Err("Upstash Box: you said no to a box for this, so none was opened".into()),
                None => {
                    connectors.ensure_box_ask(&run.connector, &run.subject).await?;
                    return Err("Upstash Box: waiting for a yes before it opens a box for this".into());
                }
            },
            BoxPermission::Off | BoxPermission::Fresh | BoxPermission::Fixed { .. } => {}
        }
        if matches!(permission, BoxPermission::Fixed { .. }) {
            return self.open_fixed(hub, account, &connectors, run, step, &keys).await;
        }
        self.open_new(hub, account, run, step, &keys, None, false).await
    }

    /// An interactive session on a new box cloning `repo`. The client polls `box_status` until it
    /// runs, then opens its tab. The user's own budget dialog is the session permission's ask; a
    /// session permission of off refuses.
    /// An agent's `open_box` passes its `opener`, so the box is that session's task once it ends.
    /// Its ask is a question on the Upstash card for that session, which only the user answers.
    pub async fn open_session(&self, hub: &Arc<Hub>, account: &Arc<Account>, repo: String, size: BoxSize, budget_usd: Option<f64>, opener: Option<Opener>) -> Result<BoxStatus, String> {
        let connectors = hub.connectors(account).await?;
        let keys = connectors.box_keys().ok_or(NOT_CONNECTED)?;
        match (connectors.box_access().session, &opener) {
            (BoxPermission::Off, _) => return Err("Upstash Box is off for sessions: choose a box permission in Connectors settings".into()),
            (BoxPermission::Ask, Some(opener)) => {
                let name = account.name_of(&opener.machine.to_string()).unwrap_or_else(|| opener.machine.to_string());
                let asked = Subject::LinearIssue { issue: format!("sessions of {name} {}", opener.tab) };
                allowed(&connectors, BoxPermission::Ask, "Sessions", INTERACTIVE_CONNECTOR, &asked).await?;
            }
            _ => {}
        }
        let repo = normalize_repo(&repo).ok_or("name a GitHub repository as owner/repo or paste its clone URL")?;
        // An agent's box signs in as the account its own session runs under; the user's, as the project's.
        let signs_in = match opener.as_ref().and_then(|o| account.tab(&o.machine.to_string(), &o.tab)) {
            Some(tab) => tab.account,
            None => project_account(hub, account, None, Some(&repo)).await?,
        };
        token_kept(&connectors, &signs_in, Some(&project_name(None, Some(&repo))))?;
        let budget_usd = box_budget(hub, account, Some(&repo), budget_usd).await?;
        if let Some(budget) = budget_usd {
            if !(budget >= BOX_USD_PER_HOUR) {
                return Err(format!("name at least ${BOX_USD_PER_HOUR:.2}, an hour on a box"));
            }
        }
        self.within_cap(&hub.db.boxes(account.id).await?)?;
        let name = machine::repo_name(&repo).unwrap_or("repo").to_owned();
        let id = new_row_id();
        let machine = Uuid::new_v4();
        let subject = Subject::LinearIssue { issue: format!("box {id}") };
        let spec = RunSpec { folder: String::new(), repo: Some(repo), args: vec!["--name".into(), name], env: session_env(INTERACTIVE_CONNECTOR, &subject), token: None, account: Some(signs_in), budget_usd };
        let fresh = BoxRow {
            id: id.clone(),
            account: account.id,
            machine,
            upstash: None,
            connector: INTERACTIVE_CONNECTOR.into(),
            subject,
            spec,
            size,
            phase: Phase::Creating,
            age: Duration::ZERO,
            work: BoxWork::Run,
        };
        if self.drive_guard(&fresh.id).is_none() {
            return Err("Upstash Box: its box is still booting; try again".into());
        }
        // Kept before the row, so the box can't end before its opener is known.
        if let Some(opener) = &opener {
            hub.db.hold_agent_box(&fresh.id, account.id, opener, unix_millis()).await?;
        }
        if let Err(e) = hub.db.reserve_box(&fresh).await {
            if opener.is_some() {
                let _ = hub.db.forget_agent_box(account.id, &fresh.id).await;
            }
            return Err(e);
        }
        let (hub, account, keys) = (hub.clone(), account.clone(), keys.clone());
        let row = fresh.clone();
        tokio::spawn(async move {
            let Some(_driving) = hub.boxes.drive_guard(&row.id) else { return };
            if let Err(e) = hub.boxes.drive(&hub, &account, row.clone(), &row.spec, &keys).await {
                eprintln!("pinned-server: could not boot interactive box {}: {e}", row.id);
            }
        });
        Ok(BoxStatus { id, phase: "creating".into(), machine: machine.to_string(), tab: None, age_secs: 0.0, budget_usd, rate_usd_per_hour: BOX_USD_PER_HOUR })
    }

    /// `delegate_task` with `target.box`: a new box, opened with the account's own connection,
    /// that runs `request` as a worker for `origin`. It answers at once with the box's machine; the
    /// box boots and starts the worker in the background, and a boot that fails ends the task on
    /// `origin`'s machine. The box signs in as the account `request.target.account` names, else as
    /// the one `origin`'s tab runs under.
    pub async fn open_delegate(&self, hub: &Arc<Hub>, account: &Arc<Account>, origin: Origin, mut request: Request, repo: Option<String>, boxed: BoxTarget) -> Result<serde_json::Value, String> {
        let connectors = hub.connectors(account).await?;
        let keys = connectors.box_keys().ok_or(NOT_CONNECTED)?;
        let asked = Subject::LinearIssue { issue: format!("workers of {} {}", origin.machine_name, origin.tab) };
        allowed(&connectors, connectors.box_access().worker, "Workers", DELEGATE_CONNECTOR, &asked).await?;
        if request.target.driver_kind.as_deref().is_some_and(|kind| kind != "claude") {
            return Err("a box runs Claude Code only: leave driverKind out".into());
        }
        let repo = boxed.repo.or(repo).as_deref().and_then(origin_repo).ok_or("a box clones a GitHub repository, and this thread's origin is none: pass target.box.repo as owner/repo")?;
        let claude = match request.target.account.take().filter(|name| !name.is_empty()) {
            Some(named) => {
                token_kept(&connectors, &named, None)?;
                named
            }
            None => {
                let signs_in = match account.tab(&origin.machine, &origin.tab) {
                    Some(tab) => tab.account,
                    None => project_account(hub, account, None, Some(&repo)).await?,
                };
                token_kept(&connectors, &signs_in, Some(&project_name(None, Some(&repo))))?;
                signs_in
            }
        };
        self.within_cap(&hub.db.boxes(account.id).await?)?;
        (request.target.machine, request.target.boxed, request.target.driver_kind) = (None, None, None);
        let budget_usd = box_budget(hub, account, Some(&repo), None).await?;
        let spec = RunSpec { folder: String::new(), repo: Some(repo), args: vec!["--name".into(), request.title.clone()], env: vec![], token: None, account: None, budget_usd };
        let subject = Subject::LinearIssue { issue: format!("task {}/{}", origin.machine, origin.task) };
        let work = BoxWork::Delegate { origin, request: Box::new(request), claude: Some(claude) };
        let row = self.reserve_work(hub, account, DELEGATE_CONNECTOR, subject, spec, boxed.size, work, &keys).await?;
        Ok(serde_json::json!({ "machine": row.machine.to_string(), "machineName": machine_name(&row.spec), "box": row.id, "workState": "working" }))
    }

    /// The global orchestrator's `send_to_captain` with `box`: the Captain of `repo` in a box
    /// opened with the account's connection, started on `text`. A box that already runs that
    /// project's Captain takes the message as a Captain on a machine does.
    #[allow(clippy::too_many_arguments)]
    pub async fn open_captain(&self, hub: &Arc<Hub>, account: &Arc<Account>, request_id: String, repo: &str, size: BoxSize, text: String, sender: String, busy: Option<BusyPolicy>) -> Result<serde_json::Value, String> {
        let connectors = hub.connectors(account).await?;
        let keys = connectors.box_keys().ok_or(NOT_CONNECTED)?;
        let repo = origin_repo(repo).ok_or("a box clones a GitHub repository, and this project's origin is none")?;
        let subject = Subject::LinearIssue { issue: format!("the Captain of {repo}") };
        allowed(&connectors, connectors.box_access().orchestrator, "Captains", CAPTAIN_CONNECTOR, &subject).await?;
        let work = BoxWork::Captain { request_id, text, sender, busy };
        if let Some(id) = hub.db.box_for(account.id, CAPTAIN_CONNECTOR, &subject).await? {
            let row = hub.db.box_by_id(account.id, &id).await?.ok_or("its box just went; send it again")?.map_err(|r| format!("box {} {}", r.id, r.why))?;
            return match row.phase {
                Phase::Running => {
                    let placed = self.place_work(hub, account, &row, &work).await.map_err(|e| format!("Upstash Box: {e}"))?;
                    Ok(serde_json::json!({ "outcome": "entregue", "tab": placed.tab, "machine": placed.machine, "machineName": machine_name(&row.spec), "box": row.id }))
                }
                Phase::Creating | Phase::Booting => Err("Upstash Box: this project's box is still booting; send it again once projects() lists its Captain".into()),
                Phase::Ending(_) => Err("Upstash Box: this project's last box is still being deleted; send it again in a minute".into()),
            };
        }
        let signs_in = project_account(hub, account, None, Some(&repo)).await?;
        token_kept(&connectors, &signs_in, Some(&project_name(None, Some(&repo))))?;
        self.within_cap(&hub.db.boxes(account.id).await?)?;
        let name = machine::repo_name(&repo).unwrap_or("repo").to_owned();
        let budget_usd = box_budget(hub, account, Some(&repo), None).await?;
        let spec = RunSpec { folder: String::new(), repo: Some(repo), args: vec!["--name".into(), format!("Captain {name}")], env: vec![], token: None, account: Some(signs_in), budget_usd };
        let row = self.reserve_work(hub, account, CAPTAIN_CONNECTOR, subject, spec, size, work, &keys).await?;
        Ok(serde_json::json!({ "outcome": "iniciado", "machine": row.machine.to_string(), "machineName": machine_name(&row.spec), "box": row.id }))
    }

    /// Writes a row for `work` and boots its box in the background. A row the subject already has
    /// is answered as it is, so a repeat opens nothing.
    #[allow(clippy::too_many_arguments)]
    async fn reserve_work(&self, hub: &Arc<Hub>, account: &Arc<Account>, connector: &str, subject: Subject, spec: RunSpec, size: BoxSize, work: BoxWork, keys: &Keys) -> Result<BoxRow, String> {
        let fresh = BoxRow { id: new_row_id(), account: account.id, machine: Uuid::new_v4(), upstash: None, connector: connector.into(), subject, spec, size, phase: Phase::Creating, age: Duration::ZERO, work };
        if self.drive_guard(&fresh.id).is_none() {
            return Err("Upstash Box: its box is still booting; try again".into());
        }
        let (row, written) = hub.db.reserve_box(&fresh).await?;
        if !written {
            return Ok(row);
        }
        let (hub, account, keys) = (hub.clone(), account.clone(), keys.clone());
        let booting = row.clone();
        tokio::spawn(async move {
            let Some(_driving) = hub.boxes.drive_guard(&booting.id) else { return };
            if let Err(e) = hub.boxes.drive(&hub, &account, booting.clone(), &booting.spec, &keys).await {
                eprintln!("pinned-server: could not boot box {} for {}: {e}", booting.id, booting.connector);
                if let BoxWork::Delegate { origin, .. } = &booting.work {
                    let end = RemoteEnd { state: "failed".into(), summary: None, error: Some(format!("Upstash Box: {e}")) };
                    let ended = Call::RemoteEnded { from: booting.machine.to_string(), tab: origin.tab.clone(), task: origin.task.clone(), end };
                    if let Err(e) = hub.call_machine(&account, &origin.machine, ended, RUN_WAIT).await {
                        eprintln!("pinned-server: could not tell {} its box failed: {}", origin.machine, String::from(e));
                    }
                }
            }
        });
        Ok(row)
    }

    /// The worker box behind `machine`, when it is one.
    pub async fn delegate_box(&self, hub: &Hub, account: &Account, machine: &str) -> Result<Option<BoxRow>, String> {
        let Ok(machine) = Uuid::parse_str(machine) else { return Ok(None) };
        Ok(hub.db.box_of(account.id, machine).await?.filter(|row| matches!(row.work, BoxWork::Delegate { .. })))
    }

    /// The worker's task ended or was cancelled, so its box goes.
    pub async fn end_delegate(&self, hub: &Hub, account: &Arc<Account>, row: BoxRow) -> Result<(), String> {
        let keys = hub.connectors(account).await?.box_keys();
        self.finish(hub, account, row, End::Done, keys.as_ref()).await
    }

    /// The box behind `machine`: its phase, age and budget, for its cost display.
    pub async fn box_status(&self, hub: &Hub, account: &Arc<Account>, machine: Uuid) -> Result<BoxStatus, String> {
        let row = hub.db.box_of(account.id, machine).await?.ok_or_else(|| "this machine is no box".to_owned())?;
        let phase = match &row.phase {
            Phase::Creating => "creating",
            Phase::Booting => "booting",
            Phase::Running => "running",
            Phase::Ending(_) => "ending",
        }
        .to_owned();
        let tab = account.placed(&row.connector, &row.subject).map(|(_, tab)| tab);
        Ok(BoxStatus { id: row.id, phase, machine: machine.to_string(), tab, age_secs: row.age.as_secs_f64(), budget_usd: row.spec.budget_usd, rate_usd_per_hour: BOX_USD_PER_HOUR })
    }

    /// Adds `extra_usd` to the box behind `machine`'s budget.
    pub async fn extend_box(&self, hub: &Hub, account: &Arc<Account>, machine: Uuid, extra_usd: f64) -> Result<BoxStatus, String> {
        if !(extra_usd > 0.0) {
            return Err("name an amount above zero".into());
        }
        let row = hub.db.box_of(account.id, machine).await?.ok_or_else(|| "this machine is no box".to_owned())?;
        if matches!(row.phase, Phase::Ending(_)) {
            return Err("this box is already ending".into());
        }
        let total = row.spec.budget_usd.unwrap_or(0.0) + extra_usd;
        box_budget(hub, account, row.spec.repo.as_deref(), Some(total)).await?;
        hub.db.set_box_budget(&row.id, total).await?;
        self.box_status(hub, account, machine).await
    }

    async fn open_fixed(
        &self,
        hub: &Hub,
        account: &Arc<Account>,
        connectors: &pinned_core::connectors::Connectors,
        run: Run,
        step: BoxStep,
        keys: &Keys,
    ) -> Result<Placement, String> {
        let proposed = new_row_id();
        let driving = self.drive_guard(&proposed);
        match connectors.claim_fixed_row(&proposed)? {
            FixedClaim::Use(existing) => {
                drop(driving);
                self.reuse_fixed(hub, account, connectors, &existing, run, step, keys).await
            }
            FixedClaim::Create => self.open_new(hub, account, run, step, keys, Some(proposed), driving.is_some()).await,
        }
    }

    async fn reuse_fixed(
        &self,
        hub: &Hub,
        account: &Arc<Account>,
        connectors: &pinned_core::connectors::Connectors,
        id: &str,
        run: Run,
        step: BoxStep,
        keys: &Keys,
    ) -> Result<Placement, String> {
        let Some(_driving) = self.drive_guard(id) else {
            return Err("Upstash Box: its box is still booting; the next check tries again".into());
        };
        match hub.db.box_by_id(account.id, id).await? {
            None => {
                drop(_driving);
                let proposed = new_row_id();
                let driving = self.drive_guard(&proposed);
                match connectors.retake_fixed_row(id, &proposed)? {
                    FixedClaim::Create => self.open_new(hub, account, run, step, keys, Some(proposed), driving.is_some()).await,
                    FixedClaim::Use(other) => {
                        drop(driving);
                        Box::pin(self.reuse_fixed(hub, account, connectors, &other, run, step, keys)).await
                    }
                }
            }
            Some(Err(_)) => Err("Upstash Box: the last box for this is still being deleted; the next check tries again".into()),
            Some(Ok(row)) => match row.phase {
                Phase::Ending(_) => Err("Upstash Box: the last box for this is still being deleted; the next check tries again".into()),
                Phase::Creating | Phase::Booting => Err("Upstash Box: its box is still booting; the next check tries again".into()),
                Phase::Running if Some(row.signs_in_as()) != run.account => Err(format!(
                    "Upstash Box: the one reused box signs in as {}, and this run's project runs on {}: choose New box each time for workflows",
                    row.signs_in_as(),
                    run.account.as_deref().unwrap_or_default()
                )),
                Phase::Running => {
                    let mut spec = run.spec();
                    spec.token = None;
                    self.place_run(hub, account, &row.machine.to_string(), &run.connector, &run.subject, &spec).await.map_err(|e| format!("Upstash Box: {e}"))
                }
            },
        }
    }

    /// A new row, unless this subject already has one.
    async fn open_new(
        &self,
        hub: &Hub,
        account: &Arc<Account>,
        run: Run,
        step: BoxStep,
        keys: &Keys,
        claimed: Option<String>,
        already_guarded: bool,
    ) -> Result<Placement, String> {
        let cooled = self.cooling.lock().unwrap().get(&(account.id, run.connector.clone())).copied();
        if let Some(left) = cooled.and_then(|at| self.policy.cooldown.checked_sub(at.elapsed())) {
            self.give_back(hub, account, claimed.as_deref()).await;
            let minutes = left.as_secs().div_ceil(60).max(1);
            return Err(format!("Upstash Box: this workflow's last box failed to boot, so pinned tries again in {minutes} min"));
        }
        if hub.db.box_for(account.id, &run.connector, &run.subject).await?.is_none() {
            let rows = hub.db.boxes(account.id).await?;
            if let Err(e) = self.within_cap(&rows) {
                self.give_back(hub, account, claimed.as_deref()).await;
                return Err(e);
            }
        }
        let mut spec = run.spec();
        spec.token = None;
        spec.budget_usd = match box_budget(hub, account, spec.repo.as_deref(), spec.budget_usd).await {
            Ok(budget) => budget,
            Err(e) => {
                self.give_back(hub, account, claimed.as_deref()).await;
                return Err(format!("Upstash Box: {e}"));
            }
        };
        let fresh = BoxRow {
            id: claimed.clone().unwrap_or_else(new_row_id),
            account: account.id,
            machine: Uuid::new_v4(),
            upstash: None,
            connector: run.connector.clone(),
            subject: run.subject.clone(),
            spec: spec.clone(),
            size: step.size,
            phase: Phase::Creating,
            age: Duration::ZERO,
            work: BoxWork::Run,
        };
        // Driven before it is written, so a reap that reads it first leaves it alone.
        let driving = self.drive_guard(&fresh.id);
        if driving.is_none() && !already_guarded {
            self.give_back(hub, account, claimed.as_deref()).await;
            return Err("Upstash Box: its box is still booting; the next check tries again".into());
        }
        let (row, _) = hub.db.reserve_box(&fresh).await?;
        if row.id != fresh.id {
            self.give_back(hub, account, claimed.as_deref()).await;
        }
        if let Phase::Ending(_) = row.phase {
            return Err("Upstash Box: the last box for this is still being deleted; the next check tries again".into());
        }
        let other = if row.id == fresh.id { None } else { self.drive_guard(&row.id) };
        if row.id != fresh.id && other.is_none() {
            return Err("Upstash Box: its box is still booting; the next check tries again".into());
        }
        // The run asked for now, which may carry newer thread messages than the one the row was made with.
        self.drive(hub, account, row, &spec, keys).await.map_err(|e| format!("Upstash Box: {e}"))
    }

    async fn give_back(&self, hub: &Hub, account: &Arc<Account>, id: Option<&str>) {
        let Some(id) = id else { return };
        if let Ok(connectors) = hub.connectors(account).await {
            if let Err(e) = connectors.release_fixed_row(id) {
                eprintln!("pinned-server: could not release box claim {id}: {e}");
            }
        }
    }

    fn within_cap(&self, rows: &[Result<BoxRow, Remains>]) -> Result<(), String> {
        let open = rows.iter().filter(|row| match row {
            Ok(row) => !matches!(row.phase, Phase::Ending(_)),
            Err(_) => true,
        }).count() as u32;
        if open < self.policy.max_open {
            return Ok(());
        }
        let n = self.policy.max_open;
        let (noun, verb) = if n == 1 { ("box", "is") } else { ("boxes", "are") };
        Err(format!("Upstash Box: {n} {noun} {verb} already open, and this account's limit is {n}"))
    }

    /// Brings the row to running: its box's node online within `boot_wait`, then `spec` answered
    /// by it within `RUN_WAIT`, which leaves a slow clone its time. A row another hub process moved
    /// is left to that process.
    async fn drive(&self, hub: &Hub, account: &Arc<Account>, row: BoxRow, spec: &RunSpec, keys: &Keys) -> Result<Placement, String> {
        if row.phase == Phase::Running {
            return self.dispatch(hub, account, &row, spec).await;
        }
        let machine = row.machine.to_string();
        let booted = if row.phase == Phase::Booting && account.is_online(&machine) {
            Ok(Boot::Online)
        } else {
            // A row resumed after a restart keeps some time, since its age counts the hub's downtime.
            let left = self.policy.boot_wait.saturating_sub(row.age).max(self.policy.boot_wait / 2);
            match tokio::time::timeout(left, self.boot(hub, account, &row, keys)).await {
                Ok(booted) => booted,
                Err(_) => Err(format!("the box did not boot within {} s", left.as_secs())),
            }
        };
        let booted = match booted {
            Ok(Boot::Moved) => return Err(format!("box {} is driven by another hub; the next check tries again", row.id)),
            Ok(Boot::Online) => self.dispatch(hub, account, &row, spec).await,
            Err(e) => Err(e),
        };
        if booted.is_ok() {
            // Another hub may have moved it on already, which is just as good.
            hub.db.set_box_phase(&row.id, &Phase::Booting, &Phase::Running).await?;
        }
        if let Err(e) = &booted {
            eprintln!("pinned-server: box {} of account {} failed: {e}", row.id, account.id);
            self.cooling.lock().unwrap().insert((account.id, row.connector.clone()), Instant::now());
            let failed = Phase::Ending(End::Failed(e.clone()));
            for from in [Phase::Creating, Phase::Booting] {
                if hub.db.set_box_phase(&row.id, &from, &failed).await? {
                    let mut row = row.clone();
                    row.phase = failed.clone();
                    self.record_agent_box(hub, account, &row).await;
                    if let Err(e) = self.end(hub, account, &row.remains(), Some(keys)).await {
                        eprintln!("pinned-server: could not delete box {}: {e}", row.id);
                    }
                    break;
                }
            }
        }
        booted
    }

    /// Creating → booting, until the box's node is online. Each step checks what is done already:
    /// the box, by the row or by its label; the node, by whether it is online.
    async fn boot(&self, hub: &Hub, account: &Arc<Account>, row: &BoxRow, keys: &Keys) -> Result<Boot, String> {
        let key = keys.api_key.as_str();
        let mut upstash = row.upstash.clone();
        if row.phase == Phase::Creating {
            let (id, made) = match upstash {
                Some(id) => (id, false),
                None => match self.api.list(key, &row_label(&row.id)).await?.into_iter().next() {
                    Some(listed) => (listed.id, false),
                    None => {
                        let new = NewBox {
                            name: format!("pinned-{}", row.id),
                            labels: vec![self.label.clone(), row_label(&row.id)],
                            size: row.size,
                            init_command: format!("sh {DIR}/start.sh || true"),
                        };
                        (self.api.create(key, new).await?, true)
                    }
                },
            };
            if !hub.db.box_created(&row.id, &id).await? {
                // The row ended or another hub created its own box meanwhile, so nothing else
                // knows of this one; once its key is disconnected no sweep can find it either.
                if made {
                    self.api.delete(key, &id).await?;
                }
                return Ok(Boot::Moved);
            }
            upstash = Some(id);
        }
        let id = upstash.ok_or_else(|| format!("box {} is booting without a box", row.id))?;
        let machine = row.machine.to_string();
        if !account.is_online(&machine) {
            self.boot_node(hub, account, row, &id, keys).await?;
            while !account.is_online(&machine) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Ok(Boot::Online)
    }

    /// Gives the box's machine a fresh token, writes what its node signs in and runs with, and
    /// runs the bootstrap, which starts the node.
    async fn boot_node(&self, hub: &Hub, account: &Arc<Account>, row: &BoxRow, id: &BoxId, keys: &Keys) -> Result<(), String> {
        let key = keys.api_key.as_str();
        let connectors = hub.connectors(account).await?;
        let token = hub.db.put_machine(row.machine, account.id, &machine_name(&row.spec), "linux").await?;
        hub.push_machines(account).await;
        let creds = Credentials {
            hub: hub.public_url.clone(),
            email: account.email.clone(),
            user_token: String::new(),
            machine_id: row.machine.to_string(),
            machine_token: token,
        };
        let login = connectors.get_settings().github.map_or_else(|| "pinned".to_owned(), |g| g.login);
        // Nothing promises that a file write makes the folders above it.
        let made = self.api.exec(key, id, format!("mkdir -p {DIR}/bin")).await?;
        if made.exit_code != 0 {
            return Err(format!("could not make {DIR}: {}", tail(&made)));
        }
        self.api.write(key, id, &format!("{DIR}/account.json"), serde_json::to_string_pretty(&creds).unwrap()).await?;
        let signs_in = row.signs_in_as();
        let claude_token = connectors.box_claude_token(&signs_in).ok_or_else(|| format!("No Claude token is kept for {signs_in}: {ADD_TOKEN}."))?;
        let keys = Keys { api_key: keys.api_key.clone(), claude_token };
        self.api.write(key, id, &format!("{DIR}/box.env"), box_env(&keys, connectors.github_token().as_deref())).await?;
        self.api.write(key, id, &format!("{DIR}/start.sh"), start_script(&login, row.spec.repo.as_deref())).await?;
        let exec = self.api.exec(key, id, bootstrap_script(&hub.public_url)).await?;
        if exec.exit_code != 0 {
            return Err(format!("the bootstrap failed: {}", tail(&exec)));
        }
        Ok(())
    }

    /// Hands the run to the row's machine, unless its session there already works on it. The
    /// machine answers a run sent again with the session it started.
    async fn dispatch(&self, hub: &Hub, account: &Arc<Account>, row: &BoxRow, spec: &RunSpec) -> Result<Placement, String> {
        match &row.work {
            BoxWork::Run => self.place_run(hub, account, &row.machine.to_string(), &row.connector, &row.subject, spec).await,
            work => self.place_work(hub, account, row, work).await,
        }
    }

    /// Hands a worker or a Captain's message to the row's machine. The machine answers a worker sent
    /// again with the task it started, and a message sent again with its first outcome.
    async fn place_work(&self, hub: &Hub, account: &Arc<Account>, row: &BoxRow, work: &BoxWork) -> Result<Placement, String> {
        let machine = row.machine.to_string();
        let repo = row.spec.repo.clone();
        let token = hub.connectors(account).await?.github_token().filter(|_| repo.as_deref().is_some_and(github::hosts));
        let call = |call| hub.call_machine(account, &machine, call, RUN_WAIT);
        let answer = match work {
            BoxWork::Run => return Err("a run is no worker or Captain".into()),
            BoxWork::Delegate { origin, request, .. } => call(Call::Delegate { origin: origin.clone(), request: request.clone(), repo, token }).await.map_err(String::from)?,
            BoxWork::Captain { request_id, text, sender, busy } => {
                let repo = repo.ok_or("the box has no repository to clone")?;
                let path = call(Call::Clone { repo, token, dest: None }).await.map_err(String::from)?;
                let path = path.as_str().ok_or("the box cloned the repository without naming its folder")?.to_owned();
                call(Call::Enqueue { path, text: text.clone(), request_id: request_id.clone(), sender: sender.clone(), busy: *busy }).await.map_err(String::from)?
            }
        };
        let tab = answer["tabId"].as_str().or(answer["tab"].as_str()).ok_or("the box started no thread")?;
        Ok(Placement { machine, tab: tab.to_owned() })
    }

    /// Hands `spec` to `machine` for this connector and subject.
    async fn place_run(&self, hub: &Hub, account: &Arc<Account>, machine: &str, connector: &str, subject: &Subject, spec: &RunSpec) -> Result<Placement, String> {
        if let Some((id, tab)) = account.placed(connector, subject) {
            if id == machine {
                return Ok(Placement { machine: id, tab });
            }
        }
        let mut spec = spec.clone();
        spec.token = hub.connectors(account).await?.github_token().filter(|_| spec.repo.as_deref().is_some_and(github::hosts));
        // A box's node keeps no agent accounts: it runs as the one whose token its box.env holds.
        spec.account = None;
        let value = hub.call_machine(account, machine, crate::hub::run_call(spec, subject), RUN_WAIT).await.map_err(String::from)?;
        let tab = value.as_str().map(str::to_owned).ok_or_else(|| "the box started the session without a tab id".to_owned())?;
        Ok(Placement { machine: machine.to_owned(), tab })
    }

    /// Ending → gone, each step a no-op once done: the machine, so its node can't sign in again,
    /// then the box, then the row. Without keys the row stays, to be retried once they are back.
    async fn end(&self, hub: &Hub, account: &Arc<Account>, row: &Remains, keys: Option<&Keys>) -> Result<(), String> {
        let machine = row.machine.to_string();
        hub.db.remove_machine(account.id, row.machine).await?;
        account.went_offline(&machine, None);
        hub.push_machines(account).await;
        let keys = keys.ok_or_else(|| format!("Upstash Box is not connected, so box {} stays until it is", row.upstash.as_ref().map_or(&row.id, |b| &b.0)))?;
        let key = keys.api_key.as_str();
        match &row.upstash {
            Some(id) => self.api.delete(key, id).await?,
            None => {
                for listed in self.api.list(key, &row_label(&row.id)).await? {
                    self.api.delete(key, &listed.id).await?;
                }
            }
        }
        hub.db.delete_box(&row.id).await?;
        self.timers.lock().unwrap().remove(&row.id);
        eprintln!("pinned-server: deleted box {} of account {} ({})", row.id, account.id, row.why);
        Ok(())
    }

    /// Ends the row for `why`, unless it is ending already, and deletes its box.
    async fn finish(&self, hub: &Hub, account: &Arc<Account>, mut row: BoxRow, why: End, keys: Option<&Keys>) -> Result<(), String> {
        let failed = matches!(why, End::Failed(_));
        if !matches!(row.phase, Phase::Ending(_)) {
            let ending = Phase::Ending(why);
            if !hub.db.set_box_phase(&row.id, &row.phase, &ending).await? {
                return Err(format!("box {} changed while pinned ended it; it ends on the next check", row.id));
            }
            row.phase = ending;
        }
        self.record_agent_box(hub, account, &row).await;
        self.end(hub, account, &row.remains(), keys).await?;
        if !failed {
            if let Ok(connectors) = hub.connectors(account).await {
                if let Err(e) = connectors.forget_box_grant(&row.connector, &row.subject) {
                    eprintln!("pinned-server: could not clear the box answer for {}: {e}", row.id);
                }
            }
        }
        Ok(())
    }

    /// Saves the task record of an ending box an agent opened, under the opener's machine, where
    /// `delegation_stats` counts it, then forgets the opener. Once per box: a box no agent opened, or
    /// one already recorded, has no opener. A failure is logged and tried on the next look.
    async fn record_agent_box(&self, hub: &Hub, account: &Account, row: &BoxRow) {
        let Phase::Ending(why) = &row.phase else { return };
        let recorded = async {
            let Some((opener, started)) = hub.db.agent_box(account.id, &row.id).await? else { return Ok(()) };
            let mut record = agent_box_record(row, &opener, started, unix_millis(), why);
            record.machine_name = hub.db.machine_name(account.id, opener.machine).await?.unwrap_or_default();
            hub.db.save_task_record(account.id, opener.machine, &record).await?;
            hub.db.forget_agent_box(account.id, &row.id).await
        };
        if let Err(e) = recorded.await {
            eprintln!("pinned-server: could not record box {} as its agent's task: {e}", row.id);
        }
    }

    /// The session for `subject` opened its pull request, so its box ends once it stops working.
    pub async fn work_done(&self, hub: &Hub, account: &Account, connector: &str, subject: &Subject) -> Result<(), String> {
        if let Some(row) = hub.db.box_for(account.id, connector, subject).await? {
            self.timers.lock().unwrap().entry(row).or_default().done = true;
        }
        Ok(())
    }

    /// Removing a box's machine deletes the box too. Answers whether `machine` ran in one.
    pub async fn end_for_machine(&self, hub: &Hub, account: &Arc<Account>, machine: Uuid) -> Result<bool, String> {
        let Some(row) = hub.db.box_of(account.id, machine).await? else { return Ok(false) };
        let keys = hub.connectors(account).await?.box_keys();
        self.finish(hub, account, row, End::Removed, keys.as_ref()).await?;
        Ok(true)
    }

    /// Deletes every box of the account, before its Box key goes.
    pub async fn end_all(&self, hub: &Hub, account: &Arc<Account>) -> Result<(), String> {
        let keys = hub.connectors(account).await?.box_keys();
        for row in hub.db.boxes(account.id).await? {
            match row {
                Ok(row) => self.finish(hub, account, row, End::Disconnected, keys.as_ref()).await?,
                Err(remains) => self.end(hub, account, &remains, keys.as_ref()).await?,
            }
        }
        Ok(())
    }

    /// One pass over the account's rows: boots rows nobody drives, as after a restart, ends running
    /// rows whose session ended, finishes ending rows, and every `sweep` deletes this hub's boxes
    /// that no row names.
    pub async fn reap(&self, hub: &Arc<Hub>, account: &Arc<Account>) {
        if let Err(e) = self.reap_once(hub, account).await {
            eprintln!("pinned-server: could not look after account {}'s boxes: {e}", account.id);
        }
    }

    async fn reap_once(&self, hub: &Arc<Hub>, account: &Arc<Account>) -> Result<(), String> {
        let connectors = hub.connectors(account).await?;
        let keys = connectors.box_keys();
        let due = self.swept.lock().unwrap().get(&account.id).is_none_or(|at| at.elapsed() >= self.policy.sweep);
        // Listed before the rows are read, so every box listed has its row among them.
        let listed = match &keys {
            Some(keys) if due => match self.api.list(&keys.api_key, &self.label).await {
                Ok(listed) => {
                    self.swept.lock().unwrap().insert(account.id, Instant::now());
                    Some(listed)
                }
                // Only the sweep needs the list; the rows' own steps still run.
                Err(e) => {
                    eprintln!("pinned-server: could not list account {}'s boxes: {e}", account.id);
                    None
                }
            },
            _ => None,
        };
        let rows = hub.db.boxes(account.id).await?;
        let now = Instant::now();
        for row in rows {
            let row = match row {
                Ok(row) => row,
                Err(remains) => {
                    if let Err(e) = self.end(hub, account, &remains, keys.as_ref()).await {
                        eprintln!("pinned-server: could not end a box of account {}: {e}", account.id);
                    }
                    continue;
                }
            };
            if self.driving.lock().unwrap().contains(&row.id) {
                continue;
            }
            let ended = match &row.phase {
                Phase::Creating | Phase::Booting => match &keys {
                    Some(keys) => {
                        let (hub, account, keys) = (hub.clone(), account.clone(), keys.clone());
                        tokio::spawn(async move {
                            let Some(_driving) = hub.boxes.drive_guard(&row.id) else { return };
                            if let Err(e) = hub.boxes.drive(&hub, &account, row.clone(), &row.spec, &keys).await {
                                eprintln!("pinned-server: could not finish booting a box of account {}: {e}", account.id);
                            }
                        });
                        continue;
                    }
                    None => Some(End::Disconnected),
                },
                Phase::Running => {
                    let gone = listed.as_ref().zip(row.upstash.as_ref()).is_some_and(|(listed, id)| !listed.iter().any(|b| b.id == *id));
                    // A budget outlives idleness: the box ends when its money runs out, whatever it does.
                    let over_budget = over_budget(&row.spec, row.age);
                    // Later runs reuse this row. The fresh-box timers would delete it between them.
                    if connectors.box_access().owns_row(&row.id) {
                        gone.then_some(End::Vanished)
                    } else {
                        let held = connectors.holds_box(&row.connector, &row.subject);
                        let machine = row.machine.to_string();
                        // A worker's or a Captain's box holds that one thread and the threads it starts.
                        let seen = match row.work {
                            BoxWork::Run => account.seen(&machine, &row.connector, &row.subject),
                            _ => account.seen_any(&machine),
                        };
                        let mut timers = self.timers.lock().unwrap();
                        let slot = timers.entry(row.id.clone()).or_default();
                        if held {
                            slot.quiet_since = None;
                            slot.exited_since = None;
                            slot.gone_since = None;
                            slot.offline_since = None;
                            (row.age >= self.policy.max_life || over_budget).then_some(End::Expired)
                        } else if over_budget {
                            Some(End::Expired)
                        } else {
                            // An interactive session is quiet while its human thinks; only its
                            // budget, its closed or exited tab, or going missing ends it.
                            let seen = match (row.connector.as_str(), seen) {
                                (INTERACTIVE_CONNECTOR, Seen::Tab { running, .. }) => Seen::Tab { running, quiet: false },
                                _ => seen,
                            };
                            let verdict = verdict(row.age, seen, slot, now, &self.policy);
                            if gone { Some(End::Vanished) } else { verdict }
                        }
                    }
                }
                Phase::Ending(_) => None,
            };
            let finished = match ended {
                Some(why) => self.finish(hub, account, row, why, keys.as_ref()).await,
                None if matches!(row.phase, Phase::Ending(_)) => {
                    // A hub that stopped between ending the row and recording it records it now.
                    self.record_agent_box(hub, account, &row).await;
                    self.end(hub, account, &row.remains(), keys.as_ref()).await
                }
                None => Ok(()),
            };
            if let Err(e) = finished {
                eprintln!("pinned-server: could not end a box of account {}: {e}", account.id);
            }
        }
        if let (Some(listed), Some(keys)) = (listed, &keys) {
            for orphan in listed {
                let row = orphan.labels.iter().find_map(|l| l.strip_prefix("p:"));
                if let Some(row) = row {
                    if hub.db.box_exists(row).await? {
                        continue;
                    }
                }
                eprintln!("pinned-server: deleting box {} that no row of this hub names", orphan.id);
                self.api.delete(&keys.api_key, &orphan.id).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn policy() -> Policy {
        Policy { idle: 20 * MIN, exited: 20 * MIN, tab_gone: 20 * MIN, offline: 20 * MIN, max_life: 20 * MIN, ..Policy::default() }
    }

    /// The verdict after `seen` held for `held`, judged at the start and at the end.
    fn after(seen: Seen, held: Duration, age: Duration) -> Option<End> {
        let (start, mut timers) = (Instant::now(), Timers::default());
        assert_eq!(verdict(Duration::ZERO, seen, &mut timers, start, &policy()), None);
        verdict(age, seen, &mut timers, start + held, &policy())
    }

    #[test]
    fn each_end_rule_fires_once_its_condition_held_for_its_limit() {
        let quiet = Seen::Tab { running: true, quiet: true };
        for (seen, end) in [
            (Seen::Offline, End::Offline),
            (Seen::NoTab, End::TabClosed),
            (Seen::Tab { running: false, quiet: true }, End::Exited),
            (quiet, End::Idle),
        ] {
            assert_eq!(after(seen, 19 * MIN, MIN), None, "{seen:?} for 19 of 20 minutes");
            assert_eq!(after(seen, 21 * MIN, MIN), Some(end), "{seen:?} for 21 of 20 minutes");
        }
        assert_eq!(after(Seen::Tab { running: true, quiet: false }, 21 * MIN, MIN), None, "a working session never idles");
    }

    fn spec_with(budget_usd: Option<f64>) -> RunSpec {
        RunSpec { folder: String::new(), repo: Some("https://github.com/o/r.git".into()), args: vec!["--name".into(), "r".into()], env: vec![], token: None, account: None, budget_usd }
    }

    #[test]
    fn an_agents_box_is_its_task_billed_for_its_life_and_recorded_with_how_it_ended() {
        let row = |budget_usd: Option<f64>| BoxRow {
            id: "0123456789abcdef".into(),
            account: Uuid::nil(),
            machine: Uuid::from_u128(9),
            upstash: None,
            connector: INTERACTIVE_CONNECTOR.into(),
            subject: Subject::LinearIssue { issue: "box 0123456789abcdef".into() },
            spec: RunSpec { repo: Some("https://github.com/acme/site.git".into()), args: vec!["--name".into(), "site".into()], ..spec_with(budget_usd) },
            size: BoxSize::Small,
            phase: Phase::Ending(End::TabClosed),
            age: Duration::ZERO,
            work: BoxWork::Run,
        };
        let opener = Opener { machine: Uuid::from_u128(7), tab: "tab-1".into(), role: ActorRole::Captain };
        let half_hour = 30 * 60_000;
        let record = agent_box_record(&row(None), &opener, 1_000, 1_000 + half_hour, &End::TabClosed);
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            serde_json::json!({
                "id": "0123456789abcdef", "machine": "00000000-0000-0000-0000-000000000007", "machine_name": "", "title": "Box · site",
                "delegator": { "kind": "captain", "tab": "tab-1", "project": "github.com/acme/site", "project_name": "site" },
                "provider": "claude", "account": "claude", "target": "box", "started": 1_000, "ended": 1_000 + half_hour,
                "usage": { "tokens_in": 0, "tokens_out": 0, "wall_ms": half_hour, "box_usd": 0.5 * BOX_USD_PER_HOUR, "box_ms": half_hour },
                "outcome": "done",
            })
        );
        let ended = |budget: Option<f64>, why: End| {
            let record = agent_box_record(&row(budget), &opener, 1_000, 1_000 + half_hour, &why);
            (record.outcome, record.reason, record.limit)
        };
        assert_eq!(ended(None, End::Failed("the bootstrap failed: no disk".into())), (TaskOutcome::Failed, Some("the bootstrap failed: no disk".into()), None));
        assert_eq!(ended(Some(0.04), End::Expired), (TaskOutcome::Limit, None, Some("budget".into())), "half an hour spent its 4 cents");
        assert_eq!(ended(Some(1.0), End::Expired), (TaskOutcome::Limit, None, Some("time".into())));
        assert_eq!(ended(None, End::Removed), (TaskOutcome::Cancelled, None, None));
        assert_eq!(ended(None, End::Idle), (TaskOutcome::Done, None, None));
        let worker = Opener { role: ActorRole::Workflow, ..opener };
        assert_eq!(agent_box_record(&row(None), &worker, 5, 1, &End::Done).delegator.kind, DelegatorKind::Session, "a workflow session counts as a session");
        assert_eq!(agent_box_record(&row(None), &worker, 5, 1, &End::Done).usage.box_ms, Some(0), "a clock that went back bills nothing");
    }

    #[test]
    fn a_budget_ends_the_box_when_its_money_runs_out() {
        let hour = Duration::from_secs(3600);
        assert!(!over_budget(&spec_with(None), 100 * hour), "no budget runs free");
        assert!(!over_budget(&spec_with(Some(1.0)), 9 * hour), "$1 buys ten hours");
        assert!(over_budget(&spec_with(Some(1.0)), 10 * hour), "the tenth hour spends the last cent");
        assert!(over_budget(&spec_with(Some(0.1)), hour + Duration::from_secs(1)), "$0.10 buys one hour");
    }

    #[test]
    fn normalize_repo_takes_shorthand_and_https() {
        assert_eq!(normalize_repo("o/r"), Some("https://github.com/o/r.git".into()));
        assert_eq!(normalize_repo("https://github.com/o/r"), Some("https://github.com/o/r.git".into()));
        assert_eq!(normalize_repo("https://github.com/o/r.git"), Some("https://github.com/o/r.git".into()));
        assert_eq!(normalize_repo(""), None);
        assert_eq!(normalize_repo("just-a-name"), None);
        assert_eq!(normalize_repo("git@github.com:o/r.git"), None, "ssh stays out: the box clones with a token");
        assert_eq!(normalize_repo("https://example.com/o/r.git"), None);
        assert_eq!(origin_repo("git@github.com:o/r.git"), Some("https://github.com/o/r.git".into()), "a checkout's ssh origin clones over https");
        assert_eq!(origin_repo("github.com/o/r"), Some("https://github.com/o/r.git".into()), "the repo key projects() lists");
        assert_eq!(origin_repo("git@gitlab.com:o/r.git"), None);
    }

    #[test]
    fn max_life_ends_a_box_whatever_its_session_does() {
        let working = Seen::Tab { running: true, quiet: false };
        let mut timers = Timers::default();
        assert_eq!(verdict(19 * MIN, working, &mut timers, Instant::now(), &policy()), None);
        assert_eq!(verdict(21 * MIN, working, &mut timers, Instant::now(), &policy()), Some(End::Expired));
    }

    #[test]
    fn working_resets_quiet() {
        let p = policy();
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p), None);
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: false }, &mut t, start + 15 * MIN, &p), None, "working again");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 16 * MIN, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 35 * MIN, &p), None, "quiet for 19 minutes since it worked");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 37 * MIN, &p), Some(End::Idle));
    }

    #[test]
    fn a_box_whose_pull_request_is_open_ends_soon_after_its_session_stops() {
        let p = Policy { done: 2 * MIN, ..policy() };
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 3 * MIN, &p), None, "quiet without a pull request waits out idle");
        t.done = true;
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: false }, &mut t, start + 4 * MIN, &p), None, "still working");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 5 * MIN, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 6 * MIN, &p), None, "quiet for 1 of 2 minutes");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 7 * MIN, &p), Some(End::Done));
    }

    #[test]
    fn a_dropped_link_does_not_restart_the_quiet_clock() {
        let p = policy();
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p), None);
        assert_eq!(verdict(MIN, Seen::Offline, &mut t, start + 10 * MIN, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 19 * MIN, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 21 * MIN, &p), Some(End::Idle));
    }

    #[test]
    fn phases_survive_the_database() {
        for phase in [
            Phase::Creating,
            Phase::Booting,
            Phase::Running,
            Phase::Ending(End::Idle),
            Phase::Ending(End::TabClosed),
            Phase::Ending(End::Done),
            Phase::Ending(End::Failed("the bootstrap failed: curl: (22) 404".into())),
        ] {
            let (stored, reason) = phase.stored();
            assert_eq!(Phase::load(stored, reason), phase);
        }
    }

    #[test]
    fn a_hub_labels_its_boxes_within_upstashs_20_characters() {
        let label = hub_label("https://pinned-hub-production.up.railway.app");
        assert_eq!(label.len(), 16);
        assert!(label.starts_with("hub:"));
        assert_ne!(label, hub_label("http://127.0.0.1:8797"));
        assert_eq!(row_label(&new_row_id()).len(), 18);
    }

    #[test]
    fn the_scripts_a_box_runs_carry_no_secret() {
        let keys = Keys { api_key: "box_SECRETKEY".into(), claude_token: "sk-ant-oat01-SECRETCLAUDE".into() };
        let scripts = [bootstrap_script("https://hub.example"), start_script("felipe", Some("acme/app"))];
        for script in &scripts {
            for secret in ["box_SECRETKEY", "SECRETCLAUDE", "ghp_SECRETGITHUB", "machine-token"] {
                assert!(!script.contains(secret), "{secret} in {script}");
            }
            assert!(!script.contains("{{"), "{script}");
        }
        assert!(scripts[0].contains("curl -fsSL 'https://hub.example/download/pinned-server-aarch64-unknown-linux-musl'"));
        assert!(scripts[1].contains("git config --global user.email 'felipe@users.noreply.github.com'"));
        assert!(scripts[1].contains(r#"const clones = ["app"].map"#), "{}", scripts[1]);
        assert!(start_script("felipe", None).contains("const clones = [].map"));
        assert_eq!(
            box_env(&keys, Some("ghp_SECRETGITHUB")),
            "CLAUDE_CODE_OAUTH_TOKEN='sk-ant-oat01-SECRETCLAUDE'\nGH_TOKEN='ghp_SECRETGITHUB'\n"
        );
        assert_eq!(start_script("o'brien", None).lines().find(|l| l.contains("user.name")), Some("git config --global user.name 'o'\\''brien'"));
    }

    #[test]
    fn a_box_machine_is_named_after_its_session() {
        let spec = |args: &[&str]| RunSpec { folder: "/repo".into(), repo: None, args: args.iter().map(|a| a.to_string()).collect(), env: vec![], token: None, account: None, budget_usd: None };
        assert_eq!(machine_name(&spec(&["--effort", "high", "--name", "ENG-142 Fix login", "prompt"])), "Box · ENG-142 Fix login");
        assert_eq!(machine_name(&spec(&["prompt"])), "Box · pinned");
        assert_eq!(machine_name(&spec(&["--name", &"x".repeat(200)])).chars().count(), 100);
    }

    #[test]
    fn a_failed_bootstrap_quotes_the_end_of_its_output() {
        let long = Exec { exit_code: 1, output: format!("{}\nlast line", "x".repeat(2000)), error: "curl: (22) 404\n".into() };
        let quoted = tail(&long);
        assert_eq!(quoted.chars().count(), TAIL);
        assert!(quoted.ends_with("last linecurl: (22) 404"));
    }
}
