//! The hub's MCP server for external agents, over Streamable HTTP: one JSON-RPC message per POST,
//! answered with one JSON body. It holds no session state, so any hub instance answers.
//!
//! Every call carries an access token from `oauth`, which names the account, the scopes and the
//! projects the user picked. The tools only reuse what the app's own clients already ask machines
//! for: a session starts as a connector's run does (`Call::Run`), the orchestrator opens as the
//! app opens it (`Call::OpenOrchestrator`), text goes in as a typed prompt (`Call::Prompt`), and
//! replies come out of the transcript (`Call::Thread`). Nothing here answers a permission prompt
//! or starts a session that skips them.

use crate::db::{AgentGrant, AgentSession, Reserved};
use crate::http::Server;
use crate::hub::{Account, Hub, Unanswered};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pinned_core::agent::{AgentStatus, Role, CONNECTOR_ENV};
use pinned_core::wire::{AgentProject, Call, RunSpec, TabView};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Newest first. A client asking for another gets the newest, which it may refuse.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
/// The connector id an agent's session carries, `mcp:<session>`, which finds its tab again on any
/// machine and across the machine's restarts, since a tab's environment is saved with it.
const SESSION_PREFIX: &str = "mcp:";
const START_WAIT: Duration = Duration::from_secs(120);
const CALL_WAIT: Duration = Duration::from_secs(30);
/// How many sessions one agent may start in an hour, so a looping agent can't fill a machine.
const STARTS_PER_HOUR: i64 = 10;
const MAX_TEXT: usize = 20_000;
const ITEM_TEXT: usize = 4000;
const DEFAULT_ITEMS: usize = 20;
const MAX_ITEMS: usize = 100;

pub async fn not_offered() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim()).filter(|t| !t.is_empty())
}

/// MCP requires a server to check `Origin`, against DNS rebinding. A hosted client sends none; a
/// page may only call from the hub's own origin.
fn origin_allowed(headers: &HeaderMap, public: &str) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else { return true };
    let (Ok(origin), Ok(public)) = (origin.to_str(), url::Url::parse(public)) else { return false };
    url::Url::parse(origin).is_ok_and(|o| o.origin() == public.origin())
}

fn unauthorized(public: &str, invalid: bool) -> Response {
    let metadata = format!("{public}/.well-known/oauth-protected-resource/mcp");
    let challenge = match invalid {
        true => format!("Bearer error=\"invalid_token\", error_description=\"the token is unknown, expired or revoked\", resource_metadata=\"{metadata}\""),
        false => format!("Bearer resource_metadata=\"{metadata}\", scope=\"sessions orchestrator\""),
    };
    let mut response = (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized", "error_description": "connect this agent to Pinned first" }))).into_response();
    response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_str(&challenge).expect("a header-safe challenge"));
    response
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

pub async fn post(State(server): State<Arc<Server>>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let hub = &server.hub;
    if !origin_allowed(&headers, &hub.public_url) {
        return (StatusCode::FORBIDDEN, "this origin may not call the MCP endpoint").into_response();
    }
    let grant = match bearer(&headers) {
        None => return unauthorized(&hub.public_url, false),
        Some(token) => match hub.db.agent_by_token(token).await {
            Ok(Some(grant)) => grant,
            Ok(None) => return unauthorized(&hub.public_url, true),
            Err(e) => return crate::http::internal_response(e),
        },
    };
    if let Some(version) = headers.get("mcp-protocol-version").and_then(|v| v.to_str().ok()) {
        if !PROTOCOL_VERSIONS.contains(&version) {
            return (StatusCode::BAD_REQUEST, Json(rpc_error(Value::Null, -32600, &format!("unsupported MCP-Protocol-Version {version}")))).into_response();
        }
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(rpc_error(Value::Null, -32700, &format!("parse error: {e}")))).into_response(),
    };
    if message.is_array() {
        return (StatusCode::BAD_REQUEST, Json(rpc_error(Value::Null, -32600, "batches are not supported"))).into_response();
    }
    // A notification or a response to us needs no answer.
    let Some(id) = message.get("id").cloned().filter(|_| message.get("method").is_some()) else {
        return StatusCode::ACCEPTED.into_response();
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let answer = match message["method"].as_str().unwrap_or("") {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools(&grant) })),
        "tools/call" => match hub.account(grant.account).await {
            Ok(account) => Ok(call(&Ctx { hub, grant: &grant, account }, &params).await),
            Err(e) => return crate::http::internal_response(e),
        },
        other => Err(rpc_error(id.clone(), -32601, &format!("method not found: {other}"))),
    };
    let body = match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => error,
    };
    let mut response = Json(body).into_response();
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn initialize(params: &Value) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or("");
    let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "pinned", "title": "Pinned", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Pinned runs coding agents on the user's own machines. Call list_projects first: you may only work in the projects listed there. \
            Sessions ask the user before risky actions; when a session reports needs_permission or waiting_for_input, tell the user to answer it in Pinned. \
            Never retry a start or a message with a new request_id unless read_session shows it did not happen.",
    })
}

