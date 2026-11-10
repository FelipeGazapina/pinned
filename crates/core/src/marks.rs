//! How the user filed each conversation in the sidebar: a name, a pin, a settle, a snooze, what they
//! saw last, a hand-made order, auto-settle left off, and the archive. The hub keeps them per account,
//! so every device and every agent of the account sees the same sidebar.

use crate::wire::MachineId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Longest session id, title or folder a mark keeps.
pub const MAX_TEXT: usize = 2000;
/// Most changes one call may make.
pub const MAX_CHANGES: usize = 2000;

/// One conversation's marks, under Claude Code's id for it, which outlives tabs and restarts.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct SessionMarks {
    pub session: String,
    /// The name the user gave it, shown in place of the session's own title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Its place among pinned threads, lowest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<u32>,
    /// Its place among active threads the user dragged into an order, lowest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    /// Unix seconds the user settled it. It stays settled until a later prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled: Option<f64>,
    /// Unix seconds the user un-settled it, which restarts auto-settle's count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsettled: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed: Option<Snooze>,
    /// Unix seconds of the last activity the user saw. Unset, a finished turn shows as unread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<f64>,
    /// Auto-settle leaves it where it is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_auto_settle: bool,
    /// The user archived it: its tab closed, and Settings › Archive lists it to bring back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<Archived>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Snooze {
    /// Unix seconds it comes back to the inbox.
    pub until: f64,
    /// Unix seconds it was snoozed.
    pub at: f64,
    /// The usage limit's `resume_at` this snooze waits out, when it snoozed a limited thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// What Unarchive needs to start the conversation again, and what the Archive page shows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Archived {
    /// The machine whose tab it was, which resumes it.
    pub machine: MachineId,
    /// The tab's folder, where the session resumes.
    pub cwd: String,
    /// The thread's title when it was archived.
    pub name: String,
    pub project: String,
    /// Unix seconds it was archived.
    pub at: f64,
    /// Unix seconds of its first message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<f64>,
    /// The name the user had given it, back on the thread when it is unarchived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// It was pinned, and is pinned again when it is unarchived.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
}

/// One change to one mark. `{"title":"Fix login"}`, `{"pin":null}`, `"clear"`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Title(Option<String>),
    Pin(Option<u32>),
    Order(Option<u32>),
    Settled(Option<f64>),
    Unsettled(Option<f64>),
    Snoozed(Option<Snooze>),
    Seen(Option<f64>),
    NoAutoSettle(bool),
    Archived(Option<Archived>),
    /// Drops every mark of the session.
    Clear,
}

/// `{"session":"5f1c…","mark":{"title":"Fix login"}}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct MarkChange {
    pub session: String,
    pub mark: Mark,
}

impl SessionMarks {
    pub fn new(session: &str) -> SessionMarks {
        SessionMarks { session: session.into(), ..SessionMarks::default() }
    }

    pub fn apply(&mut self, mark: Mark) {
        match mark {
            Mark::Title(title) => self.title = title,
            Mark::Pin(pin) => self.pin = pin,
            Mark::Order(order) => self.order = order,
            Mark::Settled(at) => self.settled = at,
            Mark::Unsettled(at) => self.unsettled = at,
            Mark::Snoozed(snooze) => self.snoozed = snooze,
            Mark::Seen(at) => self.seen = at,
            Mark::NoAutoSettle(off) => self.no_auto_settle = off,
            Mark::Archived(archived) => self.archived = archived,
            Mark::Clear => *self = SessionMarks::new(&self.session),
        }
    }

    /// No mark left, so the hub keeps no row for it.
    pub fn is_empty(&self) -> bool {
        *self == SessionMarks::new(&self.session)
    }
}

