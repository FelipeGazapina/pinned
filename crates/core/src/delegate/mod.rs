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

const STEP_KEEP: usize = 8;
const STEP_CHARS: usize = 400;
pub const DEFAULT_TIMEOUT_MS: u64 = 10 * 60 * 1000;
pub const DEFAULT_BUDGET_MS: u64 = 2 * 60 * 60 * 1000;
pub const DEFAULT_LIMIT_MS: u64 = 8 * 60 * 60 * 1000;
const QUIET_AFTER_REPLY_MS: u64 = 30 * 60 * 1000;
const MODEL_CACHE: Duration = Duration::from_secs(10 * 60);
const MODEL_MISS: Duration = Duration::from_secs(15);
const CATALOG_WAIT: Duration = Duration::from_secs(2);

pub fn delegate_block(role: Option<crate::agent::Role>, child: bool) -> Option<&'static str> {
    if child {
        Some("a delegated child cannot start another pinned worker")
    } else if role == Some(crate::agent::Role::Worker) {
        Some("a worker cannot start another pinned worker")
    } else {
        None
    }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    #[serde(default = "default_budget")]
    pub budget_ms: u64,
    #[serde(default = "default_limit")]
    pub limit_ms: u64,
    #[serde(default)]
    pub interaction: Interaction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<String>,
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
    /// The full latest assistant message, kept after the step ring drops it.
    pub assistant: Option<String>,
    pub mode: Mode,
    pub delivered: bool,
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
    pub background: Vec<Background>,
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
            background: vec![],
        }
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
    TurnEnd { text: String, error: Option<String> },
}

/// Headless argv for one provider. The binary is not included.
pub struct SpawnSpec<'a> {
    pub task: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub access: Access,
    pub mcp_config: Option<&'a Path>,
    pub exe: &'a Path,
    pub sock: &'a Path,
    pub task_id: &'a str,
    pub reminders: bool,
}

mod catalog;
mod drivers;
mod registry;
mod request;

pub use catalog::*;
pub use drivers::*;
pub use registry::*;
pub use request::*;

#[cfg(test)]
pub(crate) use drivers::{driver, replay};

