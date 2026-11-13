//! A child agent on another provider. One driver row builds the CLI and reads its stream. The
//! registry remembers the task. Parsing a line does not touch a process.

use crate::accounts::{AccountId, Provider};
use crate::subagents::{self, Subagent};
use crate::thread::{self, Item};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const STEP_KEEP: usize = 8;
const STEP_CHARS: usize = 400;
pub const DEFAULT_TIMEOUT_MS: u64 = 10 * 60 * 1000;
pub const DEFAULT_BUDGET_MS: u64 = 2 * 60 * 60 * 1000;
pub const DEFAULT_LIMIT_MS: u64 = 8 * 60 * 60 * 1000;
pub const QUIET_AFTER_REPLY_MS: u64 = 30 * 60 * 1000;
/// How long a worker box may take to boot and connect to the hub. Past it, its task fails.
pub const BOX_CONNECT_WAIT: Duration = Duration::from_secs(5 * 60);
/// How long a task on another machine may run past its time limit before this machine ends it
/// itself. The machine that runs it holds it to the limit and reports its end, unless it is gone.
pub const REMOTE_GRACE: Duration = Duration::from_secs(5 * 60);
/// The hub's answer about a worker box whose node has not connected yet.
pub const BOX_BOOTING: &str = "the box is still booting";
/// The hub's answer about a worker box whose node connected and then dropped its link. Its task
/// waits: the node may come back, and the hub ends a box that stays offline.
pub const BOX_OFFLINE: &str = "the box is offline";

/// Whether the hub answered that the account has no such machine any more: a box that ended, or
/// a machine someone removed.
pub fn machine_gone(error: &str) -> bool {
    error.starts_with("this account has no machine ")
}

/// "5 min" for a whole number of minutes, else seconds.
pub fn wait_span(wait: Duration) -> String {
    match wait.as_secs() {
        secs if secs >= 60 && secs % 60 == 0 => format!("{} min", secs / 60),
        secs => format!("{secs} s"),
    }
}
#[cfg(unix)]
const EXIT_GRACE: Duration = Duration::from_secs(5);
const STDERR_CLOSE_WAIT: Duration = Duration::from_secs(1);
const MODEL_CACHE: Duration = Duration::from_secs(10 * 60);
const MODEL_MISS: Duration = Duration::from_secs(15);
const CATALOG_WAIT: Duration = Duration::from_secs(2);

/// A delegated child runs headless, so no thread shows its workers and no idle session takes
/// their notices: it never starts pinned workers. Worker threads do, as deep as the cap allows.
pub fn delegate_block(child: bool) -> Option<&'static str> {
    child.then_some("a delegated child cannot start another pinned worker")
}

/// A missing model keeps the parent's model only when the child runs on the same provider.
/// A different provider gets that provider's own default, which is no `--model`.
pub fn child_model(parent: Provider, child: Provider, inherited: Option<String>, requested: Option<String>) -> Option<String> {
    let requested = requested.filter(|model| !model.is_empty());
    if requested.is_some() {
        return requested;
    }
    if parent != child {
        return None;
    }
    inherited.filter(|model| !model.is_empty())
}

/// What the child is allowed to do. Never wider than the parent tab: plan narrows a bypassed
/// parent to read-only, and a parent that asks before acting does not hand the child a bypass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Full,
    Auto,
    ReadOnly,
}

impl Access {
    pub fn derive(bypass: bool, plan: bool) -> Access {
        if plan {
            Access::ReadOnly
        } else if bypass {
            Access::Full
        } else {
            Access::Auto
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Access::Full => "full",
            Access::Auto => "auto",
            Access::ReadOnly => "read-only",
        }
    }

    /// The `runtimeMode` string. Plan is read-only, which is narrower than `auto` and is not a mode of its own.
    pub fn runtime_mode(self) -> &'static str {
        match self {
            Access::Full => "full-access",
            Access::Auto | Access::ReadOnly => "auto",
        }
    }
}

/// The grant line a delegation brief carries when the orchestrator names tools. Empty grants
/// name nothing: the child uses what the task needs, and says NEEDS followed by tools when it
/// needs more.
pub fn granted_brief(task: &str, tools: &[String]) -> String {
    if tools.is_empty() {
        return task.to_owned();
    }
    format!("Granted tools: {}.\n\n{task}", tools.join(", "))
}

/// `asked` cannot be wider than `ceiling`. A plan request is already read-only before this.
/// An explicit `auto`, and an omitted mode, narrow to read-only under that ceiling. `full-access` does not.
pub fn resolve_access(ceiling: Access, request: &Request) -> Result<Access, String> {
    if request.interaction == Interaction::Plan {
        return Ok(Access::ReadOnly);
    }
    let asked = match request.runtime {
        None => ceiling,
        Some(RuntimeMode::Auto) if ceiling == Access::ReadOnly => Access::ReadOnly,
        Some(RuntimeMode::Auto) => Access::Auto,
        Some(RuntimeMode::FullAccess) => Access::Full,
    };
    if tighten(ceiling, asked) != asked {
        return Err(format!(
            "runtimeMode \"{}\" is wider than this session's \"{}\"",
            match asked {
                Access::Full => "full-access",
                Access::Auto => "auto",
                Access::ReadOnly => "read-only",
            },
            match ceiling {
                Access::Full => "full-access",
                Access::Auto => "auto",
                Access::ReadOnly => "read-only",
            }
        ));
    }
    Ok(asked)
}

/// `asked` cannot be wider than `ceiling`. A plan request is already read-only before this.
pub fn tighten(ceiling: Access, asked: Access) -> Access {
    match (ceiling, asked) {
        (Access::ReadOnly, _) => Access::ReadOnly,
        (Access::Auto, Access::Full) => Access::Auto,
        (_, asked) => asked,
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Isolation {
    Shared,
    Worktree,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Async,
    Wait,
}

/// How wide a child may act. Omitted means the parent's own mode.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeMode {
    FullAccess,
    Auto,
}

/// `plan` is read-only. `inherit` and `default` leave the parent's mode alone.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Interaction {
    #[default]
    Inherit,
    Default,
    Plan,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Target {
    #[serde(default)]
    pub account: Option<AccountId>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// Set when the request named `driverKind`, so a custom account can be checked against it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_kind: Option<String>,
    /// Another machine of the account, by id or name, that runs the child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// A new Upstash Box, opened with the account's connection on the hub, that runs the child.
    #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
    pub boxed: Option<BoxTarget>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct BoxTarget {
    /// None follows the project's and the account's default box size when the box opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<crate::connectors::BoxSize>,
    /// The repository the box clones. None clones the caller's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// The tab on another machine that delegated a task here. The hub stamps `machine` from the
/// socket the request came on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Origin {
    pub machine: String,
    #[serde(default)]
    pub machine_name: String,
    pub tab: String,
    /// The caller's own id for the task, which the end notice names.
    pub task: String,
    /// The widest access the caller may hand a child.
    pub access: Access,
    /// Who asked, for the record this machine keeps of the task it runs.
    #[serde(default)]
    pub delegator: Delegator,
    /// The delegation limits the caller runs under, which the worker here is held to.
    #[serde(default)]
    pub limits: crate::taste::Limits,
}

impl Origin {
    /// The parent a task started for this origin has in the registry. No local tab has this id.
    pub fn parent_key(&self) -> String {
        Origin::key(&self.machine, &self.tab)
    }

    /// The `parent_key` of tab `tab` on machine `machine`.
    pub fn key(machine: &str, tab: &str) -> String {
        format!("remote:{machine}:{tab}")
    }

    /// The machine and tab a `parent_key` names, for a parent that is a tab on another machine.
    pub fn of_key(key: &str) -> Option<(&str, &str)> {
        key.strip_prefix("remote:")?.split_once(':')
    }
}

/// An end the hub keeps for 30 days because the machine whose tab delegated the task answered that
/// it did not take it: it forgot the task, as a restart does, or had it already. `HubCall::UnclaimedEnds`
/// lists them, and the task's status still answers from one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct UnclaimedEnd {
    /// The task id the delegating machine gave it.
    pub task: String,
    /// The machine whose tab delegated it, and that tab.
    pub origin_machine: String,
    pub origin_tab: String,
    /// The machine that ran it.
    pub from_machine: String,
    pub end: RemoteEnd,
    /// When the hub took the end, and when the delegating machine answered it did not take it,
    /// unix milliseconds.
    pub held_at: u64,
    pub unclaimed_at: u64,
}

/// How a task that runs on another machine ended, as that machine reports it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct RemoteEnd {
    /// `result_available`, `failed` or `cancelled`.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What the worker used, its box's time included: the delegating task shows it. An older
    /// machine sends none. The machine that ran the task records the run, so this is for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::usage::TaskUsage>,
}

/// What a task that runs on another machine waits on the user for: its thread there is blocked on
/// an approval or a question. The machine that runs it reports it as it starts and again, as `None`,
/// once it is answered, and the thread that delegated the task keeps it until then.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct RemoteAsk {
    pub kind: crate::global_orchestrator::AskKind,
    /// The tool an approval is for, such as `Bash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// What the thread asks, as its session said it.
    pub ask: String,
    /// Unix seconds the thread started to wait.
    pub since: u64,
}

/// The delegation request, after the MCP arguments have been read once.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Request {
    pub task: String,
    pub title: String,
    pub target: Target,
    pub isolation: Isolation,
    pub mode: Mode,
    pub timeout_ms: u64,
    /// As asked; `run_limits` settles both under the caller's time limit.
    #[serde(default)]
    pub budget_ms: Option<u64>,
    #[serde(default)]
    pub limit_ms: Option<u64>,
    #[serde(default)]
    pub interaction: Interaction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Tools the child may use. The child sees them as its grant; empty grants what the task
    /// needs. A child that needs more says NEEDS followed by the tools, and the parent answers
    /// with a wider grant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<String>,
    /// Whether the account's placement policy places this worker, over its rule for heavy work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heavy: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkState {
    Working,
    /// The process exited and a descendant is still working.
    WaitingForChildren,
    ResultAvailable,
    Failed,
    Cancelled,
}

impl WorkState {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkState::Working => "working",
            WorkState::WaitingForChildren => "waiting_for_children",
            WorkState::ResultAvailable => "result_available",
            WorkState::Failed => "failed",
            WorkState::Cancelled => "cancelled",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, WorkState::ResultAvailable | WorkState::Failed | WorkState::Cancelled)
    }
}

fn default_budget() -> u64 {
    DEFAULT_BUDGET_MS
}

fn default_limit() -> u64 {
    DEFAULT_LIMIT_MS
}

/// Who is calling. A tab session sends `PINNED_TAB`. A child sends `PINNED_TASK`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    pub tab: Option<String>,
    pub task: Option<String>,
}

impl Caller {
    pub fn key(&self) -> String {
        if let Some(tab) = self.tab.as_deref().filter(|tab| !tab.is_empty()) {
            return format!("tab:{tab}");
        }
        if let Some(task) = self.task.as_deref().filter(|task| !task.is_empty()) {
            return format!("task:{task}");
        }
        "none".into()
    }
}

/// The process ended, and the task is waiting on descendants before this becomes its state.
#[derive(Clone, Debug)]
pub struct HeldExit {
    pub summary: Option<String>,
    pub error: Option<String>,
    pub failed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parent {
    Tab(String),
    Task(String),
}

#[derive(Clone, Debug)]
pub struct Task {
    pub id: String,
    pub parent: Parent,
    pub tool: Option<String>,
    pub title: String,
    pub account: AccountId,
    pub provider: Provider,
    pub model: Option<String>,
    pub cwd: PathBuf,
    /// The widest access this task may hand to a child of its own.
    pub access: Access,
    pub started: u64,
    pub ended: Option<u64>,
    pub state: WorkState,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub conversation: Option<String>,
    pub steps: Vec<Item>,
    pub newest_text: Option<String>,
    pub mode: Mode,
    pub delivered: bool,
    /// Unix seconds `delivered` was set, when it was set since this was recorded.
    pub delivered_at: Option<u64>,
    /// This task's folder is a worktree `new_worktree` made.
    pub isolated: bool,
    /// `tab:<id>` or `task:<id>` of the session that started it. Idempotency and ownership use it.
    pub caller_key: String,
    pub client_request_id: Option<String>,
    pub role: Option<String>,
    pub held: Option<HeldExit>,
    pub parent_session: Option<String>,
    pub ended_by: Option<String>,
    pub run: Run,
    /// The pinned thread that shows this child under its parent. Tab ids do not outlive the
    /// process, so it is not saved: a restart ties the task to its worker's restored tab again, by
    /// `worker_session`.
    pub thread: Option<Thread>,
    /// The Claude session of the thread that runs this task, which a restart resumes.
    pub worker_session: Option<String>,
    /// Loaded from disk as a thread's worker that `restore_tabs` is to bring back. It ends if no
    /// restored tab has `worker_session`.
    pub resuming: bool,
    /// The machine that runs this task for its parent here, when it runs elsewhere. Saved, so a
    /// restarted origin still takes the end that machine sends.
    pub remote: Option<RemoteTarget>,
    /// The tab the machine that runs this task knows its parent by. A restart gives that tab a new
    /// id here, and the other machine keeps naming the old one.
    pub remote_tab: Option<String>,
    /// The delegation limits it runs under. Its own children are held to them or tighter.
    pub limits: crate::taste::Limits,
    /// What it used so far, as its transcript reports it.
    pub usage: crate::usage::TaskUsage,
    pub delegator: Delegator,
    pub effort: Option<String>,
    /// The limit that stopped it, when one did.
    pub limit: Option<String>,
    /// Where it was asked to run, when a usage limit moved it elsewhere.
    pub moved: Option<Moved>,
    pub review: Option<Review>,
    /// The thread's tab, kept for the record after the tab is gone.
    pub thread_tab: Option<String>,
    /// The hub holds this task's record as it is now.
    pub recorded: bool,
    /// Where it runs and who chose that, for a task this machine's caller delegated.
    pub placement: Option<crate::placement::Placed>,
}

/// A task that a usage limit moved: what the request asked for, and why it ran elsewhere.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub account: AccountId,
    pub model: Option<String>,
    pub reason: String,
}

/// Where a task delegated to another machine or a box runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTarget {
    /// The hub's id for the machine, a box's included.
    pub machine: String,
    pub name: String,
    /// The worker thread there, once it opened.
    pub tab: Option<String>,
    /// When this machine last asked that machine how the task goes.
    pub looked: Option<Instant>,
    /// What the worker there waits on the user for, while it does.
    pub waiting: Option<RemoteAsk>,
}

