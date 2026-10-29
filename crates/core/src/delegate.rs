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

struct Driver {
    args: fn(&SpawnSpec) -> Vec<String>,
    parse: fn(&str) -> Vec<StreamEvent>,
    resume: fn(&str) -> Vec<String>,
}

pub fn driver_args(provider: Provider, spec: &SpawnSpec) -> Vec<String> {
    (driver(provider).args)(spec)
}

pub fn resume_args(provider: Provider, session: &str) -> Vec<String> {
    (driver(provider).resume)(session)
}

fn driver(provider: Provider) -> &'static Driver {
    match provider {
        Provider::Claude => &CLAUDE,
        Provider::Codex => &CODEX,
        Provider::Cursor => &CURSOR,
    }
}

const CLAUDE: Driver = Driver {
    args: claude_args,
    parse: parse_claude,
    resume: resume_flag,
};
const CODEX: Driver = Driver {
    args: codex_args,
    parse: parse_codex,
    resume: codex_resume,
};
const CURSOR: Driver = Driver {
    args: cursor_args,
    parse: parse_cursor,
    resume: resume_flag,
};

fn claude_args(spec: &SpawnSpec) -> Vec<String> {
    let mut args = vec![
        "-p".into(),
        spec.task.into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ];
    if let Some(model) = spec.model {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(effort) = spec.effort {
        args.extend(["--effort".into(), effort.into()]);
    }
    match spec.access {
        Access::Full => args.push("--dangerously-skip-permissions".into()),
        Access::Auto => args.extend(["--permission-mode".into(), "auto".into()]),
        Access::ReadOnly => args.extend(["--permission-mode".into(), "plan".into()]),
    }
    if let Some(config) = spec.mcp_config {
        args.extend(["--mcp-config".into(), config.display().to_string()]);
    }
    args
}

fn codex_args(spec: &SpawnSpec) -> Vec<String> {
    let mut args = vec![
        "exec".into(),
        "--json".into(),
        "--skip-git-repo-check".into(),
    ];
    if let Some(model) = spec.model {
        args.extend(["-m".into(), model.into()]);
    }
    if let Some(effort) = spec.effort {
        args.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
    }
    match spec.access {
        Access::Full => args.push("--dangerously-bypass-approvals-and-sandbox".into()),
        Access::Auto => args.push("--full-auto".into()),
        Access::ReadOnly => args.extend(["-s".into(), "read-only".into()]),
    }
    args.extend(codex_mcp_flags(
        spec.exe,
        spec.sock,
        None,
        Some(spec.task_id),
        spec.reminders,
    ));
    args.push(spec.task.into());
    args
}

/// `--trust` only trusts the workspace. `--force` is the approval bypass, so auto leaves it off
/// and plan uses `--mode plan` instead.
fn cursor_args(spec: &SpawnSpec) -> Vec<String> {
    let mut args = vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--trust".into(),
    ];
    match spec.access {
        Access::Full => args.push("--force".into()),
        Access::Auto => {}
        Access::ReadOnly => args.extend(["--mode".into(), "plan".into()]),
    }
    if let Some(model) = spec.model {
        args.extend(["--model".into(), model.into()]);
    }
    args.push(spec.task.into());
    args
}

fn resume_flag(session: &str) -> Vec<String> {
    vec!["--resume".into(), session.into()]
}

fn codex_resume(session: &str) -> Vec<String> {
    vec!["resume".into(), session.into()]
}

/// Codex reads MCP servers from `-c` config, with the socket and tab written out. It does not
/// expand a `${PINNED_SOCK}` the way Claude's mcp config does.
pub fn codex_mcp_flags(
    exe: &Path,
    sock: &Path,
    tab: Option<&str>,
    task: Option<&str>,
    reminders: bool,
) -> Vec<String> {
    let mut args = vec![
        "-c".into(),
        format!(
            "mcp_servers.pinned.command={}",
            toml_str(&exe.display().to_string())
        ),
    ];
    let mcp_args = if reminders {
        r#"["mcp"]"#
    } else {
        r#"["mcp","--no-reminders"]"#
    };
    args.extend(["-c".into(), format!("mcp_servers.pinned.args={mcp_args}")]);
    args.extend([
        "-c".into(),
        format!(
            "mcp_servers.pinned.env.PINNED_SOCK={}",
            toml_str(&sock.display().to_string())
        ),
    ]);
    if let Some(tab) = tab {
        args.extend([
            "-c".into(),
            format!("mcp_servers.pinned.env.PINNED_TAB={}", toml_str(tab)),
        ]);
    }
    if let Some(task) = task {
        args.extend([
            "-c".into(),
            format!("mcp_servers.pinned.env.PINNED_TASK={}", toml_str(task)),
        ]);
    }
    args
}

