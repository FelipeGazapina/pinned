use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use crate::agent::{Message, When, SOCK_ENV, TAB_ENV, TASK_ENV};
use crate::delegate::{self, Mode};
use crate::ipc::UnixStream;

pub const SUBCOMMAND: &str = "mcp";
pub const TOOL: &str = "remind_me";
const LIST_TOOL: &str = "list_reminders";
const CANCEL_TOOL: &str = "cancel_reminder";
const CAPABILITIES_TOOL: &str = "orchestrator_capabilities";
const DELEGATE_TOOL: &str = "delegate_task";
const STATUS_TOOL: &str = "task_status";
const CANCEL_TASK_TOOL: &str = "task_cancel";
pub const SERVER: &str = "pinned";
const NO_REMINDERS: &str = "--no-reminders";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Reminder {
    pub id: u64,
    pub message: String,
    pub due_unix_secs: u64,
}

pub struct Reminders {
    file: Option<PathBuf>,
    pending: Mutex<Vec<Reminder>>,
}

impl Reminders {
    pub fn load(config_dir: Option<PathBuf>) -> Self {
        let file = config_dir.map(|c| c.join("reminders.json"));
        let pending = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { file, pending: Mutex::new(pending) }
    }

    pub fn add(&self, message: String, when: &When) -> Result<String, String> {
        let now = Local::now();
        let due = due_time(now, when)?;
        let mut pending = self.pending.lock().unwrap();
        let id = pending.iter().map(|r| r.id).max().map_or(1, |last| last + 1);
        pending.push(Reminder { id, message, due_unix_secs: due.timestamp().max(0) as u64 });
        self.save(&pending);
        Ok(format!("Reminder #{id} set for {}.", due.format("%H:%M on %Y-%m-%d")))
    }

    pub fn list(&self) -> String {
        let mut pending = self.pending.lock().unwrap().clone();
        pending.sort_by_key(|r| r.due_unix_secs);
        if pending.is_empty() {
            return "No reminders pending.".into();
        }
        let lines: Vec<String> = pending
            .iter()
            .map(|r| {
                let due = Local.timestamp_opt(r.due_unix_secs as i64, 0).single().map_or_else(|| r.due_unix_secs.to_string(), |d| d.format("%H:%M on %Y-%m-%d").to_string());
                format!("#{} at {due}: {}", r.id, r.message)
            })
            .collect();
        lines.join("\n")
    }

    pub fn cancel(&self, id: u64) -> Result<String, String> {
        let mut pending = self.pending.lock().unwrap();
        let before = pending.len();
        pending.retain(|r| r.id != id);
        if pending.len() == before {
            return Err(format!("No pending reminder #{id}."));
        }
        self.save(&pending);
        Ok(format!("Reminder #{id} cancelled."))
    }

    /// Removes and returns the reminders due by `now`, in unix seconds.
    pub fn take_due(&self, now: u64) -> Vec<Reminder> {
        let mut pending = self.pending.lock().unwrap();
        let (due, later): (Vec<_>, Vec<_>) = pending.drain(..).partition(|r| r.due_unix_secs <= now);
        *pending = later;
        if !due.is_empty() {
            self.save(&pending);
        }
        due
    }

    fn save(&self, pending: &[Reminder]) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, serde_json::to_vec_pretty(pending).unwrap());
    }
}

fn due_time<Tz: TimeZone>(now: DateTime<Tz>, when: &When) -> Result<DateTime<Tz>, String> {
    match when {
        When::InMinutes(minutes) if minutes.is_finite() && *minutes > 0.0 => Ok(now + Duration::seconds((minutes * 60.0).round() as i64)),
        When::InMinutes(_) => Err("in_minutes must be a number above 0".into()),
        When::At(at) => {
            let time = NaiveTime::parse_from_str(at.trim(), "%H:%M").map_err(|_| format!("at must be HH:MM in 24-hour time, got {at:?}"))?;
            next_at(&now, time).ok_or_else(|| format!("{at} does not exist today (clock change)"))
        }
    }
}

fn next_at<Tz: TimeZone>(now: &DateTime<Tz>, time: NaiveTime) -> Option<DateTime<Tz>> {
    let tz = now.timezone();
    [0, 1].into_iter().find_map(|days| {
        let day = now.date_naive() + Duration::days(days);
        let candidate = tz.from_local_datetime(&day.and_time(time)).earliest()?;
        (candidate > *now).then_some(candidate)
    })
}