#[cfg(test)]
mod tests {
    use super::*;

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
                },
                isolation: Isolation::Worktree,
                mode: Mode::Wait,
                timeout_ms: 5000,
                budget_ms: 90_000,
                limit_ms: 600_000,
                interaction: Interaction::Plan,
                runtime: None,
                role: None,
                client_request_id: None,
            }
        );
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
                },
                isolation: Isolation::Shared,
                mode: Mode::Async,
                timeout_ms: DEFAULT_TIMEOUT_MS,
                budget_ms: 7_200_000,
                limit_ms: 28_800_000,
                interaction: Interaction::Inherit,
                runtime: None,
                role: None,
                client_request_id: None,
            }
        );
        assert_eq!(
            parse_request(&json!({"task": "x", "budgetMs": 0})).unwrap_err(),
            "budgetMs must be a number of milliseconds above 0"
        );
        assert_eq!(
            parse_request(&json!({"task": "x", "budgetMs": 36_000_000})).unwrap_err(),
            "budgetMs must be below limitMs, which is 28800000 ms; pass a larger limitMs"
        );
        let long = parse_request(&json!({"task": "x", "budgetMs": 36_000_000, "limitMs": 72_000_000})).unwrap();
        assert_eq!((long.budget_ms, long.limit_ms), (36_000_000, 72_000_000));
        assert_eq!(parse_request(&json!({})).unwrap_err(), "task is required");
        assert_eq!(
            parse_request(&json!({"task": "x", "mode": "soon"})).unwrap_err(),
            "mode must be async or wait, not \"soon\""
        );
    }

    #[test]
    fn a_limit_alone_puts_the_budget_notice_before_it() {
        let limits = |args: Value| parse_request(&args).map(|request| (request.budget_ms, request.limit_ms));
        assert_eq!(limits(json!({"task": "x", "limitMs": 3_600_000})), Ok((1_800_000, 3_600_000)));
        assert_eq!(limits(json!({"task": "x", "limitMs": 43_200_000})), Ok((7_200_000, 43_200_000)));
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
                    efforts: vec!["fast".into()]
                },
                ModelChoice {
                    id: "grok-4.7".into(),
                    label: "Grok 4.7".into(),
                    efforts: vec!["low".into(), "high".into()]
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
                models: provider_models(Provider::Claude),
                constraints: vec![],
            },
            ListedAccount {
                id: "codex".into(),
                provider: Provider::Codex,
                name: "Codex".into(),
                signed_in: false,
                bin_found: false,
                models: vec![],
                constraints: codex_constraints(),
            },
            ListedAccount {
                id: "cursor".into(),
                provider: Provider::Cursor,
                name: "Cursor".into(),
                signed_in: true,
                bin_found: true,
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
            json!({"id": "grok-4.7", "label": "Grok 4.7", "options": [{"id": "effort", "values": ["high"], "default": null}]})
        );
        assert_eq!(value["providers"][2]["canRunCrossProviderChildTask"], false);
        assert_eq!(
            value["providers"][2]["constraints"][0],
            "Cursor supports only its built-in account."
        );
        assert_eq!(
            value["providers"][2]["constraints"][1],
            "A worker or a delegated child cannot start another pinned worker. A Cursor session cannot delegate."
        );
        let found = capabilities(&parent, &accounts, Some("grok"));
        assert_eq!(found["providers"].as_array().unwrap().len(), 1);
        assert_eq!(found["providers"][0]["providerInstanceId"], "cursor");
        assert_eq!(found["providers"][0]["models"][0]["id"], "grok-4.7");
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
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
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
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }

    #[cfg(unix)]
    fn stopped(pid: i32) -> bool {
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

    fn scratch(name: &str) -> PathBuf {
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
    fn a_cancelled_task_keeps_the_reply_so_far() {
        let file = scratch("cancel-keeps").join("delegates.json");
        let mut seen = vec![];
        for how in ["task_cancel", "limit", "quit", "crash"] {
            let delegates = Delegates::load(Some(file.clone()));
            let task = new_on(&delegates, Provider::Cursor);
            for line in [INIT, REPLY] {
                delegates.apply(&task.id, ChildEvent::Line(line.to_owned()), None);
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
        assert_eq!(
            seen,
            vec![
                json!(["task_cancel", "cancelled", "PR opened", "the parent asked"]),
                json!(["limit", "cancelled", "PR opened", "pinned stopped it after 8 h, its limit"]),
                json!(["quit", "cancelled", "PR opened", "stopped by restart"]),
                json!(["crash", "cancelled", "PR opened", "stopped by restart"]),
            ]
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
        let file = scratch("reload-ended").join("delegates.json");
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
    }

    #[test]
    fn a_saved_file_from_before_the_limits_loads_with_the_defaults() {
        let file = scratch("old-file").join("delegates.json");
        let old = r#"{"tasks":[{"id":"task-0ld0","parent_kind":"tab","parent_id":"tab-1","title":"Old","account":"claude","provider":"claude","cwd":"/tmp","access":"auto","started":1000,"ended":2000,"state":"result_available","summary":"done","mode":"async","caller_key":"tab:tab-1"}]}"#;
        std::fs::write(&file, old).unwrap();
        let status = Delegates::load(Some(file)).status(&parent(), "task-0ld0").unwrap();
        assert_eq!(
            json!([status["workState"], status["summary"], status["endedBy"], status["budgetSecs"], status["limitSecs"]]),
            json!(["result_available", "done", null, 7200, 28800]),
            "{status}"
        );
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
        let first = match delegates.claim(spec("tab:tab-1", "tab-1", "req-7")) {
            Claim::Fresh(task) => task.id,
            Claim::Existing(id) => id,
        };
        delegates.apply(&first, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        let again = match delegates.claim(spec("tab:tab-1", "tab-1", "req-7")) {
            Claim::Existing(id) => id,
            Claim::Fresh(task) => task.id,
        };
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
        let fresh_id = match delegates.claim(spec("tab:tab-1", "tab-1", "req-8")) {
            Claim::Fresh(task) => task.id,
            Claim::Existing(id) => id,
        };
        assert_ne!(fresh_id, first);
        let fresh = delegates.started_json(&fresh_id).unwrap();
        assert_eq!(fresh["workState"], "working");
        let next = format!("end your turn and do not poll; pinned delivers the notice when the turn ends or when this session is idle, or the user sends it from the task's card; then call task_status with this taskId \"{fresh_id}\"");
        assert_eq!(fresh["next"], next);
        let status = delegates.status(&Caller { tab: Some("tab-1".into()), task: None }, &fresh_id).unwrap();
        assert_eq!(status["workState"], "working");
        assert_eq!(status["next"], next);
        let other = match delegates.claim(spec("tab:tab-2", "tab-2", "req-7")) {
            Claim::Fresh(task) => task.id,
            Claim::Existing(id) => id,
        };
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
}