fn toml_str(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn parse_claude(line: &str) -> Vec<StreamEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    if let Some(event) = conversation_or_final(&value) {
        return vec![event];
    }
    if value["type"] == "system" {
        return vec![];
    }
    thread::items_of(&value)
        .into_iter()
        .map(StreamEvent::Step)
        .collect()
}

fn parse_cursor(line: &str) -> Vec<StreamEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    if let Some(event) = conversation_or_final(&value) {
        return vec![event];
    }
    match value["type"].as_str() {
        Some("thinking") if value["subtype"] == "delta" => {
            let text = value["text"].as_str().unwrap_or("").to_owned();
            if text.is_empty() {
                vec![]
            } else {
                vec![StreamEvent::Step(Item::Thinking {
                    id: String::new(),
                    at: String::new(),
                    text,
                })]
            }
        }
        Some("tool_call") if value["subtype"] == "started" => {
            cursor_tool(&value, false).into_iter().collect()
        }
        Some("tool_call") if value["subtype"] == "completed" => {
            cursor_tool(&value, true).into_iter().collect()
        }
        Some("assistant") => {
            let text = value["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("");
            if text.trim().is_empty() {
                vec![]
            } else {
                vec![StreamEvent::Step(Item::Text {
                    id: "assistant".into(),
                    at: String::new(),
                    text,
                })]
            }
        }
        _ => vec![],
    }
}

fn cursor_tool(value: &Value, completed: bool) -> Option<StreamEvent> {
    let call = value["tool_call"].as_object()?;
    let (key, body) = call.iter().find(|(key, _)| key.ends_with("ToolCall"))?;
    let name = key.strip_suffix("ToolCall").unwrap_or(key).to_owned();
    let id = value["call_id"]
        .as_str()
        .or(body["toolCallId"].as_str())
        .unwrap_or("")
        .to_owned();
    let command = body["args"]["command"].as_str().unwrap_or("").to_owned();
    let summary = if command.is_empty() {
        body["args"]["description"]
            .as_str()
            .or(body["description"].as_str())
            .unwrap_or("")
            .to_owned()
    } else {
        command.clone()
    };
    if !completed {
        return Some(StreamEvent::Step(Item::Tool {
            id,
            at: String::new(),
            name,
            summary: summary.clone(),
            detail: summary,
            args: vec![],
        }));
    }
    let stdout = body["result"]["success"]["stdout"]
        .as_str()
        .or(body["result"]["success"]["interleavedOutput"].as_str())
        .unwrap_or("");
    let error = body["result"].get("error").is_some()
        || body["result"]["success"]["exitCode"]
            .as_i64()
            .is_some_and(|code| code != 0);
    Some(StreamEvent::Step(Item::Result {
        tool: id,
        at: String::new(),
        error,
        text: stdout.to_owned(),
        links: vec![],
    }))
}

/// `codex exec --json` events, from the documented schema. A live CLI was not available to capture them.
fn parse_codex(line: &str) -> Vec<StreamEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    match value["type"].as_str() {
        Some("thread.started") => value["thread_id"]
            .as_str()
            .map(|id| vec![StreamEvent::Conversation(id.to_owned())])
            .unwrap_or_default(),
        Some("item.started") => codex_item(&value["item"], false),
        Some("item.completed") => codex_item(&value["item"], true),
        Some("turn.failed") => {
            let message = value["error"]["message"]
                .as_str()
                .or(value["error"].as_str())
                .unwrap_or("the agent failed");
            vec![StreamEvent::Final {
                text: String::new(),
                error: Some(message.to_owned()),
            }]
        }
        Some("error") => {
            let message = value["message"].as_str().unwrap_or("the agent failed");
            vec![StreamEvent::Final {
                text: String::new(),
                error: Some(message.to_owned()),
            }]
        }
        _ => vec![],
    }
}

