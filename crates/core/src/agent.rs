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

const ORCHESTRATOR_PROMPT: &str = "You are this project's Capitão in pinned: the session that leads its workers. Talk like a session, not a dispatcher: think out loud, explain your plan, and report substance. Prefer delegating over doing it yourself. Before fanning out, look up memory_search for how this project likes to work (worker counts, model preferences, conventions) and save what you learn with memory_save. Hand every piece of work to a worker with the delegate_task tool, and write each brief so it stands alone: the goal, the files involved, the constraints, and how to verify the result. Pass isolation \"worktree\" when a worker changes files, so it works in its own checkout. Grant each worker the tools it needs with the tools argument; an empty grant leaves that to the task. When angles genuinely disagree, run one worker per angle on the same brief marked explore-only, then compare their reports and continue with the best, telling the user what each found. When a worker is stuck or failed, first try to unblock it: re-brief with narrower scope, grant wider tools, or hand the task to another worker on a different model. Step in and do the task yourself only when reassigning won't fix it, and say so when you do. Workers are threads the user can open and steer. A worker may start workers of its own, whose news goes to it rather than to you; agents lists your whole tree. To run a worker on another account, provider, model or effort, such as Cursor or Codex, call orchestrator_capabilities and pass its choice as target. When you start workers, answer with what you understood, how you split the work, and one line per worker saying what you asked it to do, with the key files and how it should verify. You get a message when a worker finishes or needs input: report what it did or found and what happens next, never just that it finished. A worker that needs more tools says NEEDS followed by the tools: answer with a wider grant. Keep the project's memory: save durable facts with memory_save (the user's preferences, decisions, conventions, architecture facts), look them up with memory_search, and remove wrong ones with memory_forget.";
const WORKER_PROMPT: &str = "You are a worker in pinned, started by another session of this project. Do the task you were given fully and verify it. Stay within your granted tools when the brief names any; when you need more, say NEEDS followed by the tools and keep working with what you have. An explore-only brief means investigate and report, changing nothing. When a part of the task stands on its own, you may hand it to a worker of your own with the delegate_task tool: you get a message when it finishes or needs input, and agents lists your workers. End with a concise report of what changed and how you verified it, which the session that started you reads. Save durable facts about the project you learn with memory_save.";