// Claude Code expands `${PINNED_SOCK}` from the session's own environment. A node passes
// `reminders: false` so the same server can delegate without offering desktop notifications.
pub fn write_mcp_config(path: &Path, exe: &Path, reminders: bool) -> std::io::Result<()> {
    let mut args = vec![SUBCOMMAND.to_owned()];
    if !reminders {
        args.push(NO_REMINDERS.to_owned());
    }
    let config = json!({
        "mcpServers": { SERVER: {
            "command": exe,
            "args": args,
            "env": {
                SOCK_ENV: format!("${{{SOCK_ENV}}}"),
                TAB_ENV: format!("${{{TAB_ENV}}}"),
                TASK_ENV: format!("${{{TASK_ENV}}}"),
            },
        } }
    });
    std::fs::write(path, serde_json::to_vec_pretty(&config)?)
}

pub fn permissions() -> Vec<String> {
    [TOOL, LIST_TOOL, CANCEL_TOOL, CAPABILITIES_TOOL, DELEGATE_TOOL, STATUS_TOOL, CANCEL_TASK_TOOL]
        .map(|tool| format!("mcp__{SERVER}__{tool}"))
        .into()
}

pub fn serve(args: &[String]) {
    let reminders = !args.iter().any(|arg| arg == NO_REMINDERS);
    let stdin = std::io::stdin();
    let _ = serve_io(reminders, stdin.lock(), std::io::stdout());
}

/// One JSON-RPC session. `reminders` hides the desktop tools; delegation stays.
/// Each `tools/call` runs on its own thread, so a wait does not block the next call.
pub fn serve_io(reminders: bool, input: impl BufRead, output: impl Write + Send + 'static) -> std::io::Result<()> {
    let output = Arc::new(Mutex::new(output));
    let mut calls = Vec::new();
    for line in input.lines() {
        let line = line?;
        let Some(request) = serde_json::from_str::<Value>(&line).ok() else { continue };
        let Some(id) = request.get("id").cloned() else { continue };
        if request["method"] == "tools/call" {
            let output = output.clone();
            let params = request["params"].clone();
            calls.push(std::thread::spawn(move || {
                let result = call(&params, reminders, &output);
                let _ = write_msg(&output, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }));
            continue;
        }
        let outcome = match request["method"].as_str() {
            Some("initialize") => Ok(json!({
                "protocolVersion": request["params"]["protocolVersion"],
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER, "version": env!("CARGO_PKG_VERSION") },
            })),
            Some("tools/list") => Ok(json!({ "tools": tool_definitions(reminders) })),
            Some("ping") => Ok(json!({})),
            other => Err(format!("method not found: {other:?}")),
        };
        let response = match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } }),
        };
        if write_msg(&output, &response).is_err() {
            return Ok(());
        }
    }
    for call in calls {
        let _ = call.join();
    }
    Ok(())
}

fn write_msg(output: &Mutex<impl Write>, value: &Value) -> std::io::Result<()> {
    let mut output = output.lock().unwrap();
    writeln!(output, "{value}")?;
    output.flush()
}

