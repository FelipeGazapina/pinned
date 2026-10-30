use crate::accounts::AccountId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SOCK_ENV: &str = "PINNED_SOCK";
pub const TAB_ENV: &str = "PINNED_TAB";
pub const TASK_ENV: &str = "PINNED_TASK";

/// Set on a project's orchestrator and its workers, with the project in `PROJECT_ENV`, since a
/// worker runs in a worktree whose folder is not the project's.
pub const ROLE_ENV: &str = "PINNED_ROLE";
pub const PROJECT_ENV: &str = "PINNED_PROJECT";

const ORCHESTRATOR_PROMPT: &str = "You are this project's orchestrator in pinned. You cannot edit files. Hand every piece of work to a worker with the delegate tool, and write each brief so it stands alone: the goal, the files involved, the constraints, and how to verify the result. Workers are threads the user can open and steer; agents lists them. To run a task on another provider or model, such as Cursor or Codex, call orchestrator_capabilities and then delegate_task. You get a message when a worker finishes or needs input: review what it reports, and tell the user what was done. Keep the project's memory: save durable facts with memory_save (the user's preferences, decisions, conventions, architecture facts), look them up with memory_search, and remove wrong ones with memory_forget.";
const WORKER_PROMPT: &str = "You are a worker of this project's orchestrator in pinned. Do the task you were given fully and verify it. End with a concise report of what changed and how you verified it, which the orchestrator reads. Save durable facts about the project you learn with memory_save.";

/// The part a session plays in its project's orchestration.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Hands every request to workers.
    Orchestrator,
    /// Does one task the orchestrator delegated.
    Worker,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Orchestrator => "orchestrator",
            Role::Worker => "worker",
        }
    }

    pub fn parse(name: &str) -> Option<Role> {
        [Role::Orchestrator, Role::Worker].into_iter().find(|r| r.name() == name)
    }

    /// What the role's sessions are told on top of Claude Code's own system prompt.
    pub fn prompt(self) -> &'static str {
        match self {
            Role::Orchestrator => ORCHESTRATOR_PROMPT,
            Role::Worker => WORKER_PROMPT,
        }
    }

    pub fn disallowed_tools(self) -> &'static [&'static str] {
        match self {
            Role::Orchestrator => &["Edit", "Write", "NotebookEdit"],
            Role::Worker => &[],
        }
    }
}

/// The role and project a session's environment names, for an orchestrator's or a worker's session.
pub fn role_of(env: &[(String, String)]) -> Option<(Role, PathBuf)> {
    let var = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    Some((Role::parse(var(ROLE_ENV)?)?, PathBuf::from(var(PROJECT_ENV)?)))
}

pub fn role_env(role: Role, project: &Path) -> Vec<(String, String)> {
    vec![(ROLE_ENV.to_owned(), role.name().to_owned()), (PROJECT_ENV.to_owned(), project.display().to_string())]
}

/// What an orchestrator or a worker asks of its project through pinned's MCP tools.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "name", rename_all = "snake_case")]
pub enum ProjectTool {
    /// Starts a worker on `task`, in a worktree of its own when the project is a git repository.
    Delegate {
        task: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Agents,
    MemorySearch {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    MemorySave {
        title: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<i64>,
    },
    MemoryForget { id: i64 },
}
/// Set on sessions a connector started, so `pinned stage` knows which issue and flow it reports on.
pub const CONNECTOR_ENV: &str = "PINNED_CONNECTOR";
pub const ISSUE_ENV: &str = "PINNED_ISSUE";
/// Set on sessions a Slack connector started, so `pinned slack reply` knows the thread it answers.
pub const SLACK_CHANNEL_ENV: &str = "PINNED_SLACK_CHANNEL";
pub const SLACK_THREAD_ENV: &str = "PINNED_SLACK_THREAD";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, schemars::JsonSchema)]
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

#[derive(Serialize, Deserialize, Clone, Debug, schemars::JsonSchema)]
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
    /// Claude Code's permission mode, such as `plan` or `default`. Absent when the hook does not report one.
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub hook_feedback: bool,
}

/// Where a connector's session is in its work on an issue. Each stage maps to a Linear column.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, schemars::JsonSchema)]
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
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
                vec![
                    (SLACK_CHANNEL_ENV.to_owned(), channel.clone()),
                    (SLACK_THREAD_ENV.to_owned(), thread.clone()),
                ]
            }
        }
    }
}

/// The connector and subject a session's environment names, for a connector's session.
pub fn session_of(env: &[(String, String)]) -> Option<(String, Subject)> {
    let var = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let subject = match (
        var(ISSUE_ENV),
        var(SLACK_CHANNEL_ENV),
        var(SLACK_THREAD_ENV),
    ) {
        (_, Some(channel), Some(thread)) => Subject::SlackThread { channel, thread },
        (Some(issue), _, _) => Subject::LinearIssue { issue },
        _ => return None,
    };
    Some((var(CONNECTOR_ENV)?, subject))
}

pub fn session_env(connector: &str, subject: &Subject) -> Vec<(String, String)> {
    let mut env = vec![(CONNECTOR_ENV.to_owned(), connector.to_owned())];
    env.extend(subject.env());
    env
}

#[derive(Serialize, Deserialize, Debug, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum When {
    InMinutes(f64),
    /// 24-hour `HH:MM` on the local clock.
    At(String),
}