/// A child's thread: a worker tab that runs it, or one that shows a headless child's steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub tab: String,
    /// The tab's own session is the child. A headless child (Cursor, Codex) runs outside it.
    pub runs: bool,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub begun: Instant,
    pub last_line: Instant,
    pub budget_ms: u64,
    pub limit_ms: u64,
    pub told_late: bool,
    pub told_pending: bool,
    pub open_turns: u32,
    pub last_error: Option<String>,
    pub background: Vec<Background>,
    /// Time that does not count toward the time limit: the worker waited on the user.
    pub paused: Duration,
    /// Since when the worker waits on the user (an approval or a question), while it does.
    pub user_wait: Option<Instant>,
    /// The thread's turn finished and the task is held only for work it left running: a limit
    /// that passes now ends it with its reply as the result.
    pub turn_done: bool,
    /// What a held thread task still runs in its session, as `task_status` lists it.
    pub held_on: Vec<String>,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            begun: Instant::now(),
            last_line: Instant::now(),
            budget_ms: DEFAULT_BUDGET_MS,
            limit_ms: DEFAULT_LIMIT_MS,
            told_late: false,
            told_pending: false,
            open_turns: 0,
            last_error: None,
            background: vec![],
            paused: Duration::ZERO,
            user_wait: None,
            turn_done: false,
            held_on: vec![],
        }
    }
}

