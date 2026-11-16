//! What a run's bot did, for the members who open the run.
//!
//! Read from the session in the run's own box, never anywhere else: the box must be a machine of
//! the team's account (`boxes` is keyed by it), and its tab is found as `calls` finds the tab it
//! types notices into (`calls::session_tab`). The session is read with `Call::Thread`, the call
//! the app's own thread view makes, so no new call reaches a machine.
//!
//! Every line is plain text on one line: secrets hidden by `surface::mask_secrets`, then cut to
//! `LINE_CHARS`. At most the last `LINES` lines answer a call. A run that is not the team's, a box
//! that is not the team's and a malformed id are all refused as an unknown run.

use super::{calls, db, Scope};
use crate::hub::Hub;
use pinned_core::surface::mask_secrets;
use pinned_core::teams::RunLogLine;
use pinned_core::wire::Call as MachineCall;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// The most lines a call answers: the last ones.
const LINES: usize = 200;
/// The most characters one line keeps before its "…".
const LINE_CHARS: usize = 500;
/// How long the hub waits on the box to read its transcript.
const READ_WAIT: Duration = Duration::from_secs(20);
/// What every refusal says, so a run of another team, a box that is not the team's and an id
/// that is no id cannot be told apart from a run that does not exist.
const NO_SUCH_RUN: &str = "the team has no such run";

/// The last 200 lines of the session in the run's box, oldest first; `[]` while the run has no
/// box or its box has no session yet. Any member reads them, ended runs too while their box exists.
pub async fn run_log(hub: &Arc<Hub>, scope: &Scope, run: &str) -> Result<Vec<RunLogLine>, String> {
    let team = &scope.team;
    let run_id = Uuid::parse_str(run.trim()).map_err(|_| NO_SUCH_RUN)?;
    let run = db::run(&hub.db, team.id, run_id).await?.ok_or(NO_SUCH_RUN)?;
    let Some(machine) = db::run_box(&hub.db, team.id, run_id).await? else { return Ok(vec![]) };
    let machine_id = Uuid::parse_str(&machine).map_err(|_| NO_SUCH_RUN)?;
    if hub.db.box_of(team.id, machine_id).await?.is_none() {
        return Err(NO_SUCH_RUN.into());
    }
    let Some(tab) = calls::session_tab(team, &machine, &run.workflow) else { return Ok(vec![]) };
    let call = MachineCall::Thread { tab: tab.id, from: 0, limit: Some(LINES as u32), before: None };
    let page = hub.call_machine(team, &machine, call, READ_WAIT).await.map_err(|e| format!("could not read the run's session: {}", String::from(e)))?;
    Ok(lines_of(&page))
}

/// The log lines of a `ThreadPage`: what the bot wrote, the tools it ran and what people typed.
/// Its thinking and tool output stay out, since output can hold whatever a file held.
fn lines_of(page: &Value) -> Vec<RunLogLine> {
    let items = page["items"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut lines: Vec<RunLogLine> = items
        .iter()
        .filter_map(|item| {
            let (who, text) = match item["kind"].as_str()? {
                "text" | "error" => ("bot", item["text"].as_str()?),
                "user" | "steer" => ("person", item["text"].as_str()?),
                "tool" => (item["name"].as_str()?, item["summary"].as_str().unwrap_or_default()),
                _ => return None,
            };
            Some(RunLogLine { at: unix_of(item["at"].as_str().unwrap_or_default()), who: one_line(who, usize::MAX), text: one_line(text, LINE_CHARS) })
        })
        .collect();
    lines.drain(..lines.len().saturating_sub(LINES));
    lines
}

/// `text` with secrets hidden, on one line, cut to `max` characters and a "…".
fn one_line(text: &str, max: usize) -> String {
    let flat = mask_secrets(text).replace(['\r', '\n'], " ");
    match flat.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &flat[..at]),
        None => flat,
    }
}

/// Unix seconds of a transcript's RFC 3339 timestamp, 0 when it has none.
fn unix_of(timestamp: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(timestamp).map_or(0, |at| at.timestamp())
}
