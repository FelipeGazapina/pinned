//! pinned Teams: an account with no login (`kind = 'team'`) that its members act on through the
//! hub, one door for every call (`HubCall::Team`). The team keeps its workflows, schedules, boxes
//! and keys the way a person's account keeps them, under the team's own account id.
//!
//! Ids are uuids as text; times are unix seconds.

use crate::wire::HubCall;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// What a member may do on a team. The owner made it and holds its keys.
///
/// `"owner"`, `"member"`
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Member => "member",
        }
    }

    /// The role a `team_members.role` column holds; anything else reads as a member.
    pub fn parse(text: &str) -> Role {
        if text == "owner" { Role::Owner } else { Role::Member }
    }
}

/// Which of the team's keys are set, never a value.
///
/// `{"upstash":true,"claude":true,"github":false,"slack":false}`
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct CredentialState {
    #[serde(default)]
    pub upstash: bool,
    #[serde(default)]
    pub claude: bool,
    #[serde(default)]
    pub github: bool,
    #[serde(default)]
    pub slack: bool,
}

/// A team as one of its members sees it, with that member's role.
///
/// `{"id":"6f1c…","name":"Acme","owner":"0b7e…","role":"owner","credential":{"upstash":false,"claude":false,"github":false,"slack":false}}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Team {
    pub id: String,
    pub name: String,
    /// The owner's account id.
    pub owner: String,
    pub role: Role,
    #[serde(default)]
    pub credential: CredentialState,
}

/// `{"account":"0b7e…","email":"ana@acme.com","role":"member","groups":["design"]}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Member {
    pub account: String,
    pub email: String,
    pub role: Role,
    /// The handles of the groups the member is in.
    #[serde(default)]
    pub groups: Vec<String>,
}

/// A new invite, the only time its code is shown. `hint` names it after: its first group.
///
/// `{"code":"ABCD-EFGH-JKMN-PQRS","hint":"ABCD"}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct TeamInvite {
    pub code: String,
    pub hint: String,
}

/// A member's shared project attached to the team: its bots work in it.
///
/// `{"key":"github.com/acme/site","name":"site","repo":"github.com/acme/site","member":"0b7e…"}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct TeamProject {
    pub key: String,
    pub name: String,
    pub repo: String,
    /// The member who attached it.
    pub member: String,
}

/// People a bot asks by one name: `@design`.
///
/// `{"handle":"design","about":"UI calls","members":["0b7e…"],"fallback":null,"slack_group":""}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Group {
    pub handle: String,
    #[serde(default)]
    pub about: String,
    /// Account ids.
    #[serde(default)]
    pub members: Vec<String>,
    /// Who is asked when no member of the group is a member of the team any more.
    #[serde(default)]
    pub fallback: Option<String>,
    /// A Slack user group to mention, empty for none.
    #[serde(default)]
    pub slack_group: String,
}

/// Who a bot's call goes to.
///
/// `{"kind":"person","account":"0b7e…"}`, `{"kind":"group","handle":"design"}`, `{"kind":"owner"}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Who {
    Person { account: String },
    Group { handle: String },
    Owner,
}

impl fmt::Display for Who {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Who::Person { account } => f.write_str(account),
            Who::Group { handle } => write!(f, "@{handle}"),
            Who::Owner => f.write_str("the owner"),
        }
    }
}

/// Why a bot's session stops to ask people.
///
/// `"before_irreversible"`, `"in_doubt"`, `"on_limit"`, `"on_failure"`
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Situation {
    BeforeIrreversible,
    InDoubt,
    OnLimit,
    OnFailure,
}

impl Situation {
    pub fn as_str(self) -> &'static str {
        match self {
            Situation::BeforeIrreversible => "before_irreversible",
            Situation::InDoubt => "in_doubt",
            Situation::OnLimit => "on_limit",
            Situation::OnFailure => "on_failure",
        }
    }
}