fn codex_item(item: &Value, completed: bool) -> Vec<StreamEvent> {
    let id = item["id"].as_str().unwrap_or("").to_owned();
    match item["type"].as_str() {
        Some("reasoning") if completed => text_step(&id, item["text"].as_str().unwrap_or(""), true),
        Some("agent_message") if completed => {
            text_step(&id, item["text"].as_str().unwrap_or(""), false)
        }
        Some("command_execution") if !completed => {
            let command = item["command"].as_str().unwrap_or("").to_owned();
            vec![StreamEvent::Step(Item::Tool {
                id,
                at: String::new(),
                name: "Bash".into(),
                summary: command.clone(),
                detail: command,
                args: vec![],
            })]
        }
        Some("command_execution") => {
            let output = item["aggregated_output"].as_str().unwrap_or("").to_owned();
            let error = item["exit_code"].as_i64().is_some_and(|code| code != 0);
            vec![StreamEvent::Step(Item::Result {
                tool: id,
                at: String::new(),
                error,
                text: output,
                links: vec![],
            })]
        }
        Some("file_change") if completed => {
            let summary = item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|change| change["path"].as_str())
                .collect::<Vec<_>>()
                .join(", ");
            vec![StreamEvent::Step(Item::Tool {
                id,
                at: String::new(),
                name: "file_change".into(),
                summary: summary.clone(),
                detail: summary,
                args: vec![],
            })]
        }
        _ => vec![],
    }
}

fn text_step(id: &str, text: &str, thinking: bool) -> Vec<StreamEvent> {
    if text.is_empty() {
        return vec![];
    }
    let item = if thinking {
        Item::Thinking {
            id: id.into(),
            at: String::new(),
            text: text.into(),
        }
    } else {
        Item::Text {
            id: id.into(),
            at: String::new(),
            text: text.into(),
        }
    };
    vec![StreamEvent::Step(item)]
}

fn conversation_or_final(value: &Value) -> Option<StreamEvent> {
    if value["type"] == "result" {
        let text = value["result"].as_str().unwrap_or("").to_owned();
        let error = (value["is_error"].as_bool() == Some(true)).then(|| {
            if text.is_empty() {
                "the agent failed".into()
            } else {
                text.clone()
            }
        });
        return Some(StreamEvent::Final { text, error });
    }
    if value["type"] == "system" && value["subtype"] == "init" {
        return value["session_id"]
            .as_str()
            .map(|id| StreamEvent::Conversation(id.to_owned()));
    }
    None
}

#[cfg(test)]
struct Outcome {
    conversation: Option<String>,
    steps: Vec<Item>,
    text: Option<String>,
    done: bool,
    error: Option<String>,
}

#[cfg(test)]
fn replay(provider: Provider, lines: &str) -> Outcome {
    let mut outcome = Outcome {
        conversation: None,
        steps: vec![],
        text: None,
        done: false,
        error: None,
    };
    for line in lines.lines() {
        for event in (driver(provider).parse)(line) {
            match event {
                StreamEvent::Conversation(id) => outcome.conversation = Some(id),
                StreamEvent::Step(item) => {
                    if !outcome.done {
                        if let Item::Text { text, .. } = &item {
                            outcome.text = Some(text.clone());
                        }
                    }
                    push_step(&mut outcome.steps, item);
                }
                StreamEvent::Final { text, error } => {
                    outcome.text = Some(text);
                    outcome.error = error;
                    outcome.done = true;
                }
            }
        }
    }
    outcome
}

fn push_step(steps: &mut Vec<Item>, item: Item) {
    if let Item::Thinking { text: more, .. } = &item {
        if let Some(Item::Thinking { text, .. }) = steps.last_mut() {
            if !text.is_empty() && !more.is_empty() {
                text.push(' ');
            }
            text.push_str(more);
            let clipped = thread::clip(text, STEP_CHARS);
            *text = clipped;
            return;
        }
    }
    steps.push(clip_item(item));
    let extra = steps.len().saturating_sub(STEP_KEEP);
    if extra > 0 {
        steps.drain(..extra);
    }
}

fn clip_item(item: Item) -> Item {
    match item {
        Item::Text { id, at, text } => Item::Text {
            id,
            at,
            text: thread::clip(&text, STEP_CHARS),
        },
        Item::Thinking { id, at, text } => Item::Thinking {
            id,
            at,
            text: thread::clip(&text, STEP_CHARS),
        },
        Item::Tool {
            id,
            at,
            name,
            summary,
            detail,
            args,
        } => Item::Tool {
            id,
            at,
            name,
            summary: thread::clip(&summary, STEP_CHARS),
            detail: thread::clip(&detail, STEP_CHARS),
            args,
        },
        Item::Result {
            tool,
            at,
            error,
            text,
            links,
        } => Item::Result {
            tool,
            at,
            error,
            text: thread::clip(&text, STEP_CHARS),
            links,
        },
        other => other,
    }
}

