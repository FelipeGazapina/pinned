use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration as StdDuration;

use crate::agent::{Message, ProjectTool, Role, When, ROLE_ENV, SOCK_ENV, TAB_ENV};
use crate::ipc::UnixStream;

pub const SUBCOMMAND: &str = "mcp";
pub const TOOL: &str = "remind_me";
const LIST_TOOL: &str = "list_reminders";
const CANCEL_TOOL: &str = "cancel_reminder";
pub const SERVER: &str = "pinned";
/// Leaves the reminders tools out, on a machine with no desktop to show them on.
pub const NO_REMINDERS: &str = "--no-reminders";

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

// Claude Code expands `${PINNED_SOCK}` from the session's own environment, and `${PINNED_ROLE:-}`
// to nothing in a session without a role.
pub fn write_mcp_config(path: &Path, exe: &Path, reminders: bool) -> std::io::Result<()> {
    let args = if reminders { vec![SUBCOMMAND] } else { vec![SUBCOMMAND, NO_REMINDERS] };
    let config = serde_json::json!({
        "mcpServers": { SERVER: {
            "command": exe,
            "args": args,
            "env": {
                SOCK_ENV: format!("${{{SOCK_ENV}}}"),
                TAB_ENV: format!("${{{TAB_ENV}:-}}"),
                ROLE_ENV: format!("${{{ROLE_ENV}:-}}"),
            },
        } }
    });
    std::fs::write(path, serde_json::to_vec_pretty(&config)?)
}

pub fn permissions() -> Vec<String> {
    let project = project_tools().map(|(_, tool)| tool["name"].as_str().unwrap().to_owned());
    [TOOL, LIST_TOOL, CANCEL_TOOL].map(String::from).into_iter().chain(project).map(|tool| format!("mcp__{SERVER}__{tool}")).collect()
}

pub fn serve(reminders: bool) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        // Notifications carry no id and get no answer.
        let Some(id) = request.get("id").cloned() else { continue };
        let outcome = match request["method"].as_str() {
            Some("initialize") => Ok(serde_json::json!({
                "protocolVersion": request["params"]["protocolVersion"],
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER, "version": env!("CARGO_PKG_VERSION") },
            })),
            Some("tools/list") => Ok(serde_json::json!({ "tools": tool_definitions(reminders, std::env::var(ROLE_ENV).ok().as_deref().and_then(Role::parse)) })),
            Some("tools/call") => Ok(call(&request["params"])),
            Some("ping") => Ok(serde_json::json!({})),
            other => Err(format!("method not found: {other:?}")),
        };
        let response = match outcome {
            Ok(result) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } }),
        };
        if writeln!(stdout, "{response}").and_then(|_| stdout.flush()).is_err() {
            return;
        }
    }
}

fn tool_definitions(reminders: bool, role: Option<Role>) -> serde_json::Value {
    let project = project_tools().into_iter().filter(|(orchestrator_only, _)| role.is_some_and(|r| r == Role::Orchestrator || !orchestrator_only)).map(|(_, tool)| tool);
    let mut tools = if reminders { reminder_tools() } else { vec![] };
    tools.extend(project);
    serde_json::Value::Array(tools)
}

fn reminder_tools() -> Vec<serde_json::Value> {
    vec![
        remind_definition(),
        serde_json::json!({
            "name": LIST_TOOL,
            "description": "List the reminders still pending, each with its id, time and message.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        serde_json::json!({
            "name": CANCEL_TOOL,
            "description": "Cancel a pending reminder by the id list_reminders or remind_me gave.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "The reminder's id." } },
                "required": ["id"],
            },
        }),
    ]
}

/// Each project tool, named as `ProjectTool` tags it, and whether only an orchestrator has it.
fn project_tools() -> [(bool, serde_json::Value); 5] {
    let no_args = serde_json::json!({ "type": "object", "properties": {} });
    [
        (true, serde_json::json!({
            "name": "delegate",
            "description": "Start a worker thread on a task. It works in its own git worktree when the project is a repository, and you get a message when it finishes or needs input. Returns the worker's tab id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "A self-contained brief: the goal, the files involved, the constraints, and how to verify the result." },
                    "title": { "type": "string", "description": "A few words naming the task, shown on the worker's thread." },
                },
                "required": ["task"],
            },
        })),
        (true, serde_json::json!({
            "name": "agents",
            "description": "List your workers, each with its tab id, title, status and folder.",
            "inputSchema": no_args,
        })),
        (false, serde_json::json!({
            "name": "remember",
            "description": "Save a durable fact about this project, such as a user preference, a decision, a convention or an architecture fact. Every later session of the project's orchestrator and its workers starts with it.",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string", "description": "The fact, in one or two sentences." } },
                "required": ["text"],
            },
        })),
        (false, serde_json::json!({
            "name": "forget",
            "description": "Remove a memory that is wrong or no longer holds, by the id memories lists.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "The memory's id." } },
                "required": ["id"],
            },
        })),
        (false, serde_json::json!({
            "name": "memories",
            "description": "List this project's memories with their ids.",
            "inputSchema": no_args,
        })),
    ]
}