fn tool_definitions(reminders: bool) -> Value {
    let mut tools = vec![
        json!({
            "name": CAPABILITIES_TOOL,
            "description": "The accounts this machine can run a child agent on, and the access this session inherits.",
            "inputSchema": {
                "type": "object",
                "properties": { "search": { "type": "string", "description": "Filters accounts and models." } },
            },
        }),
        json!({
            "name": DELEGATE_TOOL,
            "description": "Run a child agent on a signed-in account. The child receives only task, with no parent history. mode \"async\" (the default) returns at once: pinned types a notice into this session when the child finishes, so end your turn and do not poll. mode \"wait\" blocks until the child finishes and returns the task_status body, including summary and error. A wait that times out does not cancel the child; the notice still comes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "The whole assignment the child sees. It does not see this session's history." },
                    "title": { "type": "string", "description": "Short label for the card. The first line of task is used when this is omitted." },
                    "target": {
                        "type": "object",
                        "description": "Which account and model run the child. Omit providerInstanceId to use this session's account.",
                        "properties": {
                            "providerInstanceId": { "type": "string", "description": "An account id from orchestrator_capabilities." },
                            "model": { "type": "string", "description": "A model id that account lists. Omit it on the same provider to keep this session's model. Omit it on a different provider to use that provider's default." },
                            "options": {
                                "type": "object",
                                "description": "Model options. Pass effort or reasoning_effort, not both.",
                                "properties": {
                                    "effort": { "type": "string", "description": "Reasoning effort, one of the model's option values." },
                                    "reasoning_effort": { "type": "string", "description": "Same as effort." },
                                },
                            },
                        },
                    },
                    "isolation": { "type": "string", "enum": ["shared", "worktree"], "description": "shared runs in this checkout. worktree makes a separate checkout and runs the child there." },
                    "mode": { "type": "string", "enum": ["async", "wait"], "default": "async", "description": "async returns at once and types a notice into this session when the child finishes, so end your turn and do not poll. wait blocks until the result is in this call." },
                    "timeoutMs": { "type": "number", "description": "How long wait blocks, in milliseconds. The child keeps running after a timeout, and the notice still comes." },
                    "interactionMode": { "type": "string", "enum": ["inherit", "default", "plan"], "description": "plan is read-only. inherit and default leave this session's mode as it is." },
                    "runtimeMode": { "type": "string", "enum": ["full-access", "auto"], "description": "How wide the child may act. Anything wider than this session is rejected. A bypassed session can hand the child auto. Omit it to inherit this session's mode." },
                    "role": { "type": "string", "description": "A label for the child, such as explore. The child does not see it." },
                    "clientRequestId": { "type": "string", "description": "The same caller and clientRequestId returns the task already started. If that task has finished, the answer is its status, including summary and error. A new attempt uses a new id." },
                },
                "required": ["task"],
            },
        }),
        json!({
            "name": STATUS_TOOL,
            "description": "The child task's state. Reading a finished task marks its result delivered.",
            "inputSchema": {
                "type": "object",
                "properties": { "taskId": { "type": "string" } },
                "required": ["taskId"],
            },
        }),
        json!({
            "name": CANCEL_TASK_TOOL,
            "description": "Stop a child task. A finished task keeps its result.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "taskId": { "type": "string" },
                    "reason": { "type": "string" },
                },
                "required": ["taskId"],
            },
        }),
    ];
    if reminders {
        tools.insert(0, remind_definition());
        tools.insert(1, json!({
            "name": LIST_TOOL,
            "description": "List the reminders still pending, each with its id, time and message.",
            "inputSchema": { "type": "object", "properties": {} },
        }));
        tools.insert(2, json!({
            "name": CANCEL_TOOL,
            "description": "Cancel a pending reminder by the id list_reminders or remind_me gave.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "The reminder's id." } },
                "required": ["id"],
            },
        }));
    }
    Value::Array(tools)
}

fn remind_definition() -> Value {
    json!({
        "name": TOOL,
        "description": "Show the user a desktop notification later. Pass exactly one of in_minutes (for \"in 5, 10, 20 minutes\") or at (for a clock time such as \"7pm\", written as 24-hour HH:MM in the user's local time). at means the next time the clock reads that, so tomorrow if it already passed today.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "What the notification says." },
                "in_minutes": { "type": "number", "description": "Minutes from now." },
                "at": { "type": "string", "description": "Local 24-hour time, HH:MM." },
            },
            "required": ["message"],
        },
    })
}

fn remind_request(args: &serde_json::Value) -> Result<Message, String> {
    let message = args["message"].as_str().ok_or("message is required")?.to_owned();
    let when = match (args["in_minutes"].as_f64(), args["at"].as_str()) {
        (Some(minutes), None) => When::InMinutes(minutes),
        (None, Some(at)) => When::At(at.to_owned()),
        _ => return Err("pass exactly one of in_minutes or at".into()),
    };
    Ok(Message::Remind { message, when })
}