fn last_text(steps: &[Item]) -> Option<String> {
    steps.iter().rev().find_map(|item| match item {
        Item::Text { text, .. } => Some(text.clone()),
        _ => None,
    })
}

/// T3's `delegate_task` arguments, accepted under those names.
pub fn parse_request(args: &Value) -> Result<Request, String> {
    let task = args["task"]
        .as_str()
        .map(str::trim)
        .filter(|task| !task.is_empty())
        .ok_or("task is required")?
        .to_owned();
    let title = match args["title"]
        .as_str()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        Some(title) => thread::clip(title, 80),
        None => thread::clip(
            task.lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("task")
                .trim(),
            80,
        ),
    };
    let target = &args["target"];
    let account = optional_str(&target["providerInstanceId"]);
    let model = optional_str(&target["model"]);
    if let Some(model) = &model {
        if !token_ok(model) {
            return Err(format!("{model:?} is not a model id"));
        }
    }
    let effort = optional_str(&target["options"]["effort"])
        .or(optional_str(&target["options"]["reasoning_effort"]));
    if let Some(effort) = &effort {
        if !token_ok(effort) {
            return Err(format!("{effort:?} is not an effort"));
        }
    }
    let isolation = match args["isolation"].as_str() {
        None | Some("shared") => Isolation::Shared,
        Some("worktree") => Isolation::Worktree,
        Some(other) => {
            return Err(format!(
                "isolation must be shared or worktree, not {other:?}"
            ))
        }
    };
    let mode = match args["mode"].as_str() {
        None | Some("async") => Mode::Async,
        Some("wait") => Mode::Wait,
        Some(other) => return Err(format!("mode must be async or wait, not {other:?}")),
    };
    let timeout_ms = match args.get("timeoutMs") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT_MS,
        Some(value) => value
            .as_u64()
            .filter(|ms| *ms > 0)
            .ok_or("timeoutMs must be a number of milliseconds above 0")?,
    };
    let plan = args["interactionMode"].as_str() == Some("plan");
    Ok(Request {
        task,
        title,
        target: Target {
            account,
            model,
            effort,
        },
        isolation,
        mode,
        timeout_ms,
        plan,
    })
}

fn optional_str(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn token_ok(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-.[]_".contains(&byte))
}

const CLAUDE_EFFORT: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-fable-5-1", "Fable 5.1"),
    ("claude-haiku-5-5", "Haiku 5.5"),
];
const CURSOR_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "fast"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
}

pub fn provider_models(provider: Provider) -> Vec<ModelChoice> {
    match provider {
        Provider::Claude => CLAUDE_MODELS
            .iter()
            .map(|(id, label)| ModelChoice {
                id: (*id).into(),
                label: (*label).into(),
                efforts: CLAUDE_EFFORT.iter().map(|e| (*e).into()).collect(),
            })
            .collect(),
        Provider::Codex => vec![],
        Provider::Cursor => vec![],
    }
}

/// `{model, effort}` becomes `model-effort` when `--list-models` printed that id.
pub fn map_cursor_model(ids: &[String], model: &str, effort: Option<&str>) -> String {
    match effort.map(str::trim).filter(|effort| !effort.is_empty()) {
        Some(effort) if ids.iter().any(|id| id == &format!("{model}-{effort}")) => {
            format!("{model}-{effort}")
        }
        _ => model.to_owned(),
    }
}

pub fn cursor_models(list: &str) -> Vec<ModelChoice> {
    let listed = parse_model_list(list);
    let mut order = vec![];
    let mut groups: HashMap<String, (String, Vec<String>)> = HashMap::new();
    for (id, label) in &listed {
        let (base, effort) = split_cursor_id(id);
        let entry = groups.entry(base.clone()).or_insert_with(|| {
            order.push(base.clone());
            (String::new(), vec![])
        });
        let cleaned = clean_label(label);
        if entry.0.is_empty() || cleaned.len() < entry.0.len() {
            entry.0 = cleaned;
        }
        if let Some(effort) = effort {
            if !entry.1.iter().any(|have| have == &effort) {
                entry.1.push(effort);
            }
        }
    }
    order
        .into_iter()
        .map(|id| {
            let (label, mut efforts) = groups.remove(&id).unwrap_or_default();
            efforts.sort_by_key(|effort| {
                CURSOR_EFFORTS
                    .iter()
                    .position(|known| known == effort)
                    .unwrap_or(CURSOR_EFFORTS.len())
            });
            ModelChoice { id, label, efforts }
        })
        .collect()
}

