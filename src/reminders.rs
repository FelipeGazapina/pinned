use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration as StdDuration;

use crate::agent::{Message, When, SOCK_ENV};

pub const SUBCOMMAND: &str = "mcp";
pub const TOOL: &str = "remind_me";
const LIST_TOOL: &str = "list_reminders";
const CANCEL_TOOL: &str = "cancel_reminder";
pub const SERVER: &str = "pinned";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Reminder {
    id: u64,
    message: String,
    due_unix_secs: u64,
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

    fn take_due(&self, now: u64) -> Vec<Reminder> {
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

pub fn watch(app: tauri::AppHandle) {
    use tauri::Manager;
    std::thread::spawn(move || loop {
        std::thread::sleep(StdDuration::from_secs(1));
        let now = Local::now().timestamp().max(0) as u64;
        for reminder in app.state::<Reminders>().take_due(now) {
            notify(&reminder.message);
        }
    });
}

fn notify(message: &str) {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{escaped}\" with title \"Reminder\" sound name \"Glass\"");
    let _ = std::process::Command::new("osascript").args(["-e", &script]).status();
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

// Claude Code expands `${PINNED_SOCK}` from the session's own environment.
pub fn write_mcp_config(path: &Path, exe: &Path) -> std::io::Result<()> {
    let config = serde_json::json!({
        "mcpServers": { SERVER: {
            "command": exe,
            "args": [SUBCOMMAND],
            "env": { SOCK_ENV: format!("${{{SOCK_ENV}}}") },
        } }
    });
    std::fs::write(path, serde_json::to_vec_pretty(&config)?)
}

pub fn permissions() -> Vec<String> {
    [TOOL, LIST_TOOL, CANCEL_TOOL].map(|tool| format!("mcp__{SERVER}__{tool}")).into()
}

pub fn serve() {
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
            Some("tools/list") => Ok(serde_json::json!({ "tools": tool_definitions() })),
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

fn tool_definitions() -> serde_json::Value {
    serde_json::json!([
        remind_definition(),
        {
            "name": LIST_TOOL,
            "description": "List the reminders still pending, each with its id, time and message.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": CANCEL_TOOL,
            "description": "Cancel a pending reminder by the id list_reminders or remind_me gave.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "The reminder's id." } },
                "required": ["id"],
            },
        },
    ])
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
        _ => Err(format!("unknown tool {}", params["name"])),
    };
    let reply = request.and_then(|request| send(&request));
    let (text, is_error) = match reply {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    serde_json::json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
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
}