fn call(params: &Value, reminders: bool, output: &Mutex<impl Write>) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    if !reminders && matches!(name, TOOL | LIST_TOOL | CANCEL_TOOL) {
        return tool_result(Err("reminders are not available on this machine".into()));
    }
    if name == DELEGATE_TOOL {
        return delegate_call(params, output);
    }
    let args = &params["arguments"];
    let request = match name {
        TOOL => remind_request(args),
        LIST_TOOL => Ok(Message::ListReminders),
        CANCEL_TOOL => args["id"].as_u64().map(|id| Message::CancelReminder { id }).ok_or("id is required".to_owned()),
        CAPABILITIES_TOOL => Ok(Message::Capabilities {
            tab: session_var(TAB_ENV),
            task: session_var(TASK_ENV),
            search: args["search"].as_str().unwrap_or("").to_owned(),
        }),
        STATUS_TOOL => required(args, "taskId").map(|id| Message::TaskStatus { tab: session_var(TAB_ENV), task: session_var(TASK_ENV), id }),
        CANCEL_TASK_TOOL => required(args, "taskId").map(|id| Message::TaskCancel {
            tab: session_var(TAB_ENV),
            task: session_var(TASK_ENV),
            id,
            reason: args["reason"].as_str().unwrap_or("the parent asked").to_owned(),
        }),
        _ => Err(format!("unknown tool {name}")),
    };
    tool_result(request.and_then(|request| send(&request)))
}

fn delegate_call(params: &Value, output: &Mutex<impl Write>) -> Value {
    let mut request = match delegate::parse_request(&params["arguments"]) {
        Ok(request) => request,
        Err(error) => return tool_result(Err(error)),
    };
    let tab = session_var(TAB_ENV);
    let parent = session_var(TASK_ENV);
    if tab.is_none() {
        request.mode = Mode::Wait;
    }
    let wait = request.mode == Mode::Wait;
    let timeout = StdDuration::from_millis(request.timeout_ms);
    let tool = params.get("_meta").and_then(|meta| meta.get("claudecode/toolUseId")).and_then(Value::as_str).map(str::to_owned);
    let progress = params.get("_meta").and_then(|meta| meta.get("progressToken")).filter(|token| !token.is_null()).cloned();
    let started = match send(&Message::Delegate { tab: tab.clone(), task: parent.clone(), tool, request: Box::new(request) }) {
        Ok(text) => text,
        Err(error) => return tool_result(Err(error)),
    };
    if !wait {
        return tool_result(Ok(started));
    }
    let value: Value = serde_json::from_str(&started).unwrap_or(json!({ "raw": started }));
    let id = value["taskId"].as_str().unwrap_or("").to_owned();
    if id.is_empty() {
        return tool_result(Ok(started));
    }
    let deadline = Instant::now() + timeout;
    let began = Instant::now();
    let mut next_note = began + progress_every();
    let mut latest = String::new();
    loop {
        if progress.is_some() && Instant::now() >= next_note {
            let note = json!({
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": { "progressToken": progress.clone().unwrap_or(Value::Null), "progress": began.elapsed().as_secs(), "message": latest },
            });
            let _ = write_msg(output, &note);
            next_note = Instant::now() + progress_every();
        }
        match send(&Message::TaskStatus { tab: tab.clone(), task: parent.clone(), id: id.clone() }) {
            Ok(text) => {
                let mut status: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                latest = latest_step(&status);
                let state = status["workState"].as_str().unwrap_or("");
                if state != "working" && state != "waiting_for_children" {
                    status["next"] = json!("the result is in summary");
                    return tool_result(Ok(status.to_string()));
                }
            }
            Err(error) => return tool_result(Err(error)),
        }
        if Instant::now() >= deadline {
            let mut pending = value.clone();
            pending["next"] = json!("still working; the wait timed out; pinned will type a notice into this session when it finishes");
            return tool_result(Ok(pending.to_string()));
        }
        let pause = progress_every().min(StdDuration::from_millis(200));
        std::thread::sleep(pause);
    }
}

fn progress_every() -> StdDuration {
    std::env::var("PINNED_DELEGATE_PROGRESS_MS").ok().and_then(|text| text.parse().ok()).filter(|ms: &u64| *ms > 0).map(StdDuration::from_millis).unwrap_or(StdDuration::from_secs(10))
}

fn latest_step(status: &Value) -> String {
    let Some(step) = status["recentSteps"].as_array().and_then(|steps| steps.last()) else { return String::new() };
    match step["kind"].as_str() {
        Some("text" | "thinking" | "error" | "note") => step["text"].as_str().unwrap_or("").to_owned(),
        Some("tool") => step["summary"].as_str().filter(|text| !text.is_empty()).or(step["name"].as_str()).unwrap_or("").to_owned(),
        Some("result") => step["text"].as_str().unwrap_or("").to_owned(),
        _ => String::new(),
    }
}