fn parse_model_list(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (id, label) = line.split_once(" - ")?;
            let id = id.trim();
            if id.is_empty() || id.contains(' ') {
                return None;
            }
            Some((id.to_owned(), label.trim().to_owned()))
        })
        .collect()
}

fn split_cursor_id(id: &str) -> (String, Option<String>) {
    let (body, fast) = match id.strip_suffix("-fast") {
        Some(body) if !body.is_empty() => (body, true),
        _ => (id, false),
    };
    for effort in ["xhigh", "medium", "high", "low", "max"] {
        if let Some(root) = body.strip_suffix(&format!("-{effort}")) {
            if !root.is_empty() {
                return (root.to_owned(), Some(effort.to_owned()));
            }
        }
    }
    if fast {
        (body.to_owned(), Some("fast".into()))
    } else {
        (id.to_owned(), None)
    }
}

fn clean_label(label: &str) -> String {
    let mut label = label
        .trim()
        .trim_end_matches(" (current)")
        .trim()
        .to_owned();
    let suffixes = [
        " Extra High Fast",
        " Extra High",
        " Low Fast",
        " Medium Fast",
        " High Fast",
        " Max Fast",
        " Low",
        " Medium",
        " High",
        " Max",
        " Fast",
    ];
    while let Some(shorter) = suffixes.iter().find_map(|suffix| label.strip_suffix(suffix).map(|rest| rest.trim().to_owned())) {
        label = shorter;
    }
    label.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub struct ListedAccount {
    pub id: String,
    pub provider: Provider,
    pub name: String,
    pub signed_in: bool,
    pub bin_found: bool,
    pub models: Vec<ModelChoice>,
    pub constraints: Vec<String>,
}

pub struct Inherited {
    pub account: String,
    pub model: Option<String>,
    pub access: Access,
}

pub fn capabilities(parent: &Inherited, accounts: &[ListedAccount], search: Option<&str>) -> Value {
    let needle = search
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.to_ascii_lowercase());
    let accounts = accounts
        .iter()
        .filter_map(|account| account_json(account, needle.as_deref()))
        .collect::<Vec<_>>();
    json!({
        "providerInstanceId": parent.account,
        "model": parent.model,
        "access": parent.access.as_str(),
        "accounts": accounts,
    })
}

fn account_json(account: &ListedAccount, needle: Option<&str>) -> Option<Value> {
    let driver_kind = serde_json::to_value(account.provider)
        .ok()?
        .as_str()?
        .to_owned();
    let account_hit = needle.is_none_or(|needle| {
        account.id.to_ascii_lowercase().contains(needle)
            || account.name.to_ascii_lowercase().contains(needle)
            || driver_kind.to_ascii_lowercase().contains(needle)
    });
    let models: Vec<&ModelChoice> = account
        .models
        .iter()
        .filter(|model| {
            account_hit
                || needle.is_none_or(|needle| {
                    model.id.to_ascii_lowercase().contains(needle)
                        || model.label.to_ascii_lowercase().contains(needle)
                })
        })
        .collect();
    if !account_hit && models.is_empty() {
        return None;
    }
    Some(json!({
        "providerInstanceId": account.id,
        "driverKind": driver_kind,
        "displayName": account.name,
        "models": models.iter().map(|model| model_json(model)).collect::<Vec<_>>(),
        "canRunChildTask": account.signed_in && account.bin_found,
        "constraints": account.constraints,
    }))
}

fn model_json(model: &ModelChoice) -> Value {
    let options = if model.efforts.is_empty() {
        vec![]
    } else {
        vec![json!({"id": "effort", "values": model.efforts})]
    };
    json!({"id": model.id, "label": model.label, "options": options})
}

pub fn codex_constraints() -> Vec<String> {
    vec!["Any model id works. Pass it as target.model.".into()]
}

pub fn cursor_constraints() -> Vec<String> {
    vec!["Cursor supports only its built-in account.".into()]
}

pub struct ChildSpec<'a> {
    pub bin: &'a Path,
    pub argv: &'a [String],
    pub cwd: &'a Path,
    pub env: &'a [(String, String)],
    pub path: Option<&'a str>,
    pub strip: &'a [&'a str],
}