/// Who a bot asks in each situation. Every situation goes to the owner by default.
///
/// `{"before_irreversible":{"kind":"owner"},"in_doubt":{"kind":"group","handle":"design"},"on_limit":{"kind":"owner"},"on_failure":{"kind":"owner"}}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct CallRules {
    #[serde(default = "owner")]
    pub before_irreversible: Who,
    #[serde(default = "owner")]
    pub in_doubt: Who,
    #[serde(default = "owner")]
    pub on_limit: Who,
    #[serde(default = "owner")]
    pub on_failure: Who,
}

fn owner() -> Who {
    Who::Owner
}

impl Default for CallRules {
    fn default() -> Self {
        CallRules { before_irreversible: Who::Owner, in_doubt: Who::Owner, on_limit: Who::Owner, on_failure: Who::Owner }
    }
}

impl CallRules {
    pub fn who(&self, situation: Situation) -> &Who {
        match situation {
            Situation::BeforeIrreversible => &self.before_irreversible,
            Situation::InDoubt => &self.in_doubt,
            Situation::OnLimit => &self.on_limit,
            Situation::OnFailure => &self.on_failure,
        }
    }
}

/// What one run of a bot may spend.
///
/// `{"usd":5.0,"minutes":30}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct BotLimits {
    pub usd: f64,
    pub minutes: u32,
}

impl Default for BotLimits {
    fn default() -> Self {
        let policy = TeamPolicy::default();
        BotLimits { usd: policy.run_usd, minutes: policy.run_minutes }
    }
}

/// A team workflow with who it asks and what a run may spend.
///
/// `{"workflow":"c1","name":"Fix bugs","owner":"0b7e…","calls":{…},"limits":{"usd":5.0,"minutes":30},"on":true,"on_by":"0b7e…"}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Bot {
    /// The id of the team account's workflow (a `Connector`).
    pub workflow: String,
    #[serde(default)]
    pub name: String,
    /// The member who answers for the bot.
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub calls: CallRules,
    #[serde(default)]
    pub limits: BotLimits,
    #[serde(default)]
    pub on: bool,
    /// Who turned it on last.
    #[serde(default)]
    pub on_by: Option<String>,
    /// Run once, then discard: when its run ends the bot and its workflow are deleted.
    #[serde(default)]
    pub once: bool,
}

/// `"working"`, `"waiting"`, `"taken_over"`, `"done"`, `"stopped"`, `"gave_up"`, `"failed"`
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Working,
    /// A call of the run waits on people.
    Waiting,
    /// A member drives the session (`Run::controller`).
    TakenOver,
    Done,
    Stopped,
    /// Nobody answered its call within `TeamPolicy::give_up_after_secs`.
    GaveUp,
    Failed,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Working => "working",
            RunState::Waiting => "waiting",
            RunState::TakenOver => "taken_over",
            RunState::Done => "done",
            RunState::Stopped => "stopped",
            RunState::GaveUp => "gave_up",
            RunState::Failed => "failed",
        }
    }

    pub fn parse(text: &str) -> Option<RunState> {
        serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
    }

    /// Whether the run is over: nothing moves it again.
    pub fn ended(self) -> bool {
        matches!(self, RunState::Done | RunState::Stopped | RunState::GaveUp | RunState::Failed)
    }
}

/// `{"id":"…","workflow":"c1","state":"working","controller":null,"cost_usd":0.4,"active_secs":120,"started_at":1763300000,"ended_at":null,"outcome":null,"started_by":"0b7e…"}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Run {
    pub id: String,
    pub workflow: String,
    pub state: RunState,
    /// The member who took it over.
    #[serde(default)]
    pub controller: Option<String>,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub active_secs: u64,
    pub started_at: i64,
    #[serde(default)]
    pub ended_at: Option<i64>,
    #[serde(default)]
    pub outcome: Option<String>,
    /// Who or what started it: a member's account id, or `schedule:<id>`.
    #[serde(default)]
    pub started_by: String,
}

