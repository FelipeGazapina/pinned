use serde::{Deserialize, Serialize};

pub const SOCK_ENV: &str = "PINNED_SOCK";
pub const TAB_ENV: &str = "PINNED_TAB";
/// Set on sessions a connector started, so `pinned stage` knows which issue and flow it reports on.
pub const CONNECTOR_ENV: &str = "PINNED_CONNECTOR";
pub const ISSUE_ENV: &str = "PINNED_ISSUE";
/// Set on sessions a Slack connector started, so `pinned slack reply` knows the thread it answers.
pub const SLACK_CHANNEL_ENV: &str = "PINNED_SLACK_CHANNEL";
pub const SLACK_THREAD_ENV: &str = "PINNED_SLACK_THREAD";

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

/// What a connector's session works on, which its callbacks into pinned report against.
/// Untagged, so a stage message from a Linear session reads `"issue": "<id>"` as it always has.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Subject {
    /// Linear's id for the issue, which `pinned stage` moves.
    LinearIssue { issue: String },
    /// The Slack thread `pinned slack reply` answers in, and whose filed issue `pinned stage` moves.
    SlackThread { channel: String, thread: String },
}

impl Subject {
    /// The session's environment that names this subject.
    pub fn env(&self) -> Vec<(String, String)> {
        match self {
            Subject::LinearIssue { issue } => vec![(ISSUE_ENV.to_owned(), issue.clone())],
            Subject::SlackThread { channel, thread } => {
                vec![(SLACK_CHANNEL_ENV.to_owned(), channel.clone()), (SLACK_THREAD_ENV.to_owned(), thread.clone())]
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum When {
    InMinutes(f64),
    /// 24-hour `HH:MM` on the local clock.
    At(String),
}

/// How much of the subscription's rate limits the account has used, as Claude Code's statusline
/// data reports it. Claude Code leaves out a window it has no figure for.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<LimitWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<LimitWindow>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct LimitWindow {
    /// From 0 to 100.
    pub used_percentage: f64,
    /// Unix time the window resets.
    pub resets_at: u64,
}

impl Usage {
    /// This report over the `known` one: a window this report left out keeps its known figure
    /// until that window resets.
    pub fn over(self, known: Usage, now: u64) -> Usage {
        let live = |w: Option<LimitWindow>| w.filter(|w| w.resets_at > now);
        Usage { five_hour: self.five_hour.or(live(known.five_hour)), seven_day: self.seven_day.or(live(known.seven_day)) }
    }
}

/// One line on the harness socket.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Message {
    Agent(AgentEvent),
    Stage {
        connector: String,
        #[serde(flatten)]
        subject: Subject,
        stage: Stage,
    },
    SlackReply { connector: String, channel: String, thread: String, text: String },
    LinearCreate { connector: String, channel: String, thread: String, title: String, description: String },
    Remind { message: String, when: When },
    ListReminders,
    CancelReminder { id: u64 },
    Usage(Usage),
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: LimitWindow = LimitWindow { used_percentage: 23.5, resets_at: 1792400000 };

    #[test]
    fn a_window_left_out_keeps_its_figure_only_until_it_resets() {
        let known = Usage { five_hour: Some(WINDOW), seven_day: Some(LimitWindow { used_percentage: 41.2, resets_at: 1792900000 }) };
        let report = Usage { five_hour: None, seven_day: Some(LimitWindow { used_percentage: 42.0, resets_at: 1792900000 }) };
        assert_eq!(
            report.over(known, 1792399999),
            Usage { five_hour: Some(WINDOW), seven_day: Some(LimitWindow { used_percentage: 42.0, resets_at: 1792900000 }) },
        );
        assert_eq!(report.over(known, 1792400000).five_hour, None);
        assert_eq!(Usage::default().over(known, 1792400000), Usage { five_hour: None, seven_day: known.seven_day });
    }

    #[test]
    fn a_usage_message_keeps_its_wire_shape() {
        let line = r#"{"kind":"usage","five_hour":{"used_percentage":23.5,"resets_at":1792400000}}"#;
        let message: Message = serde_json::from_str(line).unwrap();
        assert!(matches!(&message, Message::Usage(usage) if *usage == Usage { five_hour: Some(WINDOW), seven_day: None }));
        assert_eq!(serde_json::to_string(&message).unwrap(), line);
    }
}