/// How much of the subscription's rate limits the account has used, as Claude Code's statusline
/// data reports it. Claude Code leaves out a window it has no figure for.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<LimitWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<LimitWindow>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, schemars::JsonSchema)]
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
        Usage {
            five_hour: self.five_hour.or(live(known.five_hour)),
            seven_day: self.seven_day.or(live(known.seven_day)),
        }
    }
}

/// One line on the harness socket.
#[derive(Serialize, Deserialize, Debug, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Message {
    Agent(AgentEvent),
    Stage {
        connector: String,
        #[serde(flatten)]
        subject: Subject,
        stage: Stage,
    },
    SlackReply {
        connector: String,
        channel: String,
        thread: String,
        text: String,
    },
    LinearCreate {
        connector: String,
        channel: String,
        thread: String,
        title: String,
        description: String,
        /// Who the issue is assigned to, by name or email; empty assigns the Linear account's owner.
        #[serde(default)]
        assignee: String,
    },
    LinearTake {
        connector: String,
        channel: String,
        thread: String,
        issue: String,
    },
    Remind {
        message: String,
        when: When,
    },
    ListReminders,
    CancelReminder { id: u64 },
    /// Not flattened, since a tool's own `kind` argument would collide with the message's tag.
    Project { tab: String, tool: ProjectTool },
    Usage {
        #[serde(flatten)]
        usage: Usage,
        /// The account whose usage this is; sessions of pinned before accounts name none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<AccountId>,
    },
    /// Starts a child agent. `tab` is the session that asked, when it has one. `task` is the
    /// parent task when the asker is itself a child (`PINNED_TASK`).
    Delegate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
        request: Box<crate::delegate::Request>,
    },
    TaskStatus {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        id: String,
    },
    TaskCancel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        id: String,
        #[serde(default)]
        reason: String,
    },
    Capabilities {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        #[serde(default)]
        search: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: LimitWindow = LimitWindow {
        used_percentage: 23.5,
        resets_at: 1792400000,
    };

    #[test]
    fn a_window_left_out_keeps_its_figure_only_until_it_resets() {
        let known = Usage {
            five_hour: Some(WINDOW),
            seven_day: Some(LimitWindow {
                used_percentage: 41.2,
                resets_at: 1792900000,
            }),
        };
        let report = Usage {
            five_hour: None,
            seven_day: Some(LimitWindow {
                used_percentage: 42.0,
                resets_at: 1792900000,
            }),
        };
        assert_eq!(
            report.over(known, 1792399999),
            Usage {
                five_hour: Some(WINDOW),
                seven_day: Some(LimitWindow {
                    used_percentage: 42.0,
                    resets_at: 1792900000
                })
            },
        );
        assert_eq!(report.over(known, 1792400000).five_hour, None);
        assert_eq!(
            Usage::default().over(known, 1792400000),
            Usage {
                five_hour: None,
                seven_day: known.seven_day
            }
        );
    }

    #[test]
    fn a_project_tool_message_names_the_tab_and_the_tool() {
        let line = r#"{"kind":"project","tab":"tab-3","tool":{"name":"delegate","task":"Add a --json flag","title":"JSON output"}}"#;
        let message: Message = serde_json::from_str(line).unwrap();
        let delegate = ProjectTool::Delegate { task: "Add a --json flag".into(), title: Some("JSON output".into()) };
        assert!(matches!(&message, Message::Project { tab, tool } if tab == "tab-3" && *tool == delegate));
        assert_eq!(serde_json::to_string(&message).unwrap(), line);
        let save: Message = serde_json::from_str(r#"{"kind":"project","tab":"tab-3","tool":{"name":"memory_save","title":"Deploys","text":"From tags.","kind":"decision","id":2}}"#).unwrap();
        let tool = ProjectTool::MemorySave { title: "Deploys".into(), text: "From tags.".into(), kind: Some("decision".into()), id: Some(2) };
        assert!(matches!(&save, Message::Project { tool: t, .. } if *t == tool));
    }

    #[test]
    fn a_sessions_environment_names_its_role_and_project() {
        let env = [vec![(CONNECTOR_ENV.to_owned(), "c1".to_owned())], role_env(Role::Worker, Path::new("/repo"))].concat();
        assert_eq!(env[1..], [("PINNED_ROLE".to_owned(), "worker".to_owned()), ("PINNED_PROJECT".to_owned(), "/repo".to_owned())]);
        assert_eq!(role_of(&env), Some((Role::Worker, PathBuf::from("/repo"))));
        assert_eq!(role_of(&[("PINNED_ROLE".into(), "boss".into()), ("PINNED_PROJECT".into(), "/repo".into())]), None);
        assert_eq!(role_of(&[("PINNED_ROLE".into(), "orchestrator".into())]), None, "a role without its project");
    }

    #[test]
    fn a_usage_message_keeps_its_wire_shape() {
        let line =
            r#"{"kind":"usage","five_hour":{"used_percentage":23.5,"resets_at":1792400000}}"#;
        let message: Message = serde_json::from_str(line).unwrap();
        assert!(
            matches!(&message, Message::Usage { usage, account: None } if *usage == Usage { five_hour: Some(WINDOW), seven_day: None })
        );
        assert_eq!(serde_json::to_string(&message).unwrap(), line);
        let line = r#"{"kind":"usage","five_hour":{"used_percentage":23.5,"resets_at":1792400000},"account":"work"}"#;
        let message: Message = serde_json::from_str(line).unwrap();
        assert!(matches!(&message, Message::Usage { account: Some(a), .. } if a == "work"));
        assert_eq!(serde_json::to_string(&message).unwrap(), line);
    }
}
