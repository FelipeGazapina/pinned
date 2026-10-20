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
}

impl AgentStatus {
    pub const ALL: [(AgentStatus, &'static str, &'static str); 3] = [
        (AgentStatus::Working, "working", "UserPromptSubmit"),
        (AgentStatus::Done, "done", "Stop"),
        (AgentStatus::NeedsInput, "needs-input", "Notification"),
    ];
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentEvent {
    pub status: AgentStatus,
    pub cwd: Option<String>,
    pub message: Option<String>,
    /// Id of the tab whose session fired the hook.
    pub tab: Option<String>,
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

/// One line on the harness socket.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Message {
    Agent(AgentEvent),
    Stage { connector: String, issue: String, stage: Stage },
}
