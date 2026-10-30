//! Walks one run of a graph. Pure: it asks for each side effect as an `Effect`, and the hub
//! performs it and hands the result back with `supply`. The hub stores `Run` after every step, so a
//! restart calls `drive` again and gets the same effect back until it is supplied.
//!
//! One run has one Claude session. The first `talk` opens it; later ones type into it. When the tab
//! is gone the run resumes the same Claude conversation, and when that conversation is gone too it
//! opens a new one seeded with the run's history, so the new session knows everything so far.

use super::{Answers, Graph, NodeId, PortId};
use crate::agent::Subject;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct RunId(pub String);

/// Every graph the hub holds, by connector id. `call` reads its target here.
pub type Library = BTreeMap<String, Graph>;

/// One execution. Owned by the hub, stored beside `connectors.json`, never sent to apps.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Run {
    pub id: RunId,
    /// The connector whose trigger started the run.
    pub connector: String,
    pub subject: Subject,
    pub status: Status,
    /// The connector whose `talk` opened the session; its settings, env and `followed` are the
    /// session's. Fixed once set.
    pub session_owner: Option<String>,
    pub session: Session,
    /// What has happened so far, in order. The history a new session is seeded with.
    pub log: Vec<Entry>,
    /// The data flowing through the run, as n8n's item: `trigger` holds the input, and each
    /// classify node writes its answers under its name.
    pub context: Value,
    // TODO(runner): the private walk state (stack of frames per called graph, cursor per frame,
    // the pending effect) lives here, private to this module.
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Running,
    /// Waiting for the session to end its turn.
    Waiting,
    Done,
    Failed { why: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Session {
    None,
    /// A tab is running the conversation. `claude` is Claude Code's session id once a hook reported it.
    Live { machine: String, tab: String, claude: Option<String> },
    /// The tab is gone. The next `talk` resumes `claude`, or opens a seeded session when it is None.
    Detached { claude: Option<String> },
}

/// One thing that happened, for the log and for seeding a new session.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    /// RFC 3339.
    pub at: String,
    /// The connector whose graph holds `node`; differs from `Run.connector` inside a `call`.
    pub connector: String,
    pub node: NodeId,
    pub what: Value,
}

/// A side effect the hub performs. Every effect is safe to perform twice.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Mark the subject handled on `connector`, so the poll does not start a second run.
    Claim { connector: String, key: String },
    Classify { connector: String, node: NodeId, input: String },
    Page { connector: String, group: String },
    StagePicked { connector: String },
    /// Open the session with `owner`'s settings. `prompt` is the owner's usual opening prompt plus
    /// the talk's text, and, when `seeded`, the run's history.
    Open { owner: String, prompt: String, seeded: bool },
    /// Reopen the tab on `claude --resume <claude>`, then type `text`.
    Resume { owner: String, claude: String, text: String },
    /// Type into the live tab.
    Type { text: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Supplied {
    Done,
    Classified(Answers),
    Opened { machine: String, tab: String },
    /// `claude --resume` found no conversation; the runner opens a seeded session next.
    NoConversation,
    Failed(String),
}

/// News from outside the run that it may be waiting for.
#[derive(Clone, Debug, PartialEq)]
pub enum Wake {
    /// The tab's status went done or failed. `claude` is the id a hook reported, if any.
    TurnEnded { claude: Option<String> },
    TabGone,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Drive {
    /// Perform this, then `supply` the result.
    Act(Effect),
    /// Store the run and wait for a `Wake`.
    Park,
    Done,
}

/// Starts a run at the trigger of `library[connector]`. `input` is the trigger's text: the Linear
/// issue or the Slack message, which goes into `context.trigger` and the first log entry.
pub fn start(library: &Library, connector: &str, subject: Subject, input: Value, at: &str) -> Run {
    let _ = (library, connector, subject, input, at);
    todo!()
}

/// The next step. Returns the same `Act` until `supply` answers it.
pub fn drive(run: &mut Run, library: &Library) -> Drive {
    let _ = (run, library);
    todo!("depth first; classify picks a port via the gate; talk opens, resumes, seeds or types per Session")
}

pub fn supply(run: &mut Run, supplied: Supplied, at: &str) {
    let _ = (run, supplied, at);
    todo!()
}

pub fn push(run: &mut Run, wake: Wake, at: &str) {
    let _ = (run, wake, at);
    todo!()
}

/// The text a new session is seeded with: the run's log and context as JSON, said plainly.
pub fn history(run: &Run) -> String {
    let _ = run;
    todo!()
}

/// The port a classify node takes for `answers`. Pure; `pages(port)` says whether that port's
/// branch pages a group before it reaches a talk, a call or a stop.
pub fn port_for(classify: &super::Classify, answers: &Answers, pages: &dyn Fn(&PortId) -> bool) -> PortId {
    let _ = (classify, answers, pages);
    todo!("Gate::Direct: the answer to branch_on; Gate::Decide: today's Decide::pick")
}