fn remind_definition() -> serde_json::Value {
    serde_json::json!({
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

fn call(params: &serde_json::Value) -> serde_json::Value {
    let args = &params["arguments"];
    let request = match params["name"].as_str() {
        Some(TOOL) => remind_request(args),
        Some(LIST_TOOL) => Ok(Message::ListReminders),
        Some(CANCEL_TOOL) => args["id"].as_u64().map(|id| Message::CancelReminder { id }).ok_or("id is required".to_owned()),
        Some(name) => project_request(name, args, std::env::var(TAB_ENV).ok()),
        None => Err(format!("unknown tool {}", params["name"])),
    };
    let reply = request.and_then(|request| send(&request));
    let (text, is_error) = match reply {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    serde_json::json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn project_request(name: &str, args: &serde_json::Value, tab: Option<String>) -> Result<Message, String> {
    let mut tool = if args.is_object() { args.clone() } else { serde_json::json!({}) };
    tool["tool"] = name.into();
    let tool: ProjectTool = serde_json::from_value(tool).map_err(|e| format!("{name}: {e}"))?;
    let tab = tab.filter(|t| !t.is_empty()).ok_or_else(|| format!("{TAB_ENV} is not set: run this inside a pinned tab"))?;
    Ok(Message::Project { tab, tool })
}

fn send(message: &Message) -> Result<String, String> {
    let sock = std::env::var(SOCK_ENV).map_err(|_| format!("{SOCK_ENV} is not set: run this inside a pinned tab"))?;
    let mut stream = UnixStream::connect(sock).map_err(|e| format!("pinned is not running: {e}"))?;
    let _ = stream.set_read_timeout(Some(StdDuration::from_secs(10)));
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
    fn a_session_gets_the_project_tools_its_role_has() {
        let names = |role| tool_definitions(true, role).as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(names(None), ["remind_me", "list_reminders", "cancel_reminder"]);
        assert_eq!(names(Some(Role::Worker)), ["remind_me", "list_reminders", "cancel_reminder", "remember", "forget", "memories"]);
        assert_eq!(names(Some(Role::Orchestrator)), ["remind_me", "list_reminders", "cancel_reminder", "delegate", "agents", "remember", "forget", "memories"]);
        let on_a_node = tool_definitions(false, Some(Role::Worker)).as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(on_a_node, ["remember", "forget", "memories"]);
        assert_eq!(permissions()[3..], ["mcp__pinned__delegate", "mcp__pinned__agents", "mcp__pinned__remember", "mcp__pinned__forget", "mcp__pinned__memories"]);

        let ask = |name, args, tab: Option<&str>| project_request(name, &args, tab.map(Into::into)).map(|m| serde_json::to_string(&m).unwrap());
        assert_eq!(ask("forget", serde_json::json!({"id": 3}), Some("tab-2")), Ok(r#"{"kind":"project","tab":"tab-2","tool":"forget","id":3}"#.into()));
        assert_eq!(ask("agents", serde_json::Value::Null, Some("tab-2")), Ok(r#"{"kind":"project","tab":"tab-2","tool":"agents"}"#.into()));
        assert_eq!(ask("delegate", serde_json::json!({}), Some("tab-2")), Err("delegate: missing field `task`".into()));
        assert_eq!(ask("memories", serde_json::json!({}), Some("")), Err("PINNED_TAB is not set: run this inside a pinned tab".into()));
    }

    #[test]
    fn the_mcp_server_hears_which_tab_and_role_runs_it() {
        let file = std::env::temp_dir().join(format!("pinned-mcp-config-{}.json", std::process::id()));
        write_mcp_config(&file, Path::new("/opt/pinned/bin/pinned"), true).unwrap();
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["pinned"]["env"], serde_json::json!({ "PINNED_SOCK": "${PINNED_SOCK}", "PINNED_TAB": "${PINNED_TAB:-}", "PINNED_ROLE": "${PINNED_ROLE:-}" }));
        assert_eq!(config["mcpServers"]["pinned"]["args"], serde_json::json!(["mcp"]));
        write_mcp_config(&file, Path::new("/opt/pinned/bin/pinned"), false).unwrap();
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["pinned"]["args"], serde_json::json!(["mcp", "--no-reminders"]));
        let _ = std::fs::remove_file(file);
    }
}