fn session_var(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty() && !value.starts_with('$'))
}

fn required(args: &Value, key: &str) -> Result<String, String> {
    args[key].as_str().filter(|value| !value.is_empty()).map(str::to_owned).ok_or_else(|| format!("{key} is required"))
}

fn tool_result(reply: Result<String, String>) -> Value {
    let (text, is_error) = match reply {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn send(message: &Message) -> Result<String, String> {
    let timeout = match message {
        Message::Delegate { .. } => StdDuration::from_secs(120),
        _ => StdDuration::from_secs(10),
    };
    let sock = std::env::var(SOCK_ENV).map_err(|_| format!("{SOCK_ENV} is not set: run this inside a pinned tab"))?;
    let mut stream = UnixStream::connect(sock).map_err(|e| format!("pinned is not running: {e}"))?;
    let _ = stream.set_read_timeout(Some(timeout));
    writeln!(stream, "{}", serde_json::to_string(message).unwrap()).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).map_err(|e| format!("pinned did not answer: {e}"))?;
    serde_json::from_str::<Result<String, String>>(&reply).map_err(|e| format!("pinned sent a bad answer {reply:?}: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at_noon() -> DateTime<FixedOffset> {
        FixedOffset::west_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 10, 21, 12, 0, 0).unwrap()
    }

    #[test]
    fn minutes_count_from_now() {
        let due = due_time(at_noon(), &When::InMinutes(20.0)).unwrap();
        assert_eq!(due.to_rfc3339(), "2026-10-21T12:20:00-03:00");
    }

    #[test]
    fn clock_time_later_today_stays_today() {
        let due = due_time(at_noon(), &When::At("19:00".into())).unwrap();
        assert_eq!(due.to_rfc3339(), "2026-10-21T19:00:00-03:00");
    }

    #[test]
    fn clock_time_already_passed_rolls_to_tomorrow() {
        let due = due_time(at_noon(), &When::At("07:30".into())).unwrap();
        assert_eq!(due.to_rfc3339(), "2026-10-22T07:30:00-03:00");
        let due = due_time(at_noon(), &When::At("12:00".into())).unwrap();
        assert_eq!(due.to_rfc3339(), "2026-10-22T12:00:00-03:00");
    }

    #[test]
    fn bad_requests_are_refused() {
        assert!(due_time(at_noon(), &When::InMinutes(0.0)).is_err());
        assert!(due_time(at_noon(), &When::InMinutes(-3.0)).is_err());
        assert!(due_time(at_noon(), &When::At("7pm".into())).is_err());
        assert!(due_time(at_noon(), &When::At("25:00".into())).is_err());
    }

    #[test]
    fn only_due_reminders_are_taken_and_they_are_remembered_across_launches() {
        let dir = std::env::temp_dir().join(format!("pinned-reminders-{}", std::process::id()));
        let reminders = Reminders::load(Some(dir.clone()));
        *reminders.pending.lock().unwrap() = vec![
            Reminder { id: 1, message: "soon".into(), due_unix_secs: 100 },
            Reminder { id: 2, message: "later".into(), due_unix_secs: 200 },
        ];
        reminders.save(&reminders.pending.lock().unwrap());

        let reloaded = Reminders::load(Some(dir.clone()));
        assert_eq!(reloaded.take_due(150), vec![Reminder { id: 1, message: "soon".into(), due_unix_secs: 100 }]);
        assert_eq!(reloaded.take_due(150), vec![]);
        assert_eq!(Reminders::load(Some(dir.clone())).take_due(300), vec![Reminder { id: 2, message: "later".into(), due_unix_secs: 200 }]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_tool_wants_exactly_one_of_minutes_or_a_clock_time() {
        let ask = |args| remind_request(&args).map(|m| format!("{m:?}"));
        assert_eq!(ask(serde_json::json!({"message": "x", "in_minutes": 5})), Ok(r#"Remind { message: "x", when: InMinutes(5.0) }"#.into()));
        assert_eq!(ask(serde_json::json!({"message": "x", "at": "19:00"})), Ok(r#"Remind { message: "x", when: At("19:00") }"#.into()));
        let both = serde_json::json!({"message": "x", "in_minutes": 5, "at": "19:00"});
        assert_eq!(ask(both), Err("pass exactly one of in_minutes or at".into()));
        assert_eq!(ask(serde_json::json!({"message": "x"})), Err("pass exactly one of in_minutes or at".into()));
        assert_eq!(ask(serde_json::json!({"in_minutes": 5})), Err("message is required".into()));
    }

    #[test]
    fn reminders_get_increasing_ids_and_can_be_cancelled() {
        let reminders = Reminders::load(None);
        assert_eq!(reminders.list(), "No reminders pending.");
        assert!(reminders.add("one".into(), &When::InMinutes(10.0)).unwrap().starts_with("Reminder #1 set"));
        assert!(reminders.add("two".into(), &When::InMinutes(5.0)).unwrap().starts_with("Reminder #2 set"));
        let listed = reminders.list();
        let lines: Vec<_> = listed.lines().collect();
        assert!(lines[0].starts_with("#2 at ") && lines[0].ends_with(": two"), "{listed}");
        assert!(lines[1].starts_with("#1 at ") && lines[1].ends_with(": one"), "{listed}");

        assert_eq!(reminders.cancel(2), Ok("Reminder #2 cancelled.".to_owned()));
        assert_eq!(reminders.cancel(2), Err("No pending reminder #2.".to_owned()));
        assert!(reminders.add("three".into(), &When::InMinutes(1.0)).unwrap().starts_with("Reminder #2 set"));
    }

    #[test]
    fn delegate_task_names_every_argument() {
        let tools = tool_definitions(true);
        let tool = tools.as_array().unwrap().iter().find(|tool| tool["name"] == "delegate_task").unwrap();
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        let mut names: Vec<_> = props.keys().cloned().collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "clientRequestId".to_owned(),
                "interactionMode".into(),
                "isolation".into(),
                "mode".into(),
                "role".into(),
                "runtimeMode".into(),
                "target".into(),
                "task".into(),
                "timeoutMs".into(),
                "title".into(),
            ]
        );
        assert_eq!(props["mode"]["default"], "async");
        assert_eq!(props["mode"]["enum"], json!(["async", "wait"]));
        assert_eq!(props["isolation"]["enum"], json!(["shared", "worktree"]));
        assert_eq!(props["interactionMode"]["enum"], json!(["inherit", "default", "plan"]));
        assert_eq!(props["runtimeMode"]["enum"], json!(["full-access", "auto"]));
        assert_eq!(props["task"]["description"], "The whole assignment the child sees. It does not see this session's history.");
        assert_eq!(
            props["clientRequestId"]["description"],
            "The same caller and clientRequestId returns the task already started. If that task has finished, the answer is its status, including summary and error. A new attempt uses a new id."
        );
        assert_eq!(props["isolation"]["description"], "shared runs in this checkout. worktree makes a separate checkout and runs the child there.");
        assert_eq!(props["target"]["properties"]["options"]["properties"]["effort"]["type"], "string");
        assert_eq!(
            props["target"]["properties"]["model"]["description"],
            "A model id that account lists. Omit it on the same provider to keep this session's model. Omit it on a different provider to use that provider's default."
        );
        assert_eq!(props["target"]["properties"]["options"]["properties"]["reasoning_effort"]["type"], "string");
    }

    #[test]
    #[cfg(unix)]
    fn a_wait_returns_the_summary_while_a_second_call_answers_and_progress_is_sent() {
        use std::os::unix::net::UnixListener;
        use std::sync::{Arc, Mutex};
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("pinned-mcp-wait-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let previous = (
            std::env::var(SOCK_ENV).ok(),
            std::env::var(TAB_ENV).ok(),
            std::env::var(TASK_ENV).ok(),
            std::env::var("PINNED_DELEGATE_PROGRESS_MS").ok(),
        );
        std::env::set_var(TAB_ENV, "tab-1");
        std::env::remove_var(TASK_ENV);
        std::env::set_var("PINNED_DELEGATE_PROGRESS_MS", "200");

        let sock = dir.join("sock");
        let listener = UnixListener::bind(&sock).unwrap();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                let mut line = String::new();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let msg: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                let body = if msg.get("Delegate").is_some() {
                    json!({"taskId":"task-abc","title":"say ship","providerInstanceId":"claude","model":null,"workState":"working","cwd":"/tmp","next":"call task_status with taskId \"task-abc\""})
                } else if started.elapsed() < StdDuration::from_millis(1300) {
                    json!({"taskId":"task-abc","workState":"working","summary":null,"error":null,"recentSteps":[{"kind":"tool","id":"1","at":"","name":"Bash","summary":"echo hi","detail":"echo hi","args":[]}],"elapsedSecs":1,"conversationId":null,"cwd":"/tmp"})
                } else {
                    json!({"taskId":"task-abc","workState":"result_available","summary":"Ship it","error":null,"recentSteps":[{"kind":"text","id":"2","at":"","text":"Ship it"}],"elapsedSecs":2,"conversationId":null,"cwd":"/tmp"})
                };
                let _ = writeln!(stream, "{}", serde_json::to_string(&Ok::<_, String>(body.to_string())).unwrap());
            }
        });
        std::env::set_var(SOCK_ENV, &sock);
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let seen = Arc::new(Mutex::new(false));
        let watch = buf.clone();
        let flag = seen.clone();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while start.elapsed() < StdDuration::from_secs(8) {
                let text = String::from_utf8_lossy(&watch.lock().unwrap()).into_owned();
                if text.contains("\"id\":2") && !text.contains("Ship it") {
                    *flag.lock().unwrap() = true;
                    return;
                }
                std::thread::sleep(StdDuration::from_millis(20));
            }
        });
        let input = format!(
            "{}\n{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delegate_task","arguments":{"task":"say ship","mode":"wait","timeoutMs":5000},"_meta":{"progressToken":7}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"ping"})
        );
        serve_io(true, std::io::Cursor::new(input.into_bytes()), Shared(buf.clone())).unwrap();
        assert!(*seen.lock().unwrap(), "ping did not answer while the wait was still running");
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let mut progress_ok = false;
        let mut body = None;
        for line in text.lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            if value["method"] == "notifications/progress"
                && value["params"]["progressToken"] == 7
                && value["params"]["progress"] == 1
                && value["params"]["message"] == "echo hi"
            {
                progress_ok = true;
            }
            if value["id"] == 1 {
                body = Some(serde_json::from_str::<Value>(value["result"]["content"][0]["text"].as_str().unwrap()).unwrap());
            }
            if value["id"] == 2 {
                assert_eq!(value["result"], json!({}));
            }
        }
        assert!(progress_ok, "{text}");
        let body = body.unwrap();
        assert_eq!(body["taskId"], "task-abc");
        assert_eq!(body["workState"], "result_available");
        assert_eq!(body["summary"], "Ship it");
        assert_eq!(body["error"], Value::Null);
        assert_eq!(body["next"], "the result is in summary");

        let sock = dir.join("timeout.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let cancels = Arc::new(Mutex::new(0u32));
        let count = cancels.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                let mut line = String::new();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let msg: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                if msg.get("TaskCancel").is_some() {
                    *count.lock().unwrap() += 1;
                }
                let body = json!({"taskId":"task-slow","title":"say ship","providerInstanceId":"claude","model":null,"workState":"working","summary":null,"error":null,"cwd":"/tmp","next":"call task_status with taskId \"task-slow\""});
                let _ = writeln!(stream, "{}", serde_json::to_string(&Ok::<_, String>(body.to_string())).unwrap());
            }
        });
        std::env::set_var(SOCK_ENV, &sock);
        let input = format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delegate_task","arguments":{"task":"say ship","mode":"wait","timeoutMs":400}}})
        );
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        serve_io(true, std::io::Cursor::new(input.into_bytes()), Shared(buf.clone())).unwrap();
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let value: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let body: Value = serde_json::from_str(value["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["workState"], "working");
        assert_eq!(body["taskId"], "task-slow");
        assert_eq!(body["next"], "still working; the wait timed out; pinned will type a notice into this session when it finishes");
        assert_eq!(*cancels.lock().unwrap(), 0);

        restore_var(SOCK_ENV, previous.0);
        restore_var(TAB_ENV, previous.1);
        restore_var(TASK_ENV, previous.2);
        restore_var("PINNED_DELEGATE_PROGRESS_MS", previous.3);
        let _ = std::fs::remove_dir_all(dir);
    }

struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn restore_var(name: &str, value: Option<String>) {
    match value {
        Some(value) => std::env::set_var(name, value),
        None => std::env::remove_var(name),
    }
}
}
