use serde::{Deserialize, Serialize};

pub const SOCK_ENV: &str = "PINNED_SOCK";

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
}