pub fn spawn(
    spec: ChildSpec,
    on_event: impl Fn(ChildEvent) + Send + 'static,
) -> Result<u32, String> {
    if !spec.bin.is_file() {
        return Err(format!(
            "{} is not installed on this machine",
            spec.bin.display()
        ));
    }
    let mut cmd = crate::process::command(spec.bin);
    cmd.args(spec.argv)
        .current_dir(spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in spec.strip {
        cmd.env_remove(key);
    }
    cmd.env_remove(crate::agent::TAB_ENV);
    cmd.env_remove(crate::agent::TASK_ENV);
    for (key, value) in spec.env {
        cmd.env(key, value);
    }
    if let Some(path) = spec.path {
        cmd.env("PATH", path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", spec.bin.display()))?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or("the child has no stdout")?;
    let mut stderr = child.stderr.take().ok_or("the child has no stderr")?;
    let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stored = stderr_buf.clone();
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        if buf.len() > 4096 {
            buf.drain(..buf.len() - 4096);
        }
        *stored.lock().unwrap() = buf;
    });
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            on_event(ChildEvent::Line(line));
        }
        let code = child.wait().ok().and_then(|status| status.code());
        let _ = stderr_thread.join();
        let stderr = String::from_utf8_lossy(&stderr_buf.lock().unwrap())
            .trim()
            .to_owned();
        on_event(ChildEvent::Exited { code, stderr });
    });
    Ok(pid)
}

