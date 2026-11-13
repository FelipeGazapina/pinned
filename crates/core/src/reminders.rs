use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use crate::agent::{Message, ProjectTool, Role, When, ROLE_ENV, SOCK_ENV, TAB_ENV, TASK_ENV};
use crate::convex;
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
const REVIEW_TOOL: &str = "task_review";
const STATS_TOOL: &str = "delegation_stats";
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

// Claude Code expands `${PINNED_SOCK}` from the session's own environment, and `${PINNED_TAB:-}`,
// `${PINNED_TASK:-}`, and `${PINNED_ROLE:-}` to nothing where they are unset. A node passes
// `reminders: false` so the same server delegates without offering desktop notifications.
pub fn write_mcp_config(path: &Path, exe: &Path, reminders: bool) -> std::io::Result<()> {
    let args = if reminders { vec![SUBCOMMAND] } else { vec![SUBCOMMAND, NO_REMINDERS] };
    let config = json!({
        "mcpServers": { SERVER: {
            "command": exe,
            "args": args,
            "env": {
                SOCK_ENV: format!("${{{SOCK_ENV}}}"),
                TAB_ENV: format!("${{{TAB_ENV}:-}}"),
                TASK_ENV: format!("${{{TASK_ENV}:-}}"),
                ROLE_ENV: format!("${{{ROLE_ENV}:-}}"),
            },
        } }
    });
    std::fs::write(path, serde_json::to_vec_pretty(&config)?)
}

pub fn permissions() -> Vec<String> {
    let project = project_tools().map(|(_, tool)| tool["name"].as_str().unwrap().to_owned());
    [TOOL, LIST_TOOL, CANCEL_TOOL, CAPABILITIES_TOOL, DELEGATE_TOOL, STATUS_TOOL, CANCEL_TASK_TOOL, REVIEW_TOOL, STATS_TOOL]
        .map(String::from)
        .into_iter()
        .chain(project)
        .chain(convex::TOOLS.map(String::from))
        .map(|tool| format!("mcp__{SERVER}__{tool}"))
        .collect()
}

pub fn serve(reminders: bool) {
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
            Some("tools/list") => {
                let child = std::env::var(TASK_ENV).ok().is_some_and(|value| !value.is_empty());
                let seat = Seat::of(session_var(ROLE_ENV).as_deref(), child);
                let mut tools = tool_definitions(reminders, child, seat);
                if let (Some(listed), Some(offer)) = (tools.as_array_mut(), convex_offer(child)) {
                    listed.extend(convex::tool_definitions(&offer));
                }
                Ok(json!({ "tools": tools }))
            }
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

/// The part a session's tab plays, as its `PINNED_ROLE` names it, which picks the tools it sees.
/// A delegated child plays none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seat {
    GlobalOrchestrator,
    Captain,
    TasteChat,
    Other,
}

impl Seat {
    fn of(role: Option<&str>, child: bool) -> Seat {
        match role {
            _ if child => Seat::Other,
            Some(crate::global_orchestrator::ROLE) => Seat::GlobalOrchestrator,
            Some(crate::taste::CHAT_ROLE) => Seat::TasteChat,
            Some(role) if Role::parse(role) == Some(Role::Orchestrator) => Seat::Captain,
            _ => Seat::Other,
        }
    }
}

/// Who a project tool is offered to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Audience {
    /// Sessions that may start workers: every one but a delegated child and the global orchestrator.
    Spawners,
    Everyone,
    GlobalOrchestrator,
    Captains,
    /// A thread the Way of working page opened to adjust the account's tastes.
    TasteChat,
}

fn tool_definitions(reminders: bool, child: bool, seat: Seat) -> Value {
    // The Capitão, a plain thread and a worker all fan out to workers the same way, so all get
    // delegate_task and agents. A delegated child never starts more workers. The global orchestrator
    // does no project work: it hands every request to a Capitão.
    let spawner = delegate::delegate_block(child).is_none();
    let global = seat == Seat::GlobalOrchestrator;
    let offered = |audience: Audience| match audience {
        Audience::Spawners => spawner && !global,
        Audience::Everyone => true,
        Audience::GlobalOrchestrator => global,
        Audience::Captains => seat == Seat::Captain,
        Audience::TasteChat => seat == Seat::TasteChat,
    };
    let mut tools = if reminders { reminder_tools() } else { vec![] };
    if !global {
        tools.extend(delegation_tools().into_iter().filter(|tool| (tool["name"] != DELEGATE_TOOL && tool["name"] != REVIEW_TOOL) || spawner));
    }
    // Whoever picks workers reads how earlier ones went, the global orchestrator included.
    if !child {
        tools.push(stats_tool());
    }
    tools.extend(project_tools().into_iter().filter(|(audience, _)| offered(*audience)).map(|(_, tool)| tool));
    Value::Array(tools)
}

/// What the hub lets this session read from Convex. A delegated child runs under its parent's tab,
/// so it gets none, and any failure offers nothing.
fn convex_offer(child: bool) -> Option<convex::Offer> {
    let tab = session_var(TAB_ENV).filter(|_| !child)?;
    let text = send(&Message::Project { tab, tool: ProjectTool::Convex { ask: convex::Ask::Offer } }).ok()?;
    serde_json::from_str(&text).ok()
}

