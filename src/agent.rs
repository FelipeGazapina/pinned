use serde::{Deserialize, Serialize};

pub const SOCK_ENV: &str = "PINNED_SOCK";
pub const TAB_ENV: &str = "PINNED_TAB";
/// Set on sessions a connector started, so `pinned stage` knows which issue and flow it reports on.
pub const CONNECTOR_ENV: &str = "PINNED_CONNECTOR";
pub const ISSUE_ENV: &str = "PINNED_ISSUE";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentStatus {
    Working,
    Done,
    NeedsInput,
    /// The turn died on an API error, such as a usage limit.
    Failed,
}

impl AgentStatus {
    pub const ALL: [(AgentStatus, &'static str, &'static str); 4] = [
        (AgentStatus::Working, "working", "UserPromptSubmit"),
        (AgentStatus::Done, "done", "Stop"),
        (AgentStatus::NeedsInput, "needs-input", "Notification"),
        (AgentStatus::Failed, "failed", "StopFailure"),
    ];
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentEvent {
    pub status: AgentStatus,
    pub cwd: Option<String>,
    pub message: Option<String>,
    /// Id of the tab whose session fired the hook.
    pub tab: Option<String>,
    /// Claude Code's session id, which `claude --resume` takes.
    pub session: Option<String>,
    /// A few words on what the session is about, once Claude Code named it or the user prompted.
    pub title: Option<String>,
    /// Unix time the usage limit that stopped the turn lifts, when the failure was one.
    pub resume_at: Option<u64>,
    /// What the user asked, on the event that starts a turn.
    pub prompt: Option<String>,
    /// Git tree of the working tree, on the events that start and end a turn.
    pub snapshot: Option<String>,
}

/// Where a connector's session is in its work on an issue. Each stage maps to a Linear column.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// The harness picked the issue up and opened a session for it.
    Picked,
    /// The session is about to change code.
    Coding,
    /// The session opened the pull request.
    Review,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum When {
    InMinutes(f64),
    /// 24-hour `HH:MM` on the local clock.
    At(String),
}

/// One line on the harness socket.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Message {
    Agent(AgentEvent),
    Stage { connector: String, issue: String, stage: Stage },
    Remind { message: String, when: When },
    ListReminders,
    CancelReminder { id: u64 },
}