pub fn kill(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = crate::process::command("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }
}

struct Cache {
    at: Instant,
    list: String,
}

pub struct Delegates {
    next: AtomicU32,
    tasks: Mutex<Vec<Task>>,
    pids: Mutex<HashMap<String, u32>>,
    cursor: Mutex<Option<Cache>>,
}

impl Default for Delegates {
    fn default() -> Self {
        Self {
            next: AtomicU32::new(1),
            tasks: Mutex::new(vec![]),
            pids: Mutex::new(HashMap::new()),
            cursor: Mutex::new(None),
        }
    }
}

pub struct NewTask {
    pub parent: Parent,
    pub tool: Option<String>,
    pub title: String,
    pub account: AccountId,
    pub provider: Provider,
    pub model: Option<String>,
    pub cwd: PathBuf,
    pub mode: Mode,
    pub access: Access,
}

impl Delegates {
    pub fn begin(&self, new: NewTask) -> Task {
        let id = format!("task-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let task = Task {
            id,
            parent: new.parent,
            tool: new.tool,
            title: new.title,
            account: new.account,
            provider: new.provider,
            model: new.model,
            cwd: new.cwd,
            access: new.access,
            started: unix_now(),
            ended: None,
            state: WorkState::Working,
            summary: None,
            error: None,
            conversation: None,
            steps: vec![],
            mode: new.mode,
            delivered: false,
        };
        self.tasks.lock().unwrap().push(task.clone());
        task
    }

    pub fn remember_pid(&self, id: &str, pid: u32) {
        self.pids.lock().unwrap().insert(id.to_owned(), pid);
    }

    pub fn fail(&self, id: &str, error: String) {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.iter_mut().find(|task| task.id == id) {
            if task.state == WorkState::Working {
                task.state = WorkState::Failed;
                task.error = Some(error);
                task.ended = Some(unix_now());
            }
        }
    }

    /// Applies one child event. Returns the parent tab when an async task just finished.
    pub fn apply(&self, id: &str, event: ChildEvent) -> Option<String> {
        let mut tasks = self.tasks.lock().unwrap();
        let index = tasks.iter().position(|task| task.id == id)?;
        let became = match event {
            ChildEvent::Line(line) => {
                let provider = tasks[index].provider;
                let events = (driver(provider).parse)(&line);
                let mut became = false;
                for event in events {
                    became |= tasks[index].note(event);
                }
                became
            }
            ChildEvent::Exited { code, stderr } => tasks[index].exited(code, stderr),
        };
        (became && tasks[index].mode == Mode::Async)
            .then(|| root_tab(&tasks, tasks[index].parent.clone()))
            .flatten()
    }

    pub fn started_json(&self, id: &str) -> Result<Value, String> {
        let tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| format!("no task {id}"))?;
        Ok(json!({
            "taskId": task.id,
            "title": task.title,
            "providerInstanceId": task.account,
            "model": task.model,
            "workState": task.state.as_str(),
            "cwd": task.cwd.display().to_string(),
            "next": next_line(task),
        }))
    }

    pub fn status(&self, id: &str) -> Result<Value, String> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or_else(|| format!("no task {id}"))?;
        if task.state.terminal() {
            task.delivered = true;
        }
        Ok(status_json(task))
    }

    pub fn cancel(&self, id: &str, reason: &str) -> Result<Value, String> {
        let pid = {
            let mut tasks = self.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or_else(|| format!("no task {id}"))?;
            if task.state.terminal() {
                return Ok(status_json(task));
            }
            task.state = WorkState::Cancelled;
            task.error = Some(reason.to_owned());
            task.ended = Some(unix_now());
            self.pids.lock().unwrap().remove(id)
        };
        if let Some(pid) = pid {
            kill(pid);
        }
        self.status(id)
    }

    /// The notice for `tab`, and the task ids it covers. Delivery marks them, so a failed type can retry.
    pub fn pending_notice(&self, tab: &str) -> Option<(String, Vec<String>)> {
        let tasks = self.tasks.lock().unwrap();
        let ready: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                !task.delivered
                    && task.mode == Mode::Async
                    && matches!(task.state, WorkState::ResultAvailable | WorkState::Failed)
                    && root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab)
            })
            .map(|(index, _)| index)
            .collect();
        if ready.is_empty() {
            return None;
        }
        let lines = ready
            .iter()
            .map(|index| notice_line(&tasks[*index]))
            .collect::<Vec<_>>()
            .join("\n");
        let ids = ready.iter().map(|index| tasks[*index].id.clone()).collect();
        Some((lines, ids))
    }

    pub fn mark_delivered(&self, ids: &[String]) {
        let mut tasks = self.tasks.lock().unwrap();
        for task in tasks.iter_mut() {
            if ids.iter().any(|id| id == &task.id) {
                task.delivered = true;
            }
        }
    }

    pub fn lineage(&self, id: &str) -> Option<Lineage> {
        let tasks = self.tasks.lock().unwrap();
        let task = tasks.iter().find(|task| task.id == id)?;
        Some(Lineage {
            account: task.account.clone(),
            provider: task.provider,
            model: task.model.clone(),
            cwd: task.cwd.clone(),
            access: task.access,
        })
    }

    pub fn cards(&self, tab: &str) -> Vec<Subagent> {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .iter()
            .filter(|task| root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab))
            .map(|task| Subagent {
                id: task.id.clone(),
                tool: task.tool.clone().unwrap_or_default(),
                parent: match &task.parent {
                    Parent::Task(id) => Some(id.clone()),
                    Parent::Tab(_) => None,
                },
                name: task.title.clone(),
                kind: task.provider.spec().name.into(),
                model: task.model.clone(),
                state: card_state(task.state),
                updated: stamp(task.ended.unwrap_or(task.started)),
                steps: task.steps.clone(),
            })
            .collect()
    }

    pub fn steps_for(&self, tab: &str, tool: &str) -> Option<Vec<Item>> {
        if tool.is_empty() {
            return None;
        }
        let tasks = self.tasks.lock().unwrap();
        tasks
            .iter()
            .find(|task| {
                task.tool.as_deref() == Some(tool)
                    && root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab)
            })
            .map(|task| task.steps.clone())
    }

    pub fn open(&self, id: &str) -> Result<OpenInfo, String> {
        let tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| format!("no task {id}"))?;
        let conversation = task
            .conversation
            .clone()
            .ok_or_else(|| format!("{id} has no conversation to resume"))?;
        Ok(OpenInfo {
            cwd: task.cwd.clone(),
            account: task.account.clone(),
            provider: task.provider,
            conversation,
        })
    }

    pub fn running_under(&self, tab: &str) -> Vec<String> {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .iter()
            .filter(|task| {
                task.state == WorkState::Working
                    && root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab)
            })
            .map(|task| task.id.clone())
            .collect()
    }

    pub fn running(&self) -> Vec<String> {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .filter(|task| task.state == WorkState::Working)
            .map(|task| task.id.clone())
            .collect()
    }

    /// `--list-models` text, kept for ten minutes, or fifteen seconds when the CLI said nothing.
    pub fn cursor_catalog(&self, bin: &Path, path: Option<&str>) -> String {
        if let Some(cache) = self.cursor.lock().unwrap().as_ref() {
            let fresh = if cache.list.trim().is_empty() {
                MODEL_MISS
            } else {
                MODEL_CACHE
            };
            if cache.at.elapsed() < fresh {
                return cache.list.clone();
            }
        }
        let list = list_cursor_text(bin, path);
        *self.cursor.lock().unwrap() = Some(Cache {
            at: Instant::now(),
            list: list.clone(),
        });
        list
    }

    pub fn cursor_ids(&self, bin: &Path, path: Option<&str>) -> Vec<String> {
        parse_model_list(&self.cursor_catalog(bin, path))
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }
}