/// Refuses a call whose changes a hub should not keep: too many, an empty or overlong session id,
/// or overlong text.
pub fn check(changes: &[MarkChange]) -> Result<(), String> {
    if changes.len() > MAX_CHANGES {
        return Err(format!("make at most {MAX_CHANGES} mark changes at once"));
    }
    for change in changes {
        if change.session.is_empty() || change.session.len() > MAX_TEXT {
            return Err("a mark needs a session id".into());
        }
        let text = match &change.mark {
            Mark::Title(Some(title)) => title.len(),
            Mark::Archived(Some(a)) => a.cwd.len().max(a.name.len()).max(a.project.len()).max(a.machine.len()).max(a.title.as_ref().map_or(0, String::len)),
            _ => 0,
        };
        if text > MAX_TEXT {
            return Err(format!("a mark's text may be at most {MAX_TEXT} bytes"));
        }
    }
    Ok(())
}

/// Applies `changes` in order to `rows`, the current marks of the sessions they name, and answers
/// every session they touched as it is now, in the order each was first named.
pub fn apply(rows: &mut BTreeMap<String, SessionMarks>, changes: Vec<MarkChange>) -> Vec<SessionMarks> {
    let mut touched: Vec<String> = vec![];
    for MarkChange { session, mark } in changes {
        if !touched.contains(&session) {
            touched.push(session.clone());
        }
        rows.entry(session.clone()).or_insert_with(|| SessionMarks::new(&session)).apply(mark);
    }
    touched.into_iter().map(|session| rows[&session].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change(session: &str, mark: Mark) -> MarkChange {
        MarkChange { session: session.into(), mark }
    }

    #[test]
    fn a_change_reads_as_the_mark_it_sets() {
        let parsed: Vec<MarkChange> = serde_json::from_value(json!([
            { "session": "s1", "mark": { "title": "Fix login" } },
            { "session": "s1", "mark": { "pin": null } },
            { "session": "s2", "mark": { "snoozed": { "until": 200.0, "at": 100.0 } } },
            { "session": "s2", "mark": "clear" },
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            vec![
                change("s1", Mark::Title(Some("Fix login".into()))),
                change("s1", Mark::Pin(None)),
                change("s2", Mark::Snoozed(Some(Snooze { until: 200.0, at: 100.0, limit: None }))),
                change("s2", Mark::Clear),
            ]
        );
    }

    #[test]
    fn changes_land_in_order_and_answer_each_session_once() {
        let mut rows = BTreeMap::from([("s1".to_string(), SessionMarks { pin: Some(0), ..SessionMarks::new("s1") })]);
        let touched = apply(
            &mut rows,
            vec![
                change("s2", Mark::Title(Some("Draft".into()))),
                change("s1", Mark::Settled(Some(50.0))),
                change("s2", Mark::Title(Some("Final".into()))),
                change("s1", Mark::Pin(None)),
            ],
        );
        assert_eq!(
            serde_json::to_value(&touched).unwrap(),
            json!([{ "session": "s2", "title": "Final" }, { "session": "s1", "settled": 50.0 }])
        );
    }

    #[test]
    fn clearing_every_mark_leaves_an_empty_row() {
        let mut rows = BTreeMap::new();
        let touched = apply(&mut rows, vec![change("s1", Mark::NoAutoSettle(true)), change("s1", Mark::Seen(Some(9.0))), change("s1", Mark::Clear)]);
        assert!(touched[0].is_empty());
        assert_eq!(serde_json::to_value(&touched[0]).unwrap(), json!({ "session": "s1" }));
    }

    #[test]
    fn overlong_text_and_missing_ids_are_refused() {
        assert_eq!(check(&[change("", Mark::Clear)]), Err("a mark needs a session id".into()));
        assert_eq!(check(&[change("s1", Mark::Title(Some("x".repeat(MAX_TEXT + 1))))]), Err(format!("a mark's text may be at most {MAX_TEXT} bytes")));
        assert_eq!(check(&vec![change("s1", Mark::Clear); MAX_CHANGES + 1]), Err(format!("make at most {MAX_CHANGES} mark changes at once")));
        assert_eq!(check(&[change("s1", Mark::Title(Some("ok".into())))]), Ok(()));
    }
}
