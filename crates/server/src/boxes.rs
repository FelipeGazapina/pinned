//! The Upstash Boxes a workflow runs in. The account's box permission chooses a new box per issue
//! or thread, one box reused by later runs, a question before any box, or none. A box boots as a
//! node of the account, is given the run, and is deleted with its machine once the session ends.
//! The `boxes` table is the only source of truth. Every step checks it before acting, so the first
//! pass after a restart finishes whatever the last hub left halfway.

use crate::hub::{Account, Hub, RUN_WAIT};
use pinned_core::delegate::{wait_span, BOX_BOOTING, BOX_CONNECT_WAIT, BOX_OFFLINE};
use futures_util::future::BoxFuture;
use pinned_core::account::Credentials;
use pinned_core::accounts::{self, Provider};
use pinned_core::agent::{session_env, Subject, CAPTAIN_CONNECTOR, DELEGATE_CONNECTOR, INTERACTIVE_CONNECTOR};
use pinned_core::connectors::{unix_millis, BoxAccess, BoxEndRelease, BoxPermission, BoxSize, BoxStep, Connectors, FixedClaim, Placement, Run};
use pinned_core::delegate::{BoxTarget, Delegator, DelegatorKind, Origin, RemoteEnd, Request, RunTarget, TaskOutcome, TaskRecord, TaskUsage};
use pinned_core::surface::ActorRole;
use pinned_core::global_orchestrator::BusyPolicy;
use serde::{Deserialize, Serialize};
use pinned_core::upstash::{self, BoxId, BoxUsage, Exec, Keys, Listed, NewBox, PinnedBox, PinnedCount};
use pinned_core::wire::{BoxStatus, Call, ProjectView, RunSpec, box_usd_per_hour};
use pinned_core::github;
use pinned_core::machine;
use pinned_core::taste::{BoxSizeChoice, TasteScope, Tastes};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const DIR: &str = "/workspace/home/.pinned";
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
            boot_wait: BOX_CONNECT_WAIT,
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

/// The verdict on a box a workflow run holds. The run waits between its session's turns, so only
/// its node going missing, its max life, its budget or its box vanishing ends the box, and
/// `fail_held_runs` fails the run with it. Its idle, exited and gone timers are cleared, so one
/// that started before the run held the box can't end it as Done, Idle or Exited while its node is
/// offline, which `verdict` reads as a condition it can't see.
fn held_verdict(age: Duration, seen: Seen, t: &mut Timers, now: Instant, p: &Policy) -> Option<End> {
    t.quiet_since = None;
    t.exited_since = None;
    t.gone_since = None;
    let seen = match seen {
        Seen::Offline => Seen::Offline,
        _ => Seen::Tab { running: true, quiet: false },
    };
    verdict(age, seen, t, now, p, false)
}

/// Why a box's work failed when the box ended for `why`, or None when it ended for its work's end.
fn box_end_error(p: &Policy, why: &End) -> Option<String> {
    Some(match why {
        End::Done => return None,
        End::Failed(e) => format!("Upstash Box: {e}"),
        End::Offline => format!("Upstash Box: its box went offline and did not come back within {}", wait_span(p.offline)),
        why => format!("Upstash Box: the box ended before its task did: {}", ended_because(why)),
    })
}

/// Why a workflow run fails when its box ends for `why`. `Done` too: a run that already finished
/// holds nothing, so `box_ended` leaves it, and one still going fails.
fn run_end_error(p: &Policy, why: &End) -> String {
    box_end_error(p, why).unwrap_or_else(|| format!("Upstash Box: the box ended before its run did: {}", ended_because(why)))
}

/// What a box's end does to the subject of the workflow run it failed. Ends the owner chose keep
/// it handled for the owner to run again: a budget or max life (`Expired`, both), a removed box
/// machine and a disconnected Upstash. A box that vanished from Upstash, went offline or failed to
/// boot frees it once (`Connectors::box_ended`).
fn release_for(why: &End) -> BoxEndRelease {
    match why {
        End::Expired | End::Removed | End::Disconnected => BoxEndRelease::Keep,
        _ => BoxEndRelease::Once,
    }
}

/// Why a box ended, as the task it ran hears it.
fn ended_because(why: &End) -> String {
    match why {
        End::TabClosed => "its worker's thread was closed".into(),
        End::Exited => "its worker exited".into(),
        End::Idle => "its worker sat idle".into(),
        End::Done => "its work was done".into(),
        End::Offline => "it went offline".into(),
        End::Expired => "it ran out of time or budget".into(),
        End::Removed => "someone removed its machine".into(),
        End::Vanished => "it was deleted outside pinned".into(),
        End::Disconnected => "Upstash Box was disconnected".into(),
        End::Failed(e) => e.clone(),
    }
}

/// How long the hub waits on the delegating machine to take a box's end.
const ORIGIN_WAIT: Duration = Duration::from_secs(30);

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
    /// `quiet`: nothing holds the box, so the session is neither working nor waiting on an open
    /// questionnaire.
    Tab { running: bool, quiet: bool },
}

/// When each condition of a running row began, by the hub's clock. A restart resets them, and
/// `max_life` still bounds what a box costs.
#[derive(Default, Debug)]
pub(crate) struct Timers {
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
/// The size a box that clones `repo` opens at: `asked` when the call names one, else the
/// project's and then the account's default box size in the way of working, else small. Every
/// path that opens a box takes its size from here.
pub async fn box_size(hub: &Hub, account: &Account, repo: Option<&str>, asked: Option<BoxSize>) -> Result<BoxSizeChoice, String> {
    Ok(choose_box_size(&hub.db.tastes(account.id).await?.unwrap_or_default(), repo, asked))
}

/// `box_size` on tastes already read.
fn choose_box_size(tastes: &Tastes, repo: Option<&str>, asked: Option<BoxSize>) -> BoxSizeChoice {
    // A workflow may name its repo as owner/repo, which `repo_key` does not read.
    let repo = repo.map(|repo| origin_repo(repo).unwrap_or_else(|| repo.to_owned()));
    tastes.box_size(repo.as_deref(), asked)
}

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
        End::Expired if over_budget(&row.spec, row.size, Duration::from_millis(box_ms)) => (TaskOutcome::Limit, None, Some("budget".to_owned())),
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
    let usage = TaskUsage { wall_ms: box_ms, box_ms: Some(box_ms), box_usd: Some(box_ms as f64 / 3_600_000.0 * box_usd_per_hour(row.size)), ..TaskUsage::default() };
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

/// A worker box's answer to the `delegate_task` that made it, and to every repeat of it.
fn started_box(row: &BoxRow) -> serde_json::Value {
    serde_json::json!({ "machine": row.machine.to_string(), "machineName": machine_name(&row.spec), "box": row.id, "workState": "working" })
}

/// Whether the spec's budget paid for `age` at the hourly rate of a box of `size`. None budgets run free.
fn over_budget(spec: &RunSpec, size: BoxSize, age: Duration) -> bool {
    spec.budget_usd.is_some_and(|b| b > 0.0 && age.as_secs_f64() * box_usd_per_hour(size) / 3600.0 >= b)
}

/// What the hub answers about the worker box on `machine` while its node is offline. A row turns
/// `Running` once its node connected and took its task, so one before that never connected and still
/// boots; one after it dropped its link, and the reaper ends it once it stays offline past
/// `Policy::offline`. An ending row whose machine is `removed` outlives that machine only while its box
/// can't be deleted (as with Upstash disconnected), so it reads as gone, as it will once the row
/// goes: an older parent would otherwise wait on it. Its origin was told why it ended unless the tell
/// failed, the hub restarted before it, or the origin cancelled it; then the origin reads it as gone.
pub fn offline_answer(phase: &Phase, removed: bool, machine: &str) -> String {
    match phase {
        Phase::Creating | Phase::Booting => BOX_BOOTING.to_owned(),
        Phase::Ending(_) if removed => format!("this account has no machine {machine}"),
        Phase::Running | Phase::Ending(_) => BOX_OFFLINE.to_owned(),
    }
}

/// `delegating` is a box whose tab waits on workers in other boxes. It is quiet, or done with its own
/// turn, while they run, so neither starts its Idle, Done or Exited timer: only its max life (and,
/// before this, its budget) ends it then, so their ends find it.
pub(crate) fn verdict(age: Duration, seen: Seen, t: &mut Timers, now: Instant, p: &Policy, delegating: bool) -> Option<End> {
    // `None` is a condition this look can't see, as the session's while its node is offline: its
    // timer neither starts nor restarts, so a dropped link doesn't buy a quiet box more time.
    let (offline, gone, exited, quiet) = match seen {
        Seen::Offline => (Some(true), None, None, None),
        Seen::NoTab => (Some(false), Some(true), None, None),
        Seen::Tab { .. } if delegating => (Some(false), Some(false), Some(false), Some(false)),
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

/// A project's short name: its repository's, else its folder's.
fn project_short<'a>(folder: Option<&'a str>, repo: Option<&'a str>) -> Option<&'a str> {
    repo.and_then(machine::repo_name)
        .or_else(|| folder.and_then(|f| std::path::Path::new(f).file_name()?.to_str()))
}

/// A project's name as a refusal reads it: its repository's, else its folder's.
pub(crate) fn project_name(folder: Option<&str>, repo: Option<&str>) -> String {
    project_short(folder, repo)
        .unwrap_or("this project")
        .to_owned()
}

/// Upstash's docs give no limit on a box's name, so a box name stays within this many characters.
const BOX_NAME_MAX: usize = 40;

/// What a box is for, as its Upstash name says it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BoxRole {
    Worker,
    Captain,
    Workflow,
    Session,
}

impl BoxRole {
    /// The role of a row's box, from the work the row was written with.
    fn of(row: &BoxRow) -> BoxRole {
        match &row.work {
            BoxWork::Delegate { .. } => BoxRole::Worker,
            BoxWork::Captain { .. } => BoxRole::Captain,
            BoxWork::Run if row.connector == INTERACTIVE_CONNECTOR => BoxRole::Session,
            BoxWork::Run => BoxRole::Workflow,
        }
    }

    fn word(self) -> &'static str {
        match self {
            BoxRole::Worker => "worker",
            BoxRole::Captain => "captain",
            BoxRole::Workflow => "workflow",
            BoxRole::Session => "session",
        }
    }
}

/// The name a box carries on Upstash: `<project>-<role>-<short id>`, where the project is the
/// repository's short name, else its folder's. Pinned finds its boxes by label, not by this name,
/// so it only tells the dashboard's reader which project a box works on.
fn box_name(folder: Option<&str>, repo: Option<&str>, role: BoxRole, row_id: &str) -> String {
    let short: String = row_id.chars().take(6).collect();
    let tail = format!("-{}-{short}", role.word());
    let project = sanitize_project(
        project_short(folder, repo).unwrap_or(""),
        BOX_NAME_MAX - tail.len(),
    );
    let project = if project.is_empty() {
        "project"
    } else {
        &project
    };
    format!("{project}{tail}")
}

/// `name` as lowercase ASCII letters, digits and `-`: accents folded where simple, every other
/// character a `-`, repeated `-` collapsed, and the result cut to `max` characters.
fn sanitize_project(name: &str, max: usize) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        let c = match c {
            'a'..='z' | '0'..='9' => c,
            _ => fold_accent(c).unwrap_or('-'),
        };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out.truncate(max);
    out.trim_matches('-').to_owned()
}