fn delegation_tools() -> Vec<Value> {
    vec![
        json!({
            "name": CAPABILITIES_TOOL,
            "description": "Call this before the first delegate_task and pick providerInstanceId, model, and options from it. It lists the accounts this machine can run a child agent on, and the access this session inherits.",
            "inputSchema": {
                "type": "object",
                "properties": { "search": { "type": "string", "description": "Filters accounts and models." } },
            },
        }),
        json!({
            "name": DELEGATE_TOOL,
            "description": "Start a worker: a child agent on a signed-in account, with the model and effort target names. Called from a pinned thread, the worker is a thread nested under this one, in its own worktree with isolation \"worktree\", that the user can open and steer; the answer names its tabId, and agents lists it. The task ends with the worker's first finished turn, and its summary is that turn's last reply. Claude Code runs in the thread itself; Cursor and Codex run headless and the thread shows their steps. The child receives only task, with no parent history. mode \"async\" (the default) returns at once. The notice is typed automatically only when this session is idle; otherwise the user sends it from the task's card. End your turn and do not poll. mode \"wait\" blocks until the child finishes and returns the task_status body, including summary and error. A wait that times out does not cancel the child. A reply while the child still runs background work is not its answer: the task keeps working, this session gets one notice, and task_status holds the reply so far. When a headless task ends, pinned stops the child, the processes under it and every background shell it started, including a server the task asked it to leave running, so start a long-lived server yourself. A process that a foreground command leaves behind with & can outlive the task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "The whole assignment the child sees. It does not see this session's history." },
                    "title": { "type": "string", "description": "Short label for the card. The first line of task is used when this is omitted." },
                    "target": {
                        "type": "object",
                        "description": "Which account and model run the child, and where. Omit providerInstanceId to use this session's account. Leave machine and box out and pinned places heavy workers by the account's placement policy, and runs the rest on this machine; name a machine to override it. The answer's placement says where it runs, who chose it and why.",
                        "properties": {
                            "machine": { "type": "string", "description": "Another machine of this pinned account, by the id or name projects() lists. The worker runs there, in that machine's checkout of this repo (cloned when it has none), as a thread nested under this one; task_status, task_cancel and the end notice work from here. An offline or unknown machine is an error, never this machine. \"here\" keeps the worker on this machine on purpose, and \"auto\" places it by the policy even when it is not heavy." },
                            "box": {
                                "description": "Run the worker in a new Upstash Box that the hub opens with this account's Upstash connection, cloning this repo (or repo). providerInstanceId picks which Claude token, stored on the Upstash card in Connectors, the box signs in with; the stored default when omitted. The box is deleted when the task ends or is cancelled. The Workers box permission in Connectors decides whether a box may open.",
                                "oneOf": [
                                    { "type": "boolean" },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "size": { "type": "string", "enum": ["small", "medium", "large"], "description": "Leave it out to use the default box size: the project's, else the account's in Settings › Way of working, else small." },
                                            "repo": { "type": "string", "description": "owner/repo or a GitHub clone URL. Defaults to this checkout's origin." },
                                        },
                                    },
                                ],
                            },
                            "providerInstanceId": { "type": "string", "description": "An account id from orchestrator_capabilities." },
                            "driverKind": { "type": "string", "description": "claude, codex, or cursor. When providerInstanceId is omitted, this is that provider's builtin account." },
                            "model": { "type": "string", "description": "A model id that account lists. Omit it on the same provider to keep this session's model. Omit it on a different provider to use that provider's default." },
                            "options": {
                                "description": "Model options. An object with effort or reasoning_effort, or an array of {id, value}. Pass one effort, not both.",
                                "oneOf": [
                                    {
                                        "type": "object",
                                        "properties": {
                                            "effort": { "type": "string", "description": "Reasoning effort, one of the model's option values. Left out, the project's default effort applies; above its most effort, the call is refused." },
                                            "reasoning_effort": { "type": "string", "description": "Same as effort." },
                                        },
                                    },
                                    {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "id": { "type": "string" },
                                                "value": { "type": "string" },
                                            },
                                            "required": ["id", "value"],
                                        },
                                    },
                                ],
                            },
                        },
                    },
                    "isolation": { "type": "string", "enum": ["shared", "worktree"], "description": "shared runs in this checkout. worktree makes a separate checkout and runs the child there." },
                    "heavy": { "type": "boolean", "description": "Whether the account's placement policy picks the machine for this worker when target names none. Left out, a worker with isolation worktree is heavy and a shared one is not." },
                    "mode": { "type": "string", "enum": ["async", "wait"], "default": "async", "description": "async returns at once. The notice is typed automatically only when this session is idle; otherwise the user sends it from the task's card. End your turn and do not poll. wait blocks until the result is in this call." },
                    "timeoutMs": { "type": "number", "description": "How long wait blocks, in milliseconds. The child keeps running after a timeout." },
                    "budgetMs": { "type": "number", "description": "How long the child may work before pinned tells this session how long it has run, in milliseconds. The default is 7200000, two hours, or half of the run limit when that is shorter. It must be below the run limit. Time counts only while the machine is awake." },
                    "limitMs": { "type": "number", "description": "When pinned cancels the child, in milliseconds. The default and the most is the project's time limit in Settings › Way of working, eight hours unless changed; a longer one is cut to it. Time counts only while the machine is awake." },
                    "interactionMode": { "type": "string", "enum": ["inherit", "default", "plan"], "description": "plan is read-only. inherit and default leave this session's mode as it is." },
                    "runtimeMode": { "type": "string", "enum": ["inherit", "full-access", "auto"], "description": "How wide the child may act. inherit, or omitting it, keeps this session's mode. Anything wider than this session is rejected. A bypassed session can hand the child auto." },
                    "role": { "type": "string", "description": "A label for the child, such as explore. The child does not see it." },
                    "tools": { "type": "array", "items": { "type": "string" }, "description": "The tools the child may use, such as Bash, Grep and Glob. The child sees them as its grant. Omit it to grant what the task needs. A child that needs more says NEEDS followed by the tools, and this session answers with a wider grant." },
                    "clientRequestId": { "type": "string", "description": "The same caller and clientRequestId returns the task already started. If that task has finished, the answer is its status, including summary and error. A new attempt uses a new id." },
                },
                "required": ["task"],
            },
        }),
        json!({
            "name": STATUS_TOOL,
            "description": "The child task's state. Every body has next, which says what to do now. A session does not poll a running task: it ends its turn, and pinned types a notice when the task finishes. A headless task gets no notice, so it calls this again when next says to. Reading a finished task marks its result delivered.",
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
        json!({
            "name": REVIEW_TOOL,
            "description": "Say whether you used a finished task's result: accepted when you kept it, redone when you threw it away or had it done again. delegation_stats counts these per model. pinned marks a task accepted by itself when the pull request its thread watched merges, and redone when you start a task with the same title again; your verdict overrides both.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "taskId": { "type": "string" },
                    "verdict": { "type": "string", "enum": ["accepted", "redone"] },
                    "note": { "type": "string", "description": "Why, in a few words." },
                },
                "required": ["taskId", "verdict"],
            },
        }),
    ]
}

fn stats_tool() -> Value {
    json!({
        "name": STATS_TOOL,
        "description": "How delegated tasks went, per model, across every machine of the account (this machine's alone when it has no hub): runs, the share that finished done, median minutes and median cost, the limits that stopped them, and how many results were accepted or redone. Read it before picking a model for a worker.",
        "inputSchema": {
            "type": "object",
            "properties": { "days": { "type": "number", "enum": [1, 7, 30], "description": "The period, in days. 7 when omitted." } },
        },
    })
}