/// How a person answered a call.
///
/// `{"kind":"choice"}`, `{"kind":"pass_to","handle":"design"}`, `{"kind":"take_over"}`, `{"kind":"stop"}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerKind {
    /// `Answer::choice` and `Answer::text` go back to the session.
    Choice,
    /// The call goes to the group `handle` instead.
    PassTo { handle: String },
    TakeOver,
    Stop,
}

/// A client sends `by` and `at` empty; the hub sets them.
///
/// `{"by":"0b7e…","choice":"Ship it","text":"","kind":{"kind":"choice"},"at":1763300000}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Answer {
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub choice: Option<String>,
    #[serde(default)]
    pub text: String,
    pub kind: AnswerKind,
    #[serde(default)]
    pub at: i64,
}

/// A run's question to people.
///
/// `{"id":"…","run":"…","workflow":"c1","situation":"in_doubt","asked":[{"kind":"owner"}],"question":"Which color?","choices":["red","blue"],"created_at":1763300000,"escalated":false,"answer":null}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Call {
    pub id: String,
    pub run: String,
    pub workflow: String,
    pub situation: Situation,
    #[serde(default)]
    pub asked: Vec<Who>,
    pub question: String,
    #[serde(default)]
    pub choices: Vec<String>,
    pub created_at: i64,
    /// Whether it went on to the owner after `TeamPolicy::escalate_after_secs`.
    #[serde(default)]
    pub escalated: bool,
    #[serde(default)]
    pub answer: Option<Answer>,
}

/// The hub's timings and default limits for every team. Not on the wire.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TeamPolicy {
    /// A call nobody answered goes to the owner after this long.
    pub escalate_after_secs: u64,
    /// A call nobody answered ends its run (`RunState::GaveUp`) after this long.
    pub give_up_after_secs: u64,
    /// A new bot's `BotLimits`.
    pub run_usd: f64,
    pub run_minutes: u32,
}

impl Default for TeamPolicy {
    fn default() -> Self {
        TeamPolicy { escalate_after_secs: 1800, give_up_after_secs: 7200, run_usd: 5.0, run_minutes: 30 }
    }
}

impl TeamPolicy {
    /// The policy with its times divided by `divide_by`, each at least 1, for a fast dev hub.
    /// Money is unchanged.
    pub fn scaled(&self, divide_by: u32) -> TeamPolicy {
        let divide_by = divide_by.max(1);
        TeamPolicy {
            escalate_after_secs: (self.escalate_after_secs / u64::from(divide_by)).max(1),
            give_up_after_secs: (self.give_up_after_secs / u64::from(divide_by)).max(1),
            run_usd: self.run_usd,
            run_minutes: (self.run_minutes / divide_by).max(1),
        }
    }
}

/// What a member asks about a team, inside `HubCall::Team`. Every op but `List`, `Create` and
/// `Join` names the team in `HubCall::Team::team`.
///
/// `{"op":"create","name":"Acme"}`, `{"op":"answer_call","id":"…","answer":{"kind":{"kind":"choice"},"choice":"red"}}`,
/// `{"op":"hub","call":{"cmd":"get_settings"}}`, `{"op":"ask_people","situation":"in_doubt","question":"Which color?","choices":["red","blue"]}`
#[derive(Serialize, Deserialize, Clone, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TeamOp {
    /// `Vec<Team>`: the teams the person is a member of.
    List,
    /// `Team`: a new team the person owns.
    Create { name: String },
    /// `Team`: joins by an invite code.
    Join { code: String },
    /// Owner only.
    Rename { name: String },
    /// `Vec<Member>`.
    Members,
    /// Owner only. `{"code":"ABCD-…","hint":"ABCD"}`: the code is shown once.
    Invite,
    /// Owner only: the invite whose code starts with `hint`.
    RevokeInvite { hint: String },
    /// Owner only.
    RemoveMember { account: String },
    /// A member leaves; the owner cannot.
    Leave,
    /// Attaches the person's own shared project `key` to the team.
    AttachProject { key: String },
    DetachProject { key: String },
    /// `Vec<TeamProject>`.
    Projects,
    /// `Vec<Group>`.
    Groups,
    SaveGroup { group: Group },
    JoinGroup { handle: String },
    LeaveGroup { handle: String },
    /// `Vec<Bot>`.
    Bots,
    SaveBot { bot: Bot },
    TurnBotOn { workflow: String, on: bool },
    /// `Run`.
    RunNow { workflow: String },
    /// `Vec<Run>`.
    Runs,
    /// `Vec<Call>`: the open calls, and the last answered.
    Calls,
    AnswerCall { id: String, answer: Answer },
    TakeOver { run: String },
    GiveBack { run: String },
    StopRun { run: String },
    /// Text to the team's chat; answers its reply.
    Chat { text: String },
    /// A personal `HubCall` run on the team's account, if the hub's `hub_call_allowed` lets the
    /// role. A `HubCall` as JSON, which the hub parses: a recursive schema overflows the wire
    /// checker's stack.
    Hub { call: Value },
    /// From a box's session only, on the team account's machine: asks the people its bot's
    /// `CallRules` name for `situation`. The session never names people.
    AskPeople {
        situation: Situation,
        question: String,
        #[serde(default)]
        choices: Vec<String>,
    },
}