struct Ctx<'a> {
    hub: &'a Arc<Hub>,
    grant: &'a AgentGrant,
    account: Arc<Account>,
}

struct Tool {
    name: &'static str,
    scope: Option<&'static str>,
    definition: fn() -> Value,
}

const TOOLS: [Tool; 7] = [
    Tool { name: "list_projects", scope: None, definition: list_projects_definition },
    Tool { name: "start_session", scope: Some("sessions"), definition: start_session_definition },
    Tool { name: "send_message", scope: Some("sessions"), definition: send_message_definition },
    Tool { name: "read_session", scope: Some("sessions"), definition: read_session_definition },
    Tool { name: "list_sessions", scope: Some("sessions"), definition: list_sessions_definition },
    Tool { name: "message_orchestrator", scope: Some("orchestrator"), definition: message_orchestrator_definition },
    Tool { name: "read_orchestrator", scope: Some("orchestrator"), definition: read_orchestrator_definition },
];

/// The tools the grant's scopes reach.
fn tools(grant: &AgentGrant) -> Vec<Value> {
    TOOLS.iter().filter(|t| t.scope.is_none_or(|s| grant.scopes.iter().any(|g| g == s))).map(|t| (t.definition)()).collect()
}

const REQUEST_ID: &str = "A key you make up for this one request, such as a UUID. Sending the same request_id again answers what the first call did and never acts twice. A new request needs a new request_id.";
const PROJECT_ID: &str = "A project id from list_projects.";

fn list_projects_definition() -> Value {
    json!({
        "name": "list_projects",
        "title": "List projects",
        "description": "The projects the user let you work in, each with its id, name, machine and whether that machine is online.",
        "inputSchema": { "type": "object", "properties": {} },
        "annotations": { "readOnlyHint": true },
    })
}

fn start_session_definition() -> Value {
    json!({
        "name": "start_session",
        "title": "Start a coding session",
        "description": "Start a new coding agent session in a project with a task. It runs on the user's machine with the user's own permission settings: it asks the user before risky actions and you can't answer those prompts. Returns a session_id for read_session and send_message.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": PROJECT_ID },
                "task": { "type": "string", "description": "The whole assignment: the goal, the constraints, and how to verify the result." },
                "title": { "type": "string", "description": "A few words naming the session." },
                "request_id": { "type": "string", "description": REQUEST_ID },
            },
            "required": ["project", "task", "request_id"],
        },
        "annotations": { "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
    })
}

fn send_message_definition() -> Value {
    json!({
        "name": "send_message",
        "title": "Message a session",
        "description": "Send a follow-up message to a session you started. Refused while the session is starting, working or waiting on the user, so it never answers a prompt meant for the user; read_session says when it may take a message (done or failed).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "message": { "type": "string" },
                "request_id": { "type": "string", "description": REQUEST_ID },
            },
            "required": ["session_id", "message", "request_id"],
        },
        "annotations": { "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
    })
}

fn read_options() -> Value {
    json!({
        "cursor": { "type": "integer", "description": "The cursor the last read returned, to get only what came after it." },
        "limit": { "type": "integer", "description": "At most this many conversation items, the latest; 20 when left out, 100 at most." },
    })
}

fn read_session_definition() -> Value {
    let mut properties = read_options();
    properties["session_id"] = json!({ "type": "string" });
    json!({
        "name": "read_session",
        "title": "Read a session",
        "description": "A session's state and its latest conversation. state is starting, working, done, failed, needs_permission, waiting_for_input, ended, offline or not_found. needs_permission and waiting_for_input mean the user has to answer in Pinned.",
        "inputSchema": { "type": "object", "properties": properties, "required": ["session_id"] },
        "annotations": { "readOnlyHint": true },
    })
}

fn list_sessions_definition() -> Value {
    json!({
        "name": "list_sessions",
        "title": "List sessions",
        "description": "The sessions you started, newest first, with their state.",
        "inputSchema": { "type": "object", "properties": {} },
        "annotations": { "readOnlyHint": true },
    })
}

fn message_orchestrator_definition() -> Value {
    json!({
        "name": "message_orchestrator",
        "title": "Message the project's orchestrator",
        "description": "Send a message to the project's orchestrator, the agent that plans the project's work and delegates it to worker sessions. When it isn't running this starts it and delivers nothing yet: send the message again with the same request_id once read_orchestrator says done. Refused while the orchestrator is starting, working or waiting on the user.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": PROJECT_ID },
                "message": { "type": "string" },
                "request_id": { "type": "string", "description": REQUEST_ID },
            },
            "required": ["project", "message", "request_id"],
        },
        "annotations": { "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
    })
}