/// The unaccented letter of a lowercase Latin letter with an accent, when it has one.
fn fold_accent(c: char) -> Option<char> {
    Some(match c {
        'à'..='å' => 'a',
        'ç' => 'c',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ñ' => 'n',
        'ò'..='ö' | 'ø' => 'o',
        'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        _ => return None,
    })
}

/// The account's Upstash Box usage for the Usage page, and pinned's own count of the box time its
/// delegated tasks used this UTC month. The key stays here: an error that names it is scrubbed.
pub async fn usage(hub: &Hub, account: &Arc<Account>) -> Result<BoxUsage, String> {
    let Some(keys) = hub.connectors(account).await?.box_keys() else {
        return Ok(BoxUsage::default());
    };
    let since = upstash::month_start(unix_millis() / 1000);
    let (box_ms, tasks) = hub.db.box_ms_since(account.id, since * 1000).await?;
    let mut usage = BoxUsage { connected: true, pinned: PinnedCount { box_hours: box_ms as f64 / 3_600_000.0, tasks, since }, ..BoxUsage::default() };
    match upstash::list_all(&keys.api_key).await {
        Ok(listed) => {
            let (mut boxes, totals) = upstash::usage_boxes(&listed, since);
            let opened = hub.db.opened_boxes(account.id).await?;
            for row in &mut boxes {
                row.pinned = opened.iter().find(|o| o.upstash == row.id).map(|o| PinnedBox { project: Some(project_name(o.folder.as_deref(), o.repo.as_deref())), role: o.role.clone(), machine: o.machine.clone() });
            }
            (usage.boxes, usage.upstash) = (boxes, totals);
        }
        Err(e) => usage.error = Some(format!("Upstash did not answer: {}", e.replace(&keys.api_key, "…"))),
    }
    Ok(usage)
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

/// What the node and its sessions run with, as `start.sh` sources it: the secrets, and, when
/// `leaves`, `PINNED_BOX=1`, which tells the node it runs in a box and may exit for good once the hub
/// has refused it for 15 minutes.
fn box_env(keys: &Keys, github: Option<&str>, leaves: bool) -> String {
    let mut env = format!("CLAUDE_CODE_OAUTH_TOKEN='{}'\n", quoted(&keys.claude_token));
    if let Some(token) = github {
        env += &format!("GH_TOKEN='{}'\n", quoted(token));
    }
    if leaves {
        env += &format!("{}=1\n", crate::box_exit::BOX_ENV);
    }
    env
}

/// Whether the node in row `row`'s box may leave once the hub has refused it for 15 minutes. Not
/// in the fixed box ("One box, reused"): the reaper ends that row only when Upstash stops listing
/// the box, so a node that left would leave every later run failing with its machine offline. A
/// fixed box its hub has disowned keeps running until the fixed-box limit (#247) lands.
fn leaves_when_refused(access: &BoxAccess, row: &str) -> bool {
    !access.owns_row(row)
}

/// A box's budget pays for at least an hour on its size.
fn minimum_budget(size: BoxSize, budget: f64) -> Result<(), String> {
    let hour = box_usd_per_hour(size);
    if budget >= hour {
        return Ok(());
    }
    Err(format!("name at least ${hour:.2}, an hour on a {} box", size.name()))
}

/// What the app shows of a box: its phase, age, budget and the hourly rate of its size.
fn status_of(row: BoxRow, tab: Option<String>) -> BoxStatus {
    let phase = match &row.phase {
        Phase::Creating => "creating",
        Phase::Booting => "booting",
        Phase::Running => "running",
        Phase::Ending(_) => "ending",
    }
    .to_owned();
    BoxStatus { id: row.id, phase, machine: row.machine.to_string(), tab, age_secs: row.age.as_secs_f64(), budget_usd: row.spec.budget_usd, rate_usd_per_hour: box_usd_per_hour(row.size) }
}

/// The box to create for `row`, under the hub label `label`. Only the fixed box ("One box,
/// reused") keeps running while idle, at Upstash's fixed monthly price per box; every other box
/// is a standard one that Upstash bills for its active CPU time and pauses when idle.
fn new_box(access: &BoxAccess, row: &BoxRow, label: &str) -> NewBox {
    NewBox {
        name: box_name(Some(row.spec.folder.as_str()).filter(|f| !f.is_empty()), row.spec.repo.as_deref(), BoxRole::of(row), &row.id),
        labels: vec![label.to_owned(), row_label(&row.id)],
        size: row.size,
        keep_alive: access.owns_row(&row.id),
        init_command: format!("sh {DIR}/start.sh || true"),
    }
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

/// The key the way of working keeps `repo`'s project under, as in `github.com/acme/site`, from
/// any form a box takes: owner/repo, a clone URL, or the key itself.
pub fn project_key(repo: &str) -> Option<String> {
    origin_repo(repo).as_deref().and_then(machine::repo_key)
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
    pub async fn open_session(&self, hub: &Arc<Hub>, account: &Arc<Account>, repo: String, size: Option<BoxSize>, budget_usd: Option<f64>, opener: Option<Opener>) -> Result<BoxStatus, String> {
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
        let size = box_size(hub, account, Some(&repo), size).await?.size();
        let budget_usd = box_budget(hub, account, Some(&repo), budget_usd).await?;
        if let Some(budget) = budget_usd {
            minimum_budget(size, budget)?;
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
        Ok(status_of(fresh, None))
    }

    /// `delegate_task` with `target.box`: a new box, opened with the account's own connection,
    /// that runs `request` as a worker for `origin`. It answers at once with the box's machine; the
    /// box boots and starts the worker in the background, and a boot that fails ends the task on
    /// `origin`'s machine. The box signs in as the account `request.target.account` names, else as
    /// the one `origin`'s tab runs under.
    pub async fn open_delegate(&self, hub: &Arc<Hub>, account: &Arc<Account>, origin: Origin, mut request: Request, repo: Option<String>, boxed: BoxTarget) -> Result<serde_json::Value, String> {
        let connectors = hub.connectors(account).await?;
        let keys = connectors.box_keys().ok_or(NOT_CONNECTED)?;
        // The row's subject names the origin task, so a repeat of a start finds the box that start
        // made, with no box key, cap or account checks to pass a second time.
        let subject = Subject::LinearIssue { issue: format!("task {}/{}", origin.machine, origin.task) };
        if let Some(row) = hub.db.box_row_for(account.id, DELEGATE_CONNECTOR, &subject).await? {
            return Ok(started_box(&row));
        }
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
        let work = BoxWork::Delegate { origin, request: Box::new(request), claude: Some(claude) };
        let row = self.reserve_work(hub, account, DELEGATE_CONNECTOR, subject, spec, boxed.size, work, &keys).await?;
        Ok(started_box(&row))
    }

    /// The global orchestrator's `send_to_captain` with `box`: the Captain of `repo` in a box
    /// opened with the account's connection, started on `text`. A box that already runs that
    /// project's Captain takes the message as a Captain on a machine does.
    #[allow(clippy::too_many_arguments)]
    pub async fn open_captain(&self, hub: &Arc<Hub>, account: &Arc<Account>, request_id: String, repo: &str, size: Option<BoxSize>, text: String, sender: String, busy: Option<BusyPolicy>) -> Result<serde_json::Value, String> {
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

    /// Writes a row for `work` and boots its box in the background, at `size` or the default box
    /// size of `spec`'s repo. A row the subject already has is answered as it is, so a repeat
    /// opens nothing.
    #[allow(clippy::too_many_arguments)]
    async fn reserve_work(&self, hub: &Arc<Hub>, account: &Arc<Account>, connector: &str, subject: Subject, spec: RunSpec, size: Option<BoxSize>, work: BoxWork, keys: &Keys) -> Result<BoxRow, String> {
        let size = box_size(hub, account, spec.repo.as_deref(), size).await?.size();
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
            }
        });
        Ok(row)
    }

    /// The worker box behind `machine`, when it is one.
    pub async fn delegate_box(&self, hub: &Hub, account: &Account, machine: &str) -> Result<Option<BoxRow>, String> {
        let Ok(machine) = Uuid::parse_str(machine) else { return Ok(None) };
        Ok(hub.db.box_of(account.id, machine).await?.filter(|row| matches!(row.work, BoxWork::Delegate { .. })))
    }

    /// The worker's task ended or was cancelled, so its box goes for `why`: `Done`, or `Offline`
    /// for a cancel while its node was offline. The machine that delegated the task knows already.
    pub async fn end_delegate(&self, hub: &Hub, account: &Arc<Account>, row: BoxRow, why: End) -> Result<(), String> {
        let key = hub.connectors(account).await?.box_api_key();
        self.finish_told(hub, account, row, why, key.as_deref(), false).await
    }

    /// The box behind `machine`: its phase, age and budget, for its cost display.
    pub async fn box_status(&self, hub: &Hub, account: &Arc<Account>, machine: Uuid) -> Result<BoxStatus, String> {
        let row = hub.db.box_of(account.id, machine).await?.ok_or_else(|| "this machine is no box".to_owned())?;
        let tab = account.placed(&row.connector, &row.subject).map(|(_, tab)| tab);
        Ok(status_of(row, tab))
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
        let size = match box_size(hub, account, spec.repo.as_deref(), step.size).await {
            Ok(choice) => choice.size(),
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
            size,
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
                Err(_) => Err(format!("the box did not connect within {}", pinned_core::delegate::wait_span(self.policy.boot_wait))),
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
                    self.tell_origin(hub, account, &row, &End::Failed(e.clone())).await;
                    if row.work == BoxWork::Run && row.connector != INTERACTIVE_CONNECTOR {
                        if let Ok(connectors) = hub.connectors(account).await {
                            connectors.box_boot_failed(&row.subject);
                        }
                    }
                    self.record_agent_box(hub, account, &row).await;
                    if let Err(e) = self.end(hub, account, &row.remains(), Some(&keys.api_key)).await {
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
                        let access = hub.connectors(account).await?.box_access();
                        (self.api.create(key, new_box(&access, row, &self.label)).await?, true)
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
        let leaves = leaves_when_refused(&connectors.box_access(), &row.id);
        self.api.write(key, id, &format!("{DIR}/box.env"), box_env(&keys, connectors.github_token().as_deref(), leaves)).await?;
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
    /// then the box, then the row. Deleting needs only the Box API key, not the Claude token. Without
    /// it the row stays, to be retried once it is back.
    async fn end(&self, hub: &Hub, account: &Arc<Account>, row: &Remains, key: Option<&str>) -> Result<(), String> {
        let machine = row.machine.to_string();
        hub.db.remove_machine(account.id, row.machine).await?;
        account.went_offline(&machine, None);
        hub.push_machines(account).await;
        let key = key.ok_or_else(|| format!("Upstash Box is not connected, so box {} stays until it is", row.upstash.as_ref().map_or(&row.id, |b| &b.0)))?;
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

    /// A worker box ended before its task did: the machine whose tab delegated the task hears it
    /// failed, so it does not wait on it. One that can't hear it now gets it from the hub's held ends. A box that ends for its task's end, `Done`, has nothing to tell.
    async fn tell_origin(&self, hub: &Hub, account: &Account, row: &BoxRow, why: &End) {
        let BoxWork::Delegate { origin, .. } = &row.work else { return };
        let Some(error) = self.box_end_error(why) else { return };
        let mut end = RemoteEnd { state: "failed".into(), summary: None, error: Some(error), usage: None };
        self.add_end_box_usage(hub, account, row, &mut end).await;
        let ended = Call::RemoteEnded { from: row.machine.to_string(), tab: origin.tab.clone(), task: origin.task.clone(), end: end.clone() };
        if let Err(e) = hub.call_machine(account, &origin.machine, ended, ORIGIN_WAIT).await {
            eprintln!("pinned-server: could not tell {} that box {} ended: {}", origin.machine, row.id, String::from(e));
            // Held for it, unless the worker's own end is held already: that one stays.
            match crate::remote::hold_end(hub, account, &origin.machine, &row.machine.to_string(), &origin.tab, &origin.task, end, false).await {
                Ok(true) => {}
                Ok(false) => eprintln!("pinned-server: {} is no machine of account {} any more, so box {}'s end goes untold", origin.machine, account.id, row.id),
                Err(e) => eprintln!("pinned-server: box {}'s end goes untold: {e}", row.id),
            }
        }
    }

    /// Why a box's work failed when the box ended for `why`, or None when it ended for its work's end.
    fn box_end_error(&self, why: &End) -> Option<String> {
        box_end_error(&self.policy, why)
    }

    /// A workflow box ended before its run did: the runs it
    /// holds fail with why. A box that ran out of budget or reached its max life keeps their
    /// subjects handled, so only their owner runs them again; so do a removed machine and a disconnected Upstash; any other end frees each subject for
    /// its next trigger once (`BoxEndRelease`). Nothing opens another box or session for them. For the fixed row, which many runs share, only the runs whose
    /// session is on its machine. For another row, a run of its subject that started after the row was written is not its run, so a retried end (the row stays `Ending`) never fails a newer
    /// run of the subject that works elsewhere. A run that already finished holds nothing, so this does nothing.
    /// An error leaves the runs as they were: the caller keeps the row `Ending`, so the reaper's
    /// next pass tries again before it deletes the row.
    async fn fail_held_runs(&self, hub: &Hub, account: &Account, row: &BoxRow, why: &End) -> Result<(), String> {
        if row.work != BoxWork::Run || row.connector == INTERACTIVE_CONNECTOR {
            return Ok(());
        }
        let error = run_end_error(&self.policy, why);
        let failed = async {
            let connectors = hub.connectors(account).await?;
            // The run this box held started before its row was written (now less its age).
            let written = unix_millis().saturating_sub(u64::try_from(row.age.as_millis()).unwrap_or(u64::MAX));
            let holder = (!connectors.box_access().owns_row(&row.id)).then_some((row.connector.as_str(), &row.subject, written));
            connectors.box_ended(&row.machine.to_string(), holder, &error, release_for(why)).await
        };
        let n = failed.await.map_err(|e| format!("could not fail the runs of box {}, so it stays until the next check: {e}", row.id))?;
        if n > 0 {
            eprintln!("pinned-server: box {} of account {} ended, so {n} workflow run(s) failed", row.id, account.id);
        }
        Ok(())
    }

    /// The box time a worker box's task has used, in ms: once it ended, what the box's task
    /// records were billed, which `Hub::record_tasks` bills at the box rate; before that, or when no
    /// record came, the box's life so far.
    async fn box_time(&self, hub: &Hub, account: &Account, row: &BoxRow, ended: bool) -> u64 {
        let age = u64::try_from(row.age.as_millis()).unwrap_or(u64::MAX);
        if !ended {
            return age;
        }
        match hub.db.billed_box_ms(account.id, row.machine, "").await {
            Ok(billed) if billed > 0 => billed,
            _ => age,
        }
    }

    fn with_box_time(usage: Option<TaskUsage>, size: BoxSize, ms: u64) -> TaskUsage {
        let mut usage = usage.unwrap_or_default();
        usage.wall_ms = usage.wall_ms.max(ms);
        usage.box_ms = Some(ms);
        usage.box_usd = Some(ms as f64 / 3_600_000.0 * box_usd_per_hour(size));
        usage
    }

    /// Adds the box's cost to the `usage` of a worker's status answer, as `delegate_task`'s
    /// caller shows it.
    pub async fn add_box_usage(&self, hub: &Hub, account: &Account, row: &BoxRow, answer: &mut serde_json::Value) {
        let ended = matches!(answer["workState"].as_str(), Some("result_available" | "failed" | "cancelled"));
        let ms = self.box_time(hub, account, row, ended).await;
        let usage = Self::with_box_time(serde_json::from_value(answer["usage"].clone()).ok(), row.size, ms);
        answer["usage"] = serde_json::to_value(usage).unwrap_or_default();
    }

    /// Adds the box's cost to the usage of an end the worker's machine reports.
    pub async fn add_end_box_usage(&self, hub: &Hub, account: &Account, row: &BoxRow, end: &mut RemoteEnd) {
        let ms = self.box_time(hub, account, row, true).await;
        end.usage = Some(Self::with_box_time(end.usage, row.size, ms));
    }

    /// Ends the row for `why`, unless it is ending already, and deletes its box.
    async fn finish(&self, hub: &Hub, account: &Arc<Account>, row: BoxRow, why: End, key: Option<&str>) -> Result<(), String> {
        self.finish_told(hub, account, row, why, key, true).await
    }

    /// `finish`, telling a worker box's origin only when `tell`.
    async fn finish_told(&self, hub: &Hub, account: &Arc<Account>, mut row: BoxRow, why: End, key: Option<&str>, tell: bool) -> Result<(), String> {
        let failed = matches!(why, End::Failed(_));
        if !matches!(row.phase, Phase::Ending(_)) {
            let ending = Phase::Ending(why.clone());
            if !hub.db.set_box_phase(&row.id, &row.phase, &ending).await? {
                return Err(format!("box {} changed while pinned ended it; it ends on the next check", row.id));
            }
            row.phase = ending;
            if tell {
                self.tell_origin(hub, account, &row, &why).await;
            }
        }
        self.fail_held_runs(hub, account, &row, &why).await?;
        self.record_agent_box(hub, account, &row).await;
        self.end(hub, account, &row.remains(), key).await?;
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
        let key = hub.connectors(account).await?.box_api_key();
        self.finish(hub, account, row, End::Removed, key.as_deref()).await?;
        Ok(true)
    }

    /// Deletes every box of the account, before its Box key goes.
    pub async fn end_all(&self, hub: &Hub, account: &Arc<Account>) -> Result<(), String> {
        let key = hub.connectors(account).await?.box_api_key();
        for row in hub.db.boxes(account.id).await? {
            match row {
                Ok(row) => self.finish(hub, account, row, End::Disconnected, key.as_deref()).await?,
                Err(remains) => self.end(hub, account, &remains, key.as_deref()).await?,
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
        // Only a box's boot needs both keys. Listing and deleting need the Box API key alone.
        let keys = connectors.box_keys();
        let key = connectors.box_api_key();
        let due = self.swept.lock().unwrap().get(&account.id).is_none_or(|at| at.elapsed() >= self.policy.sweep);
        // Listed before the rows are read, so every box listed has its row among them.
        let listed = match &key {
            Some(key) if due => match self.api.list(key, &self.label).await {
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
        // The machines whose tabs delegated a task that still runs in a worker box.
        let delegating: HashSet<String> = rows
            .iter()
            .filter_map(|row| match row {
                Ok(BoxRow { phase, work: BoxWork::Delegate { origin, .. }, .. }) if !matches!(phase, Phase::Ending(_)) => Some(origin.machine.clone()),
                _ => None,
            })
            .collect();
        let now = Instant::now();
        for row in rows {
            let row = match row {
                Ok(row) => row,
                Err(remains) => {
                    if let Err(e) = self.end(hub, account, &remains, key.as_deref()).await {
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
                    let over_budget = over_budget(&row.spec, row.size, row.age);
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
                            let verdict = held_verdict(row.age, seen, slot, now, &self.policy);
                            if over_budget {
                                Some(End::Expired)
                            } else if gone {
                                Some(End::Vanished)
                            } else {
                                verdict
                            }
                        } else if over_budget {
                            Some(End::Expired)
                        } else {
                            // An interactive session is quiet while its human thinks; only its
                            // budget, its closed or exited tab, or going missing ends it.
                            let seen = match (row.connector.as_str(), seen) {
                                (INTERACTIVE_CONNECTOR, Seen::Tab { running, .. }) => Seen::Tab { running, quiet: false },
                                _ => seen,
                            };
                            let was_offline = slot.offline_since;
                            let verdict = verdict(row.age, seen, slot, now, &self.policy, delegating.contains(&machine));
                            if matches!(row.work, BoxWork::Delegate { .. }) {
                                match (was_offline, slot.offline_since) {
                                    (None, Some(_)) => eprintln!("pinned-server: box {} of account {} went offline", row.id, account.id),
                                    (Some(at), None) => eprintln!("pinned-server: box {} of account {} is back after {} s", row.id, account.id, now.duration_since(at).as_secs()),
                                    _ => {}
                                }
                            }
                            if gone { Some(End::Vanished) } else { verdict }
                        }
                    }
                }
                Phase::Ending(_) => None,
            };
            let finished = match ended {
                Some(why) => self.finish(hub, account, row, why, key.as_deref()).await,
                None if matches!(row.phase, Phase::Ending(_)) => {
                    // A hub that stopped between ending the row and recording it records it now,
                    // and fails the runs it held.
                    if let Phase::Ending(why) = &row.phase {
                        self.fail_held_runs(hub, account, &row, why).await?;
                    }
                    self.record_agent_box(hub, account, &row).await;
                    self.end(hub, account, &row.remains(), key.as_deref()).await
                }
                None => Ok(()),
            };
            if let Err(e) = finished {
                eprintln!("pinned-server: could not end a box of account {}: {e}", account.id);
            }
        }
        if let (Some(listed), Some(key)) = (listed, &key) {
            let named = |row: String| -> BoxFuture<'_, Result<bool, String>> { Box::pin(async move { hub.db.box_exists(&row).await }) };
            for failed in delete_orphans(self.api.as_ref(), key, listed, named).await {
                eprintln!("pinned-server: {failed}");
            }
        }
        Ok(())
    }
}

/// Deletes the listed boxes no row names. Every orphan is tried: one that will not delete, or whose
/// row could not be looked up (it is then kept), is reported and the pass goes on. Returns one line
/// per failure, for the caller to log once. No line carries the key, even when the error text does.
async fn delete_orphans<'a>(
    api: &dyn BoxApi,
    key: &str,
    listed: Vec<Listed>,
    named: impl Fn(String) -> BoxFuture<'a, Result<bool, String>>,
) -> Vec<String> {
    let scrubbed = |text: String| if key.is_empty() { text } else { text.replace(key, "…") };
    let mut failures = Vec::new();
    for orphan in listed {
        let row = orphan.labels.iter().find_map(|l| l.strip_prefix("p:"));
        if let Some(row) = row {
            match named(row.to_owned()).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => {
                    failures.push(scrubbed(format!("could not look up the row of box {}, so it stays: {e}", orphan.id)));
                    continue;
                }
            }
        }
        eprintln!("pinned-server: deleting box {} that no row of this hub names", orphan.id);
        if let Err(e) = api.delete(key, &orphan.id).await {
            failures.push(scrubbed(format!("could not delete box {} that no row of this hub names: {e}", orphan.id)));
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    const ROW_ID: &str = "7fc0a1b2c3d4e5f6";

    /// A Box API that only deletes: it keeps what it was asked and refuses the ids in `refuse`.
    #[derive(Default)]
    struct Deleting {
        refuse: Vec<&'static str>,
        asked: Mutex<Vec<(String, String)>>,
    }

    impl BoxApi for Deleting {
        fn create<'a>(&'a self, _: &'a str, _: NewBox) -> BoxFuture<'a, Result<BoxId, String>> {
            unreachable!()
        }
        fn list<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<Vec<Listed>, String>> {
            unreachable!()
        }
        fn write<'a>(&'a self, _: &'a str, _: &'a BoxId, _: &'a str, _: String) -> BoxFuture<'a, Result<(), String>> {
            unreachable!()
        }
        fn exec<'a>(&'a self, _: &'a str, _: &'a BoxId, _: String) -> BoxFuture<'a, Result<Exec, String>> {
            unreachable!()
        }
        fn delete<'a>(&'a self, key: &'a str, id: &'a BoxId) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move {
                self.asked.lock().unwrap().push((key.to_owned(), id.0.clone()));
                if self.refuse.contains(&id.0.as_str()) { Err(format!("Upstash Box answered 500: {} would not go for {key}", id.0)) } else { Ok(()) }
            })
        }
    }

    fn orphan(id: &str) -> Listed {
        Listed { id: BoxId(id.into()), labels: vec!["hub:000000000000".into(), "p:ffffffffffffffff".into()] }
    }

    const KEY: &str = "box_fake-key-0000";

    /// Runs the sweep. A row in `named` is named; a row in `unreadable` fails its lookup; any other is an orphan's.
    fn sweep_with(api: &Deleting, listed: Vec<Listed>, named: &'static [&'static str], unreadable: &'static [&'static str]) -> Vec<String> {
        let lookup = move |row: String| -> BoxFuture<'static, Result<bool, String>> {
            Box::pin(async move { if unreadable.contains(&row.as_str()) { Err(format!("database says no ({KEY})")) } else { Ok(named.contains(&row.as_str())) } })
        };
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(delete_orphans(api, KEY, listed, lookup))
    }

    fn sweep(api: &Deleting, listed: Vec<Listed>, named: &'static [&'static str]) -> Vec<String> {
        sweep_with(api, listed, named, &[])
    }

    fn asked(api: &Deleting) -> Vec<String> {
        api.asked.lock().unwrap().iter().map(|(_, id)| id.clone()).collect()
    }

    #[test]
    fn the_orphan_sweep_deletes_the_next_orphan_after_one_fails() {
        let api = Deleting { refuse: vec!["box_first"], ..Deleting::default() };
        let failed = sweep(&api, vec![orphan("box_first"), orphan("box_second")], &[]);
        assert_eq!(failed, ["could not delete box box_first that no row of this hub names: Upstash Box answered 500: box_first would not go for …"]);
        assert_eq!(asked(&api), ["box_first", "box_second"], "the failed delete does not stop the pass");
    }

    #[test]
    fn the_orphan_sweep_keeps_a_box_a_row_names_and_succeeds_when_every_delete_does() {
        let api = Deleting::default();
        let mut kept = orphan("box_kept");
        kept.labels = vec!["p:eeeeeeeeeeeeeeee".into()];
        assert_eq!(sweep(&api, vec![kept, orphan("box_gone")], &["eeeeeeeeeeeeeeee"]), Vec::<String>::new());
        assert_eq!(asked(&api), ["box_gone"]);
    }

    #[test]
    fn the_orphan_sweep_error_carries_no_key() {
        let api = Deleting { refuse: vec!["box_first", "box_second"], ..Deleting::default() };
        let failed = sweep_with(&api, vec![orphan("box_first"), orphan("box_second"), unreadable_orphan("box_third")], &[], &["dddddddddddddddd"]);
        assert_eq!(failed.len(), 3, "{failed:?}");
        assert!(failed.iter().all(|f| !f.contains(KEY)), "{failed:?}");
        assert!(api.asked.lock().unwrap().iter().all(|(key, _)| key == KEY), "the key goes to the API, and only there");
    }

    fn unreadable_orphan(id: &str) -> Listed {
        Listed { id: BoxId(id.into()), labels: vec!["hub:000000000000".into(), "p:dddddddddddddddd".into()] }
    }

    #[test]
    fn an_orphan_whose_row_lookup_fails_stays_and_does_not_stop_the_pass() {
        let api = Deleting { refuse: vec!["box_first"], ..Deleting::default() };
        let listed = vec![orphan("box_first"), unreadable_orphan("box_unreadable"), orphan("box_last")];
        let failed = sweep_with(&api, listed, &[], &["dddddddddddddddd"]);
        assert_eq!(
            failed,
            [
                "could not delete box box_first that no row of this hub names: Upstash Box answered 500: box_first would not go for …",
                "could not look up the row of box box_unreadable, so it stays: database says no (…)",
            ]
        );
        assert_eq!(asked(&api), ["box_first", "box_last"], "the box whose row lookup failed is never deleted, and the pass reaches the next one");
    }

    #[test]
    fn a_box_is_named_for_its_project_role_and_row() {
        assert_eq!(
            box_name(Some("/home/u/my app"), None, BoxRole::Worker, ROW_ID),
            "my-app-worker-7fc0a1"
        );
        assert_eq!(
            box_name(None, Some("acme/client-apps"), BoxRole::Captain, ROW_ID),
            "client-apps-captain-7fc0a1"
        );
        assert_eq!(
            box_name(
                None,
                Some("git@github.com:Acme/Site.git"),
                BoxRole::Session,
                "0123456789abcdef"
            ),
            "site-session-012345"
        );
        assert_eq!(
            box_name(Some("/srv/Site"), None, BoxRole::Workflow, ROW_ID),
            "site-workflow-7fc0a1"
        );
    }

    #[test]
    fn a_box_name_folds_accents_and_replaces_other_characters() {
        assert_eq!(
            box_name(Some("/srv/Clientes Ações"), None, BoxRole::Worker, ROW_ID),
            "clientes-acoes-worker-7fc0a1"
        );
        assert_eq!(sanitize_project("a  --  b__c", 40), "a-b-c");
    }

    #[test]
    fn a_box_name_fits_the_limit_and_ends_on_a_letter() {
        let name = box_name(
            Some("/srv/client-apps-and-more-stuff-here-yes-indeed-long"),
            None,
            BoxRole::Worker,
            ROW_ID,
        );
        assert_eq!(name, "client-apps-and-more-stuff-worker-7fc0a1");
        assert_eq!(name.len(), BOX_NAME_MAX);
        assert_eq!(
            box_name(
                Some("/srv/aaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbb"),
                None,
                BoxRole::Workflow,
                ROW_ID
            ),
            "aaaaaaaaaaaaaaaaaaaaaaa-workflow-7fc0a1"
        );
    }

    #[test]
    fn a_box_name_without_a_usable_project_says_project() {
        assert_eq!(
            box_name(None, None, BoxRole::Workflow, ROW_ID),
            "project-workflow-7fc0a1"
        );
        assert_eq!(
            box_name(Some("/srv/!!! ***"), None, BoxRole::Worker, ROW_ID),
            "project-worker-7fc0a1"
        );
        assert_eq!(
            box_name(Some(""), None, BoxRole::Captain, ROW_ID),
            "project-captain-7fc0a1"
        );
    }

    #[test]
    fn a_box_role_comes_from_its_work() {
        let row = |work: BoxWork, connector: &str| BoxRow {
            id: ROW_ID.into(),
            account: Uuid::nil(),
            machine: Uuid::from_u128(9),
            upstash: None,
            connector: connector.into(),
            subject: Subject::LinearIssue {
                issue: "box".into(),
            },
            spec: spec_with(None),
            size: BoxSize::Small,
            phase: Phase::Creating,
            age: Duration::ZERO,
            work,
        };
        let captain = BoxWork::Captain {
            request_id: "r".into(),
            text: "t".into(),
            sender: "s".into(),
            busy: None,
        };
        assert_eq!(
            BoxRole::of(&row(captain, CAPTAIN_CONNECTOR)),
            BoxRole::Captain
        );
        assert_eq!(BoxRole::of(&row(BoxWork::Run, INTERACTIVE_CONNECTOR)), BoxRole::Session);
        assert_eq!(BoxRole::of(&row(BoxWork::Run, "nightly")), BoxRole::Workflow);
    }

    fn policy() -> Policy {
        Policy { idle: 20 * MIN, exited: 20 * MIN, tab_gone: 20 * MIN, offline: 20 * MIN, max_life: 20 * MIN, ..Policy::default() }
    }

    /// The verdict after `seen` held for `held`, judged at the start and at the end.
    fn after(seen: Seen, held: Duration, age: Duration) -> Option<End> {
        let (start, mut timers) = (Instant::now(), Timers::default());
        assert_eq!(verdict(Duration::ZERO, seen, &mut timers, start, &policy(), false), None);
        verdict(age, seen, &mut timers, start + held, &policy(), false)
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
                "usage": { "tokens_in": 0, "tokens_out": 0, "wall_ms": half_hour, "box_usd": 0.5 * box_usd_per_hour(BoxSize::Small), "box_ms": half_hour },
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
        let small = BoxSize::Small;
        assert!(!over_budget(&spec_with(None), small, 100 * hour), "no budget runs free");
        assert!(!over_budget(&spec_with(Some(1.0)), small, 9 * hour), "$1 buys ten hours");
        assert!(over_budget(&spec_with(Some(1.0)), small, 10 * hour), "the tenth hour spends the last cent");
        assert!(over_budget(&spec_with(Some(0.1)), small, hour + Duration::from_secs(1)), "$0.10 buys one hour");
    }

    #[test]
    fn a_budget_runs_out_sooner_on_a_bigger_box() {
        let hour = Duration::from_secs(3600);
        assert!(!over_budget(&spec_with(Some(1.0)), BoxSize::Medium, 4 * hour), "$1 buys five Medium hours at $0.20");
        assert!(over_budget(&spec_with(Some(1.0)), BoxSize::Medium, 5 * hour));
        assert!(!over_budget(&spec_with(Some(1.0)), BoxSize::Large, 2 * hour), "$1 buys two and a half Large hours at $0.40");
        assert!(over_budget(&spec_with(Some(1.0)), BoxSize::Large, 2 * hour + 30 * MIN));
        assert!(!over_budget(&spec_with(None), BoxSize::Large, 100 * hour), "no budget runs free on any size");
    }

    #[test]
    fn the_box_price_is_upstashs_per_size() {
        assert_eq!([BoxSize::Small, BoxSize::Medium, BoxSize::Large].map(box_usd_per_hour), [0.10, 0.20, 0.40]);
    }

    #[test]
    fn the_least_budget_is_an_hour_on_the_boxs_own_size() {
        assert_eq!(minimum_budget(BoxSize::Small, 0.10), Ok(()));
        assert_eq!(minimum_budget(BoxSize::Small, 0.09), Err("name at least $0.10, an hour on a small box".into()));
        assert_eq!(minimum_budget(BoxSize::Medium, 0.19), Err("name at least $0.20, an hour on a medium box".into()));
        assert_eq!(minimum_budget(BoxSize::Medium, 0.20), Ok(()));
        assert_eq!(minimum_budget(BoxSize::Large, 0.39), Err("name at least $0.40, an hour on a large box".into()));
        assert_eq!(minimum_budget(BoxSize::Large, 0.40), Ok(()));
    }

    fn row_of(size: BoxSize, phase: Phase, work: BoxWork, connector: &str, id: &str) -> BoxRow {
        BoxRow {
            id: id.into(),
            account: Uuid::nil(),
            machine: Uuid::from_u128(9),
            upstash: None,
            connector: connector.into(),
            subject: Subject::LinearIssue { issue: "box".into() },
            spec: RunSpec { budget_usd: Some(2.0), ..spec_with(None) },
            size,
            phase,
            age: 90 * MIN,
            work,
        }
    }

    #[test]
    fn the_chip_carries_the_rate_of_the_boxs_size() {
        for (size, rate) in [(BoxSize::Small, 0.10), (BoxSize::Medium, 0.20), (BoxSize::Large, 0.40)] {
            let status = status_of(row_of(size, Phase::Running, BoxWork::Run, INTERACTIVE_CONNECTOR, ROW_ID), Some("tab-1".into()));
            assert_eq!((status.rate_usd_per_hour, status.phase.as_str(), status.budget_usd, status.age_secs), (rate, "running", Some(2.0), 5400.0), "{size:?}");
        }
        let creating = status_of(row_of(BoxSize::Large, Phase::Creating, BoxWork::Run, INTERACTIVE_CONNECTOR, ROW_ID), None);
        assert_eq!((creating.rate_usd_per_hour, creating.phase.as_str()), (0.40, "creating"));
    }

    #[test]
    fn an_agents_box_is_billed_at_the_rate_of_its_size() {
        let opener = Opener { machine: Uuid::from_u128(7), tab: "tab-1".into(), role: ActorRole::Captain };
        let hour = 3_600_000;
        for (size, usd) in [(BoxSize::Small, 0.10), (BoxSize::Medium, 0.20), (BoxSize::Large, 0.40)] {
            let row = row_of(size, Phase::Ending(End::Done), BoxWork::Run, INTERACTIVE_CONNECTOR, ROW_ID);
            let record = agent_box_record(&row, &opener, 0, hour, &End::Done);
            assert!((record.usage.box_usd.unwrap() - usd).abs() < 1e-9, "{size:?}: {:?}", record.usage.box_usd);
        }
        let medium = row_of(BoxSize::Medium, Phase::Ending(End::Expired), BoxWork::Run, INTERACTIVE_CONNECTOR, ROW_ID);
        let limit = |hours: u64| agent_box_record(&medium, &opener, 0, hours * hour, &End::Expired).limit;
        assert_eq!((limit(5).as_deref(), limit(10).as_deref()), (Some("time"), Some("budget")), "$2 is ten Medium hours at $0.20");
    }

    #[test]
    fn a_box_end_for_budget_or_max_life_keeps_its_subject() {
        let p = Policy::default();
        let mut t = Timers::default();
        let ended = verdict(p.max_life, Seen::Tab { running: true, quiet: false }, &mut t, Instant::now(), &p, false);
        assert_eq!(ended, Some(End::Expired), "max life ends a box as Expired, as its budget does (`over_budget`)");
        assert_eq!(release_for(&End::Expired), BoxEndRelease::Keep);
    }

    #[test]
    fn a_removed_box_machine_keeps_its_subject_handled() {
        assert_eq!(release_for(&End::Removed), BoxEndRelease::Keep, "the owner removed the machine: the run fails, and only the owner runs it again");
    }

    #[test]
    fn a_disconnected_upstash_keeps_its_subject_handled() {
        assert_eq!(release_for(&End::Disconnected), BoxEndRelease::Keep, "the owner disconnected Upstash: the run fails, and only the owner runs it again");
    }

    #[test]
    fn a_vanished_box_frees_its_subject_once_like_an_offline_one() {
        // Once: the first end frees the subject and a second keeps it (`Connectors::box_ended`).
        assert_eq!(release_for(&End::Vanished), BoxEndRelease::Once);
        for end in [End::Offline, End::Failed("the box did not connect".into())] {
            assert_eq!(release_for(&end), BoxEndRelease::Once, "{end:?}");
        }
    }

    #[test]
    fn a_held_box_with_a_stale_quiet_timer_ends_offline_not_done() {
        let p = Policy::default();
        let t0 = Instant::now();
        // Quiet and done before the run held the box.
        let mut t = Timers { quiet_since: Some(t0), done: true, ..Timers::default() };
        let later = t0 + p.done.max(p.idle) + Duration::from_secs(1);
        assert_eq!(held_verdict(Duration::from_secs(60), Seen::Offline, &mut t, later, &p), None, "its node just went offline");
        assert_eq!(held_verdict(Duration::from_secs(60), Seen::Offline, &mut t, later + p.offline, &p), Some(End::Offline), "it ends offline, and its run fails");
    }

    #[test]
    fn a_held_box_that_is_offline_at_its_max_life_ends_expired_not_offline() {
        let p = Policy::default();
        let t0 = Instant::now();
        // Offline for as long as the wait, and at its max life: the max life wins, so the run's subject stays handled.
        let mut t = Timers::default();
        assert_eq!(held_verdict(p.max_life - Duration::from_secs(1), Seen::Offline, &mut t, t0, &p), None, "one second short of its max life, offline just now");
        assert_eq!(held_verdict(p.max_life, Seen::Offline, &mut t, t0 + p.offline, &p), Some(End::Expired), "offline past the wait and at its max life");
        assert_eq!(release_for(&End::Expired), BoxEndRelease::Keep, "which keeps its subject");

        // One second short of its max life it ends Offline, and only after the offline wait.
        let mut t = Timers::default();
        let short = p.max_life - Duration::from_secs(1);
        assert_eq!(held_verdict(short, Seen::Offline, &mut t, t0, &p), None, "offline just now");
        assert_eq!(held_verdict(short, Seen::Offline, &mut t, t0 + p.offline - Duration::from_secs(1), &p), None, "not yet past the wait");
        assert_eq!(held_verdict(short, Seen::Offline, &mut t, t0 + p.offline, &p), Some(End::Offline), "past the wait");
        assert_eq!(release_for(&End::Offline), BoxEndRelease::Once);
    }

    #[test]
    fn a_box_that_ends_as_done_while_its_run_goes_fails_the_run() {
        let error = run_end_error(&Policy::default(), &End::Done);
        assert_eq!(error, "Upstash Box: the box ended before its run did: its work was done");
    }

    #[test]
    fn only_the_fixed_row_gets_a_keep_alive_box() {
        let access = fixed_access(ROW_ID);
        let fixed = row_of(BoxSize::Medium, Phase::Creating, BoxWork::Run, "nightly", ROW_ID);
        assert!(new_box(&access, &fixed, "hub:000000000000").keep_alive, "the fixed row outlives the runs it serves");
        let worker = BoxWork::Captain { request_id: "r".into(), text: "t".into(), sender: "s".into(), busy: None };
        for (work, connector) in [(BoxWork::Run, "nightly"), (BoxWork::Run, INTERACTIVE_CONNECTOR), (worker, CAPTAIN_CONNECTOR)] {
            let temporary = row_of(BoxSize::Medium, Phase::Creating, work, connector, "0123456789abcdef");
            let new = new_box(&access, &temporary, "hub:000000000000");
            assert!(!new.keep_alive, "{connector}: a temporary box is a standard one");
        }
        let fresh = new_box(&BoxAccess::default(), &fixed, "hub:000000000000");
        assert!(!fresh.keep_alive, "with no fixed row named, even this id is temporary");
        assert_eq!((fresh.size, fresh.labels.len()), (BoxSize::Medium, 2), "the rest of the box is the same");
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
        assert_eq!(verdict(19 * MIN, working, &mut timers, Instant::now(), &policy(), false), None);
        assert_eq!(verdict(21 * MIN, working, &mut timers, Instant::now(), &policy(), false), Some(End::Expired));
    }

    #[test]
    fn a_box_others_delegate_from_ignores_idle_and_exited_and_still_meets_its_max_life() {
        let (start, mut t) = (Instant::now(), Timers::default());
        for seen in [Seen::Tab { running: true, quiet: true }, Seen::Tab { running: false, quiet: true }] {
            for minute in [0, 21, 60, 19 * 60] {
                assert_eq!(verdict(MIN, seen, &mut t, start + minute * MIN, &policy(), true), None, "{seen:?} at {minute} min while its worker hangs");
            }
        }
        t.done = true;
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: true }, &mut t, start + 30 * 60 * MIN, &Policy { done: MIN, ..policy() }, true), None, "nor Done");
        assert_eq!(verdict(21 * MIN, Seen::Tab { running: true, quiet: true }, &mut t, start, &policy(), true), Some(End::Expired), "max life ends it");
        // A node gone offline, or its tab closed, still ends it, by the default policy's 10 and 2 minutes.
        let p = Policy::default();
        let (mut offline, mut closed) = (Timers::default(), Timers::default());
        assert_eq!(verdict(MIN, Seen::Offline, &mut offline, start, &p, true), None);
        assert_eq!(verdict(MIN, Seen::Offline, &mut offline, start + 10 * MIN - Duration::from_secs(1), &p, true), None);
        assert_eq!(verdict(MIN, Seen::Offline, &mut offline, start + 10 * MIN, &p, true), Some(End::Offline), "a node gone offline still ends it");
        assert_eq!(verdict(MIN, Seen::NoTab, &mut closed, start, &p, true), None);
        assert_eq!(verdict(MIN, Seen::NoTab, &mut closed, start + 2 * MIN - Duration::from_secs(1), &p, true), None);
        assert_eq!(verdict(MIN, Seen::NoTab, &mut closed, start + 2 * MIN, &p, true), Some(End::TabClosed), "a closed tab still ends it after 2 minutes");
        // Its workers are done: the quiet it had then counts from now.
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: true }, &mut t, start + 40 * 60 * MIN, &policy(), false), None);
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: true }, &mut t, start + 40 * 60 * MIN + 21 * MIN, &policy(), false), Some(End::Done));
    }

    #[test]
    fn working_resets_quiet() {
        let p = policy();
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p, false), None);
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: false }, &mut t, start + 15 * MIN, &p, false), None, "working again");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 16 * MIN, &p, false), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 35 * MIN, &p, false), None, "quiet for 19 minutes since it worked");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 37 * MIN, &p, false), Some(End::Idle));
    }

    #[test]
    fn a_box_whose_pull_request_is_open_ends_soon_after_its_session_stops() {
        let p = Policy { done: 2 * MIN, ..policy() };
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p, false), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 3 * MIN, &p, false), None, "quiet without a pull request waits out idle");
        t.done = true;
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: false }, &mut t, start + 4 * MIN, &p, false), None, "still working");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 5 * MIN, &p, false), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 6 * MIN, &p, false), None, "quiet for 1 of 2 minutes");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 7 * MIN, &p, false), Some(End::Done));
    }

    #[test]
    fn a_captain_box_outlives_a_short_disconnect_and_ends_offline_after_ten_minutes() {
        // The shipped policy: a node offline for 10 minutes ends its box, and a reconnect resets that.
        let p = Policy::default();
        assert_eq!(p.offline, 10 * MIN);
        let sec = Duration::from_secs(1);
        for delegating in [true, false] {
            // A Captain quiet while its workers run, or a session at work.
            let waiting = Seen::Tab { running: true, quiet: delegating };
            let (start, mut t) = (Instant::now(), Timers::default());
            assert_eq!(verdict(MIN, waiting, &mut t, start, &p, delegating), None);
            // A hub deploy: the node drops for 30 seconds and reconnects.
            assert_eq!(verdict(MIN, Seen::Offline, &mut t, start + sec, &p, delegating), None);
            assert_eq!(verdict(MIN, Seen::Offline, &mut t, start + 31 * sec, &p, delegating), None);
            assert_eq!(verdict(MIN, waiting, &mut t, start + 40 * sec, &p, delegating), None, "back after 30 s");
            // Offline for 9 min 59 s, then back: still not ended, and the clock starts over.
            let away = start + MIN;
            assert_eq!(verdict(MIN, Seen::Offline, &mut t, away, &p, delegating), None);
            assert_eq!(verdict(MIN, Seen::Offline, &mut t, away + 10 * MIN - sec, &p, delegating), None, "9 min 59 s offline");
            assert_eq!(verdict(MIN, waiting, &mut t, away + 10 * MIN, &p, delegating), None, "back before 10 min");
            // The next drop counts from zero: 10 minutes of it end the box.
            let gone = away + 11 * MIN;
            assert_eq!(verdict(MIN, Seen::Offline, &mut t, gone, &p, delegating), None);
            assert_eq!(verdict(MIN, Seen::Offline, &mut t, gone + 10 * MIN - sec, &p, delegating), None, "a reconnect reset the clock");
            assert_eq!(verdict(MIN, Seen::Offline, &mut t, gone + 10 * MIN, &p, delegating), Some(End::Offline), "delegating: {delegating}");
        }
    }

    #[test]
    fn a_dropped_link_does_not_restart_the_quiet_clock() {
        let p = policy();
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p, false), None);
        assert_eq!(verdict(MIN, Seen::Offline, &mut t, start + 10 * MIN, &p, false), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 19 * MIN, &p, false), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 21 * MIN, &p, false), Some(End::Idle));
    }

    #[test]
    fn a_worker_box_that_never_connected_is_booting_one_that_connected_is_offline_and_an_ending_one_is_gone() {
        let answer = |phase: Phase, removed: bool| offline_answer(&phase, removed, "m-box");
        assert_eq!((answer(Phase::Creating, false), answer(Phase::Booting, false)), (BOX_BOOTING.to_owned(), BOX_BOOTING.to_owned()));
        assert_eq!(answer(Phase::Running, false), BOX_OFFLINE);
        for end in [End::Offline, End::Done, End::Expired, End::Failed("fake".into())] {
            // Until its machine goes, the origin's own end report names the reason.
            assert_eq!(answer(Phase::Ending(end.clone()), false), BOX_OFFLINE, "{end:?}");
            // A 0.18.1 parent keeps waiting on BOX_OFFLINE, with its limit paused while the worker
            // waited on the user, so a row whose box can't be deleted must read as gone.
            let gone = answer(Phase::Ending(end.clone()), true);
            assert_eq!(gone, "this account has no machine m-box", "{end:?}");
            assert!(pinned_core::delegate::machine_gone(&gone), "{end:?}");
        }
    }

    #[test]
    fn a_box_whose_link_blinks_keeps_running_and_one_that_stays_offline_ends() {
        let p = Policy::default();
        let (start, mut t) = (Instant::now(), Timers::default());
        let working = Seen::Tab { running: true, quiet: false };
        assert_eq!(verdict(30 * MIN, working, &mut t, start, &p, false), None);
        assert_eq!(verdict(31 * MIN, Seen::Offline, &mut t, start + MIN, &p, false), None);
        assert_eq!(verdict(40 * MIN, Seen::Offline, &mut t, start + 10 * MIN, &p, false), None, "offline for 9 of 10 minutes");
        assert_eq!(verdict(41 * MIN, working, &mut t, start + 11 * MIN, &p, false), None, "back within the grace");
        assert_eq!(verdict(42 * MIN, Seen::Offline, &mut t, start + 12 * MIN, &p, false), None);
        assert_eq!(verdict(51 * MIN, Seen::Offline, &mut t, start + 21 * MIN, &p, false), None, "a blink restarts the grace");
        assert_eq!(verdict(53 * MIN, Seen::Offline, &mut t, start + 23 * MIN, &p, false), Some(End::Offline));
    }

    #[test]
    fn a_flapping_box_still_ends_at_its_max_life_and_its_budget() {
        let p = Policy::default();
        // A Captain box waiting on its workers (quiet, `delegating`) too: its hold does not lift max life.
        for delegating in [false, true] {
            let (start, mut t) = (Instant::now(), Timers::default());
            let working = Seen::Tab { running: true, quiet: delegating };
            let mut minute = 0;
            // Offline 9 minutes, back 1, over and over: never offline for the whole grace.
            while minute < 4 * 60 {
                let seen = if minute % 10 == 9 { working } else { Seen::Offline };
                assert_eq!(verdict(minute * MIN, seen, &mut t, start + minute * MIN, &p, delegating), None, "minute {minute}");
                minute += 1;
            }
            assert_eq!(verdict(4 * 60 * MIN, Seen::Offline, &mut t, start + 4 * 60 * MIN, &p, delegating), Some(End::Expired), "delegating: {delegating}");
        }
        assert!(over_budget(&spec_with(Some(box_usd_per_hour(BoxSize::Small))), BoxSize::Small, 60 * MIN), "a budget ends it sooner whatever it is seen as");
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
            box_env(&keys, Some("ghp_SECRETGITHUB"), true),
            "CLAUDE_CODE_OAUTH_TOKEN='sk-ant-oat01-SECRETCLAUDE'\nGH_TOKEN='ghp_SECRETGITHUB'\nPINNED_BOX=1\n"
        );
        assert_eq!(start_script("o'brien", None).lines().find(|l| l.contains("user.name")), Some("git config --global user.name 'o'\\''brien'"));
    }

    #[test]
    fn a_box_env_tells_its_node_it_runs_in_a_box() {
        let keys = Keys { api_key: "box_SECRETKEY".into(), claude_token: "sk-ant-oat01-SECRETCLAUDE".into() };
        for github in [None, Some("ghp_SECRETGITHUB")] {
            let env = box_env(&keys, github, true);
            assert!(env.lines().any(|line| line == "PINNED_BOX=1"), "{env}");
            assert!(!env.contains("box_SECRETKEY"), "{env}");
        }
    }

    fn fixed_access(row: &str) -> BoxAccess {
        BoxAccess { workflow: BoxPermission::Fixed { row: Some(row.into()) }, ..BoxAccess::default() }
    }

    #[test]
    fn a_fixed_box_env_does_not_mark_its_node_to_leave_and_a_fresh_box_env_does() {
        let keys = Keys { api_key: "fake-api-key".into(), claude_token: "fake-claude-token".into() };
        let access = fixed_access("fixed-row");
        let fixed = box_env(&keys, None, leaves_when_refused(&access, "fixed-row"));
        assert!(!fixed.contains("PINNED_BOX"), "{fixed}");
        assert!(fixed.lines().any(|line| line.starts_with("CLAUDE_CODE_OAUTH_TOKEN=")), "{fixed}");
        for (access, row) in [(&access, "fresh-row"), (&BoxAccess::default(), "fresh-row")] {
            let fresh = box_env(&keys, None, leaves_when_refused(access, row));
            assert!(fresh.lines().any(|line| line == "PINNED_BOX=1"), "{fresh}");
        }
    }

    /// Runs the box's start script with a fake HOME and its work directory in a temp dir, with fake
    /// `node`, `git` and `gh`, and a fake `pinned-server` that records the `PINNED_BOX` it started with.
    #[cfg(unix)]
    struct StartRun {
        dir: std::path::PathBuf,
        home: std::path::PathBuf,
        bin: std::path::PathBuf,
        _root: tempfile::TempDir,
    }

    #[cfg(unix)]
    impl StartRun {
        fn new() -> StartRun {
            let keys = Keys { api_key: "fake-api-key".into(), claude_token: "fake-claude-token".into() };
            StartRun::with_env(box_env(&keys, None, true))
        }

        /// As `new`, with `env` as the box's `box.env`.
        fn with_env(env: String) -> StartRun {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::tempdir().unwrap();
            let (dir, home, bin) = (root.path().join("pinned"), root.path().join("home"), root.path().join("bin"));
            for path in [&dir, &home, &bin] {
                std::fs::create_dir_all(path).unwrap();
            }
            let executable = |path: std::path::PathBuf, body: &str| {
                std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            };
            for fake in ["node", "git", "gh"] {
                executable(bin.join(fake), "exit 0");
            }
            executable(dir.join("pinned-server"), &format!("printf '%s' \"$PINNED_BOX\" >{}/started", dir.display()));
            std::fs::write(dir.join("box.env"), env).unwrap();
            StartRun { dir, home, bin, _root: root }
        }

        /// Runs the script once and returns what the node it launched saw as `PINNED_BOX`, if it launched one.
        fn start(&self) -> Option<String> {
            let script = start_script("fake-login", None);
            let script = script.replace("DIR=/workspace/home/.pinned", &format!("DIR={}", self.dir.display()));
            assert!(script.contains(&format!("DIR={}\n", self.dir.display())), "the script no longer sets DIR on its own line");
            let _ = std::fs::remove_file(self.dir.join("started"));
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(&script)
                .env_clear()
                .env("HOME", &self.home)
                .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
                .status()
                .unwrap();
            assert!(status.success());
            for _ in 0..100 {
                if let Ok(seen) = std::fs::read_to_string(self.dir.join("started")) {
                    return Some(seen);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(!self.dir.join("node.pid").exists(), "the script launched a node that never ran");
            None
        }
    }

    #[cfg(target_os = "linux")]
    fn pair_fake(dir: &std::path::Path) {
        let creds = pinned_core::account::Credentials {
            hub: "https://hub.example".into(),
            email: "box@example.invalid".into(),
            user_token: String::new(),
            machine_id: "m1".into(),
            machine_token: "fake-machine-token".into(),
        };
        creds.save(dir).unwrap();
    }

    // The script launches the node with `setsid`, which macOS lacks; a box only ever runs on Linux.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_start_script_launches_a_paired_node_with_pinned_box_set() {
        let run = StartRun::new();
        pair_fake(&run.dir);
        assert_eq!(run.start().as_deref(), Some("1"), "the node did not start with PINNED_BOX=1");
        assert!(run.dir.join("node.pid").exists());
    }

    /// Starts row `row`'s node with the env its boot writes under `access`, then has the hub refuse
    /// it for an hour: the stand-in for `link.refused_for()` resolves 30 s past the span, as it does
    /// on the first 4001 after 15 minutes of them. Does what `node()` does when the wait ends, and
    /// returns whether the node left, how many times it ended its sessions, and whether it is paired.
    #[cfg(target_os = "linux")]
    async fn refused_for_an_hour(access: &BoxAccess, row: &str) -> (bool, usize, bool) {
        let keys = Keys { api_key: "fake-api-key".into(), claude_token: "fake-claude-token".into() };
        let run = StartRun::with_env(box_env(&keys, None, leaves_when_refused(access, row)));
        pair_fake(&run.dir);
        let seen = run.start().expect("the start script did not launch the node");
        let refused = tokio::time::sleep(pinned_core::link::BOX_REFUSED_FOR + Duration::from_secs(30));
        let in_box = crate::box_exit::is_box(Some(seen.as_str()));
        let left = tokio::time::timeout(Duration::from_secs(3600), crate::box_exit::refused_in_box(in_box, refused)).await.is_ok();
        let quit = std::cell::Cell::new(0);
        if left {
            crate::box_exit::leave(&run.dir, "fake-machine-token", || quit.set(quit.get() + 1), |_| {});
        }
        (left, quit.get(), pinned_core::account::Credentials::load(&run.dir).is_some())
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(start_paused = true)]
    async fn a_fixed_box_node_refused_well_past_the_span_stays_up_and_keeps_its_account() {
        let access = fixed_access("fixed-row");
        assert_eq!(refused_for_an_hour(&access, "fixed-row").await, (false, 0, true), "the fixed box's node left");
        // The same refusals end a fresh box's node, so the check above can fail.
        assert_eq!(refused_for_an_hour(&access, "fresh-row").await, (true, 1, false), "a fresh box's node stayed");
    }

    #[cfg(unix)]
    #[test]
    fn the_start_script_does_not_launch_a_node_that_has_no_account() {
        let run = StartRun::new();
        assert_eq!(run.start(), None, "a node started with no account.json");
        assert!(!run.dir.join("node.pid").exists());
        assert!(!run.home.join(".profile").exists(), "the script went past its account check");
    }

    // The script launches the node with `setsid`, which macOS lacks; a box only ever runs on Linux.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_container_restart_after_a_node_left_for_good_starts_no_node() {
        let run = StartRun::new();
        pair_fake(&run.dir);
        assert_eq!(run.start().as_deref(), Some("1"));
        // The node leaves for good and, in a restart, the pid file's number belongs to nothing.
        crate::box_exit::leave(&run.dir, "fake-machine-token", || {}, |_| {});
        let _ = std::fs::remove_file(run.dir.join("node.pid"));
        assert_eq!(run.start(), None, "the start script relaunched a node that left for good");
        assert!(!run.dir.join("node.pid").exists());
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

    #[test]
    fn each_kind_of_box_takes_its_size_from_the_way_of_working() {
        use pinned_core::taste::{BoxSizeSource, TastePatch};
        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { box_size: Some(BoxSize::Medium), ..TastePatch::default() }).unwrap();
        tastes.set(&TasteScope::Project { repo: "github.com/acme/site".into() }, TastePatch { box_size: Some(BoxSize::Large), ..TastePatch::default() }).unwrap();
        let sized = |repo: &str, asked: Option<BoxSize>| {
            let choice = choose_box_size(&tastes, Some(repo), asked);
            (choice.size(), choice.from())
        };
        // A delegate box: `open_delegate` turns target.box.repo into the clone URL its RunSpec keeps.
        let delegate = origin_repo("acme/site").unwrap();
        assert_eq!(delegate, "https://github.com/acme/site.git");
        assert_eq!(sized(&delegate, None), (BoxSize::Large, BoxSizeSource::Project));
        assert_eq!(sized(&delegate, Some(BoxSize::Small)), (BoxSize::Small, BoxSizeSource::Call));
        // A Captain's box: `open_captain` reads the project's origin, an SSH one too.
        let captain = origin_repo("git@github.com:acme/site.git").unwrap();
        assert_eq!(sized(&captain, None), (BoxSize::Large, BoxSizeSource::Project));
        assert_eq!(sized(&origin_repo("git@github.com:acme/api.git").unwrap(), None), (BoxSize::Medium, BoxSizeSource::Account));
        // A workflow's box: `open_new` passes the workflow's repo as it is written, owner/repo included.
        assert_eq!(sized("acme/site", None), (BoxSize::Large, BoxSizeSource::Project));
        assert_eq!(sized("acme/api", None), (BoxSize::Medium, BoxSizeSource::Account));
        assert_eq!(sized("acme/site", Some(BoxSize::Medium)), (BoxSize::Medium, BoxSizeSource::Call));
        let none = choose_box_size(&Tastes::default(), Some("acme/site"), None);
        assert_eq!((none.size(), none.from()), (BoxSize::Small, BoxSizeSource::BuiltIn));
    }
}