impl TeamOp {
    /// The personal call inside `Hub`, as the hub reads it.
    pub fn hub_call(call: Value) -> Result<HubCall, String> {
        serde_json::from_value(call).map_err(|e| format!("not a hub call: {e}"))
    }

    /// The op's wire name, for logs.
    pub fn name(&self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v["op"].as_str().map(str::to_owned)).unwrap_or_default()
    }
}

/// A team change pushed to its members' clients, in `ToClient::Team`.
///
/// `{"kind":"call_opened","call":{…}}`, `{"kind":"run_changed","run":{…}}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TeamEvent {
    CallOpened { call: Call },
    CallResolved { call: Call },
    RunChanged { run: Run },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn op(value: Value) -> TeamOp {
        let parsed: TeamOp = serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{value}: {e}"));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), value, "round trip");
        parsed
    }

    #[test]
    fn every_op_reads_its_literal_json() {
        let answer = json!({ "by": "", "choice": "red", "text": "", "kind": { "kind": "choice" }, "at": 0 });
        let group = json!({ "handle": "design", "about": "", "members": ["a1"], "fallback": null, "slack_group": "" });
        let calls = json!({ "before_irreversible": { "kind": "owner" }, "in_doubt": { "kind": "group", "handle": "design" }, "on_limit": { "kind": "person", "account": "a1" }, "on_failure": { "kind": "owner" } });
        let bot = json!({ "workflow": "c1", "name": "Fix", "owner": null, "calls": calls, "limits": { "usd": 5.0, "minutes": 30 }, "on": false, "on_by": null, "once": false });
        let cases = [
            json!({ "op": "list" }),
            json!({ "op": "create", "name": "Acme" }),
            json!({ "op": "join", "code": "ABCD-EFGH" }),
            json!({ "op": "rename", "name": "Acme 2" }),
            json!({ "op": "members" }),
            json!({ "op": "invite" }),
            json!({ "op": "revoke_invite", "hint": "ABCD" }),
            json!({ "op": "remove_member", "account": "a1" }),
            json!({ "op": "leave" }),
            json!({ "op": "attach_project", "key": "github.com/acme/site" }),
            json!({ "op": "detach_project", "key": "github.com/acme/site" }),
            json!({ "op": "projects" }),
            json!({ "op": "groups" }),
            json!({ "op": "save_group", "group": group }),
            json!({ "op": "join_group", "handle": "design" }),
            json!({ "op": "leave_group", "handle": "design" }),
            json!({ "op": "bots" }),
            json!({ "op": "save_bot", "bot": bot }),
            json!({ "op": "turn_bot_on", "workflow": "c1", "on": true }),
            json!({ "op": "run_now", "workflow": "c1" }),
            json!({ "op": "runs" }),
            json!({ "op": "calls" }),
            json!({ "op": "answer_call", "id": "k1", "answer": answer }),
            json!({ "op": "take_over", "run": "r1" }),
            json!({ "op": "give_back", "run": "r1" }),
            json!({ "op": "stop_run", "run": "r1" }),
            json!({ "op": "chat", "text": "hi" }),
            json!({ "op": "hub", "call": { "cmd": "get_settings" } }),
            json!({ "op": "ask_people", "situation": "in_doubt", "question": "Which?", "choices": ["red", "blue"] }),
        ];
        for case in cases {
            let name = case["op"].as_str().unwrap().to_owned();
            assert_eq!(op(case).name(), name);
        }
        let TeamOp::Hub { call } = op(json!({ "op": "hub", "call": { "cmd": "run_workflow", "id": "c1" } })) else { panic!("hub") };
        assert!(matches!(TeamOp::hub_call(call), Ok(HubCall::RunWorkflow { ref id }) if id == "c1"));
        assert!(TeamOp::hub_call(json!({ "cmd": "no_such_call" })).is_err());
    }

    #[test]
    fn a_short_answer_and_bot_read_their_defaults() {
        let TeamOp::AnswerCall { answer, .. } = serde_json::from_value(json!({ "op": "answer_call", "id": "k1", "answer": { "kind": { "kind": "pass_to", "handle": "design" } } })).unwrap() else { panic!() };
        assert_eq!(answer, Answer { by: String::new(), choice: None, text: String::new(), kind: AnswerKind::PassTo { handle: "design".into() }, at: 0 });
        let bot: Bot = serde_json::from_value(json!({ "workflow": "c1" })).unwrap();
        assert_eq!(bot.calls, CallRules::default());
        assert_eq!(bot.limits, BotLimits { usd: 5.0, minutes: 30 });
        let rules: CallRules = serde_json::from_value(json!({ "in_doubt": { "kind": "group", "handle": "design" } })).unwrap();
        assert_eq!(rules.who(Situation::InDoubt), &Who::Group { handle: "design".into() });
        assert_eq!(rules.who(Situation::OnFailure), &Who::Owner);
    }

    #[test]
    fn who_shows_a_group_by_its_handle() {
        assert_eq!(Who::Group { handle: "design".into() }.to_string(), "@design");
        assert_eq!(Who::Owner.to_string(), "the owner");
        assert_eq!(Who::Person { account: "a1".into() }.to_string(), "a1");
    }

    #[test]
    fn run_states_read_their_column_and_know_an_end() {
        for state in [RunState::Working, RunState::Waiting, RunState::TakenOver, RunState::Done, RunState::Stopped, RunState::GaveUp, RunState::Failed] {
            assert_eq!(RunState::parse(state.as_str()), Some(state));
        }
        assert!(RunState::GaveUp.ended() && !RunState::TakenOver.ended());
        assert_eq!(Role::parse("owner"), Role::Owner);
        assert_eq!(Role::parse("member"), Role::Member);
    }

    #[test]
    fn a_scaled_policy_divides_times_only() {
        let fast = TeamPolicy::default().scaled(60);
        assert_eq!(fast, TeamPolicy { escalate_after_secs: 30, give_up_after_secs: 120, run_usd: 5.0, run_minutes: 1 });
        let tiny = TeamPolicy::default().scaled(100_000);
        assert_eq!((tiny.escalate_after_secs, tiny.give_up_after_secs, tiny.run_minutes), (1, 1, 1));
        assert_eq!(TeamPolicy::default().scaled(0), TeamPolicy::default());
    }

    #[test]
    fn a_team_event_reads_its_literal_json() {
        let run = json!({ "id": "r1", "workflow": "c1", "state": "taken_over", "controller": "a1", "cost_usd": 0.5, "active_secs": 60, "started_at": 1, "ended_at": null, "outcome": null, "started_by": "a1" });
        let event = json!({ "kind": "run_changed", "run": run });
        let parsed: TeamEvent = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), event);
    }
}