fn read_orchestrator_definition() -> Value {
    let mut properties = read_options();
    properties["project"] = json!({ "type": "string", "description": PROJECT_ID });
    json!({
        "name": "read_orchestrator",
        "title": "Read the project's orchestrator",
        "description": "The orchestrator's state and its latest conversation, with the same states as read_session, plus not_started.",
        "inputSchema": { "type": "object", "properties": properties, "required": ["project"] },
        "annotations": { "readOnlyHint": true },
    })
}

fn tool_result(outcome: Result<Value, String>) -> Value {
    match outcome {
        Ok(value) => json!({ "content": [{ "type": "text", "text": value.to_string() }], "structuredContent": value, "isError": false }),
        Err(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
    }
}

async fn call(ctx: &Ctx<'_>, params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let args = params.get("arguments").filter(|a| a.is_object()).cloned().unwrap_or_else(|| json!({}));
    let Some(tool) = TOOLS.iter().find(|t| t.name == name) else { return tool_result(Err(format!("unknown tool {name:?}"))) };
    if let Some(scope) = tool.scope {
        if !ctx.grant.scopes.iter().any(|s| s == scope) {
            return tool_result(Err(format!("the user did not let this agent use {name}; it needs the {scope} scope")));
        }
    }
    let outcome = match name {
        "list_projects" => list_projects(ctx).await,
        "start_session" => start_session(ctx, &args).await,
        "send_message" => send_message(ctx, &args).await,
        "read_session" => read_session(ctx, &args).await,
        "list_sessions" => list_sessions(ctx).await,
        "message_orchestrator" => message_orchestrator(ctx, &args).await,
        "read_orchestrator" => read_orchestrator(ctx, &args).await,
        _ => unreachable!("every tool has a handler"),
    };
    tool_result(outcome)
}

/// A project's id: stable for the grant, and naming no path an agent could change.
pub fn project_id(project: &AgentProject) -> String {
    let digest = Sha256::digest(format!("{}\n{}", project.machine, project.path).as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    let text = args[key].as_str().map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| format!("{key} is required"))?;
    if text.chars().count() > MAX_TEXT {
        return Err(format!("{key} is longer than {MAX_TEXT} characters"));
    }
    Ok(text)
}

fn request_id(args: &Value) -> Result<&str, String> {
    let key = text_arg(args, "request_id")?;
    if key.len() > 200 {
        return Err("request_id is longer than 200 characters".into());
    }
    Ok(key)
}

fn project<'a>(ctx: &'a Ctx<'_>, args: &Value) -> Result<&'a AgentProject, String> {
    let id = text_arg(args, "project")?;
    // The same answer for a project of another account or machine as for none, so ids reveal nothing.
    ctx.grant.projects.iter().find(|p| project_id(p) == id).ok_or_else(|| format!("no project {id} for this agent; call list_projects"))
}

fn project_view(project: &AgentProject) -> Value {
    json!({ "id": project_id(project), "name": project.name, "path": project.path })
}

fn unanswered(e: Unanswered) -> String {
    String::from(e)
}

async fn tabs(ctx: &Ctx<'_>, machine: &str) -> Result<Vec<TabView>, String> {
    let value = ctx.hub.call_machine(&ctx.account, machine, Call::Tabs, CALL_WAIT).await.map_err(unanswered)?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// What a tab's session is doing, in the words the tools use, and what it asks the user.
fn state_of(tab: &TabView) -> (&'static str, Option<String>) {
    if !tab.running {
        return ("ended", None);
    }
    let message = tab.message.clone();
    let state = match tab.status {
        None => "starting",
        Some(AgentStatus::Working) => "working",
        Some(AgentStatus::Done) => "done",
        Some(AgentStatus::Failed) => "failed",
        // Claude reports a permission menu and a question the same way; only an idle notice is no question.
        Some(AgentStatus::NeedsInput) => match message.as_deref().unwrap_or("") {
            m if m.to_lowercase().contains("permission") => "needs_permission",
            m if m.to_lowercase().contains("waiting for your input") => "done",
            _ => "waiting_for_input",
        },
    };
    (state, message.filter(|_| matches!(state, "needs_permission" | "waiting_for_input")))
}

/// Typed text ends with Enter, which picks the highlighted option of any menu on screen: a
/// permission prompt, a question, or a startup dialog such as folder trust or a project's MCP
/// servers. So text goes in only once a hook said the session is idle at its prompt. A session
/// that has not reported yet (`starting`) may still show a startup dialog.
fn may_type(state: &str) -> bool {
    matches!(state, "done" | "failed")
}

fn next_for(state: &str) -> &'static str {
    match state {
        "starting" => "it is starting, and may be showing the user a startup prompt; read again in a minute",
        "working" => "it is working; read again in a while",
        "done" | "failed" => "it may take a message now",
        "needs_permission" | "waiting_for_input" => "the user has to answer it in Pinned; tell them, then read again later",
        "ended" => "the session ended; start a new one for more work",
        "offline" => "its machine is offline; try again later",
        "not_started" => "message it to start it",
        _ => "read it again later",
    }
}

fn truncated(text: &str) -> String {
    match text.char_indices().nth(ITEM_TEXT) {
        Some((at, _)) => format!("{}… [cut]", &text[..at]),
        None => text.to_owned(),
    }
}

/// The conversation items worth an agent's context: what was said and which tools ran. Reasoning
/// and tool output stay out, since tool output can hold whatever a file held.
fn conversation(page: &Value, limit: usize) -> Value {
    let items: Vec<Value> = page["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let kind = item["kind"].as_str()?;
                    let at = item["at"].clone();
                    match kind {
                        "user" | "steer" | "text" | "error" | "note" => Some(json!({ "kind": kind, "at": at, "text": truncated(item["text"].as_str().unwrap_or("")) })),
                        "tool" => Some(json!({ "kind": "tool", "at": at, "name": item["name"], "summary": truncated(item["summary"].as_str().unwrap_or("")) })),
                        "pr" => Some(json!({ "kind": "pr", "at": at, "url": item["url"] })),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let skip = items.len().saturating_sub(limit);
    json!({ "items": items[skip..], "cursor": page["next"], "reset": page["reset"].as_bool().unwrap_or(false) })
}

async fn read_tab(ctx: &Ctx<'_>, machine: &str, tab: &TabView, args: &Value) -> Value {
    let (state, question) = state_of(tab);
    let limit = args["limit"].as_u64().map_or(DEFAULT_ITEMS, |l| (l as usize).clamp(1, MAX_ITEMS));
    let from = args["cursor"].as_u64().unwrap_or(0);
    let mut out = json!({ "state": state, "next": next_for(state), "title": tab.title });
    if let Some(question) = question {
        out["question"] = json!(question);
    }
    match ctx.hub.call_machine(&ctx.account, machine, Call::Thread { tab: tab.id.clone(), from }, CALL_WAIT).await {
        Ok(page) => out["conversation"] = conversation(&page, limit),
        Err(e) => out["conversation_error"] = json!(unanswered(e)),
    }
    out
}

async fn list_projects(ctx: &Ctx<'_>) -> Result<Value, String> {
    let machines = ctx.hub.machines(&ctx.account).await?;
    let projects: Vec<Value> = ctx
        .grant
        .projects
        .iter()
        .map(|p| {
            let machine = machines.iter().find(|m| m.id == p.machine);
            let mut view = project_view(p);
            view["machine"] = json!(machine.map_or("a removed machine", |m| m.name.as_str()));
            view["online"] = json!(machine.is_some_and(|m| m.online));
            view
        })
        .collect();
    Ok(json!({ "projects": projects, "scopes": ctx.grant.scopes }))
}

fn args_hash(tool: &str, args: &Value) -> Vec<u8> {
    let mut args = args.clone();
    if let Some(map) = args.as_object_mut() {
        map.remove("request_id");
    }
    Sha256::digest(format!("{tool}\n{args}").as_bytes()).to_vec()
}

/// Where a keyed call stands before it acts.
enum Begin {
    /// The key is new: act.
    Go,
    /// An earlier call with the key finished: this is its answer.
    Answered(Value),
    /// An earlier call with the key reached the machine and lost its answer, so it may have acted.
    Unknown(Option<Uuid>),
}

async fn begin(ctx: &Ctx<'_>, tool: &str, args: &Value, key: &str, session: Option<Uuid>) -> Result<Begin, String> {
    let hash = args_hash(tool, args);
    match ctx.hub.db.reserve_agent_call(ctx.grant.id, key, tool, &hash, session).await? {
        Reserved::New => Ok(Begin::Go),
        Reserved::Existing { tool: was, args_hash, .. } if was != tool || args_hash != hash => {
            Err(format!("request_id {key:?} was already used for another request; a new request needs a new request_id"))
        }
        Reserved::Existing { state, result, .. } if state == "done" => {
            let mut answer = result.unwrap_or(Value::Null);
            answer["replayed"] = json!(true);
            Ok(Begin::Answered(answer))
        }
        Reserved::Existing { state, session, stale, .. } if state == "unknown" || stale => Ok(Begin::Unknown(session)),
        Reserved::Existing { session, .. } => Ok(Begin::Answered(json!({
            "state": "in_progress",
            "session_id": session.map(|s| s.to_string()),
            "next": "a call with this request_id is still running; read the session in a while instead of sending it again",
        }))),
    }
}

async fn finish(ctx: &Ctx<'_>, key: &str, state: &str, result: &Value) {
    if let Err(e) = ctx.hub.db.finish_agent_call(ctx.grant.id, key, state, result).await {
        eprintln!("pinned-server: could not record agent call {key}: {e}");
    }
}

async fn forget(ctx: &Ctx<'_>, key: &str) {
    if let Err(e) = ctx.hub.db.drop_agent_call(ctx.grant.id, key).await {
        eprintln!("pinned-server: could not forget agent call {key}: {e}");
    }
}

fn session_connector(id: Uuid) -> String {
    format!("{SESSION_PREFIX}{id}")
}

fn session_tab(tabs: &[TabView], id: Uuid) -> Option<&TabView> {
    let connector = session_connector(id);
    tabs.iter().filter(|t| t.connector.as_deref() == Some(connector.as_str())).max_by_key(|t| t.opened)
}

/// A title claude takes as its `--name` value: no control characters, no leading dash.
fn title_of(args: &Value, task: &str) -> String {
    let raw = args["title"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| task.lines().next().unwrap_or(""));
    let clean: String = raw.chars().filter(|c| !c.is_control()).take(80).collect();
    let clean = clean.trim().trim_start_matches('-').trim().to_owned();
    if clean.is_empty() { "External agent task".into() } else { clean }
}

/// Text typed into a session says who sent it, so the session and the user can tell it from the user's own.
fn from_agent(name: &str, text: &str) -> String {
    format!("External agent \"{name}\" via MCP:\n\n{text}")
}

fn started(session: Uuid, project: &AgentProject) -> Value {
    json!({ "session_id": session.to_string(), "project": project_view(project), "state": "starting", "next": "read_session in a while to follow it" })
}

fn unknown_start(session: Uuid, why: &str) -> Value {
    json!({
        "session_id": session.to_string(),
        "state": "unknown",
        "why": why,
        "next": "the machine may have started it; call read_session with this session_id, and do not start it again with a new request_id unless that says not_found",
    })
}

async fn start_session(ctx: &Ctx<'_>, args: &Value) -> Result<Value, String> {
    let project = project(ctx, args)?;
    let task = text_arg(args, "task")?;
    let key = request_id(args)?;
    let title = title_of(args, task);
    let session = Uuid::new_v4();
    let session = match begin(ctx, "start_session", args, key, Some(session)).await? {
        Begin::Go => session,
        Begin::Answered(answer) => return Ok(answer),
        Begin::Unknown(session) => {
            let session = session.ok_or("this request's session was lost")?;
            // Resolved from the machine's tabs, which carry the session's id whether or not its answer came back.
            return match tabs(ctx, &project.machine).await.ok().as_deref().and_then(|t| session_tab(t, session).cloned()) {
                Some(_) => {
                    let answer = started(session, project);
                    finish(ctx, key, "done", &answer).await;
                    Ok(answer)
                }
                None => Ok(unknown_start(session, "an earlier call with this request_id lost the machine's answer")),
            };
        }
    };
    let refuse = |why: String| async move {
        forget(ctx, key).await;
        Err::<Value, String>(why)
    };
    match ctx.hub.db.agent_recent_starts(ctx.grant.id).await {
        Ok(n) if n >= STARTS_PER_HOUR => return refuse(format!("this agent started {n} sessions in the last hour; wait before starting more")).await,
        Ok(_) => {}
        Err(e) => return refuse(e).await,
    }
    let Ok(machine) = Uuid::parse_str(&project.machine) else { return refuse("the project's machine id is unreadable".into()).await };
    if !ctx.account.is_online(&project.machine) {
        return refuse("the project's machine is offline; try again when it is online".into()).await;
    }
    if let Err(e) = ctx.hub.db.add_agent_session(ctx.grant.id, session, machine, &project.path, &title).await {
        return refuse(e).await;
    }
    let run = RunSpec {
        folder: project.path.clone(),
        repo: None,
        // The user's own default mode, so the session asks before what the user's settings ask about.
        args: vec!["--name".into(), title, "--permission-mode".into(), "default".into(), from_agent(&ctx.grant.name, task)],
        env: vec![(CONNECTOR_ENV.to_owned(), session_connector(session))],
        token: None,
        account: None,
        budget_usd: None,
    };
    match ctx.hub.call_machine(&ctx.account, &project.machine, Call::Run { run }, START_WAIT).await {
        Ok(_) => {
            let answer = started(session, project);
            finish(ctx, key, "done", &answer).await;
            eprintln!("pinned-server: agent grant {} started session {session} on {}", ctx.grant.id, project.machine);
            Ok(answer)
        }
        Err(Unanswered::Refused(why)) => {
            let _ = ctx.hub.db.drop_agent_session(session).await;
            refuse(why).await
        }
        // The session may run, so the key keeps it: a retry finds its tab rather than starting a second one.
        Err(Unanswered::Lost(why)) => {
            let answer = unknown_start(session, &why);
            finish(ctx, key, "unknown", &answer).await;
            Ok(answer)
        }
    }
}

async fn own_session(ctx: &Ctx<'_>, args: &Value) -> Result<AgentSession, String> {
    let id = text_arg(args, "session_id")?;
    let missing = || format!("no session {id} for this agent; call list_sessions");
    let id = Uuid::parse_str(id).map_err(|_| missing())?;
    ctx.hub.db.agent_sessions(ctx.grant.id, Some(id)).await?.into_iter().next().ok_or_else(missing)
}

fn unknown_delivery(why: &str) -> Value {
    json!({
        "delivered": "unknown",
        "why": why,
        "next": "the message may have arrived; read the conversation, and send it again with a new request_id only if it is not there",
    })
}

/// Types `text` into `tab` once, answering whether it arrived.
async fn deliver(ctx: &Ctx<'_>, key: &str, machine: &str, tab: &str, text: String, extra: Value) -> Result<Value, String> {
    match ctx.hub.call_machine(&ctx.account, machine, Call::Prompt { tab: tab.to_owned(), text }, CALL_WAIT).await {
        Ok(_) => {
            let mut answer = json!({ "delivered": true, "next": "read it in a while for the reply" });
            if let (Some(map), Some(extra)) = (answer.as_object_mut(), extra.as_object()) {
                map.extend(extra.clone());
            }
            finish(ctx, key, "done", &answer).await;
            Ok(answer)
        }
        Err(Unanswered::Refused(why)) => {
            forget(ctx, key).await;
            Err(why)
        }
        Err(Unanswered::Lost(why)) => {
            let answer = unknown_delivery(&why);
            finish(ctx, key, "unknown", &answer).await;
            Ok(answer)
        }
    }
}

async fn send_message(ctx: &Ctx<'_>, args: &Value) -> Result<Value, String> {
    let session = own_session(ctx, args).await?;
    let text = text_arg(args, "message")?;
    let key = request_id(args)?;
    match begin(ctx, "send_message", args, key, Some(session.id)).await? {
        Begin::Go => {}
        Begin::Answered(answer) => return Ok(answer),
        Begin::Unknown(_) => return Ok(unknown_delivery("an earlier call with this request_id lost the machine's answer")),
    }
    let machine = session.machine.to_string();
    let tabs = match tabs(ctx, &machine).await {
        Ok(tabs) => tabs,
        Err(e) => {
            forget(ctx, key).await;
            return Err(e);
        }
    };
    let Some(tab) = session_tab(&tabs, session.id) else {
        forget(ctx, key).await;
        return Err("the session is not open on its machine; start a new one".into());
    };
    let (state, _) = state_of(tab);
    if !may_type(state) {
        forget(ctx, key).await;
        return Err(format!("the session is {state}, so it can't take a message now: {}", next_for(state)));
    }
    deliver(ctx, key, &machine, &tab.id, from_agent(&ctx.grant.name, text), json!({ "session_id": session.id.to_string() })).await
}

async fn read_session(ctx: &Ctx<'_>, args: &Value) -> Result<Value, String> {
    let session = own_session(ctx, args).await?;
    let machine = session.machine.to_string();
    let base = json!({ "session_id": session.id.to_string(), "title": session.title });
    let mut out = if !ctx.account.is_online(&machine) {
        json!({ "state": "offline", "next": next_for("offline") })
    } else {
        match session_tab(&tabs(ctx, &machine).await?, session.id) {
            Some(tab) => read_tab(ctx, &machine, &tab.clone(), args).await,
            None => json!({ "state": "not_found", "next": "the session is not open on its machine: it never started, or the user closed it" }),
        }
    };
    if let (Some(map), Some(base)) = (out.as_object_mut(), base.as_object()) {
        map.extend(base.clone());
    }
    Ok(out)
}

async fn list_sessions(ctx: &Ctx<'_>) -> Result<Value, String> {
    let sessions = ctx.hub.db.agent_sessions(ctx.grant.id, None).await?;
    // Each machine is asked once; an offline one, or one that doesn't answer, reports its sessions offline.
    let mut seen: std::collections::HashMap<Uuid, Option<Vec<TabView>>> = std::collections::HashMap::new();
    for machine in sessions.iter().map(|s| s.machine) {
        if let std::collections::hash_map::Entry::Vacant(slot) = seen.entry(machine) {
            let id = machine.to_string();
            slot.insert(match ctx.account.is_online(&id) {
                true => tabs(ctx, &id).await.ok(),
                false => None,
            });
        }
    }
    let mut out = vec![];
    for session in sessions {
        let state = match &seen[&session.machine] {
            None => "offline",
            Some(tabs) => session_tab(tabs, session.id).map_or("not_found", |t| state_of(t).0),
        };
        let project = ctx.grant.projects.iter().find(|p| p.machine == session.machine.to_string() && p.path == session.project);
        out.push(json!({
            "session_id": session.id.to_string(),
            "title": session.title,
            "project": project.map(project_id),
            "state": state,
            "started_at": session.created_at,
        }));
    }
    Ok(json!({ "sessions": out }))
}

fn orchestrator_tab<'a>(tabs: &'a [TabView], project: &AgentProject) -> Option<&'a TabView> {
    tabs.iter().filter(|t| t.role == Some(Role::Orchestrator) && t.cwd == project.path).min_by_key(|t| t.opened)
}

async fn message_orchestrator(ctx: &Ctx<'_>, args: &Value) -> Result<Value, String> {
    let project = project(ctx, args)?;
    let text = text_arg(args, "message")?;
    let key = request_id(args)?;
    match begin(ctx, "message_orchestrator", args, key, None).await? {
        Begin::Go => {}
        Begin::Answered(answer) => return Ok(answer),
        Begin::Unknown(_) => return Ok(unknown_delivery("an earlier call with this request_id lost the machine's answer")),
    }
    let machine = project.machine.as_str();
    let _held = ctx.hub.hold_captain(&ctx.account, machine, &project.path).await;
    let opened = ctx.hub.call_machine(&ctx.account, machine, Call::OpenOrchestrator { path: project.path.clone() }, CALL_WAIT).await;
    // Opening is idempotent on the machine, so a lost answer did nothing a retry would repeat.
    let tab = match opened.map(|v| v.as_str().map(str::to_owned)) {
        Ok(Some(tab)) => tab,
        Ok(None) => {
            forget(ctx, key).await;
            return Err("the machine opened the orchestrator without a tab id".into());
        }
        Err(e) => {
            forget(ctx, key).await;
            return Err(format!("{}; try again with the same request_id", unanswered(e)));
        }
    };
    let view = match tabs(ctx, machine).await {
        Ok(tabs) => tabs.into_iter().find(|t| t.id == tab),
        Err(e) => {
            forget(ctx, key).await;
            return Err(e);
        }
    };
    let Some(view) = view else {
        forget(ctx, key).await;
        return Err("the orchestrator's tab closed before the message reached it; try again".into());
    };
    if !view.running {
        // Nothing is typed into it yet: it may open on a prompt only the user may answer.
        // The key is released, so the same request_id delivers the message once it is ready.
        let started = ctx.hub.call_machine(&ctx.account, machine, Call::StartClaude { tab: tab.clone(), cols: 120, rows: 32 }, START_WAIT).await;
        forget(ctx, key).await;
        return match started {
            Ok(_) => Ok(json!({
                "delivered": false,
                "started": true,
                "state": "starting",
                "project": project_view(project),
                "next": "the orchestrator is starting; call read_orchestrator, and once its state is done send this message again with the same request_id",
            })),
            Err(e) => Err(format!("{}; read_orchestrator, then try again with the same request_id", unanswered(e))),
        };
    }
    let (state, _) = state_of(&view);
    if !may_type(state) {
        forget(ctx, key).await;
        return Err(format!("the orchestrator is {state}, so it can't take a message now: {}", next_for(state)));
    }
    deliver(ctx, key, machine, &tab, from_agent(&ctx.grant.name, text), json!({ "project": project_view(project) })).await
}

async fn read_orchestrator(ctx: &Ctx<'_>, args: &Value) -> Result<Value, String> {
    let project = project(ctx, args)?;
    let machine = project.machine.as_str();
    let mut out = if !ctx.account.is_online(machine) {
        json!({ "state": "offline", "next": next_for("offline") })
    } else {
        match orchestrator_tab(&tabs(ctx, machine).await?, project) {
            Some(tab) if tab.running || tab.session.is_some() => read_tab(ctx, machine, &tab.clone(), args).await,
            _ => json!({ "state": "not_started", "next": next_for("not_started") }),
        }
    };
    out["project"] = project_view(project);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinned_core::wire::Program;

    fn tab(status: Option<AgentStatus>, message: Option<&str>, running: bool) -> TabView {
        TabView {
            id: "tab-1".into(),
            cwd: "/w/app".into(),
            account: "claude".into(),
            program: Program::Agent,
            bypass: false,
            running,
            status,
            title: None,
            message: message.map(str::to_owned),
            resume_at: None,
            held: false,
            connector: None,
            subject: None,
            opened: 1,
            updated: None,
            session: None,
            interrupted: false,
            stats: None,
            role: None,
            spawned_by: None,
            orchestrator_global: false,
        }
    }

    #[test]
    fn a_session_waiting_on_the_user_never_takes_typed_text() {
        let permission = tab(Some(AgentStatus::NeedsInput), Some("Claude needs your permission to use Bash"), true);
        assert_eq!(state_of(&permission), ("needs_permission", Some("Claude needs your permission to use Bash".into())));
        assert!(!may_type(state_of(&permission).0));
        let question = tab(Some(AgentStatus::NeedsInput), Some("Claude has a question"), true);
        assert_eq!(state_of(&question).0, "waiting_for_input");
        assert!(!may_type(state_of(&question).0));
        assert!(!may_type(state_of(&tab(Some(AgentStatus::Working), None, true)).0), "a turn may show a permission menu before its hook reports it");
        let idle = tab(Some(AgentStatus::NeedsInput), Some("Claude is waiting for your input"), true);
        assert_eq!(state_of(&idle), ("done", None));
        assert!(may_type("done") && may_type("failed"));
        assert!(!may_type("starting"), "a session that hasn't reported may show a startup dialog");
        assert_eq!(state_of(&tab(Some(AgentStatus::Done), None, false)).0, "ended");
    }

    #[test]
    fn a_title_never_reads_as_a_flag() {
        assert_eq!(title_of(&json!({ "title": "--dangerously-skip-permissions" }), ""), "dangerously-skip-permissions");
        assert_eq!(title_of(&json!({}), "Fix the build\nmore"), "Fix the build");
        assert_eq!(title_of(&json!({ "title": "-" }), ""), "External agent task");
        assert!(from_agent("x", "--help").starts_with("External agent \"x\" via MCP"));
    }

    #[test]
    fn the_request_id_is_not_part_of_what_a_retry_must_repeat() {
        let a = args_hash("send_message", &json!({ "session_id": "s", "message": "m", "request_id": "1" }));
        let b = args_hash("send_message", &json!({ "session_id": "s", "message": "m", "request_id": "2" }));
        assert_eq!(a, b);
        assert_ne!(a, args_hash("send_message", &json!({ "session_id": "s", "message": "other" })));
        assert_ne!(a, args_hash("start_session", &json!({ "session_id": "s", "message": "m" })));
    }

    #[test]
    fn the_conversation_keeps_what_was_said_and_drops_reasoning_and_tool_output() {
        let page = json!({ "next": 99, "reset": false, "items": [
            { "kind": "user", "at": "t1", "text": "hi" },
            { "kind": "thinking", "at": "t2", "text": "secret reasoning" },
            { "kind": "tool", "at": "t3", "name": "Bash", "summary": "ls", "detail": "x" },
            { "kind": "result", "tool": "t3", "at": "t4", "error": false, "text": "API_KEY=1" },
            { "kind": "text", "at": "t5", "text": "done" },
        ] });
        let out = conversation(&page, 20);
        assert_eq!(out["cursor"], 99);
        let text = out.to_string();
        assert!(!text.contains("secret reasoning") && !text.contains("API_KEY"), "{text}");
        assert_eq!(out["items"].as_array().unwrap().len(), 3);
        assert_eq!(conversation(&page, 1)["items"], json!([{ "kind": "text", "at": "t5", "text": "done" }]));
    }

    #[test]
    fn origins_other_than_the_hubs_are_refused() {
        let mut headers = HeaderMap::new();
        assert!(origin_allowed(&headers, "https://hub.example"), "a hosted client sends no origin");
        headers.insert(header::ORIGIN, HeaderValue::from_static("https://hub.example"));
        assert!(origin_allowed(&headers, "https://hub.example"));
        headers.insert(header::ORIGIN, HeaderValue::from_static("https://evil.example"));
        assert!(!origin_allowed(&headers, "https://hub.example"));
        headers.insert(header::ORIGIN, HeaderValue::from_static("null"));
        assert!(!origin_allowed(&headers, "https://hub.example"));
    }

    #[test]
    fn a_grant_lists_only_the_tools_its_scopes_reach() {
        let grant = |scopes: &[&str]| AgentGrant { id: Uuid::nil(), account: Uuid::nil(), name: "a".into(), scopes: scopes.iter().map(|s| s.to_string()).collect(), projects: vec![] };
        let names = |g: &AgentGrant| tools(g).iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(names(&grant(&["orchestrator"])), ["list_projects", "message_orchestrator", "read_orchestrator"]);
        assert_eq!(names(&grant(&["sessions"])), ["list_projects", "start_session", "send_message", "read_session", "list_sessions"]);
    }
}