fn reminder_tools() -> Vec<Value> {
    vec![
        remind_definition(),
        json!({
            "name": LIST_TOOL,
            "description": "List the reminders still pending, each with its id, time and message.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
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

/// Each project tool, named as `ProjectTool` tags it, and who is offered it. Every session gets the
/// memory tools; one outside a project is told it has no memory.
fn project_tools() -> [(Audience, serde_json::Value); 20] {
    let no_args = serde_json::json!({ "type": "object", "properties": {} });
    let project = serde_json::json!({ "type": "string", "description": "The project's key from projects, its folder on this machine, or <machine>:<folder> for a folder on another machine." });
    let machine = serde_json::json!({ "type": "string", "description": "The machine's id or name from projects, with or without @. Pass it whenever projects lists the project on more than one machine." });
    let scope = serde_json::json!({ "type": "string", "description": "orchestrator (the global orchestrator), captains (every project's Captain) or project:<repo> (one project's Captain, as in project:github.com/acme/site)." });
    let model = serde_json::json!({
        "type": "object",
        "properties": { "model": { "type": "string", "description": "The model's catalog id, as in glm-5.3 or claude-opus-5-5." }, "provider": { "type": "string", "description": "The provider it runs through, when that matters." } },
        "required": ["model"],
    });
    [
        (Audience::Spawners, serde_json::json!({
            "name": "agents",
            "description": "List your workers, each with its tab id, title, status and folder.",
            "inputSchema": no_args,
        })),
        (Audience::Spawners, serde_json::json!({
            "name": "work_cost",
            "description": "What this piece of work cost so far: this session's own cost plus every worker it delegated, theirs in turn, and their Upstash Boxes, whether they ran here, on another machine or in a box. Answers total_usd (own_usd + agents_usd + box_usd), unknown (how many parts have no known cost, such as a Cursor worker: they add nothing to the total, so total_usd is then a floor, never a guess), workers, wall_ms, tokens, and tasks: each worker's id, title, model, machine, state, cost_usd (absent when unknown), box_usd, wall_ms and tokens. With taskId, that task and the workers under it instead.",
            "inputSchema": {
                "type": "object",
                "properties": { "taskId": { "type": "string", "description": "A worker you started, to read its own subtree. Omit for this session's whole tree." } },
            },
        })),
        (Audience::Everyone, serde_json::json!({
            "name": "memory_search",
            "description": "Search this project's memory: facts saved by earlier sessions, such as the user's preferences, decisions, conventions and how the code works. Matches by words and by meaning, in any language. Returns one memory per line as [id] title: text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What you want to know, in a few words or a question. Empty lists the latest memories." },
                    "limit": { "type": "integer", "description": "At most this many memories; 8 when left out." },
                },
                "required": ["query"],
            },
        })),
        (Audience::Everyone, serde_json::json!({
            "name": "memory_save",
            "description": "Save a durable fact about this project for later sessions: a user preference, a decision, a convention or an architecture fact. One fact per memory. With id, replaces that memory instead. A memory that repeats an existing one is not saved; you get the existing one's id to update.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "A few words naming the subject." },
                    "text": { "type": "string", "description": "The fact, in one to three sentences." },
                    "kind": { "type": "string", "description": "preference, decision, convention or fact." },
                    "id": { "type": "integer", "description": "The memory to replace." },
                },
                "required": ["title", "text"],
            },
        })),
        (Audience::Everyone, serde_json::json!({
            "name": "memory_forget",
            "description": "Remove a memory that is wrong or no longer holds, by its id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "The memory's id." } },
                "required": ["id"],
            },
        })),
        (Audience::Everyone, serde_json::json!({
            "name": "actions",
            "description": "List what you can do as the user through act: each action this session may run, with its class (read and write run at once; confirm waits for the user's approval in the app), on (machine: it runs on this session's machine or the one act names; hub: it runs on the account), what it does, and the JSON Schema of its args. search keeps those whose name or description holds each word.",
            "inputSchema": {
                "type": "object",
                "properties": { "search": { "type": "string", "description": "Words to match, such as machine or memory. Empty lists every action." } },
            },
        })),
        (Audience::Everyone, serde_json::json!({
            "name": "act",
            "description": "Do one thing the user can do in pinned, as the user's account: an action from actions, with its args. Every act goes in the account's audit trail. A read or write runs at once and answers JSON whose outcome is done, with result. A confirm action does not run: it answers outcome pending with an id, the user approves or denies it in the app, and a notice in this session says what happened. Never put a password, key or token in args; the user enters those in the app.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "The action's name from actions." },
                    "args": { "type": "object", "description": "The action's arguments, as its args schema in actions describes them." },
                    "request_id": { "type": "string", "description": "A new id per act, such as a UUID. Repeating it answers the first outcome instead of running again." },
                    "machine": { "type": "string", "description": "For an action on a machine: the machine's id or name from machines, when it is not this session's own." },
                },
                "required": ["action"],
            },
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "projects",
            "description": "List every project of the account as JSON: its key, name and repo, each machine that has it (online or not, with its pinned version), its Capitão's state there (livre, trabalhando or nenhum) and how many workers run, then every machine of the account with any problem, such as one that needs to update pinned. Call it to pick the project and machine before send_to_captain.",
            "inputSchema": no_args,
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "send_to_captain",
            "description": "Hand a message to a project's Capitão. It opens the Capitão when the project has none and starts it when it is stopped, and while it works the message waits in its queue, so it is never typed over its work. Returns JSON whose outcome is entregue (typed now), na_fila (queued, with its position), iniciado (started with the message) or ocupado (busy, and the settings refuse: try again later with the same request_id). The same request_id never delivers twice.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": project,
                    "message": { "type": "string", "description": "What the Capitão should do, written so it stands alone." },
                    "request_id": { "type": "string", "description": "A new id for each message, such as a UUID. Sending again with the same id is safe." },
                    "machine": machine,
                    "box": {
                        "description": "Open the Capitão in a new Upstash Box instead of on a machine: the hub clones the project's repo there with the account's own Upstash and GitHub connections and starts the Capitão on this message; a later message goes to the box that runs it. Answers iniciado with the box's machine at once; it boots in a few minutes and goes once its Capitão stays idle. The Captains box permission in Connectors decides whether a box may open.",
                        "oneOf": [
                            { "type": "boolean" },
                            { "type": "object", "properties": { "size": { "type": "string", "enum": ["small", "medium", "large"], "description": "Leave it out to use the default box size: the project's, else the account's in Settings › Way of working, else small." } } },
                        ],
                    },
                },
                "required": ["project", "message", "request_id"],
            },
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "read_captain",
            "description": "Read the latest replies of a project's Capitão.",
            "inputSchema": {
                "type": "object",
                "properties": { "project": project, "machine": machine },
                "required": ["project"],
            },
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "waiting",
            "description": "List as JSON what waits on the user across every machine: threads that ask for input and threads that stopped on an error.",
            "inputSchema": no_args,
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "sessions",
            "description": "List as JSON every session of the account on every online machine and account: loose threads the user opened, workers and Capitães. Each has its id (for read_session), project (key, name, path), machine, account (such as work or personal), role (captain, worker or thread), the Capitão a worker works for, title, branch, PR, state and last_activity (Unix seconds). state is running (working on a turn), idle (its turn ended), waiting_on_input (it asks the user mid-turn), error (it stopped on an error) or settled (its session is not running). Newest activity first. machines lists every machine of the account, so an offline one or one that needs to update pinned shows. Call it to answer what runs in a project or on a machine.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Only sessions in this project: its key from projects, its name, or its folder." },
                    "machine": { "type": "string", "description": "Only sessions on this machine: its id or name, with or without @." },
                },
            },
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "read_session",
            "description": "Read any session's state and its last messages, as read_captain reads a Capitão: a loose thread, a worker or a Capitão.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The session's id from sessions." },
                    "machine": { "type": "string", "description": "The machine sessions lists it on. Optional." },
                    "items": { "type": "integer", "description": "How many of its last messages and tool calls to read; 20 when left out, at most 50." },
                },
                "required": ["id"],
            },
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "repo_overview",
            "description": "Read-only look at a project's repository: the machine, branch and HEAD it read, size, languages, top folders, packages, CLAUDE.md, AGENTS.md and the start of the README, the latest commits and the project's memories. Use it before you hand a Capitão more than a one-liner. Off until the user turns it on in Settings > Account > Orchestrator repo access; every call is in Agent activity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "The project's key from projects." },
                    "machine": { "type": "string", "description": "The machine whose checkout to read, an id or a name. Optional: the checkout its Capitão runs in, else this machine's, else the first online one." },
                },
                "required": ["project"],
            },
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "repo_read",
            "description": "Read lines of one file in a project's repository, read-only: at most 400 lines or 24 KB a call, with next_line to read on. Files that may hold secrets, and anything git ignores, can't be read; lines that hold a token come back as [redacted].",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "The project's key from projects." },
                    "path": { "type": "string", "description": "The file's path inside the repository, such as crates/core/src/wire.rs." },
                    "machine": { "type": "string", "description": "The machine whose checkout to read, an id or a name. Optional." },
                    "rev": { "type": "string", "description": "A commit or ref to read the file at, such as HEAD~3. Optional: the working tree." },
                    "from_line": { "type": "integer", "description": "The first line, 1 when left out." },
                    "lines": { "type": "integer", "description": "How many lines, at most 400." },
                },
                "required": ["project", "path"],
            },
        })),
        (Audience::GlobalOrchestrator, serde_json::json!({
            "name": "repo_search",
            "description": "Search a project's repository, read-only. With query: the lines that hold it (40 by default, at most 100, with the total), in the files glob matches when given. With glob alone: the paths that match it, at most 200. Secrets and ignored files are never searched.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "The project's key from projects." },
                    "query": { "type": "string", "description": "Text to find; a regular expression with regex." },
                    "glob": { "type": "string", "description": "Only these files, such as **/*.rs or ui/*.js." },
                    "machine": { "type": "string", "description": "The machine whose checkout to search, an id or a name. Optional." },
                    "regex": { "type": "boolean", "description": "Read query as an extended regular expression." },
                    "case": { "type": "boolean", "description": "Match case." },
                    "max": { "type": "integer", "description": "Hits to return, 40 when left out, at most 100." },
                },
                "required": ["project"],
            },
        })),
        (Audience::TasteChat, serde_json::json!({
            "name": "taste_get",
            "description": "Read one scope of the account's way of working as JSON: effective (every field as its sessions get it), own (what the scope sets itself; a project's fields left out inherit from captains), profile, and brief (the text its sessions start with).",
            "inputSchema": { "type": "object", "properties": { "scope": scope }, "required": ["scope"] },
        })),
        (Audience::TasteChat, serde_json::json!({
            "name": "taste_set",
            "description": "Change the fields patch names in one scope of the account's way of working, keeping the rest, and answer taste_get of the scope. A project's field set to the Captains' value inherits again. A change that leaves no model on the ladder, parallel outside 1 to 100, a time limit outside 1 to 10080 minutes, or a default effort above the most effort changes nothing and fails. The Way of working page shows the change at once, and sessions started afterwards get it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": scope,
                    "patch": {
                        "type": "object",
                        "properties": {
                            "strategy": { "type": "string", "enum": ["cheap_with_guidance", "escalate_on_failure", "strongest_first"], "description": "How a worker climbs the ladder: the cheapest rung with a detailed brief, the cheapest rung moving up after two failures, or the hard-work model from the start." },
                            "ladder": { "type": "array", "items": model, "description": "The models a worker climbs, cheapest first, at least one." },
                            "hard_model": model,
                            "parallel": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Workers at once in a project; delegate_task refuses one more." },
                            "autonomy": { "type": "string", "enum": ["ask_first", "decide_and_tell", "report_at_end"] },
                            "review_diffs": { "type": "boolean", "description": "Review each worker's diff before accepting it." },
                            "second_opinion": { "type": "boolean", "description": "Another model reviews a risky change." },
                            "skills": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "enabled": { "type": "boolean" } }, "required": ["name", "enabled"] }, "description": "Skills the sessions follow; the whole list." },
                            "context": { "type": "array", "items": { "type": "string" }, "description": "Notes every turn carries; the whole list." },
                            "tier": { "type": "string", "enum": ["low", "mid", "high"], "description": "The costliest model tier a worker may run on, as orchestrator_capabilities lists it. It covers the worker's own model, not the models it picks for its subagents." },
                            "default_effort": { "type": "string", "enum": ["low", "medium", "high", "xhigh", "max"], "description": "The effort a worker gets when the call names none." },
                            "max_effort": { "type": "string", "enum": ["low", "medium", "high", "xhigh", "max"], "description": "The most effort a call may ask for." },
                            "budget_cents": { "type": "integer", "minimum": 0, "description": "What one worker may spend, in US cents; 0 sets no cap." },
                            "time_limit_min": { "type": "integer", "minimum": 1, "maximum": 10080, "description": "Minutes a worker runs before pinned stops it." },
                            "box_budget_cents": { "type": "integer", "minimum": 0, "description": "A box's budget in US cents, its default and its most; 0 sets neither." },
                            "box_size": { "type": "string", "enum": ["small", "medium", "large"], "description": "The size of a box opened with none named: captains sets the account's, a project overrides it, the orchestrator takes none. Small when nothing sets it." },
                        },
                    },
                },
                "required": ["scope", "patch"],
            },
        })),
        (Audience::TasteChat, serde_json::json!({
            "name": "taste_reset",
            "description": "Make a project's field inherit from captains again, or give the orchestrator's or the Captains' field its default back, and answer taste_get of the scope.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": scope,
                    "field": { "type": "string", "enum": ["strategy", "ladder", "hard_model", "parallel", "autonomy", "review_diffs", "second_opinion", "skills", "context", "tier", "default_effort", "max_effort", "budget_cents", "time_limit_min", "box_budget_cents", "box_size"] },
                },
                "required": ["scope", "field"],
            },
        })),
        (Audience::Captains, serde_json::json!({
            "name": "tell_orchestrator",
            "description": "Tell the global orchestrator something now, mid-work: progress, a result, or a question for the user. When the user's settings send reports by themselves, your last reply of a turn that a message from the orchestrator started reaches it without this tool.",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string", "description": "The report, written so it stands alone." } },
                "required": ["text"],
            },
        })),
    ]
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
        REVIEW_TOOL => required(args, "taskId").and_then(|id| {
            Ok(Message::TaskReview {
                tab: session_var(TAB_ENV),
                task: session_var(TASK_ENV),
                id,
                verdict: required(args, "verdict")?,
                note: args["note"].as_str().map(str::to_owned),
            })
        }),
        STATS_TOOL => Ok(Message::DelegationStats { days: args["days"].as_u64() }),
        CANCEL_TASK_TOOL => required(args, "taskId").map(|id| Message::TaskCancel {
            tab: session_var(TAB_ENV),
            task: session_var(TASK_ENV),
            id,
            reason: args["reason"].as_str().unwrap_or("the parent asked").to_owned(),
        }),
        "" => Err(format!("unknown tool {}", params["name"])),
        name if name.starts_with("convex_") => match session_var(TASK_ENV) {
            Some(_) => Err("A delegated child reads no Convex data: ask the session that started you.".into()),
            None => convex::ask(name, args).and_then(|ask| in_tab(ProjectTool::Convex { ask }, std::env::var(TAB_ENV).ok())),
        },
        name => project_request(name, args, std::env::var(TAB_ENV).ok()),
    };
    tool_result(request.and_then(|request| send(&request)))
}