pub struct Lineage {
    pub account: AccountId,
    pub provider: Provider,
    pub model: Option<String>,
    pub cwd: PathBuf,
    pub access: Access,
}

impl Task {
    fn note(&mut self, event: StreamEvent) -> bool {
        if self.state != WorkState::Working {
            return false;
        }
        match event {
            StreamEvent::Conversation(id) => {
                self.conversation = Some(id);
                false
            }
            StreamEvent::Step(item) => {
                push_step(&mut self.steps, item);
                false
            }
            StreamEvent::Final { text, error } => {
                self.summary = Some(text);
                self.state = if error.is_some() {
                    WorkState::Failed
                } else {
                    WorkState::ResultAvailable
                };
                self.error = error;
                self.ended = Some(unix_now());
                true
            }
        }
    }

    fn exited(&mut self, code: Option<i32>, stderr: String) -> bool {
        if self.state != WorkState::Working {
            return false;
        }
        self.ended = Some(unix_now());
        if code == Some(0) {
            if let Some(text) = last_text(&self.steps) {
                self.summary = Some(text);
                self.state = WorkState::ResultAvailable;
                return true;
            }
        }
        self.state = WorkState::Failed;
        self.error = Some(if stderr.is_empty() {
            format!(
                "the agent exited ({})",
                code.map_or("no status".into(), |code| code.to_string())
            )
        } else {
            stderr
        });
        true
    }
}

fn next_line(task: &Task) -> String {
    match task.mode {
        Mode::Async => "keep working; pinned will type a notice into this session when it finishes; then call task_status".into(),
        Mode::Wait => format!("call task_status with taskId \"{}\"", task.id),
    }
}

fn status_json(task: &Task) -> Value {
    let end = task.ended.unwrap_or_else(unix_now);
    json!({
        "taskId": task.id,
        "workState": task.state.as_str(),
        "summary": task.summary,
        "error": task.error,
        "recentSteps": task.steps,
        "elapsedSecs": end.saturating_sub(task.started),
        "conversationId": task.conversation,
        "cwd": task.cwd.display().to_string(),
    })
}

fn notice_line(task: &Task) -> String {
    let who = match &task.model {
        Some(model) => format!("{} · {model}", task.provider.spec().name),
        None => task.provider.spec().name.to_owned(),
    };
    let outcome = match task.state {
        WorkState::Failed => format!(
            "failed: {}",
            task.error.as_deref().unwrap_or("the agent failed")
        ),
        _ => "result available".into(),
    };
    format!("[pinned] Delegated task {} \"{}\" ({who}) finished: {outcome}. Call task_status with taskId \"{}\".", task.id, task.title, task.id)
}

fn card_state(state: WorkState) -> subagents::State {
    match state {
        WorkState::Working => subagents::State::Running,
        WorkState::ResultAvailable => subagents::State::Done,
        WorkState::Failed | WorkState::Cancelled => subagents::State::Stopped,
    }
}

fn root_tab(tasks: &[Task], mut parent: Parent) -> Option<String> {
    for _ in 0..32 {
        match parent {
            Parent::Tab(tab) => return Some(tab),
            Parent::Task(id) => parent = tasks.iter().find(|task| task.id == id)?.parent.clone(),
        }
    }
    None
}

fn list_cursor_text(bin: &Path, path: Option<&str>) -> String {
    if !bin.is_file() {
        return String::new();
    }
    let mut cmd = crate::process::command(bin);
    cmd.arg("--list-models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    output_within(cmd, Duration::from_secs(10))
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

fn output_within(mut cmd: std::process::Command, limit: Duration) -> Option<std::process::Output> {
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    };
    // try_wait reaps the child, so the stdout thread is what still holds the bytes.
    let stdout = reader.join().ok()?;
    Some(std::process::Output {
        status,
        stdout,
        stderr: vec![],
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn stamp(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|time| time.to_rfc3339())
        .unwrap_or_default()
}

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