impl Run {
    /// The milliseconds that count toward the time limit at `now`: since it began, less the time
    /// the worker waited on the user. Time pinned was down never reaches `begun` (see `SavedTask`).
    pub fn counted(&self, now: Instant) -> u64 {
        let waiting = self.user_wait.map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
        now.saturating_duration_since(self.begun).saturating_sub(self.paused).saturating_sub(waiting).as_millis() as u64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Background {
    pub kind: String,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct OpenInfo {
    pub cwd: PathBuf,
    pub account: AccountId,
    pub provider: Provider,
    pub conversation: String,
    /// The folder is a worktree this task made. The opened tab owns it.
    pub isolated: bool,
    /// What the opened tab may do. The parent's bypass is not copied.
    pub access: Access,
}

pub enum ChildEvent {
    Line(String),
    Exited { code: Option<i32>, stderr: String },
}

#[derive(Debug)]
pub enum StreamEvent {
    Conversation(String),
    /// Claude writes `system/init` at the start of every turn, including one that answers a background notification.
    TurnStart,
    Step(Item),
    Background(Vec<Background>),
    /// A finished turn's tokens, and the process's cost so far when the provider prints it.
    Usage(UsageReport),
    TurnEnd { text: String, error: Option<String> },
}

/// Headless argv for one provider. The binary is not included.
pub struct SpawnSpec<'a> {
    pub task: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub access: Access,
    pub mcp_config: Option<&'a Path>,
    /// The account's own MCP servers for this child, beside pinned's.
    pub session_mcp: Option<&'a crate::mcp::SessionFiles>,
    pub exe: &'a Path,
    pub sock: &'a Path,
    pub task_id: &'a str,
    pub reminders: bool,
}

mod catalog;
mod drivers;
mod limited;
mod metrics;
mod registry;
mod request;
mod rollup;

pub use catalog::*;
pub use drivers::*;
pub use limited::*;
pub use metrics::*;
pub use registry::*;
pub use request::*;
pub use rollup::*;

#[cfg(test)]
pub(crate) use drivers::{driver, replay};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_costly_word_in_a_model_id_outranks_a_cheap_one() {
        let tiers: Vec<_> = ["claude-opus-5-5", "claude-haiku-5-5", "opus-fast", "gpt-6-max-mini", "minimax-m3", "minimax-m3.1-flash", "qwen3.8-max", "glm-5.3"].iter().map(|id| cost_of(id).name()).collect();
        assert_eq!(tiers, ["high", "low", "high", "high", "mid", "low", "high", "mid"]);
    }

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/delegate")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    fn same(got: Vec<String>, expect: &[&str]) {
        assert_eq!(
            got,
            expect
                .iter()
                .copied()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
    }

    fn spec<'a>(task: &'a str, access: Access) -> SpawnSpec<'a> {
        SpawnSpec {
            task,
            model: None,
            effort: None,
            access,
            mcp_config: None,
            session_mcp: None,
            exe: Path::new("/bin/pinned"),
            sock: Path::new("/tmp/pinned.sock"),
            task_id: "task-1",
            reminders: true,
        }
    }

    #[test]
    fn a_task_that_looks_like_a_flag_stays_the_prompt() {
        let task = "--dangerously-skip-permissions";
        let claude = driver_args(Provider::Claude, &spec(task, Access::ReadOnly));
        assert_eq!(claude[1], "\n--dangerously-skip-permissions");
        assert!(!claude.iter().any(|arg| arg == task), "{claude:?}");
        for provider in [Provider::Codex, Provider::Cursor] {
            let args = driver_args(provider, &spec("--force", Access::Auto));
            assert_eq!(args.last().map(String::as_str), Some("\n--force"), "{provider:?}");
            assert!(!args.iter().any(|arg| arg == "--force"), "{provider:?} {args:?}");
        }
    }

    #[test]
    fn a_headless_child_loads_the_accounts_mcp_servers_beside_pinneds() {
        let load: crate::mcp::Load = serde_json::from_value(serde_json::json!({ "server": { "name": "ctx", "transport": { "type": "stdio", "command": "npx", "env": ["API_KEY"] } }, "secrets": { "env": { "API_KEY": "ctx-secret-1" } } })).unwrap();
        let files = crate::mcp::SessionFiles::write(&[&load]).unwrap().unwrap();
        let file = files.file.display().to_string();
        let mut claude = spec("do the thing", Access::Auto);
        claude.mcp_config = Some(Path::new("/tmp/mcp.json"));
        claude.session_mcp = Some(&files);
        let args = driver_args(Provider::Claude, &claude);
        assert_eq!(args[args.len() - 3..], ["--mcp-config".to_owned(), "/tmp/mcp.json".to_owned(), file.clone()]);
        let codex = driver_args(Provider::Codex, &SpawnSpec { session_mcp: Some(&files), ..spec("do the thing", Access::Auto) });
        let at = codex.iter().position(|a| a == "mcp_servers.ctx.command=\"/bin/pinned\"").unwrap_or_else(|| panic!("{codex:?}"));
        assert_eq!(codex[at + 2], format!("mcp_servers.ctx.args=[\"mcp-run\",{},\"ctx\"]", toml_str(&file)));
        assert_eq!(codex.last().map(String::as_str), Some("do the thing"), "the prompt stays last");
        let cursor = driver_args(Provider::Cursor, &SpawnSpec { session_mcp: Some(&files), ..spec("do the thing", Access::Auto) });
        assert!(!cursor.concat().contains("mcp"), "cursor-agent takes no MCP flag: {cursor:?}");
        for args in [args, codex] {
            assert!(!args.concat().contains("ctx-secret-1"), "{args:?}");
        }
    }

    #[test]
    fn each_provider_builds_the_argv_for_its_access() {
        let mut full = spec("do the thing", Access::Full);
        full.model = Some("claude-haiku-5-5");
        full.effort = Some("low");
        full.mcp_config = Some(Path::new("/tmp/mcp.json"));
        same(
            driver_args(Provider::Claude, &full),
            &[
                "-p",
                "do the thing",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "claude-haiku-5-5",
                "--effort",
                "low",
                "--dangerously-skip-permissions",
                "--mcp-config",
                "/tmp/mcp.json",
            ],
        );
        same(
            driver_args(Provider::Claude, &spec("do the thing", Access::Auto)),
            &[
                "-p",
                "do the thing",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "auto",
            ],
        );
        same(
            driver_args(Provider::Claude, &spec("do the thing", Access::ReadOnly)),
            &[
                "-p",
                "do the thing",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "plan",
            ],
        );

        let mut codex = spec("do the thing", Access::Auto);
        codex.model = Some("gpt-5");
        codex.effort = Some("high");
        same(
            driver_args(Provider::Codex, &codex),
            &[
                "exec",
                "--json",
                "--skip-git-repo-check",
                "-m",
                "gpt-5",
                "-c",
                "model_reasoning_effort=\"high\"",
                "--full-auto",
                "-c",
                "mcp_servers.pinned.command=\"/bin/pinned\"",
                "-c",
                "mcp_servers.pinned.args=[\"mcp\"]",
                "-c",
                "mcp_servers.pinned.tool_timeout_sec=86400",
                "-c",
                "mcp_servers.pinned.env.PINNED_SOCK=\"/tmp/pinned.sock\"",
                "-c",
                "mcp_servers.pinned.env.PINNED_TASK=\"task-1\"",
                "do the thing",
            ],
        );
        let head = |access, n: usize| {
            driver_args(Provider::Codex, &spec("do the thing", access))
                .into_iter()
                .take(n)
                .collect::<Vec<_>>()
        };
        same(
            head(Access::Full, 4),
            &[
                "exec",
                "--json",
                "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox",
            ],
        );
        same(
            head(Access::ReadOnly, 5),
            &["exec", "--json", "--skip-git-repo-check", "-s", "read-only"],
        );
        assert_eq!(
            driver_args(Provider::Codex, &spec("do the thing", Access::Full))
                .last()
                .map(String::as_str),
            Some("do the thing")
        );

        let mut cursor = spec("do the thing", Access::Full);
        cursor.model = Some("grok-4.7-high");
        same(
            driver_args(Provider::Cursor, &cursor),
            &[
                "-p",
                "--output-format",
                "stream-json",
                "--trust",
                "--force",
                "--model",
                "grok-4.7-high",
                "do the thing",
            ],
        );
        same(
            driver_args(Provider::Cursor, &spec("do the thing", Access::Auto)),
            &[
                "-p",
                "--output-format",
                "stream-json",
                "--trust",
                "do the thing",
            ],
        );
        same(
            driver_args(Provider::Cursor, &spec("do the thing", Access::ReadOnly)),
            &[
                "-p",
                "--output-format",
                "stream-json",
                "--trust",
                "--mode",
                "plan",
                "do the thing",
            ],
        );
    }

    #[test]
    fn a_request_grants_tools() {
        let granted = parse_request(&json!({ "task": "look", "tools": ["Bash", " Grep ", "", 7] })).unwrap();
        assert_eq!(granted.tools, ["Bash", "Grep"]);
        assert_eq!(parse_request(&json!({ "task": "look" })).unwrap().tools, Vec::<String>::new());
        assert_eq!(granted_brief("look", &granted.tools), "Granted tools: Bash, Grep.\n\nlook");
        assert_eq!(granted_brief("look", &[]), "look");
    }

    #[test]
    fn a_request_accepts_t3_argument_names() {
        let inherit = parse_request(&json!({ "task": "look", "runtimeMode": "inherit" })).unwrap();
        assert_eq!(inherit.runtime, None, "inherit is the same as leaving runtimeMode out");
        let narrow = parse_request(&json!({ "task": "look", "runtimeMode": "approval-required" })).unwrap_err();
        assert!(narrow.contains("cannot pause for approval"), "{narrow}");
        let args = json!({
            "task": "fix the flaky test",
            "title": "Flaky",
            "target": { "providerInstanceId": "cursor", "model": "grok-4.7", "options": { "reasoning_effort": "high" } },
            "isolation": "worktree",
            "mode": "wait",
            "timeoutMs": 5000,
            "budgetMs": 90000,
            "limitMs": 600000,
            "interactionMode": "plan"
        });
        assert_eq!(
            parse_request(&args).unwrap(),
            Request {
                task: "fix the flaky test".into(),
                title: "Flaky".into(),
                target: Target {
                    account: Some("cursor".into()),
                    model: Some("grok-4.7".into()),
                    effort: Some("high".into()),
                    driver_kind: None,
                    machine: None,
                    boxed: None,
                },
                isolation: Isolation::Worktree,
                mode: Mode::Wait,
                timeout_ms: 5000,
                budget_ms: Some(90_000),
                limit_ms: Some(600_000),
                interaction: Interaction::Plan,
                runtime: None,
                role: None,
                tools: vec![],
                client_request_id: None,
                heavy: None,
            }
        );
        let on = |target: Value| parse_request(&json!({"task": "x", "target": target})).map(|r| (r.target.machine, r.target.boxed));
        assert_eq!(on(json!({"machine": " mini "})), Ok((Some("mini".into()), None)));
        assert_eq!(on(json!({"box": true})), Ok((None, Some(BoxTarget { size: None, repo: None }))));
        assert_eq!(
            on(json!({"box": {"size": "large", "repo": "acme/site"}})),
            Ok((None, Some(BoxTarget { size: Some(crate::connectors::BoxSize::Large), repo: Some("acme/site".into()) })))
        );
        assert_eq!(on(json!({"box": false})), Ok((None, None)));
        assert_eq!(on(json!({"box": "yes"})), Err("target.box must be true or { size, repo }, not \"yes\"".into()));
        assert_eq!(on(json!({"machine": "mini", "box": true})), Err("pass target.machine or target.box, not both".into()));
        let effort = json!({"task": "x", "target": {"options": {"effort": "low"}}});
        assert_eq!(
            parse_request(&effort).unwrap().target.effort.as_deref(),
            Some("low")
        );
        let listed = json!({
            "task": "look",
            "target": { "driverKind": "codex", "options": [{ "id": "effort", "value": "low" }] },
        });
        let listed = parse_request(&listed).unwrap();
        assert_eq!(listed.target.account.as_deref(), Some("codex"));
        assert_eq!(listed.target.effort.as_deref(), Some("low"));
        let named = parse_request(&json!({"task": "look", "target": {"providerInstanceId": "cursor", "driverKind": "claude"}}));
        assert_eq!(
            named.unwrap_err(),
            "providerInstanceId \"cursor\" is Cursor and does not match driverKind \"claude\""
        );
        let matched = parse_request(&json!({"task": "look", "target": {"providerInstanceId": "cursor", "driverKind": "cursor"}})).unwrap();
        assert_eq!(matched.target.account.as_deref(), Some("cursor"));
        assert_eq!(matched.target.driver_kind.as_deref(), Some("cursor"));
        assert_eq!(
            require_driver("work", Provider::Codex, "claude").unwrap_err(),
            "providerInstanceId \"work\" is Codex and does not match driverKind \"claude\""
        );
        assert_eq!(
            parse_request(&json!({"task": "hello"})).unwrap(),
            Request {
                task: "hello".into(),
                title: "hello".into(),
                target: Target {
                    account: None,
                    model: None,
                    effort: None,
                    driver_kind: None,
                    machine: None,
                    boxed: None,
                },
                isolation: Isolation::Shared,
                mode: Mode::Async,
                timeout_ms: DEFAULT_TIMEOUT_MS,
                budget_ms: None,
                limit_ms: None,
                interaction: Interaction::Inherit,
                runtime: None,
                role: None,
                tools: vec![],
                client_request_id: None,
                heavy: None,
            }
        );
        assert_eq!(
            parse_request(&json!({"task": "x", "budgetMs": 0})).unwrap_err(),
            "budgetMs must be a number of milliseconds above 0"
        );
        assert_eq!(
            parse_request(&json!({"task": "x", "budgetMs": 36_000_000, "limitMs": 36_000_000})).unwrap_err(),
            "budgetMs must be below limitMs, which is 36000000 ms; pass a larger limitMs"
        );
        let long = parse_request(&json!({"task": "x", "budgetMs": 36_000_000, "limitMs": 72_000_000})).unwrap();
        assert_eq!((long.budget_ms, long.limit_ms), (Some(36_000_000), Some(72_000_000)));
        assert_eq!(parse_request(&json!({})).unwrap_err(), "task is required");
        assert_eq!(
            parse_request(&json!({"task": "x", "mode": "soon"})).unwrap_err(),
            "mode must be async or wait, not \"soon\""
        );
    }

    #[test]
    fn a_limit_alone_puts_the_budget_notice_before_it() {
        let eight_hours = 28_800_000;
        assert_eq!(run_limits(None, Some(3_600_000), eight_hours), Ok((1_800_000, 3_600_000)));
        assert_eq!(run_limits(None, Some(43_200_000), 86_400_000), Ok((7_200_000, 43_200_000)));
    }

    #[test]
    fn the_time_limit_is_the_default_and_the_ceiling_of_limit_ms() {
        let hour = 3_600_000;
        assert_eq!(run_limits(None, None, 8 * hour), Ok((2 * hour, 8 * hour)), "no limitMs runs to the time limit");
        assert_eq!(run_limits(None, None, 12 * hour), Ok((2 * hour, 12 * hour)), "a time limit above the old 8 h default is the default");
        assert_eq!(run_limits(None, Some(20 * hour), hour), Ok((hour / 2, hour)), "a longer limitMs is cut to the time limit");
        assert_eq!(run_limits(Some(30 * 60_000), Some(20 * hour), hour), Ok((30 * 60_000, hour)));
        assert_eq!(
            run_limits(Some(2 * hour), Some(20 * hour), hour),
            Err("budgetMs must be below the run limit, which is 3600000 ms; ask for less, or raise Time limit in Settings › Way of working".into())
        );
    }

    #[test]
    fn cursor_effort_uses_a_catalog_variant() {
        let ids = [
            "grok-4.7-low",
            "grok-4.7-medium",
            "grok-4.7-high",
            "grok-4.7-xhigh",
            "grok-4.7-high-fast",
            "composer-2.5",
            "composer-2.5-fast",
        ]
        .map(str::to_owned);
        assert_eq!(
            map_cursor_model(&ids, "grok-4.7", Some("high")),
            "grok-4.7-high"
        );
        assert_eq!(map_cursor_model(&ids, "grok-4.7", Some("max")), "grok-4.7");
        assert_eq!(map_cursor_model(&ids, "composer-2.5", None), "composer-2.5");
        assert_eq!(
            map_cursor_model(&ids, "composer-2.5", Some("fast")),
            "composer-2.5-fast"
        );
        let listed = "\
composer-2.5 - Composer 2.5 (current)
composer-2.5-fast - Composer 2.5 Fast
grok-4.7-low - Grok 4.7  Low
grok-4.7-high - Grok 4.7  High
";
        assert_eq!(
            cursor_models(listed),
            vec![
                ModelChoice {
                    id: "composer-2.5".into(),
                    label: "Composer 2.5".into(),
                    efforts: vec!["fast".into()],
                    cost: Cost::Mid,
                },
                ModelChoice {
                    id: "grok-4.7".into(),
                    label: "Grok 4.7".into(),
                    efforts: vec!["low".into(), "high".into()],
                    cost: Cost::Mid,
                },
            ]
        );
    }

    #[test]
    fn capabilities_list_the_parent_and_each_signed_in_account() {
        let accounts = vec![
            ListedAccount {
                id: "claude".into(),
                provider: Provider::Claude,
                name: "Claude Code".into(),
                signed_in: true,
                bin_found: true,
                limited: false,
                limit: None,
                models: provider_models(Provider::Claude),
                constraints: vec![],
            },
            ListedAccount {
                id: "codex".into(),
                provider: Provider::Codex,
                name: "Codex".into(),
                signed_in: false,
                bin_found: false,
                limited: false,
                limit: None,
                models: vec![],
                constraints: codex_constraints(),
            },
            ListedAccount {
                id: "cursor".into(),
                provider: Provider::Cursor,
                name: "Cursor".into(),
                signed_in: true,
                bin_found: true,
                limited: true,
                limit: None,
                models: cursor_models(
                    "grok-4.7-high - Grok 4.7  High\ncomposer-2.5 - Composer 2.5\n",
                ),
                constraints: cursor_constraints(),
            },
        ];
        let parent = Inherited {
            account: "claude".into(),
            model: Some("claude-haiku-5-5".into()),
            access: Access::Auto,
        };
        let value = capabilities(&parent, &accounts, None);
        assert_eq!(value["inheritedProviderInstanceId"], "claude");
        assert_eq!(value["inheritedModel"], "claude-haiku-5-5");
        assert_eq!(value["runtimeMode"], "auto");
        assert_eq!(value["features"]["appOwnedSubagents"], true);
        assert_eq!(value["features"]["asyncPolling"], false);
        assert_eq!(value["features"]["cancellation"], true);
        assert_eq!(value["providers"][0]["driverKind"], "claude");
        assert_eq!(value["providers"][0]["models"][3]["id"], "claude-haiku-5-5");
        assert_eq!(
            value["providers"][0]["models"][3]["options"][0]["values"],
            json!(["low", "medium", "high", "xhigh", "max"])
        );
        assert_eq!(value["providers"][0]["models"][3]["options"][0]["default"], Value::Null);
        assert_eq!(value["providers"][0]["runtimeModes"], json!(["auto", "full-access"]));
        assert_eq!(value["providers"][0]["canRunChildTask"], true);
        assert_eq!(value["providers"][0]["canRunCrossProviderChildTask"], true);
        assert_eq!(value["providers"][1]["canRunChildTask"], false);
        assert_eq!(value["providers"][1]["canRunCrossProviderChildTask"], false);
        assert_eq!(
            value["providers"][1]["constraints"][0],
            "Any model id works. Pass it as target.model."
        );
        assert_eq!(value["providers"][1]["models"], json!([]));
        assert_eq!(
            value["providers"][2]["models"][0],
            json!({"id": "grok-4.7", "label": "Grok 4.7", "cost": "mid", "options": [{"id": "effort", "values": ["high"], "default": null}]})
        );
        assert_eq!(value["providers"][2]["limited"], true);
        assert_eq!(value["providers"][0]["models"][3]["cost"], "low");
        assert_eq!(value["providers"][2]["canRunCrossProviderChildTask"], false);
        assert_eq!(
            value["providers"][2]["constraints"][0],
            "Cursor supports only its built-in account."
        );
        assert_eq!(
            value["providers"][2]["constraints"][1],
            "A delegated child cannot start another pinned worker. A Cursor session cannot delegate."
        );
        let found = capabilities(&parent, &accounts, Some("grok"));
        assert_eq!(found["providers"].as_array().unwrap().len(), 1);
        assert_eq!(found["providers"][0]["providerInstanceId"], "cursor");
        assert_eq!(found["providers"][0]["models"][0]["id"], "grok-4.7");
    }

    #[test]
    fn claude_lists_fable_5_2_and_keeps_fable_5_1_as_legacy() {
        let accounts = vec![ListedAccount {
            id: "claude".into(),
            provider: Provider::Claude,
            name: "Claude Code".into(),
            signed_in: true,
            bin_found: true,
            limited: false,
            limit: None,
            models: provider_models(Provider::Claude),
            constraints: vec![],
        }];
        let parent = Inherited { account: "claude".into(), model: None, access: Access::Auto };
        let value = capabilities(&parent, &accounts, None);
        let listed: Vec<(&str, &str, &str)> = value["providers"][0]["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["id"].as_str().unwrap(), m["label"].as_str().unwrap(), m["cost"].as_str().unwrap()))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("claude-fable-5-2", "Fable 5.2", "mid"),
                ("claude-opus-5-5", "Opus 5.5", "high"),
                ("claude-sonnet-5-5", "Sonnet 5.5", "mid"),
                ("claude-haiku-5-5", "Haiku 5.5", "low"),
                ("claude-fable-5-1", "Fable 5.1 (legacy)", "mid"),
                ("claude-fable-5", "Fable 5 (legacy)", "mid"),
                ("claude-opus-5", "Opus 5 (legacy)", "high"),
                ("claude-opus-4-8", "Opus 4.8 (legacy)", "high"),
                ("claude-opus-4-7", "Opus 4.7 (legacy)", "high"),
                ("claude-opus-4-6", "Opus 4.6 (legacy)", "high"),
                ("claude-opus-4-5", "Opus 4.5 (legacy)", "high"),
                ("claude-sonnet-5", "Sonnet 5 (legacy)", "mid"),
                ("claude-sonnet-4-6", "Sonnet 4.6 (legacy)", "mid"),
                ("claude-haiku-4-5", "Haiku 4.5 (legacy)", "low"),
            ]
        );
        let ids = |search| {
            capabilities(&parent, &accounts, Some(search))["providers"][0]["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("fable"), ["claude-fable-5-2", "claude-fable-5-1", "claude-fable-5"]);
        assert_eq!(ids("claude-fable-5-1"), ["claude-fable-5-1"]);
    }

    #[test]
    fn the_three_streams_end_on_their_final_text() {
        let cursor = replay(Provider::Cursor, &fixture("cursor.jsonl"));
        assert_eq!(
            cursor.conversation.as_deref(),
            Some("9fa2e973-3c1c-4a8e-9ab7-9790584c944c")
        );
        assert_eq!(cursor.text.as_deref(), Some("DONE"));
        assert_eq!(cursor.error, None);
        assert!(cursor.done);
        assert!(cursor.steps.contains(&Item::Tool {
            id: "tool_1cd000e1-3c00-4087-b89c-14f64143efb".into(),
            at: String::new(),
            name: "shell".into(),
            summary: "echo hi".into(),
            detail: "echo hi".into(),
            args: vec![],
        }));
        assert!(cursor.steps.contains(&Item::Result {
            tool: "tool_1cd000e1-3c00-4087-b89c-14f64143efb".into(),
            at: String::new(),
            error: false,
            text: "hi\n".into(),
            links: vec![],
        }));

        let claude = replay(Provider::Claude, &fixture("claude.jsonl"));
        assert_eq!(
            claude.conversation.as_deref(),
            Some("def5ce01-e13c-49b7-9713-b8c1c0b57e42")
        );
        assert_eq!(claude.text.as_deref(), Some("ok"));
        assert_eq!(claude.error, None);
        assert!(claude.done);
        assert!(claude.steps.contains(&Item::Tool {
            id: "toolu_019yShaJQSmTk9tsieBLbEZo".into(),
            at: "2026-10-29T19:56:39.494Z".into(),
            name: "mcp__probe__ping_me".into(),
            summary: String::new(),
            detail: String::new(),
            args: vec![],
        }));
        assert!(claude.steps.contains(&Item::Result {
            tool: "toolu_019yShaJQSmTk9tsieBLbEZo".into(),
            at: "2026-10-29T19:56:39.516Z".into(),
            error: false,
            text: "pong".into(),
            links: vec![],
        }));

        // The codex fixture follows the documented exec JSON schema. It was not captured from a live CLI.
        let codex = replay(Provider::Codex, &fixture("codex.jsonl"));
        assert_eq!(codex.conversation.as_deref(), Some("thread_01"));
        assert_eq!(codex.text.as_deref(), Some("Ship it"));
        assert!(codex.done, "turn.completed ends the codex run");
        assert!(codex.steps.contains(&Item::Tool {
            id: "item_2".into(),
            at: String::new(),
            name: "Bash".into(),
            summary: "cargo test".into(),
            detail: "cargo test".into(),
            args: vec![],
        }));
        assert!(codex.steps.contains(&Item::Thinking {
            id: "item_1".into(),
            at: String::new(),
            text: "Look at the tests.".into()
        }));
        assert_eq!(
            [cursor.usage, claude.usage, codex.usage],
            [
                UsageReport { tokens_in: 16_289 + 24_224, tokens_out: 71, cost_usd: None },
                UsageReport { tokens_in: 6 + 17_998 + 69_766, tokens_out: 177, cost_usd: Some(0.004517060000000001) },
                UsageReport { tokens_in: 10, tokens_out: 4, cost_usd: None },
            ],
            "each provider's turn usage, with the cost only Claude prints"
        );
        let failed = (driver(Provider::Codex).parse)(
            r#"{"type":"turn.failed","error":{"message":"rate limit"}}"#,
        );
        match failed.as_slice() {
            [StreamEvent::TurnEnd { text, error }] => {
                assert_eq!((text.as_str(), error.as_deref()), ("", Some("rate limit")))
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    fn new_cursor(delegates: &Delegates, mode: Mode) -> Task {
        delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: Some("toolu_1".into()),
            title: "Ping".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: Some("composer-2.5".into()),
            cwd: PathBuf::from("/tmp"),
            mode,
            access: Access::Auto,
            isolated: false,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        })
    }

    #[cfg(unix)]
    pub(super) fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        path
    }

    fn new_on(delegates: &Delegates, provider: Provider) -> Task {
        delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: "Fix".into(),
            account: provider.spec().builtin.into(),
            provider,
            model: None,
            cwd: PathBuf::from("/tmp"),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        })
    }

    #[test]
    fn two_callers_cannot_both_take_the_last_place_at_once() {
        let delegates = Arc::new(Delegates::default());
        let ask = |caller: usize, cwd: &str| NewTask {
            parent: Parent::Tab(format!("tab-{caller}")),
            tool: None,
            title: "Fix".into(),
            account: "claude".into(),
            provider: Provider::Claude,
            model: None,
            cwd: PathBuf::from(cwd),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: format!("tab:tab-{caller}"),
            client_request_id: None,
            role: None,
        };
        assert!(matches!(delegates.claim(ask(0, "/p/.claude/worktrees/w1"), Some(2)), Claim::Fresh(_)));
        assert!(matches!(delegates.claim(ask(0, "/elsewhere"), Some(1)), Claim::Fresh(_)), "another project has its own places");
        let start = Arc::new(std::sync::Barrier::new(8));
        let callers: Vec<_> = (1..=8)
            .map(|caller| {
                let (delegates, start, new) = (delegates.clone(), start.clone(), ask(caller, "/p"));
                std::thread::spawn(move || {
                    start.wait();
                    delegates.claim(new, Some(2))
                })
            })
            .collect();
        let claims: Vec<_> = callers.into_iter().map(|caller| caller.join().unwrap()).collect();
        let fresh = claims.iter().filter(|claim| matches!(claim, Claim::Fresh(_))).count();
        let full: Vec<_> = claims.iter().filter_map(|claim| if let Claim::Full(working) = claim { Some(*working) } else { None }).collect();
        assert_eq!((fresh, full), (1, vec![2; 7]), "the worker in p's worktree and one more fill p's two places");
    }

    fn parent() -> Caller {
        Caller { tab: Some("tab-1".into()), task: None }
    }

    #[cfg(unix)]
    struct Leftover(PathBuf);

    #[cfg(unix)]
    impl Leftover {
        fn pid(&self) -> i32 {
            let start = Instant::now();
            loop {
                if let Some(pid) = std::fs::read_to_string(&self.0).ok().and_then(|text| text.trim().parse().ok()) {
                    return pid;
                }
                assert!(start.elapsed() < Duration::from_secs(5), "{} was never written", self.0.display());
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    #[cfg(unix)]
    impl Drop for Leftover {
        fn drop(&mut self) {
            if let Some(pid) = std::fs::read_to_string(&self.0).ok().and_then(|text| text.trim().parse::<i32>().ok()) {
                super::drivers::signal(&[-pid, pid]);
            }
        }
    }

    #[cfg(unix)]
    pub(super) fn stopped(pid: i32) -> bool {
        let start = Instant::now();
        while unsafe { libc::kill(pid, 0) } == 0 {
            if start.elapsed() > Duration::from_secs(3) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    #[cfg(unix)]
    fn detached_loop_worker(lines: &[&str], pidfile: &Path) -> String {
        let replies: Vec<String> = lines.iter().map(|line| format!("echo '{line}'")).collect();
        let pidfile = pidfile.display();
        format!(
            "perl -e 'use POSIX (); POSIX::setsid(); exec @ARGV' sh -c 'echo $$ > \"$0\"; while :; do echo tick; sleep 1; done' '{pidfile}' &\nwhile [ ! -s '{pidfile}' ]; do sleep 0.05; done\n{}\nwait",
            replies.join("\n"),
        )
    }

    #[cfg(unix)]
    fn run(delegates: &Arc<Delegates>, task: &Task, bin: &Path, dir: &Path) -> (u32, std::sync::mpsc::Receiver<String>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let sink = delegates.clone();
        let id = task.id.clone();
        let pid = spawn(
            ChildSpec { bin, argv: &[], cwd: dir, env: &[], path: None, strip: &[] },
            move |event| {
                if let Some(tab) = sink.apply(&id, event, None) {
                    let _ = tx.send(tab);
                }
            },
        )
        .unwrap();
        delegates.remember_pid(&task.id, pid);
        (pid, rx)
    }

    pub(super) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-delegate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
    const REPLY: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"PR opened"}]}}"#;

    #[test]
    #[cfg(unix)]
    fn a_final_reply_ends_the_task_and_stops_what_the_worker_left_running() {
        let dir = scratch("leftover");
        let result = r#"{"type":"result","subtype":"success","is_error":false,"result":"PR opened"}"#;
        let codex = [
            r#"{"type":"thread.started","thread_id":"t1"}"#,
            r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"PR opened"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}"#,
        ];
        let mut failures = vec![];
        for (provider, lines) in [(Provider::Claude, [INIT, REPLY, result]), (Provider::Cursor, [INIT, REPLY, result]), (Provider::Codex, codex)] {
            let name = provider.spec().name;
            let leftover = Leftover(dir.join(format!("{name}.pid")));
            let bin = script(&dir, name, &detached_loop_worker(&lines, &leftover.0));
            let delegates = Arc::new(Delegates::default());
            let task = new_on(&delegates, provider);
            let (agent, told) = run(&delegates, &task, &bin, &dir);
            let background = leftover.pid();
            if told.recv_timeout(Duration::from_secs(5)).as_deref() != Ok("tab-1") {
                failures.push(format!("{name}: the parent was not told, the task is {}", delegates.status(&parent(), &task.id).unwrap()["workState"]));
                continue;
            }
            let status = delegates.status(&parent(), &task.id).unwrap();
            if (status["workState"].as_str(), status["summary"].as_str()) != (Some("result_available"), Some("PR opened")) {
                failures.push(format!("{name}: finished as {status}"));
            }
            if !stopped(background) {
                failures.push(format!("{name}: the background loop {background} kept running"));
            }
            if !stopped(agent as i32) {
                failures.push(format!("{name}: the agent {agent} kept running"));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(failures.is_empty(), "{failures:#?}");
    }

    fn state_at_each_result(recording: &str) -> Vec<(String, String)> {
        let delegates = Delegates::default();
        let task = new_on(&delegates, Provider::Claude);
        let mut seen = vec![];
        for line in recording.lines() {
            delegates.apply(&task.id, ChildEvent::Line(line.to_owned()), None);
            if line.contains(r#""type":"result""#) {
                let status = delegates.status(&parent(), &task.id).unwrap();
                seen.push((status["workState"].as_str().unwrap_or("").to_owned(), status["summary"].as_str().unwrap_or("").to_owned()));
            }
        }
        seen
    }

    const INTERIM: &str = "Background command is still running; I'll report once the completion notification arrives.";

    #[test]
    fn a_claude_result_is_the_answer_only_when_no_turn_or_background_task_is_open() {
        let pair = |state: &str, summary: &str| (state.to_owned(), summary.to_owned());
        assert_eq!(state_at_each_result(&fixture("claude-reply.jsonl")), vec![pair("result_available", "PINEAPPLE")]);
        assert_eq!(
            state_at_each_result(&fixture("claude-background-shell.jsonl")),
            vec![pair("working", INTERIM), pair("result_available", "FINAL-REPORT\nbgdone\n\n[exited with code 0]")],
            "the first result came while the background shell was pending"
        );
        assert_eq!(
            state_at_each_result(&fixture("claude-background-agent.jsonl")),
            vec![pair("working", "WAITING"), pair("result_available", "FINAL-REPORT\nDONE-SUB")],
            "Claude held the first turn's result until the subagent finished and the next turn began"
        );
    }

    #[test]
    fn a_quiet_reply_waiting_on_a_subagent_is_left_to_the_budget() {
        let delegates = Delegates::default();
        let task = new_on(&delegates, Provider::Claude);
        let recording = fixture("claude-background-agent.jsonl");
        let lines = [r#""subtype":"init""#, r#""subtype":"background_tasks_changed""#, r#""type":"result""#]
            .map(|marker| recording.lines().find(|line| line.contains(marker)).unwrap().to_owned());
        for line in lines {
            delegates.apply(&task.id, ChildEvent::Line(line), None);
        }
        later(&delegates, Duration::from_secs(31 * 60));
        let status = delegates.status(&parent(), &task.id).unwrap();
        assert_eq!((status["workState"].as_str(), status["background"][0].as_str()), (Some("working"), Some("Run sleep 15 and reply")), "{status}");
    }

    #[test]
    fn a_reply_with_background_work_pending_is_reported_once() {
        let delegates = Delegates::default();
        let task = new_on(&delegates, Provider::Claude);
        let recording = fixture("claude-background-shell.jsonl");
        let first: Vec<&str> = recording.lines().take_while(|line| !line.contains("task_updated")).collect();
        for line in first.iter().chain(first.last()) {
            delegates.apply(&task.id, ChildEvent::Line((*line).to_owned()), None);
        }
        let notice = delegates.claim_notice("tab-1").map(|(text, _)| text).unwrap_or_default();
        assert_eq!(notice, pending_line(&task.id));
    }

    fn pending_line(id: &str) -> String {
        format!("[pinned] Delegated task {id} \"Fix\" (Claude Code) replied and still runs 1 background task. pinned ends the task when that work finishes and the agent replies again, or, when only shells are left, after 30 min with no activity, and then stops what is still running. Call task_status with taskId \"{id}\" to read the reply and the work, or task_cancel to stop it now.")
    }

    #[test]
    fn a_quiet_reply_is_not_taken_while_a_later_turn_runs() {
        let recording = fixture("claude-background-shell.jsonl");
        let lines: Vec<&str> = recording.lines().collect();
        let shell_finished = lines[..13].to_vec();
        let shell_pending: Vec<&str> = lines[..9].iter().chain(&lines[12..13]).copied().collect();
        let mut seen = vec![];
        for feed in [shell_finished, shell_pending] {
            let delegates = Delegates::default();
            let task = new_on(&delegates, Provider::Claude);
            for line in feed {
                delegates.apply(&task.id, ChildEvent::Line(line.to_owned()), None);
            }
            later(&delegates, Duration::from_secs(31 * 60));
            let status = delegates.status(&parent(), &task.id).unwrap();
            seen.push(json!([status["workState"], status["summary"], status["background"], status["endedBy"]]));
        }
        assert_eq!(
            seen,
            vec![
                json!(["working", INTERIM, [], null]),
                json!(["working", INTERIM, ["Run sleep 20 then echo bgdone in background"], null]),
            ]
        );
    }

    #[test]
    fn a_resumed_task_keeps_counting_toward_its_time_limit_from_its_original_start() {
        let dir = scratch("limit-across-restart");
        let file = dir.join("delegates.json");
        let first = Delegates::load(Some(file.clone()));
        let task = new_on(&first, Provider::Claude);
        first.link_thread(&task.id, Thread { tab: "tab-1".into(), runs: true });
        first.backdate(&task.id, Duration::from_millis(DEFAULT_LIMIT_MS - 60_000));
        first.backdate_run(&task.id, Duration::from_millis(DEFAULT_LIMIT_MS - 60_000));
        first.set_worker_session(&task.id, "s-worker");
        drop(first);

        let after = Delegates::load(Some(file));
        assert_eq!(after.get(&task.id).unwrap().state, WorkState::Working, "a thread's worker is resumed by the restart, not ended");
        after.enforce_limits(Instant::now(), None);
        assert_eq!(after.get(&task.id).unwrap().state, WorkState::Working, "a minute of its limit is left");
        after.enforce_limits(Instant::now() + Duration::from_secs(120), None);
        let ended = after.get(&task.id).unwrap();
        assert_eq!(ended.state, WorkState::Cancelled);
        assert!(ended.error.unwrap().contains("its limit"), "the limit counted the time before the restart");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn after_each_stop(name: &str, provider: Provider, lines: &[&str]) -> Vec<Value> {
        let dir = scratch(name);
        let file = dir.join("delegates.json");
        let mut seen = vec![];
        for how in ["task_cancel", "limit", "quit", "crash"] {
            let delegates = Delegates::load(Some(file.clone()));
            let task = new_on(&delegates, provider);
            for line in lines {
                delegates.apply(&task.id, ChildEvent::Line((*line).to_owned()), None);
            }
            let delegates = match how {
                "task_cancel" => {
                    delegates.cancel(&parent(), &task.id, "the parent asked", None).unwrap();
                    delegates
                }
                "limit" => {
                    later(&delegates, Duration::from_millis(DEFAULT_LIMIT_MS));
                    delegates
                }
                "quit" => {
                    delegates.stop_for_restart(None);
                    delegates
                }
                _ => {
                    drop(delegates);
                    Delegates::load(Some(file.clone()))
                }
            };
            let status = delegates.status(&parent(), &task.id).unwrap();
            seen.push(json!([how, status["workState"], status["summary"], status["error"]]));
        }
        let _ = std::fs::remove_dir_all(&dir);
        seen
    }

    #[test]
    fn a_cancelled_task_keeps_the_reply_so_far() {
        assert_eq!(
            after_each_stop("cancel-keeps", Provider::Cursor, &[INIT, REPLY]),
            vec![
                json!(["task_cancel", "cancelled", "PR opened", "the parent asked"]),
                json!(["limit", "cancelled", "PR opened", "pinned stopped it after 8 h, its limit"]),
                json!(["quit", "cancelled", "PR opened", RESTART_ENDED]),
                json!(["crash", "cancelled", "PR opened", RESTART_ENDED]),
            ]
        );
    }

    #[test]
    fn a_cancelled_task_keeps_its_newest_reply() {
        let recording = fixture("claude-background-shell.jsonl");
        let before_the_final_result: Vec<&str> = recording.lines().take(17).collect();
        let shell = r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_type":"local_bash","description":"npm run dev"}]}"#;
        let result = r#"{"type":"result","subtype":"success","is_error":false,"result":"PR opened and CI is green"}"#;
        let subagent = fixture("claude-background-agent.jsonl");
        let held_first_result: Vec<&str> = subagent.lines().take(22).collect();
        let summaries = |name: &str, lines: &[&str]| after_each_stop(name, Provider::Claude, lines).into_iter().map(|row| row[2].clone()).collect::<Vec<_>>();
        assert_eq!(
            [summaries("newest-text", &before_the_final_result), summaries("newest-result", &[INIT, shell, REPLY, result]), summaries("held-result", &held_first_result)],
            [vec![json!("FINAL-REPORT\nbgdone\n\n[exited with code 0]"); 4], vec![json!("PR opened and CI is green"); 4], vec![json!("FINAL-REPORT\nDONE-SUB"); 4]],
            "a text written after the last turn end is newer than the summary that turn left, a later result is newer than both, and the first turn's result held past the second turn's text is not"
        );
    }

    #[test]
    fn a_quiet_task_whose_last_turn_failed_ends_failed() {
        let delegates = Delegates::default();
        let task = new_on(&delegates, Provider::Claude);
        let shell = r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_type":"local_bash","description":"npm run dev"}]}"#;
        let failed = r#"{"type":"result","subtype":"success","is_error":true,"result":"API Error: 529 overloaded"}"#;
        for line in [INIT, shell, failed] {
            delegates.apply(&task.id, ChildEvent::Line(line.to_owned()), None);
        }
        later(&delegates, Duration::from_secs(31 * 60));
        let status = delegates.status(&parent(), &task.id).unwrap();
        assert_eq!((status["workState"].as_str(), status["error"].as_str()), (Some("failed"), Some("API Error: 529 overloaded")), "{status}");
    }

    #[test]
    fn a_reload_keeps_how_a_task_ended_and_its_limits() {
        let dir = scratch("reload-ended");
        let file = dir.join("delegates.json");
        let delegates = Delegates::load(Some(file.clone()));
        let task = new_on(&delegates, Provider::Claude);
        delegates.set_limits(&task.id, 3_000_000, 3_600_000);
        for line in fixture("claude-background-shell.jsonl").lines().take(9) {
            delegates.apply(&task.id, ChildEvent::Line(line.to_owned()), None);
        }
        later(&delegates, Duration::from_secs(31 * 60));
        drop(delegates);
        let status = Delegates::load(Some(file)).status(&parent(), &task.id).unwrap();
        assert_eq!(
            json!([status["workState"], status["endedBy"], status["budgetSecs"], status["limitSecs"]]),
            json!(["result_available", "pinned ended it after 30 min with no activity and stopped the 1 shell it left running", 3000, 3600]),
            "{status}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn start(delegates: &Delegates, provider: Provider, model: Option<&str>, title: &str) -> Task {
        let task = delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: title.into(),
            account: provider.spec().builtin.into(),
            provider,
            model: model.map(str::to_owned),
            cwd: PathBuf::from("/tmp"),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        let captain = Delegator { kind: DelegatorKind::Captain, tab: Some("tab-1".into()), project: Some("github.com/acme/site".into()), project_name: Some("site".into()) };
        delegates.describe(&task.id, captain, Some("high".into()));
        task
    }

    fn line(delegates: &Delegates, id: &str, line: &str) {
        delegates.apply(id, ChildEvent::Line(line.to_owned()), None);
    }

    #[test]
    fn a_finished_failed_limited_and_cancelled_task_each_keep_their_record_across_a_reload() {
        let dir = scratch("records");
        let file = dir.join("delegates.json");
        let delegates = Delegates::load(Some(file.clone()));
        let done = start(&delegates, Provider::Claude, Some("claude-opus-5-5"), "Fix the login");
        line(&delegates, &done.id, INIT);
        line(&delegates, &done.id, REPLY);
        line(&delegates, &done.id, r#"{"type":"result","result":"PR opened","total_cost_usd":0.42,"usage":{"input_tokens":10,"cache_read_input_tokens":1000,"cache_creation_input_tokens":90,"output_tokens":300}}"#);
        let failed = start(&delegates, Provider::Claude, Some("claude-sonnet-5-5"), "Port the parser");
        line(&delegates, &failed.id, r#"{"type":"result","is_error":true,"result":"rate limit","usage":{"input_tokens":500,"output_tokens":20}}"#);
        let limited = start(&delegates, Provider::Cursor, Some("grok-4.7"), "Profile the sync");
        delegates.set_limits(&limited.id, 1_000, 2_000);
        let cancelled = start(&delegates, Provider::Claude, Some("claude-haiku-5-5"), "Sweep the docs");
        later(&delegates, Duration::from_secs(5));
        delegates.cancel(&parent(), &cancelled.id, "the parent asked", None).unwrap();
        drop(delegates);

        let delegates = Delegates::load(Some(file));
        let records = delegates.records();
        let by = |id: &str| records.iter().find(|r| r.id == id).unwrap().clone();
        let summary = |r: &TaskRecord| json!([r.title, r.outcome, r.reason, r.limit, r.model, r.tier, r.effort, r.delegator.label(), r.usage.tokens_in, r.usage.tokens_out, r.usage.cost_usd]);
        assert_eq!(summary(&by(&done.id)), json!(["Fix the login", "done", null, null, "claude-opus-5-5", "high", "high", "Captain site", 1100, 300, 0.42]));
        assert_eq!(summary(&by(&failed.id)), json!(["Port the parser", "failed", "rate limit", null, "claude-sonnet-5-5", "mid", "high", "Captain site", 500, 20, null]));
        let limit = by(&limited.id);
        assert_eq!(json!([limit.outcome, limit.limit, limit.model]), json!(["limit", "time", "grok-4.7"]));
        assert!(limit.reason.as_deref().is_some_and(|r| r.starts_with("pinned stopped it after")), "{:?}", limit.reason);
        assert_eq!(summary(&by(&cancelled.id)), json!(["Sweep the docs", "cancelled", "the parent asked", null, "claude-haiku-5-5", "low", "high", "Captain site", 0, 0, null]));
        for record in &records {
            assert_eq!(record.usage.wall_ms, record.ended - record.started, "{}", record.id);
            assert_eq!(record.target, RunTarget::Local);
        }
        let status = delegates.status(&parent(), &limited.id).unwrap();
        assert_eq!(status["stoppedByLimit"], "time", "{status}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_delegator_marks_a_result_accepted_and_the_same_title_started_again_marks_the_last_one_redone() {
        let delegates = Delegates::default();
        let first = start(&delegates, Provider::Codex, None, "Fix the login");
        let working = (parent(), first.id.clone());
        assert_eq!(delegates.review(&working.0, &working.1, Verdict::Accepted, None).unwrap_err(), format!("{} is still working; review it once it ends", first.id));
        line(&delegates, &first.id, r#"{"type":"turn.failed","error":{"message":"tests fail"}}"#);
        let other = Caller { tab: Some("tab-2".into()), task: None };
        assert_eq!(delegates.review(&other, &first.id, Verdict::Accepted, None).unwrap_err(), format!("task {} belongs to another session", first.id));

        let second = start(&delegates, Provider::Codex, None, " fix the LOGIN ");
        let review = delegates.get(&first.id).unwrap().review.unwrap();
        assert_eq!((review.verdict, review.auto, review.note), (Verdict::Redone, true, Some(format!("started again as {}", second.id))));
        line(&delegates, &second.id, r#"{"type":"turn.completed"}"#);
        delegates.review(&parent(), &second.id, Verdict::Accepted, Some("merged by hand".into())).unwrap();
        let third = start(&delegates, Provider::Codex, None, "Fix the login");
        let kept = delegates.get(&second.id).unwrap().review.unwrap();
        assert_eq!((kept.verdict, kept.auto, kept.note.as_deref()), (Verdict::Accepted, false, Some("merged by hand")), "a verdict the delegator gave stays");
        assert!(!delegates.accept_auto(&second.id, "its PR merged".into()));
        assert!(!delegates.get(&second.id).unwrap().review.unwrap().auto, "a signal does not overwrite the delegator");
        assert!(!delegates.accept_auto(&third.id, "its PR merged".into()), "a task still working gets no verdict");

        let sent = delegates.unrecorded(10);
        assert_eq!(sent.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [first.id.as_str(), second.id.as_str()], "only finished tasks have a record");
        delegates.mark_recorded(&sent);
        assert!(delegates.unrecorded(10).is_empty());
        line(&delegates, &third.id, r#"{"type":"turn.completed"}"#);
        delegates.mark_recorded(&delegates.unrecorded(10));
        assert!(delegates.accept_auto(&third.id, "its PR #7 merged".into()));
        assert_eq!(delegates.unrecorded(10).iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [third.id.as_str()], "a merge is sent once");
        delegates.mark_recorded(&delegates.unrecorded(10));
        assert!(!delegates.accept_auto(&third.id, "its PR #7 merged".into()), "the next poll of the merged PR changes nothing");
        assert!(delegates.unrecorded(10).is_empty(), "and sends nothing");
        delegates.review(&parent(), &first.id, Verdict::Accepted, None).unwrap();
        assert_eq!(delegates.unrecorded(10).iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [first.id.as_str()], "a new verdict is sent again");
    }

    #[test]
    fn a_saved_file_from_before_the_limits_loads_with_the_defaults() {
        let dir = scratch("old-file");
        let file = dir.join("delegates.json");
        let old = r#"{"tasks":[{"id":"task-0ld0","parent_kind":"tab","parent_id":"tab-1","title":"Old","account":"claude","provider":"claude","cwd":"/tmp","access":"auto","started":1000,"ended":2000,"state":"result_available","summary":"done","mode":"async","caller_key":"tab:tab-1"}]}"#;
        std::fs::write(&file, old).unwrap();
        let status = Delegates::load(Some(file)).status(&parent(), "task-0ld0").unwrap();
        assert_eq!(
            json!([status["workState"], status["summary"], status["endedBy"], status["budgetSecs"], status["limitSecs"]]),
            json!(["result_available", "done", null, 7200, 28800]),
            "{status}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn alive(pid: i32) -> bool {
        let out = crate::process::command("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
        let stat = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        !stat.is_empty() && !stat.starts_with('Z')
    }

    #[cfg(unix)]
    const LATE_FIRST_LINE: Duration = Duration::from_millis(1500);

    #[cfg(unix)]
    fn pending_shell_worker(dir: &Path, leftover: &Leftover) -> PathBuf {
        let recording = fixture("claude-background-shell.jsonl");
        let split = recording.find(r#"{"type":"system","subtype":"task_updated""#).unwrap();
        std::fs::write(dir.join("first.jsonl"), &recording[..split]).unwrap();
        std::fs::write(dir.join("rest.jsonl"), &recording[split..]).unwrap();
        let body = format!(
            "perl -e 'use POSIX (); POSIX::setsid(); exec @ARGV' sh -c 'echo $$ > \"$0\"; while :; do sleep 1; done' '{}' &\nwhile [ ! -s '{}' ]; do sleep 0.05; done\nsleep {}\ncat first.jsonl\nwhile [ ! -e go ]; do sleep 0.05; done\ncat rest.jsonl\nsleep 30",
            leftover.0.display(),
            leftover.0.display(),
            LATE_FIRST_LINE.as_secs_f64(),
        );
        script(dir, "claude", &body)
    }

    #[cfg(unix)]
    fn wait_for_summary(delegates: &Delegates, id: &str, summary: &str) {
        let start = Instant::now();
        while delegates.status(&parent(), id).unwrap()["summary"] != summary {
            assert!(start.elapsed() < Duration::from_secs(5), "{}", delegates.status(&parent(), id).unwrap());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn later(delegates: &Delegates, by: Duration) {
        delegates.enforce_limits(Instant::now() + by, None);
    }

    #[test]
    #[cfg(unix)]
    fn a_reply_with_a_background_shell_pending_keeps_the_shell_and_tells_the_parent() {
        let dir = scratch("pending-shell");
        let leftover = Leftover(dir.join("shell.pid"));
        let bin = pending_shell_worker(&dir, &leftover);
        let delegates = Arc::new(Delegates::default());
        let task = new_on(&delegates, Provider::Claude);
        let (agent, told) = run(&delegates, &task, &bin, &dir);
        let shell = leftover.pid();
        wait_for_summary(&delegates, &task.id, INTERIM);
        std::thread::sleep(Duration::from_millis(300));
        let status = delegates.status(&parent(), &task.id).unwrap();
        assert_eq!(status["workState"], "working", "{status}");
        assert_eq!(status["background"], json!(["Run sleep 20 then echo bgdone in background"]), "{status}");
        assert!(status["next"].as_str().unwrap_or("").contains("still runs the work listed in background"), "{status}");
        assert!(alive(shell), "pinned stopped the pending background shell {shell}");
        assert!(alive(agent as i32), "pinned stopped the agent while its shell was pending");
        assert_eq!(delegates.claim_notice("tab-1").map(|(text, _)| text), Some(pending_line(&task.id)));

        std::fs::write(dir.join("go"), "").unwrap();
        assert_eq!(told.recv_timeout(Duration::from_secs(5)).as_deref(), Ok("tab-1"));
        let status = delegates.status(&parent(), &task.id).unwrap();
        assert_eq!((status["workState"].as_str(), status["summary"].as_str()), (Some("result_available"), Some("FINAL-REPORT\nbgdone\n\n[exited with code 0]")), "{status}");
        assert!(stopped(shell), "the shell {shell} outlived the final answer");
        assert!(stopped(agent as i32), "the agent {agent} outlived the final answer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_reply_quiet_for_30_min_with_background_work_ends_with_that_reply() {
        let dir = scratch("quiet");
        let leftover = Leftover(dir.join("shell.pid"));
        let bin = pending_shell_worker(&dir, &leftover);
        let delegates = Arc::new(Delegates::default());
        let task = new_on(&delegates, Provider::Claude);
        let (agent, told) = run(&delegates, &task, &bin, &dir);
        let shell = leftover.pid();
        wait_for_summary(&delegates, &task.id, INTERIM);

        later(&delegates, Duration::from_millis(QUIET_AFTER_REPLY_MS) - LATE_FIRST_LINE / 2);
        assert_eq!(delegates.status(&parent(), &task.id).unwrap()["workState"], "working", "the quiet clock started before the last line");
        assert!(alive(shell), "pinned stopped the shell before the task was quiet for 30 min");
        later(&delegates, Duration::from_millis(QUIET_AFTER_REPLY_MS));
        assert_eq!(told.recv_timeout(Duration::from_secs(5)).as_deref(), Ok("tab-1"));
        let ended = "pinned ended it after 30 min with no activity and stopped the 1 shell it left running";
        assert_eq!(
            delegates.claim_notice("tab-1").map(|(text, _)| text),
            Some(format!("[pinned] Delegated task {id} \"Fix\" (Claude Code) finished: result available. {ended}. Call task_status with taskId \"{id}\".", id = task.id))
        );
        let status = delegates.status(&parent(), &task.id).unwrap();
        assert_eq!(
            json!([status["workState"], status["summary"], status["endedBy"], status["background"]]),
            json!(["result_available", INTERIM, ended, ["Run sleep 20 then echo bgdone in background"]]),
            "{status}"
        );
        assert!(stopped(shell), "the shell {shell} outlived the quiet task");
        assert!(stopped(agent as i32), "the agent {agent} outlived the quiet task");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_runaway_worker_is_reported_past_its_budget_then_stopped_at_its_hard_limit() {
        let dir = scratch("runaway");
        let leftover = Leftover(dir.join("loop.pid"));
        let bin = script(&dir, "cursor-agent", &detached_loop_worker(&[INIT, REPLY], &leftover.0));
        let delegates = Arc::new(Delegates::default());
        let task = new_on(&delegates, Provider::Cursor);
        delegates.set_limits(&task.id, 60_000, 240_000);
        let (agent, _) = run(&delegates, &task, &bin, &dir);
        let background = leftover.pid();
        let start = Instant::now();
        while !delegates.status(&parent(), &task.id).unwrap()["recentSteps"].to_string().contains("PR opened") {
            assert!(start.elapsed() < Duration::from_secs(5), "the reply never arrived");
            std::thread::sleep(Duration::from_millis(20));
        }
        let notice = |delegates: &Delegates| delegates.claim_notice("tab-1").map(|(text, _)| text);

        later(&delegates, Duration::from_secs(30));
        assert_eq!(notice(&delegates), None, "a task inside its budget was reported");
        later(&delegates, Duration::from_secs(60));
        assert_eq!(
            notice(&delegates),
            Some(format!("[pinned] Delegated task {id} \"Fix\" (Cursor) is still working after 1 min, past its budget of 1 min. pinned cancels it at its limit of 4 min. Call task_status with taskId \"{id}\" to read its recent steps, or task_cancel to stop it.", id = task.id))
        );
        let status = delegates.status(&parent(), &task.id).unwrap();
        assert_eq!(status["workState"], "working");
        assert!(status["next"].as_str().unwrap_or("").contains("past its budget of 1 min"), "{status}");
        later(&delegates, Duration::from_secs(120));
        assert_eq!(notice(&delegates), None, "the budget notice was sent twice");

        later(&delegates, Duration::from_secs(240));
        let finished = notice(&delegates).unwrap_or_default();
        assert!(finished.contains("finished: cancelled"), "{finished}");
        let status = delegates.status(&parent(), &task.id).unwrap();
        assert_eq!(status["workState"], "cancelled", "{status}");
        assert!(status["error"].as_str().unwrap_or("").contains("after 4 min, its limit"), "{status}");
        assert!(stopped(background), "the background loop {background} kept running");
        assert!(stopped(agent as i32), "the agent {agent} kept running");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_late_pid_and_quit_also_stop_background_shells() {
        use std::os::unix::process::CommandExt;
        let dir = scratch("late");
        let delegates = Delegates::default();
        let mut failures = vec![];
        for late in [true, false] {
            let leftover = Leftover(dir.join(format!("{late}.pid")));
            let bin = script(&dir, &format!("agent-{late}"), &detached_loop_worker(&[INIT], &leftover.0));
            let task = new_on(&delegates, Provider::Claude);
            let mut cmd = crate::process::command(&bin);
            cmd.process_group(0).stdout(Stdio::null());
            let mut agent = cmd.spawn().unwrap();
            let background = leftover.pid();
            if late {
                delegates.cancel(&parent(), &task.id, "stop", None).unwrap();
                delegates.remember_pid(&task.id, agent.id());
            } else {
                delegates.remember_pid(&task.id, agent.id());
                delegates.kill_remembered();
            }
            if !stopped(background) {
                failures.push(format!("late {late}: the background loop {background} kept running"));
            }
            let _ = agent.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    #[cfg(unix)]
    fn an_agent_that_exits_leaves_nothing_in_its_process_group() {
        let dir = scratch("exited");
        let leftover = Leftover(dir.join("server.pid"));
        let body = format!("echo '{INIT}'\necho '{REPLY}'\nsleep 30 >/dev/null 2>&1 &\necho $! > '{}'\nexit 0", leftover.0.display());
        let bin = script(&dir, "cursor-agent", &body);
        let delegates = Arc::new(Delegates::default());
        let task = new_on(&delegates, Provider::Cursor);
        let (_, told) = run(&delegates, &task, &bin, &dir);
        let server = leftover.pid();
        assert_eq!(told.recv_timeout(Duration::from_secs(5)).as_deref(), Ok("tab-1"));
        assert_eq!(delegates.status(&parent(), &task.id).unwrap()["summary"], "PR opened");
        assert!(stopped(server), "the agent exited and left {server} running in its process group");
        assert_eq!(delegates.remembered(&task.id), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_worker_that_exits_by_itself_stops_the_shells_it_started() {
        let dir = scratch("self-exit");
        let leftover = Leftover(dir.join("server.pid"));
        let server = r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_type":"local_bash","description":"npm run dev"}]}"#;
        let body = format!(
            "perl -e 'use POSIX (); POSIX::setsid(); exec @ARGV' sh -c 'echo $$ > \"$0\"; while :; do sleep 1; done' '{pidfile}' >/dev/null 2>&1 &\nwhile [ ! -s '{pidfile}' ]; do sleep 0.05; done\necho '{INIT}'\necho '{server}'\necho '{REPLY}'\nsleep 1\nexit 3",
            pidfile = leftover.0.display(),
        );
        let bin = script(&dir, "claude", &body);
        let delegates = Arc::new(Delegates::default());
        let task = new_on(&delegates, Provider::Claude);
        let (_, told) = run(&delegates, &task, &bin, &dir);
        let shell = leftover.pid();
        assert_eq!(told.recv_timeout(Duration::from_secs(5)).as_deref(), Ok("tab-1"));
        assert!(stopped(shell), "the worker exited and left its shell {shell} running");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn an_agent_whose_stderr_outlives_it_finishes_when_it_exits() {
        let dir = scratch("stderr-held");
        let mut seen = vec![];
        for (name, end) in [("reply", format!("echo '{REPLY}'\nexit 0")), ("crash", "echo boom >&2\nexit 1".to_owned())] {
            let leftover = Leftover(dir.join(format!("{name}.pid")));
            let body = format!(
                "perl -e 'use POSIX (); POSIX::setsid(); exec @ARGV' sh -c 'echo $$ > \"$0\"; exec sleep 30' '{pidfile}' >/dev/null &\nwhile [ ! -s '{pidfile}' ]; do sleep 0.05; done\necho '{INIT}'\n{end}",
                pidfile = leftover.0.display(),
            );
            let bin = script(&dir, &format!("agent-{name}"), &body);
            let delegates = Arc::new(Delegates::default());
            let task = new_on(&delegates, Provider::Cursor);
            let (_, told) = run(&delegates, &task, &bin, &dir);
            let holder = leftover.pid();
            let finished = told.recv_timeout(Duration::from_secs(5)).is_ok();
            let status = delegates.status(&parent(), &task.id).unwrap();
            seen.push(json!([name, finished, alive(holder), delegates.remembered(&task.id), status["workState"], status["summary"], status["error"]]));
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            seen,
            vec![
                json!(["reply", true, true, null, "result_available", "PR opened", null]),
                json!(["crash", true, true, null, "failed", null, "boom"]),
            ],
            "[case, finished, the stderr holder still runs, remembered pid, workState, summary, error]"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_cancel_stops_the_agent_before_it_removes_the_worktree() {
        let dir = scratch("kill-then-drop");
        let (tree, tools) = (dir.join("tree"), dir.join("tools"));
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::create_dir_all(&tools).unwrap();
        let (pidfile, witness) = (dir.join("agent.pid"), dir.join("witness"));
        script(
            &tools,
            "git",
            &format!(
                "case \"$(ps -o stat= -p \"$(cat '{}')\")\" in ''|Z*) agent=stopped ;; *) agent=running ;; esac\necho \"$1 $agent\" >> '{}'",
                pidfile.display(),
                witness.display()
            ),
        );
        let bin = script(&dir, "claude", "exec sleep 30");
        let delegates = Arc::new(Delegates::default());
        let task = new_on(&delegates, Provider::Claude);
        delegates.place(&task.id, tree.clone(), true);
        let (agent, _) = run(&delegates, &task, &bin, &dir);
        std::fs::write(&pidfile, agent.to_string()).unwrap();
        let path = format!("{}:/bin:/usr/bin", tools.display());
        delegates.cancel(&parent(), &task.id, "the parent asked", Some(&path)).unwrap();
        let seen = std::fs::read_to_string(&witness).unwrap_or_default();
        assert!(stopped(agent as i32), "the agent {agent} outlived the cancel");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(seen, "symbolic-ref stopped\nworktree stopped\n", "the state of the agent at each git call that removes its worktree");
    }

    #[cfg(unix)]
    const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"PR opened"}"#;

    #[test]
    #[cfg(unix)]
    fn an_agent_that_exits_within_the_grace_finishes_its_own_work() {
        let dir = scratch("grace-exit");
        let flushed = dir.join("flushed");
        let body = format!(
            "echo '{INIT}'\necho '{REPLY}'\necho '{RESULT}'\ni=0\nwhile [ $i -lt 100000 ]; do i=$((i+1)); done\necho flushed > '{}'",
            flushed.display()
        );
        let bin = script(&dir, "claude", &body);
        let delegates = Arc::new(Delegates::default());
        let task = new_on(&delegates, Provider::Claude);
        let (agent, told) = run(&delegates, &task, &bin, &dir);
        assert_eq!(told.recv_timeout(Duration::from_secs(5)).as_deref(), Ok("tab-1"));
        assert!(stopped(agent as i32), "the agent {agent} did not exit by itself");
        assert_eq!(std::fs::read_to_string(&flushed).ok().as_deref(), Some("flushed\n"), "pinned killed the agent before it finished after its result");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn an_agent_still_running_after_the_grace_is_killed() {
        let dir = scratch("grace-kill");
        let bin = script(&dir, "claude", &format!("echo '{INIT}'\necho '{REPLY}'\necho '{RESULT}'\nexec sleep 30"));
        let delegates = Arc::new(Delegates::default());
        let task = new_on(&delegates, Provider::Claude);
        let (agent, told) = run(&delegates, &task, &bin, &dir);
        assert_eq!(told.recv_timeout(Duration::from_secs(5)).as_deref(), Ok("tab-1"));
        assert!(alive(agent as i32), "pinned killed the agent {agent} on its result, with no grace");
        let start = Instant::now();
        while alive(agent as i32) {
            assert!(start.elapsed() < Duration::from_secs(9), "the agent {agent} outlived the grace");
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_child_process_returns_the_fixtures_text_and_cancel_kills_it() {
        let dir = std::env::temp_dir().join(format!("pinned-delegate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = fixture("cursor.jsonl");
        let bin = script(
            &dir,
            "cursor-agent",
            &format!("cat <<'EOF'\n{fixture}\nEOF"),
        );
        let delegates = Delegates::default();
        let task = new_cursor(&delegates, Mode::Wait);
        let id = task.id.clone();
        let shared = Arc::new(delegates);
        let sink = shared.clone();
        spawn(
            ChildSpec {
                bin: &bin,
                argv: &["-p".into(), "hi".into()],
                cwd: &dir,
                env: &[],
                path: None,
                strip: &[],
            },
            move |event| {
                sink.apply(&id, event, None);
            },
        )
        .unwrap();
        let start = Instant::now();
        let status = loop {
            let status = shared.status(&Caller { tab: Some("tab-1".into()), task: None }, &task.id).unwrap();
            if status["workState"] != "working" {
                break status;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the child did not finish: {status}"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(status["summary"], "DONE");
        assert_eq!(status["workState"], "result_available");
        assert_eq!(
            status["conversationId"],
            "9fa2e973-3c1c-4a8e-9ab7-9790584c944c"
        );
        let kept = shared.cancel(&Caller { tab: Some("tab-1".into()), task: None }, &task.id, "too late", None).unwrap();
        assert_eq!(kept["summary"], "DONE");
        assert_eq!(kept["workState"], "result_available");

        let sleeper = script(
            &dir,
            "sleep-agent",
            "echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\"}'\nsleep 30",
        );
        let running = new_cursor(shared.as_ref(), Mode::Async);
        let run_id = running.id.clone();
        let sink = shared.clone();
        let pid = spawn(
            ChildSpec {
                bin: &sleeper,
                argv: &[],
                cwd: &dir,
                env: &[],
                path: None,
                strip: &[],
            },
            move |event| {
                sink.apply(&run_id, event, None);
            },
        )
        .unwrap();
        shared.remember_pid(&running.id, pid);
        let cancelled = shared.cancel(&Caller { tab: Some("tab-1".into()), task: None }, &running.id, "the parent asked", None).unwrap();
        assert_eq!(cancelled["workState"], "cancelled");
        let start = Instant::now();
        while unsafe { libc::kill(pid as i32, 0) } == 0 {
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "cancel left pid {pid} running"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = pid;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn cancel_between_claim_and_remember_pid_kills_the_process() {
        use std::os::unix::process::CommandExt;
        let dir = std::env::temp_dir().join(format!("pinned-delegate-gap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = script(&dir, "sleeper", "sleep 30");
        let delegates = Delegates::default();
        let task = new_cursor(&delegates, Mode::Async);
        let mut cmd = std::process::Command::new(&bin);
        cmd.process_group(0);
        let mut child = cmd.spawn().unwrap();
        let pid = child.id();
        delegates.cancel(&Caller { tab: Some("tab-1".into()), task: None }, &task.id, "stop", None).unwrap();
        delegates.remember_pid(&task.id, pid);
        let start = Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(2), "cancel left pid {pid} running");
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_finished_childs_pid_is_forgotten_and_quit_does_not_kill_it() {
        use std::os::unix::process::CommandExt;
        let dir = std::env::temp_dir().join(format!("pinned-delegate-pid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = script(&dir, "sleeper", "sleep 30");
        let spawn_sleeper = || {
            let mut cmd = crate::process::command(&bin);
            cmd.process_group(0);
            let child = cmd.spawn().unwrap();
            let pid = child.id();
            (child, pid)
        };
        let delegates = Delegates::default();

        let failed = new_cursor(&delegates, Mode::Async);
        let (mut failed_child, failed_pid) = spawn_sleeper();
        delegates.remember_pid(&failed.id, failed_pid);
        delegates.fail(&failed.id, "boom".into(), None);
        assert_eq!(delegates.remembered(&failed.id), None, "a failed child kept its pid");
        delegates.kill_remembered();
        assert!(failed_child.try_wait().unwrap().is_none(), "quit killed pid {failed_pid} after the child had failed");

        let late = new_cursor(&delegates, Mode::Async);
        delegates.fail(&late.id, "already".into(), None);
        let (mut late_child, late_pid) = spawn_sleeper();
        delegates.remember_pid(&late.id, late_pid);
        let start = Instant::now();
        while late_child.try_wait().unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(2), "remember_pid left a finished task's pid {late_pid} running");
            std::thread::sleep(Duration::from_millis(20));
        }

        let _ = failed_child.kill();
        let _ = failed_child.wait();
        let _ = late_child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn an_isolated_task_drops_its_worktree_after_the_task_lock() {
        let root = std::env::temp_dir().join(format!("pinned-wt-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let status = crate::process::command("git").args(args).current_dir(dir).status().unwrap();
            assert!(status.success(), "git {args:?} in {} failed", dir.display());
        };
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let dir = crate::branches::new_worktree(&root, None).unwrap();
        let real = String::from_utf8(crate::process::command("/bin/sh").arg("-c").arg("command -v git").output().unwrap().stdout).unwrap();
        let real = real.trim();
        assert!(!real.is_empty(), "git is not on PATH");
        let marker = root.join("removing");
        let go = root.join("go");
        let body = format!(
            r#"if [ "$1" = worktree ] && [ "$2" = remove ]; then
  /usr/bin/touch '{marker}'
  while [ ! -f '{go}' ]; do
    /bin/sleep 0.05
  done
fi
export PATH="/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin"
exec '{real}' "$@"
"#,
            marker = marker.display(),
            go = go.display(),
        );
        let bin = root.join("bin");
        std::fs::create_dir(&bin).unwrap();
        script(&bin, "git", &body);
        struct Release(PathBuf);
        impl Drop for Release {
            fn drop(&mut self) {
                let _ = std::fs::write(&self.0, "");
            }
        }
        let _release = Release(go.clone());
        let delegates = Arc::new(Delegates::default());
        let task = delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: "Look".into(),
            account: "claude".into(),
            provider: Provider::Claude,
            model: None,
            cwd: dir.clone(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: true,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        let applying = delegates.clone();
        let id = task.id.clone();
        let path = bin.display().to_string();
        let handle = std::thread::spawn(move || {
            applying.apply(&id, ChildEvent::Exited { code: Some(1), stderr: String::new() }, Some(&path));
        });
        let started = Instant::now();
        while !marker.exists() {
            assert!(started.elapsed() < Duration::from_secs(5), "worktree remove never started");
            std::thread::sleep(Duration::from_millis(20));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let status_of = delegates.clone();
        let status_id = task.id.clone();
        std::thread::spawn(move || {
            let value = status_of.status(&Caller { tab: Some("tab-1".into()), task: None }, &status_id).unwrap();
            let _ = tx.send(value["workState"].as_str().unwrap_or("").to_owned());
        });
        let state = rx.recv_timeout(Duration::from_secs(1)).expect("status blocked behind the worktree remove");
        assert_eq!(state, "failed");
        std::fs::write(&go, "").unwrap();
        handle.join().unwrap();
        assert!(!dir.exists(), "the isolated worktree was kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_new_registry_does_not_reuse_task_ids() {
        let first = new_cursor(&Delegates::default(), Mode::Async).id;
        let second = new_cursor(&Delegates::default(), Mode::Async).id;
        assert_ne!(first, second);
        assert_eq!(first.len(), 13);
        assert_eq!(&first[..5], "task-");
        assert!(first[5..].bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(&second[..5], "task-");
        assert_ne!(first, "task-1");
        assert_ne!(second, "task-1");
    }

    #[test]
    fn the_same_caller_and_client_request_id_returns_the_existing_task() {
        let delegates = Delegates::default();
        let spec = |key: &str, tab: &str, request: &str| NewTask {
            parent: Parent::Tab(tab.into()),
            tool: None,
            title: "Ping".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: PathBuf::from("/tmp"),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: key.into(),
            client_request_id: Some(request.into()),
            role: None,
        };
        let first = delegates.begin(spec("tab:tab-1", "tab-1", "req-7")).id;
        delegates.apply(&first, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        let again = delegates.begin(spec("tab:tab-1", "tab-1", "req-7")).id;
        assert_eq!(again, first);
        let replay = delegates.started_json(&again).unwrap();
        assert_eq!(replay["summary"], "DONE");
        assert_eq!(replay["error"], Value::Null);
        assert_eq!(replay["workState"], "result_available");
        assert_eq!(replay["next"], "the result is in summary");
        let failed = new_cursor(&delegates, Mode::Async);
        delegates.apply(&failed.id, ChildEvent::Exited { code: Some(1), stderr: "boom".into() }, None);
        let failed = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &failed.id).unwrap();
        assert_eq!(failed["next"], "the error is in error");
        assert_eq!(failed["error"], "boom");
        assert_eq!(merge_cursor_list(Some("composer-2.5 - Composer\n"), ""), "composer-2.5 - Composer\n");
        assert_eq!(merge_cursor_list(Some("composer-2.5 - Composer\n"), "   "), "composer-2.5 - Composer\n");
        assert_eq!(merge_cursor_list(None, ""), "");
        assert_eq!(merge_cursor_list(Some("old\n"), "new\n"), "new\n");
        let fresh_id = delegates.begin(spec("tab:tab-1", "tab-1", "req-8")).id;
        assert_ne!(fresh_id, first);
        let fresh = delegates.started_json(&fresh_id).unwrap();
        assert_eq!(fresh["workState"], "working");
        let next = format!("end your turn and do not poll; pinned delivers the notice when the turn ends or when this session is idle, or the user sends it from the task's card; then call task_status with this taskId \"{fresh_id}\"");
        assert_eq!(fresh["next"], next);
        let status = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &fresh_id).unwrap();
        assert_eq!(status["workState"], "working");
        assert_eq!(status["next"], next);
        let other = delegates.begin(spec("tab:tab-2", "tab-2", "req-7")).id;
        assert_ne!(other, first);
    }

    #[test]
    fn open_is_refused_while_the_task_is_working_and_without_a_conversation() {
        let delegates = Delegates::default();
        let task = new_cursor(&delegates, Mode::Async);
        let caller = Caller { tab: Some("tab-1".into()), task: None };
        delegates.apply(&task.id, ChildEvent::Line(json!({"type":"system","subtype":"init","session_id":"conv-1"}).to_string()), None);
        assert_eq!(delegates.open(&caller, &task.id).unwrap_err(), format!("{} is still working", task.id));
        delegates.apply(&task.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        let opened = delegates.open(&caller, &task.id).unwrap();
        assert_eq!(opened.conversation, "conv-1");
        let bare = new_cursor(&delegates, Mode::Async);
        delegates.apply(&bare.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        assert_eq!(delegates.open(&caller, &bare.id).unwrap_err(), format!("{} has no conversation to resume", bare.id));
    }

    #[test]
    fn a_card_carries_the_start_the_error_and_the_conversation() {
        let delegates = Delegates::default();
        let failed = new_cursor(&delegates, Mode::Async);
        delegates.fail(&failed.id, "boom".into(), None);
        let done = new_cursor(&delegates, Mode::Async);
        delegates.apply(&done.id, ChildEvent::Line(json!({"type":"system","subtype":"init","session_id":"conv-9"}).to_string()), None);
        delegates.apply(&done.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        let cards = delegates.cards("tab-1");
        let failed_card = cards.iter().find(|card| card.id == failed.id).unwrap();
        assert_eq!(failed_card.state, crate::subagents::State::Failed);
        assert_eq!(failed_card.error.as_deref(), Some("boom"));
        assert!(failed_card.started.as_deref().unwrap().contains('T'), "{}", failed_card.started.as_deref().unwrap());
        let done_card = cards.iter().find(|card| card.id == done.id).unwrap();
        assert_eq!(done_card.state, crate::subagents::State::Done);
        assert_eq!(done_card.conversation.as_deref(), Some("conv-9"));
        let cancelled = new_cursor(&delegates, Mode::Async);
        delegates.cancel(&Caller { tab: Some("tab-1".into()), task: None }, &cancelled.id, "stop", None).unwrap();
        let cancelled_card = delegates.cards("tab-1").into_iter().find(|card| card.id == cancelled.id).unwrap();
        assert_eq!(cancelled_card.state, crate::subagents::State::Cancelled);
    }

    #[test]
    fn a_grandchild_notice_is_refused_from_the_root_tab() {
        let delegates = Delegates::default();
        let parent = new_cursor(&delegates, Mode::Async);
        let child = delegates.begin(NewTask {
            parent: Parent::Task(parent.id.clone()),
            tool: None,
            title: "Grandchild".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: Some("composer-2.5".into()),
            cwd: PathBuf::from("/tmp"),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: format!("task:{}", parent.id),
            client_request_id: None,
            role: None,
        });
        let headless = Caller { tab: None, task: Some(parent.id.clone()) };
        let working = delegates.status(&headless, &child.id).unwrap();
        assert_eq!(working["next"], format!("call task_status with taskId \"{}\" when you need it", child.id));
        let from_tab = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &parent.id).unwrap();
        assert!(from_tab["next"].as_str().unwrap().starts_with("end your turn"), "{}", from_tab["next"]);
        delegates.apply(&child.id, ChildEvent::Line(json!({"type":"result","result":"NESTED"}).to_string()), None);
        let root = Caller { tab: Some("tab-1".into()), task: None };
        assert_eq!(
            delegates.claim_task_notice(&root, &child.id).unwrap_err(),
            format!("task {} belongs to another session", child.id)
        );
        let status = delegates.status(&Caller { tab: None, task: Some(parent.id.clone()) }, &child.id).unwrap();
        assert_eq!(status["summary"], "NESTED");
        assert_eq!(status["workState"], "result_available");
        let direct = delegates.claim_task_notice(&root, &parent.id);
        assert!(direct.is_err(), "an unfinished parent was sent: {direct:?}");
        delegates.apply(&parent.id, ChildEvent::Line(json!({"type":"result","result":"PARENT"}).to_string()), None);
        assert_eq!(
            delegates.claim_task_notice(&root, &parent.id).unwrap(),
            format!(
                "[pinned] Delegated task {0} \"Ping\" (Cursor · composer-2.5) finished: result available. Call task_status with taskId \"{0}\".",
                parent.id
            )
        );
        assert_eq!(delegates.status(&root, &parent.id).unwrap()["summary"], "PARENT");
    }

    #[test]
    fn another_tab_cannot_read_or_cancel_a_task_or_consume_its_notice() {
        let delegates = Delegates::default();
        let task = new_cursor(&delegates, Mode::Wait);
        delegates.apply(&task.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        let (text, ids) = delegates.claim_notice("tab-1").unwrap();
        assert_eq!(ids, vec![task.id.clone()]);
        assert_eq!(
            text,
            format!("[pinned] Delegated task {} \"Ping\" (Cursor · composer-2.5) finished: result available. Call task_status with taskId \"{}\".", task.id, task.id)
        );
        assert!(delegates.claim_notice("tab-1").is_none(), "a second claim pasted the same notice");
        delegates.release_notice(&ids);
        assert_eq!(delegates.claim_notice("tab-1").unwrap().1, vec![task.id.clone()]);
        let other = Caller { tab: Some("tab-2".into()), task: None };
        assert_eq!(delegates.status(&other, &task.id).unwrap_err(), format!("task {} belongs to another session", task.id));
        assert_eq!(delegates.cancel(&other, &task.id, "no", None).unwrap_err(), format!("task {} belongs to another session", task.id));
        delegates.release_notice(&ids);
        let status = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &task.id).unwrap();
        assert_eq!(status["summary"], "DONE");
        assert_eq!(status["workState"], "result_available");
        assert!(delegates.claim_notice("tab-1").is_none());
    }

    #[test]
    fn a_full_assistant_message_survives_eight_later_steps() {
        let delegates = Delegates::default();
        let task = delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: "Look".into(),
            account: "codex".into(),
            provider: Provider::Codex,
            model: Some("gpt-5.4".into()),
            cwd: PathBuf::from("/tmp"),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        let answer = "A".repeat(500);
        delegates.apply(
            &task.id,
            ChildEvent::Line(json!({"type":"item.completed","item":{"id":"m","type":"agent_message","text":answer}}).to_string()),
            None,
        );
        for n in 0..8 {
            delegates.apply(
                &task.id,
                ChildEvent::Line(json!({"type":"item.completed","item":{"id":format!("c{n}"),"type":"command_execution","command":"cargo test","aggregated_output":"ok\n","exit_code":0,"status":"completed"}}).to_string()),
                None,
            );
        }
        delegates.apply(&task.id, ChildEvent::Exited { code: Some(0), stderr: String::new() }, None);
        let status = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &task.id).unwrap();
        assert_eq!(status["summary"], answer);
        assert_eq!(status["workState"], "result_available");
        assert_eq!(status["error"], Value::Null);
    }

    #[test]
    fn a_parent_waits_for_a_working_child_then_keeps_its_own_summary() {
        let delegates = Delegates::default();
        let parent = new_cursor(&delegates, Mode::Async);
        let child = delegates.begin(NewTask {
            parent: Parent::Task(parent.id.clone()),
            tool: None,
            title: "Child".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: Some("composer-2.5".into()),
            cwd: PathBuf::from("/tmp"),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: format!("task:{}", parent.id),
            client_request_id: None,
            role: None,
        });
        delegates.apply(&parent.id, ChildEvent::Line(json!({"type":"result","result":"PARENT"}).to_string()), None);
        let waiting = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &parent.id).unwrap();
        assert_eq!(waiting["workState"], "waiting_for_children");
        assert_eq!(waiting["summary"], "PARENT", "the reply so far shows before the task ends");
        delegates.apply(&child.id, ChildEvent::Line(json!({"type":"result","result":"CHILD"}).to_string()), None);
        let done = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &parent.id).unwrap();
        assert_eq!(done["workState"], "result_available");
        assert_eq!(done["summary"], "PARENT");
        let child_status = delegates.status(&Caller { tab: None, task: Some(parent.id.clone()) }, &child.id).unwrap();
        assert_eq!(child_status["summary"], "CHILD");
        assert_eq!(child_status["workState"], "result_available");
    }

    #[test]
    #[cfg(unix)]
    fn cancel_stops_a_working_descendant_and_drops_a_worktree_without_a_conversation() {
        let root = std::env::temp_dir().join(format!("pinned-delegate-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            assert!(crate::process::command("git").arg("-C").arg(&repo).args(args).status().unwrap().success(), "{args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let kept = crate::branches::new_worktree(&repo, None).unwrap();
        let dropped = crate::branches::new_worktree(&repo, None).unwrap();
        let delegates = std::sync::Arc::new(Delegates::default());
        let parent = delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: "Parent".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: kept.clone(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: true,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        delegates.apply(&parent.id, ChildEvent::Line(json!({"type":"system","subtype":"init","session_id":"keep-me"}).to_string()), None);
        let child = delegates.begin(NewTask {
            parent: Parent::Task(parent.id.clone()),
            tool: None,
            title: "Child".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: dropped.clone(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: true,
            caller_key: format!("task:{}", parent.id),
            client_request_id: None,
            role: None,
        });
        let sleeper = script(&root, "sleep-child", "sleep 30");
        let child_id = child.id.clone();
        let sink = delegates.clone();
        let pid = spawn(
            ChildSpec { bin: &sleeper, argv: &[], cwd: &root, env: &[], path: None, strip: &[] },
            move |event| { sink.apply(&child_id, event, None); },
        ).unwrap();
        delegates.remember_pid(&child.id, pid);
        let cancelled = delegates.cancel(&Caller { tab: Some("tab-1".into()), task: None }, &parent.id, "stop the tree", None).unwrap();
        assert_eq!(cancelled["workState"], "cancelled");
        let child_status = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &child.id).unwrap();
        assert_eq!(child_status["workState"], "cancelled");
        let start = Instant::now();
        while unsafe { libc::kill(pid as i32, 0) } == 0 {
            assert!(start.elapsed() < Duration::from_secs(2), "cancel left descendant pid {pid} running");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(kept.is_dir(), "a worktree with a conversation was removed");
        assert!(!dropped.exists(), "a worktree with no conversation survived cancel");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn a_successful_task_drops_its_worktree_when_it_has_no_conversation() {
        let root = std::env::temp_dir().join(format!("pinned-delegate-success-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            assert!(crate::process::command("git").arg("-C").arg(&repo).args(args).status().unwrap().success(), "{args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let dropped = crate::branches::new_worktree(&repo, None).unwrap();
        let kept = crate::branches::new_worktree(&repo, None).unwrap();
        let delegates = Delegates::default();
        let gone = delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: "No conversation".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: dropped.clone(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: true,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        let stay = delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: "Has conversation".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: kept.clone(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: true,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        delegates.apply(&stay.id, ChildEvent::Line(json!({"type":"system","subtype":"init","session_id":"keep-me"}).to_string()), None);
        delegates.apply(&gone.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        delegates.apply(&stay.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        assert!(!dropped.exists(), "a successful worktree with no conversation stayed");
        assert!(kept.is_dir(), "a successful worktree with a conversation was removed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_wider_runtime_mode_is_rejected_and_a_bypassed_parent_can_hand_over_auto() {
        let mut request = parse_request(&json!({"task": "look"})).unwrap();
        request.runtime = Some(RuntimeMode::FullAccess);
        assert_eq!(
            resolve_access(Access::Auto, &request).unwrap_err(),
            "runtimeMode \"full-access\" is wider than this session's \"auto\""
        );
        request.runtime = Some(RuntimeMode::Auto);
        assert_eq!(resolve_access(Access::Full, &request).unwrap(), Access::Auto);
        assert_eq!(resolve_access(Access::ReadOnly, &request).unwrap(), Access::ReadOnly);
        request.runtime = None;
        assert_eq!(resolve_access(Access::ReadOnly, &request).unwrap(), Access::ReadOnly);
        request.runtime = Some(RuntimeMode::FullAccess);
        assert_eq!(
            resolve_access(Access::ReadOnly, &request).unwrap_err(),
            "runtimeMode \"full-access\" is wider than this session's \"read-only\""
        );
        request.runtime = None;
        request.interaction = Interaction::Plan;
        assert_eq!(resolve_access(Access::Full, &request).unwrap(), Access::ReadOnly);
    }

    #[test]
    #[cfg(unix)]
    fn the_cursor_catalog_waits_for_the_first_list_and_then_returns_the_cache() {
        let dir = std::env::temp_dir().join(format!("pinned-delegate-catalog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = script(&dir, "cursor-agent", "sleep 0.2; printf 'composer-2.5 - Composer\\n'");
        let delegates = Delegates::default();
        let start = Instant::now();
        assert_eq!(delegates.cursor_catalog(&bin, None), "composer-2.5 - Composer\n");
        assert!(start.elapsed() >= Duration::from_millis(150), "the first catalog read returned before the CLI printed");
        assert!(start.elapsed() < Duration::from_secs(2), "the first catalog read waited past the CLI");
        std::fs::write(&bin, "#!/bin/sh\nsleep 30\n").unwrap();
        let start = Instant::now();
        assert_eq!(delegates.cursor_catalog(&bin, None), "composer-2.5 - Composer\n");
        assert!(start.elapsed() < Duration::from_millis(200), "a warm catalog waited on the CLI again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_cold_cursor_catalog_gives_up_after_two_seconds() {
        let dir = std::env::temp_dir().join(format!("pinned-delegate-catalog-slow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = script(&dir, "cursor-agent", "sleep 30");
        let delegates = Delegates::default();
        let start = Instant::now();
        assert_eq!(delegates.cursor_catalog(&bin, None), "");
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(1500) && elapsed < Duration::from_secs(4), "cold catalog elapsed {elapsed:?}");
        let start = Instant::now();
        assert_eq!(delegates.cursor_catalog(&bin, None), "");
        assert!(start.elapsed() < Duration::from_millis(200), "the empty catalog was fetched again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_cursor_binary_is_remembered_without_waiting() {
        let delegates = Delegates::default();
        let missing = Path::new("/no/such/cursor-agent-pinned");
        let start = Instant::now();
        assert_eq!(delegates.cursor_catalog(missing, None), "");
        assert!(start.elapsed() < Duration::from_millis(500), "a missing cursor binary blocked for {:?}", start.elapsed());
        let start = Instant::now();
        assert_eq!(delegates.cursor_catalog(missing, None), "");
        assert!(start.elapsed() < Duration::from_millis(200), "a cached miss waited {:?}", start.elapsed());
    }

    #[test]
    fn a_task_on_another_machine_ends_only_from_that_machine_and_shows_no_card_here() {
        let delegates = Delegates::default();
        let task = delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: "Slow suite".into(),
            account: "claude".into(),
            provider: Provider::Claude,
            model: None,
            cwd: std::env::temp_dir(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        delegates.set_remote(&task.id, RemoteTarget { machine: "m-mini".into(), name: "mini".into(), tab: Some("tab-9".into()), looked: None, waiting: None });
        assert_eq!(delegates.cards("tab-1"), vec![], "its thread on mini nests under tab-1 instead");
        let end = RemoteEnd { state: "result_available".into(), summary: Some("Green.".into()), error: None, usage: None };
        assert!(delegates.settle_remote(&task.id, "tab-1", "m-other", &end).is_none(), "another machine can't end it");
        assert!(delegates.settle_remote(&task.id, "tab-2", "m-mini", &end).is_none(), "nor can it end another tab's task");
        assert_eq!(delegates.settle_remote(&task.id, "tab-1", "m-mini", &end), Some("tab-1".into()));
        assert!(delegates.settle_remote(&task.id, "tab-1", "m-mini", &end).is_none(), "an end lands once");
        let (text, _) = delegates.claim_notice("tab-1").unwrap();
        assert_eq!(text, format!("[pinned] Delegated task {} \"Slow suite\" (Claude Code) finished: result available. It ran on mini, in thread tab-9. Call task_status with taskId \"{}\".", task.id, task.id));
        assert_eq!(delegates.records(), vec![], "mini, which ran it, keeps its record");

        let on_mini = Delegates::default();
        let caller = Caller { tab: Some(Origin::key("m-mac", "tab-1")), task: None };
        let there = on_mini.begin(NewTask {
            parent: Parent::Tab(Origin::key("m-mac", "tab-1")),
            tool: None,
            title: "Slow suite".into(),
            account: "claude".into(),
            provider: Provider::Claude,
            model: None,
            cwd: std::env::temp_dir(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: caller.key(),
            client_request_id: Some(task.id.clone()),
            role: None,
        });
        on_mini.cancel(&caller, &there.id, "not now", None).unwrap();
        let records = on_mini.records();
        assert_eq!(records.iter().map(|r| (r.id.as_str(), r.target)).collect::<Vec<_>>(), vec![(there.id.as_str(), RunTarget::Machine)], "it ran on another machine than its delegator");
    }

    #[test]
    fn a_task_ending_within_a_second_keeps_millisecond_stamps() {
        let delegates = Delegates::default();
        let task = delegates.begin(NewTask {
            parent: Parent::Tab("tab-1".into()),
            tool: None,
            title: "Quick".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: std::env::temp_dir(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        std::thread::sleep(Duration::from_millis(20));
        delegates.apply(&task.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        let card = delegates.cards("tab-1").into_iter().next().unwrap();
        let started = card.started.unwrap();
        assert!(started.contains('.'), "started lost its milliseconds: {started}");
        assert!(card.updated.contains('.'), "updated lost its milliseconds: {}", card.updated);
        let start = chrono::DateTime::parse_from_rfc3339(&started).unwrap();
        let end = chrono::DateTime::parse_from_rfc3339(&card.updated).unwrap();
        assert!(end > start, "a sub-second task collapsed to one stamp: {started} {end}");
    }

    fn thread_task(delegates: &Delegates, limit: Duration) -> Task {
        let task = new_on(delegates, Provider::Claude);
        delegates.link_thread(&task.id, Thread { tab: "tab-w".into(), runs: true });
        delegates.set_limits(&task.id, limit.as_millis() as u64 / 2, limit.as_millis() as u64);
        task
    }

    fn state_of(delegates: &Delegates, id: &str) -> (String, Option<String>, Option<String>) {
        let status = delegates.status(&parent(), id).unwrap();
        (status["workState"].as_str().unwrap().to_owned(), status["summary"].as_str().map(str::to_owned), status["stoppedByLimit"].as_str().map(str::to_owned))
    }

    #[test]
    fn a_worker_waiting_on_an_approval_past_its_time_limit_is_not_stopped_and_counts_on_from_where_it_paused() {
        let delegates = Delegates::default();
        let task = thread_task(&delegates, Duration::from_secs(60));
        let begun = delegates.get(&task.id).unwrap().run.begun;
        let at = |secs: u64| begun + Duration::from_secs(secs);
        delegates.note_user_waits(&[], at(10));
        delegates.note_user_waits(&[task.id.clone()], at(10));
        delegates.enforce_limits(at(600), None);
        assert_eq!(state_of(&delegates, &task.id).0, "working", "ten minutes on an approval are not work");

        delegates.note_user_waits(&[], at(600));
        assert_eq!(delegates.get(&task.id).unwrap().run.counted(at(600)), 10_000, "the clock stopped at 10 s");
        delegates.enforce_limits(at(649), None);
        assert_eq!(state_of(&delegates, &task.id).0, "working", "59 s counted");
        delegates.enforce_limits(at(651), None);
        assert_eq!(state_of(&delegates, &task.id), ("cancelled".to_owned(), None, Some("time".to_owned())), "61 s counted");
    }

    #[test]
    fn a_worker_on_another_machine_waiting_on_an_approval_pauses_the_time_limit_here() {
        let delegates = Delegates::default();
        let task = new_on(&delegates, Provider::Claude);
        delegates.set_limits(&task.id, 30_000, 60_000);
        delegates.set_remote(&task.id, RemoteTarget { machine: "m-mini".into(), name: "mini".into(), tab: Some("tab-9".into()), looked: None, waiting: None });
        let begun = delegates.get(&task.id).unwrap().run.begun;
        let at = |secs: u64| begun + Duration::from_secs(secs);
        let ask = RemoteAsk { kind: crate::global_orchestrator::AskKind::Approval, tool: Some("Bash".into()), ask: "cargo publish".into(), since: 0 };
        delegates.set_remote_waiting(&task.id, "tab-1", "m-mini", Some(ask)).unwrap();
        delegates.note_user_waits(&[], at(20));
        let past_grace = 60 + REMOTE_GRACE.as_secs() + 600;
        assert_eq!(delegates.enforce_limits(at(past_grace), None), vec![], "nothing to stop while mini's worker waits on the user");
        assert_eq!(state_of(&delegates, &task.id).0, "working");

        delegates.set_remote_waiting(&task.id, "tab-1", "m-mini", None).unwrap();
        delegates.note_user_waits(&[], at(past_grace));
        assert_eq!(delegates.get(&task.id).unwrap().run.counted(at(past_grace)), 20_000);
        assert_eq!(delegates.enforce_limits(at(past_grace + 40 + REMOTE_GRACE.as_secs() - 1), None), vec![], "its grace counts from the answer too");
        assert_eq!(delegates.enforce_limits(at(past_grace + 41 + REMOTE_GRACE.as_secs()), None).len(), 1);
        assert_eq!(state_of(&delegates, &task.id).2.as_deref(), Some("time"));
    }

    #[test]
    fn time_while_pinned_was_down_does_not_count_after_a_restart() {
        let dir = scratch("down-time");
        let file = dir.join("delegates.json");
        let first = Delegates::load(Some(file.clone()));
        let task = thread_task(&first, Duration::from_secs(3600));
        first.backdate(&task.id, Duration::from_secs(1800));
        first.backdate_run(&task.id, Duration::from_secs(1800));
        first.set_worker_session(&task.id, "s-worker");
        drop(first);
        // pinned stays down for two hours: the wall clock moves, the counted half hour does not.
        let mut saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let entry = saved["tasks"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == task.id.as_str()).unwrap();
        assert_eq!(entry["counted_ms"].as_u64().map(|ms| ms / 60_000), Some(30), "{entry}");
        entry["started"] = json!(entry["started"].as_u64().unwrap() - 2 * 3600 * 1000);
        std::fs::write(&file, saved.to_string()).unwrap();

        let after = Delegates::load(Some(file));
        after.enforce_limits(Instant::now() + Duration::from_secs(29 * 60), None);
        assert_eq!(state_of(&after, &task.id).0, "working", "59 of 60 minutes counted, though 2 h 59 min passed");
        after.enforce_limits(Instant::now() + Duration::from_secs(31 * 60), None);
        assert_eq!(state_of(&after, &task.id).2.as_deref(), Some("time"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cap_that_passes_after_the_worker_finished_its_turn_ends_the_task_with_its_reply() {
        let delegates = Delegates::default();
        let limits = crate::taste::Limits { budget_cents: 500, ..crate::taste::Limits::default() };
        let spent = |cost: f64| crate::usage::TaskUsage { cost_usd: Some(cost), ..Default::default() };

        let finished = thread_task(&delegates, Duration::from_secs(3600));
        delegates.hold_to(&finished.id, limits);
        delegates.turn_finished(&finished.id, Some("PR #12 is open and green.".into()), vec!["1 background shell".into()]);
        delegates.meter(&[(finished.id.clone(), spent(5.10))], None);
        assert_eq!(state_of(&delegates, &finished.id), ("result_available".to_owned(), Some("PR #12 is open and green.".to_owned()), None));
        let ended_by = delegates.status(&parent(), &finished.id).unwrap()["endedBy"].as_str().unwrap().to_owned();
        assert_eq!(ended_by, "pinned stopped it after it spent US$ 5.10, its budget of US$ 5.00, after its turn had finished; its last reply is the result");

        let mid_turn = thread_task(&delegates, Duration::from_secs(3600));
        delegates.hold_to(&mid_turn.id, limits);
        delegates.turn_finished(&mid_turn.id, Some("Looking.".into()), vec![]);
        delegates.turn_started(&mid_turn.id);
        delegates.meter(&[(mid_turn.id.clone(), spent(5.10))], None);
        assert_eq!(state_of(&delegates, &mid_turn.id).0, "cancelled", "a turn still running is stopped as before");
    }

    #[test]
    fn task_status_says_which_tasks_and_shells_a_finished_turn_still_waits_on() {
        let delegates = Delegates::default();
        let task = thread_task(&delegates, Duration::from_secs(3600));
        let child = delegates.begin(NewTask {
            parent: Parent::Tab("tab-w".into()),
            tool: None,
            title: "Rust half".into(),
            account: "claude".into(),
            provider: Provider::Claude,
            model: None,
            cwd: PathBuf::from("/tmp"),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: "tab:tab-w".into(),
            client_request_id: None,
            role: None,
        });
        delegates.turn_finished(&task.id, Some("Waiting on the Rust half.".into()), vec!["1 background shell".into()]);
        assert_eq!(delegates.waited_children("tab-w", Some(&task.id)), vec![(child.id.clone(), "Rust half".to_owned())]);
        let status = delegates.status(&parent(), &task.id).unwrap();
        assert_eq!(status["waitsOn"], json!({ "tasks": [{ "taskId": child.id, "title": "Rust half", "workState": "working" }], "background": ["1 background shell"] }));
        assert!(status["next"].as_str().unwrap().starts_with(&format!("its turn ended and summary holds its reply so far; it still waits on task {} \"Rust half\" (working), 1 background shell;", child.id)), "{status}");

        delegates.settle_thread(&child.id, Some("Done.".into()), None, None, vec![], None);
        assert_eq!(delegates.waited_children("tab-w", Some(&task.id)).len(), 1, "its end has not reached the worker yet");
        delegates.claim_notice("tab-w").unwrap();
        assert_eq!(delegates.waited_children("tab-w", Some(&task.id)), vec![]);
    }
}
