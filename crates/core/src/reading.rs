//! How an agent reads another session: its state in the words agents' tools use, and its latest
//! conversation. The hub's MCP server and the global orchestrator's `read_captain` both answer
//! with it.

use crate::agent::AgentStatus;
use crate::thread;
use crate::wire::TabView;
use serde_json::{json, Value};
use std::path::Path;

/// How many conversation items a read returns when not told.
pub const DEFAULT_ITEMS: usize = 20;
const ITEM_TEXT: usize = 4000;

/// What a tab's session is doing, in the words the tools use, and what it asks the user or why it failed.
pub fn state_of(tab: &TabView) -> (&'static str, Option<String>) {
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
    (state, message.filter(|_| matches!(state, "needs_permission" | "waiting_for_input" | "failed")))
}

pub fn next_for(state: &str) -> &'static str {
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
pub fn conversation(page: &Value, limit: usize) -> Value {
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

/// The end of the transcript `file` as a `Call::Thread` page, reaching back far enough to hold its
/// last `limit` conversation items when its last 8 MiB have them.
pub fn end_of(file: &Path, limit: usize) -> Result<Value, String> {
    let enough = |page: &thread::Page| serde_json::to_value(page).is_ok_and(|page| conversation(&page, limit)["items"].as_array().is_some_and(|items| items.len() >= limit));
    let page = thread::read_end(file, enough).map_err(|e| format!("could not read the transcript: {e}"))?;
    serde_json::to_value(page).map_err(|e| e.to_string())
}

/// A read of `tab`: its state, what to do next, its title, the question it asks the user or why it
/// failed, and the last `limit` items of `page`, the tab's `Call::Thread` answer, or why that failed.
pub fn read(tab: &TabView, page: Result<Value, String>, limit: usize) -> Value {
    let (state, question) = state_of(tab);
    let mut out = json!({ "state": state, "next": next_for(state), "title": tab.title });
    if let Some(said) = question {
        out[if state == "failed" { "error" } else { "question" }] = json!(said);
    }
    match page {
        Ok(page) => out["conversation"] = conversation(&page, limit),
        Err(e) => out["conversation_error"] = json!(e),
    }
    out
}

