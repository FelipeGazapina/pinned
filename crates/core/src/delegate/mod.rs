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
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

const STEP_KEEP: usize = 8;
const STEP_CHARS: usize = 400;
pub const DEFAULT_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const MODEL_CACHE: Duration = Duration::from_secs(10 * 60);
const MODEL_MISS: Duration = Duration::from_secs(15);

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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Target {
    #[serde(default)]
    pub account: Option<AccountId>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
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
    pub plan: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkState {
    Working,
    ResultAvailable,
    Failed,
    Cancelled,
}

impl WorkState {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkState::Working => "working",
            WorkState::ResultAvailable => "result-available",
            WorkState::Failed => "failed",
            WorkState::Cancelled => "cancelled",
        }
    }

    fn terminal(self) -> bool {
        self != WorkState::Working
    }
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
    pub mode: Mode,
    pub delivered: bool,
}

#[derive(Clone, Debug)]
pub struct OpenInfo {
    pub cwd: PathBuf,
    pub account: AccountId,
    pub provider: Provider,
    pub conversation: String,
}

pub enum ChildEvent {
    Line(String),
    Exited { code: Option<i32>, stderr: String },
}

#[derive(Debug)]
pub enum StreamEvent {
    Conversation(String),
    Step(Item),
    Final { text: String, error: Option<String> },
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
        let args = json!({
            "task": "fix the flaky test",
            "title": "Flaky",
            "target": { "providerInstanceId": "cursor", "model": "grok-4.7", "options": { "reasoning_effort": "high" } },
            "isolation": "worktree",
            "mode": "wait",
            "timeoutMs": 5000,
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
                    effort: Some("high".into())
                },
                isolation: Isolation::Worktree,
                mode: Mode::Wait,
                timeout_ms: 5000,
                plan: true,
            }
        );
        let effort = json!({"task": "x", "target": {"options": {"effort": "low"}}});
        assert_eq!(
            parse_request(&effort).unwrap().target.effort.as_deref(),
            Some("low")
        );
        assert_eq!(
            parse_request(&json!({"task": "hello"})).unwrap(),
            Request {
                task: "hello".into(),
                title: "hello".into(),
                target: Target {
                    account: None,
                    model: None,
                    effort: None
                },
                isolation: Isolation::Shared,
                mode: Mode::Async,
                timeout_ms: DEFAULT_TIMEOUT_MS,
                plan: false,
            }
        );
        assert_eq!(parse_request(&json!({})).unwrap_err(), "task is required");
        assert_eq!(
            parse_request(&json!({"task": "x", "mode": "soon"})).unwrap_err(),
            "mode must be async or wait, not \"soon\""
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
        assert_eq!(value["providerInstanceId"], "claude");
        assert_eq!(value["model"], "claude-haiku-5-5");
        assert_eq!(value["access"], "auto");
        assert_eq!(value["accounts"][0]["driverKind"], "claude");
        assert_eq!(value["accounts"][0]["models"][3]["id"], "claude-haiku-5-5");
        assert_eq!(
            value["accounts"][0]["models"][3]["options"][0]["values"],
            json!(["low", "medium", "high", "xhigh", "max"])
        );
        assert_eq!(value["accounts"][0]["canRunChildTask"], true);
        assert_eq!(value["accounts"][1]["canRunChildTask"], false);
        assert_eq!(
            value["accounts"][1]["constraints"][0],
            "Any model id works. Pass it as target.model."
        );
        assert_eq!(value["accounts"][1]["models"], json!([]));
        assert_eq!(
            value["accounts"][2]["models"][0],
            json!({"id": "grok-4.7", "label": "Grok 4.7", "options": [{"id": "effort", "values": ["high"]}]})
        );
        assert_eq!(
            value["accounts"][2]["constraints"][0],
            "Cursor supports only its built-in account."
        );
        let found = capabilities(&parent, &accounts, Some("grok"));
        assert_eq!(found["accounts"].as_array().unwrap().len(), 1);
        assert_eq!(found["accounts"][0]["providerInstanceId"], "cursor");
        assert_eq!(found["accounts"][0]["models"][0]["id"], "grok-4.7");
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
        assert!(
            !codex.done,
            "codex has no result line; the last agent message is the text"
        );
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
            [StreamEvent::Final { text, error }] => {
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
                sink.apply(&id, event);
            },
        )
        .unwrap();
        let start = Instant::now();
        let status = loop {
            let status = shared.status(&task.id).unwrap();
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
        assert_eq!(status["workState"], "result-available");
        assert_eq!(
            status["conversationId"],
            "9fa2e973-3c1c-4a8e-9ab7-9790584c944c"
        );
        let kept = shared.cancel(&task.id, "too late").unwrap();
        assert_eq!(kept["summary"], "DONE");
        assert_eq!(kept["workState"], "result-available");

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
                sink.apply(&run_id, event);
            },
        )
        .unwrap();
        shared.remember_pid(&running.id, pid);
        let cancelled = shared.cancel(&running.id, "the parent asked").unwrap();
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
}