/// The part a session plays in its project's orchestration.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Hands every request to workers.
    Orchestrator,
    /// Does one task another session delegated, and may delegate parts of it in turn.
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

    /// Tools the role never gets. The orchestrator keeps them all: it prefers delegating,
    /// but steps in and edits itself when a worker is stuck or failed and reassigning won't fix it.
    pub fn disallowed_tools(self) -> &'static [&'static str] {
        match self {
            Role::Orchestrator => &[],
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
    Agents,
    /// What the thread's work cost: its own session, the workers it delegated and their boxes.
    /// With `task`, that task and the workers under it instead.
    WorkCost {
        #[serde(default, rename = "taskId", skip_serializing_if = "Option::is_none")]
        task: Option<String>,
    },
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
    /// The global orchestrator's: every project of the account, the machines it is on and its Capitão.
    Projects,
    /// The global orchestrator's: hands `message` to the Capitão of `project`, a key `projects`
    /// lists or a folder of this machine, on `machine` when given.
    SendToCaptain {
        project: String,
        message: String,
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        /// Opens the Capitão in an Upstash Box instead: `true`, or `{ size }`.
        #[serde(default, rename = "box", skip_serializing_if = "Option::is_none", deserialize_with = "crate::delegate::box_or_true")]
        boxed: Option<crate::delegate::BoxTarget>,
    },
    /// The global orchestrator's: the latest replies of `project`'s Capitão.
    ReadCaptain {
        project: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// The global orchestrator's: what waits on the user across the account's machines.
    Waiting,
    /// The global orchestrator's: every session of the account, in `project` and on `machine`
    /// when given.
    Sessions {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// The global orchestrator's: the last `items` items of the session in tab `id`.
    ReadSession {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items: Option<usize>,
    },
    /// The global orchestrator's: `repo_look::RepoLook::Overview` of `project`'s checkout.
    RepoOverview {
        project: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// The global orchestrator's: `repo_look::RepoLook::Read` of `path` in `project`'s checkout.
    RepoRead {
        project: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_line: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<u32>,
    },
    /// The global orchestrator's: `repo_look::RepoLook::Search` in `project`'s checkout.
    RepoSearch {
        project: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        glob: Option<String>,
        #[serde(default)]
        regex: bool,
        #[serde(default)]
        case: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u32>,
    },
    /// A Capitão's: tells the global orchestrator `text` now, mid-work.
    TellOrchestrator { text: String },
    /// A taste chat's: `taste::ScopeView` of `scope`, as `TasteScope::parse` reads it.
    TasteGet { scope: String },
    /// A taste chat's: `Tastes::set` on the account's tastes, then `TasteGet`.
    TasteSet { scope: String, patch: crate::taste::TastePatch },
    /// A taste chat's: `Tastes::reset` on the account's tastes, then `TasteGet`.
    TasteReset { scope: String, field: crate::taste::TasteField },
    /// Everyone's: `surface::listing`, the actions `Act` runs for this session.
    Actions {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        search: Option<String>,
    },
    /// A Convex tool's ask, which the hub answers for the tab's project. `offer` comes from
    /// `pinned mcp` itself, at tools/list.
    Convex { ask: crate::convex::Ask },
    /// Everyone's: opens a questionnaire to the user on the session's tab and answers its id. A
    /// headless child has no tab: it sends its `task`, and the questionnaire goes on the tab that
    /// shows that task, or its parent's.
    QuestionUser {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        questions: Vec<crate::questions::Question>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        projects: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
    },
    /// How the questionnaire `id` stands: `{"outcome":"open"}` while it waits, else the tool's
    /// result. Only the caller that asked may read it; a headless child sends its `task`.
    QuestionStatus {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
    },
    /// Everyone's: one action of `surface::ACTIONS`, run by the hub as the user's account and audited.
    Act {
        action: String,
        #[serde(default)]
        args: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
}
/// Set on sessions a connector started, so `pinned stage` knows which issue and flow it reports on.
pub const CONNECTOR_ENV: &str = "PINNED_CONNECTOR";
pub const ISSUE_ENV: &str = "PINNED_ISSUE";
/// Set on sessions a Slack connector started, so `pinned slack reply` knows the thread it answers.
pub const SLACK_CHANNEL_ENV: &str = "PINNED_SLACK_CHANNEL";
pub const SLACK_THREAD_ENV: &str = "PINNED_SLACK_THREAD";

/// The connector interactive box sessions run under. The hub sees their tabs through it, and the
/// app treats their first prompt as the user's own rather than automation's.
pub const INTERACTIVE_CONNECTOR: &str = "session";
/// The connector a `delegate_task` worker's box runs under.
pub const DELEGATE_CONNECTOR: &str = "delegate";
/// The connector a Captain's box, opened by the global orchestrator, runs under.
pub const CAPTAIN_CONNECTOR: &str = "captain";
/// Starts the connector id a session an external agent drives through the hub's MCP endpoint
/// carries, `mcp:<session>`, which finds its tab again on any machine and across the machine's
/// restarts, since a tab's environment is saved with it.
pub const EXTERNAL_CONNECTOR_PREFIX: &str = "mcp:";

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
    /// The turn's last assistant message, as the Stop hook hands it over. Claude Code runs that hook
    /// before the message reaches the transcript, so a reader of the file can miss it. It stays on
    /// the way from the hook to the machine: out of the schema, and cleared before the event is sent on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub reply: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly: Option<LimitWindow>,
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
            monthly: self.monthly.or(live(known.monthly)),
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
    /// `pinned run set <name> <value>`: a value the session's graph run reads as `{{vars.<name>}}`.
    RunSet {
        connector: String,
        #[serde(flatten)]
        subject: Subject,
        name: String,
        value: String,
    },
    Remind {
        message: String,
        when: When,
    },
    ListReminders,
    CancelReminder { id: u64 },
    /// Not flattened, since a tool's own `kind` argument would collide with the message's tag.
    Project { tab: String, tool: ProjectTool },
    /// Claude Code's PreToolUse hook in the global orchestrator's session: may tool `tool` run
    /// with `input`? Ok("") lets it, Err says why not.
    Guard { tab: String, tool: String, input: serde_json::Value },
    Usage {
        #[serde(flatten)]
        usage: Usage,
        /// The account whose usage this is; sessions of pinned before accounts name none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<AccountId>,
    },
    /// Asks the machine to read an OpenCode Go account's allowance in the background. The
    /// statusline sends this and returns without waiting on the gateway.
    RefreshUsage { account: AccountId },
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
    /// The delegator's verdict on a finished task: `accepted` or `redone`.
    TaskReview {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        id: String,
        verdict: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// The account's delegation totals over the last `days`.
    DelegationStats {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        days: Option<u64>,
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
            monthly: Some(LimitWindow { used_percentage: 80.0, resets_at: 1793400000 }),
        };
        let report = Usage {
            five_hour: None,
            seven_day: Some(LimitWindow {
                used_percentage: 42.0,
                resets_at: 1792900000,
            }),
            monthly: None,
        };
        assert_eq!(
            report.over(known, 1792399999),
            Usage {
                five_hour: Some(WINDOW),
                seven_day: Some(LimitWindow {
                    used_percentage: 42.0,
                    resets_at: 1792900000
                }),
                monthly: known.monthly,
            },
        );
        assert_eq!(report.over(known, 1792400000).five_hour, None);
        assert_eq!(report.over(known, 1793400000).monthly, None, "a monthly window leaves once it resets");
        assert_eq!(
            Usage::default().over(known, 1792400000),
            Usage {
                five_hour: None,
                seven_day: known.seven_day,
                monthly: known.monthly,
            }
        );
    }

    #[test]
    fn a_project_tool_message_names_the_tab_and_the_tool() {
        let line = r#"{"kind":"project","tab":"tab-3","tool":{"name":"agents"}}"#;
        let message: Message = serde_json::from_str(line).unwrap();
        assert!(matches!(&message, Message::Project { tab, tool } if tab == "tab-3" && *tool == ProjectTool::Agents));
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
            matches!(&message, Message::Usage { usage, account: None } if *usage == Usage { five_hour: Some(WINDOW), seven_day: None, monthly: None })
        );
        assert_eq!(serde_json::to_string(&message).unwrap(), line);
        let monthly = r#"{"kind":"usage","monthly":{"used_percentage":100.0,"resets_at":1793400000}}"#;
        let message: Message = serde_json::from_str(monthly).unwrap();
        assert!(matches!(&message, Message::Usage { usage, .. } if usage.monthly == Some(LimitWindow { used_percentage: 100.0, resets_at: 1793400000 })));
        assert_eq!(serde_json::to_string(&message).unwrap(), monthly);
        let line = r#"{"kind":"usage","five_hour":{"used_percentage":23.5,"resets_at":1792400000},"account":"work"}"#;
        let message: Message = serde_json::from_str(line).unwrap();
        assert!(matches!(&message, Message::Usage { account: Some(a), .. } if a == "work"));
        assert_eq!(serde_json::to_string(&message).unwrap(), line);
    }
}