/// A child task always waits for its own delegate call. A session with no tab does too.
/// A tab session keeps the mode it asked for.
fn session_mode(tab: Option<&str>, task: Option<&str>, asked: Mode) -> Mode {
    if task.is_some() || tab.is_none() { Mode::Wait } else { asked }
}

fn delegate_call(params: &Value, output: &Mutex<impl Write>) -> Value {
    let mut request = match delegate::parse_request(&params["arguments"]) {
        Ok(request) => request,
        Err(error) => return tool_result(Err(error)),
    };
    let tab = session_var(TAB_ENV);
    let parent = session_var(TASK_ENV);
    request.mode = session_mode(tab.as_deref(), parent.as_deref(), request.mode);
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
                    if status.get("next").and_then(Value::as_str).is_none() {
                        status["next"] = json!("the result is in summary");
                    }
                    return tool_result(Ok(status.to_string()));
                }
            }
            Err(error) => return tool_result(Err(error)),
        }
        if Instant::now() >= deadline {
            let mut pending = value.clone();
            pending["next"] = if tab.is_none() {
                json!(format!("call task_status with taskId \"{id}\" when you need it"))
            } else {
                json!("still working; the wait timed out; pinned will type a notice into this session when it finishes")
            };
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

fn project_request(name: &str, args: &serde_json::Value, tab: Option<String>) -> Result<Message, String> {
    let mut tool = if args.is_object() { args.clone() } else { serde_json::json!({}) };
    tool["name"] = name.into();
    let tool: ProjectTool = serde_json::from_value(tool).map_err(|e| format!("{name}: {e}"))?;
    in_tab(tool, tab)
}

fn in_tab(tool: ProjectTool, tab: Option<String>) -> Result<Message, String> {
    let tab = tab.filter(|t| !t.is_empty()).ok_or_else(|| format!("{TAB_ENV} is not set: run this inside a pinned tab"))?;
    Ok(Message::Project { tab, tool })
}

fn send(message: &Message) -> Result<String, String> {
    let timeout = match message {
        Message::Delegate { .. } => StdDuration::from_secs(120),
        // tools/list waits on it, at a session's start.
        Message::Project { tool: ProjectTool::Convex { ask: convex::Ask::Offer }, .. } => StdDuration::from_secs(5),
        // These may wait on the hub and another machine, which allow a minute.
        Message::Project {
            tool:
                ProjectTool::Projects
                | ProjectTool::SendToCaptain { .. }
                | ProjectTool::ReadCaptain { .. }
                | ProjectTool::Waiting
                | ProjectTool::Sessions { .. }
                | ProjectTool::ReadSession { .. }
                | ProjectTool::RepoOverview { .. }
                | ProjectTool::RepoRead { .. }
                | ProjectTool::RepoSearch { .. }
                | ProjectTool::TellOrchestrator { .. }
                | ProjectTool::TasteGet { .. }
                | ProjectTool::TasteSet { .. }
                | ProjectTool::TasteReset { .. }
                | ProjectTool::Convex { .. }
                | ProjectTool::Act { .. },
            ..
        } => StdDuration::from_secs(70),
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
    fn a_child_task_waits_for_its_own_delegate_call() {
        assert_eq!(session_mode(Some("tab-1"), None, Mode::Async), Mode::Async);
        assert_eq!(session_mode(Some("tab-1"), Some("task-1"), Mode::Async), Mode::Wait);
        assert_eq!(session_mode(None, Some("task-1"), Mode::Async), Mode::Wait);
        assert_eq!(session_mode(None, None, Mode::Async), Mode::Wait);
    }

    #[test]
    fn delegate_task_names_every_argument() {
        let tools = tool_definitions(true, false, Seat::Other);
        let tool = tools.as_array().unwrap().iter().find(|tool| tool["name"] == "delegate_task").unwrap();
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        let mut names: Vec<_> = props.keys().cloned().collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "budgetMs".to_owned(),
                "clientRequestId".into(),
                "heavy".into(),
                "interactionMode".into(),
                "isolation".into(),
                "limitMs".into(),
                "mode".into(),
                "role".into(),
                "runtimeMode".into(),
                "target".into(),
                "task".into(),
                "timeoutMs".into(),
                "title".into(),
                "tools".into(),
            ]
        );
        assert!(
            tool["description"].as_str().unwrap().contains("typed automatically only when this session is idle")
                && tool["description"].as_str().unwrap().contains("the user sends it from the task's card"),
            "{}",
            tool["description"]
        );
        assert!(
            props["mode"]["description"].as_str().unwrap().contains("typed automatically only when this session is idle")
                && props["mode"]["description"].as_str().unwrap().contains("the user sends it from the task's card"),
            "{}",
            props["mode"]["description"]
        );
        assert_eq!(props["mode"]["default"], "async");
        assert_eq!(props["mode"]["enum"], json!(["async", "wait"]));
        assert_eq!(props["isolation"]["enum"], json!(["shared", "worktree"]));
        assert_eq!(props["interactionMode"]["enum"], json!(["inherit", "default", "plan"]));
        assert_eq!(props["runtimeMode"]["enum"], json!(["inherit", "full-access", "auto"]));
        assert_eq!(props["task"]["description"], "The whole assignment the child sees. It does not see this session's history.");
        assert_eq!(
            props["clientRequestId"]["description"],
            "The same caller and clientRequestId returns the task already started. If that task has finished, the answer is its status, including summary and error. A new attempt uses a new id."
        );
        assert_eq!(props["isolation"]["description"], "shared runs in this checkout. worktree makes a separate checkout and runs the child there.");
        assert_eq!(props["target"]["properties"]["options"]["oneOf"][0]["properties"]["effort"]["type"], "string");
        assert_eq!(props["target"]["properties"]["options"]["oneOf"][1]["type"], "array");
        assert_eq!(props["target"]["properties"]["driverKind"]["type"], "string");
        assert_eq!(
            props["target"]["properties"]["model"]["description"],
            "A model id that account lists. Omit it on the same provider to keep this session's model. Omit it on a different provider to use that provider's default."
        );
        assert_eq!(props["target"]["properties"]["options"]["oneOf"][0]["properties"]["reasoning_effort"]["type"], "string");
    }

    #[test]
    #[cfg(unix)]
    fn a_wait_returns_the_summary_while_a_second_call_answers_and_progress_is_sent() {
        use std::os::unix::net::UnixListener;
        use std::sync::{Arc, Mutex};
        let _guard = session_env_lock();
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

        std::env::remove_var(TAB_ENV);
        std::env::set_var(TASK_ENV, "task-parent");
        let sock = dir.join("headless-timeout.sock");
        let listener = UnixListener::bind(&sock).unwrap();
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
                let body = json!({"taskId":"task-slow","title":"say ship","providerInstanceId":"claude","model":null,"workState":"working","summary":null,"error":null,"cwd":"/tmp"});
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
        assert_eq!(body["next"], "call task_status with taskId \"task-slow\" when you need it");

        restore_var(SOCK_ENV, previous.0);
        restore_var(TAB_ENV, previous.1);
        restore_var(TASK_ENV, previous.2);
        restore_var("PINNED_DELEGATE_PROGRESS_MS", previous.3);
        let _ = std::fs::remove_dir_all(dir);
    }

#[cfg(unix)]
struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

#[cfg(unix)]
impl Write for Shared {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
fn session_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap()
}

#[cfg(unix)]
fn restore_var(name: &str, value: Option<String>) {
    match value {
        Some(value) => std::env::set_var(name, value),
        None => std::env::remove_var(name),
    }
}

    #[test]
    fn a_session_gets_the_project_tools_a_delegated_child_lacks() {
        let names = |child| tool_definitions(true, child, Seat::Other).as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        let reminders = ["remind_me", "list_reminders", "cancel_reminder"];
        let delegation = ["orchestrator_capabilities", "delegate_task", "task_status", "task_cancel", "task_review", "delegation_stats"];
        let memory = ["memory_search", "memory_save", "memory_forget", "actions", "act"];
        assert_eq!(names(false), [&reminders[..], &delegation, &["agents", "work_cost"], &memory].concat());
        assert_eq!(names(true), [&reminders[..], &["orchestrator_capabilities", "task_status", "task_cancel"], &memory].concat());
        let on_a_node = tool_definitions(false, false, Seat::Other).as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(on_a_node, [&delegation[..], &["agents", "work_cost"], &memory].concat());
        assert_eq!(
            permissions()[9..],
            [
                "mcp__pinned__agents",
                "mcp__pinned__work_cost",
                "mcp__pinned__memory_search",
                "mcp__pinned__memory_save",
                "mcp__pinned__memory_forget",
                "mcp__pinned__actions",
                "mcp__pinned__act",
                "mcp__pinned__projects",
                "mcp__pinned__send_to_captain",
                "mcp__pinned__read_captain",
                "mcp__pinned__waiting",
                "mcp__pinned__sessions",
                "mcp__pinned__read_session",
                "mcp__pinned__repo_overview",
                "mcp__pinned__repo_read",
                "mcp__pinned__repo_search",
                "mcp__pinned__taste_get",
                "mcp__pinned__taste_set",
                "mcp__pinned__taste_reset",
                "mcp__pinned__tell_orchestrator",
                "mcp__pinned__convex_tables",
                "mcp__pinned__convex_data",
                "mcp__pinned__convex_query",
                "mcp__pinned__convex_run",
                "mcp__pinned__convex_logs",
            ]
        );

        let ask = |name, args, tab: Option<&str>| project_request(name, &args, tab.map(Into::into)).map(|m| serde_json::to_string(&m).unwrap());
        assert_eq!(ask("agents", serde_json::Value::Null, Some("tab-2")), Ok(r#"{"kind":"project","tab":"tab-2","tool":{"name":"agents"}}"#.into()));
        assert_eq!(ask("work_cost", serde_json::json!({}), Some("tab-2")), Ok(r#"{"kind":"project","tab":"tab-2","tool":{"name":"work_cost"}}"#.into()));
        assert_eq!(ask("work_cost", serde_json::json!({"taskId": "task-7"}), Some("tab-2")), Ok(r#"{"kind":"project","tab":"tab-2","tool":{"name":"work_cost","taskId":"task-7"}}"#.into()));
        assert_eq!(ask("memory_forget", serde_json::json!({"id": 3}), Some("tab-2")), Ok(r#"{"kind":"project","tab":"tab-2","tool":{"name":"memory_forget","id":3}}"#.into()));
        assert_eq!(ask("memory_save", serde_json::json!({"title": "Deploys"}), Some("tab-2")), Err("memory_save: missing field `text`".into()));
        assert_eq!(ask("delegate", serde_json::json!({ "task": "Fix it." }), Some("tab-2")), Err("delegate: unknown variant `delegate`, expected one of `agents`, `work_cost`, `memory_search`, `memory_save`, `memory_forget`, `projects`, `send_to_captain`, `read_captain`, `waiting`, `sessions`, `read_session`, `repo_overview`, `repo_read`, `repo_search`, `tell_orchestrator`, `taste_get`, `taste_set`, `taste_reset`, `actions`, `convex`, `act`".into()));
        assert_eq!(ask("agents", serde_json::json!({}), Some("")), Err("PINNED_TAB is not set: run this inside a pinned tab".into()));
        assert_eq!(
            ask("act", serde_json::json!({ "action": "rename_machine", "args": { "name": "Mini", "id": "m1" }, "request_id": "r-1" }), Some("tab-2")),
            Ok(r#"{"kind":"project","tab":"tab-2","tool":{"name":"act","action":"rename_machine","args":{"name":"Mini","id":"m1"},"request_id":"r-1"}}"#.into())
        );
        assert_eq!(ask("actions", serde_json::Value::Null, Some("tab-2")), Ok(r#"{"kind":"project","tab":"tab-2","tool":{"name":"actions"}}"#.into()));
        assert_eq!(ask("act", serde_json::json!({}), Some("tab-2")), Err("act: missing field `action`".into()));
    }

    #[cfg(unix)]
    fn offered(role: Option<&str>, task: Option<&str>) -> Vec<String> {
        let _guard = session_env_lock();
        let previous = (std::env::var(ROLE_ENV).ok(), std::env::var(TASK_ENV).ok(), std::env::var(TAB_ENV).ok());
        std::env::remove_var(TAB_ENV);
        match role {
            Some(value) => std::env::set_var(ROLE_ENV, value),
            None => std::env::remove_var(ROLE_ENV),
        }
        match task {
            Some(value) => std::env::set_var(TASK_ENV, value),
            None => std::env::remove_var(TASK_ENV),
        }
        let input = format!("{}\n", json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        serve_io(true, std::io::Cursor::new(input.into_bytes()), Shared(buf.clone())).unwrap();
        restore_var(ROLE_ENV, previous.0);
        restore_var(TASK_ENV, previous.1);
        restore_var(TAB_ENV, previous.2);
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let value: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        value["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap().to_owned()).collect()
    }

    /// Runs `requests` through `pinned mcp` as tab `tab-1` (a delegated child with `task`), against
    /// a machine socket that answers each line it hears with the next of `answers`. Gives back the
    /// JSON-RPC replies and the socket lines.
    #[cfg(unix)]
    fn over_socket(task: Option<&str>, requests: &[Value], answers: Vec<Result<String, String>>) -> (Vec<Value>, Vec<String>) {
        use std::io::{BufRead, Write};
        let _guard = session_env_lock();
        let dir = std::env::temp_dir().join(format!("pinned-mcp-convex-{}-{}", std::process::id(), task.unwrap_or("none")));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("s");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        listener.set_nonblocking(true).unwrap();
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server = {
            let (heard, done) = (heard.clone(), done.clone());
            std::thread::spawn(move || {
                let mut answers = answers.into_iter();
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    let Ok((stream, _)) = listener.accept() else {
                        std::thread::sleep(StdDuration::from_millis(5));
                        continue;
                    };
                    stream.set_nonblocking(false).unwrap();
                    let mut line = String::new();
                    std::io::BufReader::new(&stream).read_line(&mut line).unwrap();
                    heard.lock().unwrap().push(line.trim().to_owned());
                    let answer = answers.next().unwrap_or(Err("no answer left".into()));
                    writeln!(&stream, "{}", serde_json::to_string(&answer).unwrap()).unwrap();
                }
            })
        };
        let previous = [SOCK_ENV, TAB_ENV, TASK_ENV, ROLE_ENV].map(|name| (name, std::env::var(name).ok()));
        std::env::set_var(SOCK_ENV, &sock);
        std::env::set_var(TAB_ENV, "tab-1");
        std::env::remove_var(ROLE_ENV);
        match task {
            Some(task) => std::env::set_var(TASK_ENV, task),
            None => std::env::remove_var(TASK_ENV),
        }
        let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        serve_io(true, std::io::Cursor::new(input.into_bytes()), Shared(buf.clone())).unwrap();
        for (name, value) in previous {
            restore_var(name, value);
        }
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
        let replies = String::from_utf8(buf.lock().unwrap().clone()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let heard = heard.lock().unwrap().clone();
        (replies, heard)
    }

    #[test]
    #[cfg(unix)]
    fn an_offer_adds_the_convex_tools_and_a_convex_call_reaches_the_machine_as_one_line() {
        let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let names = |reply: &Value| reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        let (replies, heard) = over_socket(None, std::slice::from_ref(&list), vec![Ok(r#"{"deployment":"quick-lemur-789","kind":"dev","tables":["invoices"]}"#.into())]);
        assert_eq!(heard, [r#"{"kind":"project","tab":"tab-1","tool":{"name":"convex","ask":{"op":"offer"}}}"#]);
        let offered = names(&replies[0]);
        assert_eq!(offered[offered.len() - 3..], ["act", "convex_tables", "convex_data"], "a table limit offers two tools after the rest");
        assert!(replies[0]["result"]["tools"].as_array().unwrap().last().unwrap()["description"].as_str().unwrap().contains("quick-lemur-789 (dev)"));

        let (replies, _) = over_socket(None, std::slice::from_ref(&list), vec![Err("Convex is not connected for `github.com/acme/site`.".into())]);
        assert!(!names(&replies[0]).iter().any(|n| n.starts_with("convex_")), "a refusal offers nothing");
        let (replies, heard) = over_socket(Some("task-1"), std::slice::from_ref(&list), vec![]);
        assert!(heard.is_empty() && !names(&replies[0]).iter().any(|n| n.starts_with("convex_")), "a delegated child runs under its parent's tab and asks nothing");

        let call = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "convex_data", "arguments": { "table": "invoices", "limit": 5 } } });
        let (replies, heard) = over_socket(None, std::slice::from_ref(&call), vec![Ok(r#"{"table":"invoices","documents":[]}"#.into())]);
        assert_eq!(heard, [r#"{"kind":"project","tab":"tab-1","tool":{"name":"convex","ask":{"op":"data","table":"invoices","limit":5,"cursor":null,"oldest_first":false}}}"#]);
        assert_eq!(replies[0]["result"], json!({ "content": [{ "type": "text", "text": r#"{"table":"invoices","documents":[]}"# }], "isError": false }));
        let (replies, heard) = over_socket(Some("task-1"), std::slice::from_ref(&call), vec![]);
        assert!(heard.is_empty());
        assert_eq!(replies[0]["result"]["content"][0]["text"], "A delegated child reads no Convex data: ask the session that started you.");
    }

    #[test]
    #[cfg(unix)]
    fn a_worker_is_offered_the_delegate_tools_and_a_child_is_not() {
        let plain = offered(None, None);
        assert!(plain.contains(&"delegate_task".to_owned()), "{plain:?}");
        assert!(plain.contains(&"agents".to_owned()), "{plain:?}");
        assert!(!plain.contains(&"delegate".to_owned()), "delegate_task is the one delegation tool: {plain:?}");
        let empty_task = offered(None, Some(""));
        assert!(empty_task.contains(&"delegate_task".to_owned()), "{empty_task:?}");
        assert_eq!(offered(Some("orchestrator"), None), [&plain[..], &["tell_orchestrator".to_owned()]].concat());
        assert_eq!(offered(Some("worker"), None), plain, "a worker starts workers the way a plain thread does");
        let child = offered(None, Some("task-1"));
        assert!(!child.contains(&"delegate_task".to_owned()), "{child:?}");
        assert!(!child.contains(&"delegate".to_owned()), "{child:?}");
        let child_orchestrator = offered(Some("orchestrator"), Some("task-1"));
        assert!(!child_orchestrator.contains(&"delegate_task".to_owned()), "{child_orchestrator:?}");
        assert!(!child_orchestrator.contains(&"delegate".to_owned()), "{child_orchestrator:?}");
    }

    #[test]
    #[cfg(unix)]
    fn the_global_orchestrator_routes_and_remembers_and_only_a_capitao_reports_to_it() {
        let reminders = ["remind_me", "list_reminders", "cancel_reminder"];
        let memory = ["memory_search", "memory_save", "memory_forget", "actions", "act"];
        let today = [&reminders[..], &["orchestrator_capabilities", "delegate_task", "task_status", "task_cancel", "task_review", "delegation_stats", "agents", "work_cost"], &memory].concat();
        assert_eq!(offered(None, None), today, "a plain thread sees what it saw before the global orchestrator");
        assert_eq!(offered(Some("worker"), None), today);
        assert_eq!(offered(Some("orchestrator"), None), [&today[..], &["tell_orchestrator"]].concat());
        assert_eq!(offered(Some("global-orchestrator"), None), [&reminders[..], &["delegation_stats"], &memory, &["projects", "send_to_captain", "read_captain", "waiting", "sessions", "read_session", "repo_overview", "repo_read", "repo_search"]].concat());
        assert_eq!(offered(Some("taste-chat"), None), [&today[..], &["taste_get", "taste_set", "taste_reset"]].concat(), "only a taste chat may change the way of working");
        assert_eq!(offered(Some("taste-chat"), Some("task-1")), [&reminders[..], &["orchestrator_capabilities", "task_status", "task_cancel"], &memory].concat(), "a taste chat's delegated child can't");
        assert_eq!(offered(Some("global-orchestrator"), Some("task-1")), [&reminders[..], &["orchestrator_capabilities", "task_status", "task_cancel"], &memory].concat(), "a delegated child plays no role");

        let ask = |name, args| project_request(name, &args, Some("tab-9".into())).map(|m| serde_json::to_string(&m).unwrap());
        assert_eq!(
            ask("send_to_captain", serde_json::json!({ "project": "github.com/o/site", "message": "Ship the form.", "request_id": "r-1", "machine": "mini" })),
            Ok(r#"{"kind":"project","tab":"tab-9","tool":{"name":"send_to_captain","project":"github.com/o/site","message":"Ship the form.","request_id":"r-1","machine":"mini"}}"#.into())
        );
        assert_eq!(ask("send_to_captain", serde_json::json!({ "project": "site", "message": "x" })), Err("send_to_captain: missing field `request_id`".into()));
        assert_eq!(ask("tell_orchestrator", serde_json::json!({ "text": "Halfway." })), Ok(r#"{"kind":"project","tab":"tab-9","tool":{"name":"tell_orchestrator","text":"Halfway."}}"#.into()));
        assert_eq!(ask("waiting", serde_json::Value::Null), Ok(r#"{"kind":"project","tab":"tab-9","tool":{"name":"waiting"}}"#.into()));
        assert_eq!(ask("sessions", serde_json::json!({ "project": "client-apps" })), Ok(r#"{"kind":"project","tab":"tab-9","tool":{"name":"sessions","project":"client-apps"}}"#.into()));
        assert_eq!(ask("read_session", serde_json::json!({ "id": "tab-1a-3", "items": 5 })), Ok(r#"{"kind":"project","tab":"tab-9","tool":{"name":"read_session","id":"tab-1a-3","items":5}}"#.into()));
        assert_eq!(
            ask("repo_read", serde_json::json!({ "project": "github.com/o/r", "path": "src/a.rs", "from_line": 401 })),
            Ok(r#"{"kind":"project","tab":"tab-9","tool":{"name":"repo_read","project":"github.com/o/r","path":"src/a.rs","from_line":401}}"#.into())
        );
        assert_eq!(
            ask("repo_search", serde_json::json!({ "project": "github.com/o/r", "query": "Call::", "max": 500 })),
            Ok(r#"{"kind":"project","tab":"tab-9","tool":{"name":"repo_search","project":"github.com/o/r","query":"Call::","regex":false,"case":false,"max":500}}"#.into())
        );
    }

    #[test]
    fn the_mcp_server_hears_which_tab_and_role_runs_it() {
        let file = std::env::temp_dir().join(format!("pinned-mcp-config-{}.json", std::process::id()));
        write_mcp_config(&file, Path::new("/opt/pinned/bin/pinned"), true).unwrap();
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["pinned"]["env"], serde_json::json!({ "PINNED_SOCK": "${PINNED_SOCK}", "PINNED_TAB": "${PINNED_TAB:-}", "PINNED_TASK": "${PINNED_TASK:-}", "PINNED_ROLE": "${PINNED_ROLE:-}" }));
        assert_eq!(config["mcpServers"]["pinned"]["args"], serde_json::json!(["mcp"]));
        write_mcp_config(&file, Path::new("/opt/pinned/bin/pinned"), false).unwrap();
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["pinned"]["args"], serde_json::json!(["mcp", "--no-reminders"]));
        let _ = std::fs::remove_file(file);
    }
}
