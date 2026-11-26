//! pinned Teams on the hub. A team is an account of kind `team` with no login; a member reaches
//! it only through `call`, which checks the membership (`team_scope`) on every call and hands the
//! op the team's account. An op never sees the member's own account, only its id (`Scope::member`).
//!
//! Each op is one function in one file: membership.rs, projects.rs, groups.rs, bots.rs, calls.rs,
//! chat.rs, triggers.rs, run_log.rs. Their SQL lives in db.rs, one function per query.

pub mod bots;
pub mod calls;
pub mod chat;
pub mod db;
pub mod groups;
pub mod membership;
pub mod projects;
pub mod queue;
pub mod run_log;
pub mod triggers;

use crate::hub::{Account, Hub};
use pinned_core::surface::{self, Side};
use pinned_core::teams::{Role, TeamEvent, TeamOp};
use pinned_core::wire::{HubCall, ToClient};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// A member's call on one team, after `team_scope` let it through.
pub struct Scope {
    /// The team's account: every workflow, schedule, box and key of the team is stored under it.
    pub team: Arc<Account>,
    /// The member who asked. Only the id: an op never reads the member's own account.
    pub member: Uuid,
    pub role: Role,
}

impl Scope {
    /// Refuses a member who is not the team's owner.
    pub fn owner_only(&self) -> Result<(), String> {
        match self.role {
            Role::Owner => Ok(()),
            Role::Member => Err("only the team's owner does this".into()),
        }
    }
}

/// The team's account and the person's role on it, or an error when the person is not a member.
/// Reads `team_members` on every call, with no cache, so a removal holds at once.
pub async fn team_scope(hub: &Arc<Hub>, person: &Arc<Account>, team: Uuid) -> Result<(Arc<Account>, Role), String> {
    let role = db::role(&hub.db, team, person.id).await?.ok_or("you are not a member of that team")?;
    Ok((hub.account(team).await?, role))
}

/// Parses each client-sent account id and checks that every one is a current member of `team`,
/// answering them in order without repeats. An id that is malformed, unknown or a member of
/// another team only gets the one message, so the answer never says which accounts exist or who is
/// on another team. Every account id a client sends to a team op goes through it at save time.
pub async fn members_only(hub: &Arc<Hub>, team: Uuid, ids: &[&str]) -> Result<Vec<Uuid>, String> {
    let refused = |raw: &str| format!("no member {} on this team", raw.chars().take(64).collect::<String>());
    let mut parsed = Vec::with_capacity(ids.len());
    for raw in ids {
        parsed.push(Uuid::parse_str(raw).map_err(|_| refused(raw))?);
    }
    let members = db::members_among(&hub.db, team, &parsed).await?;
    let mut kept: Vec<Uuid> = Vec::with_capacity(parsed.len());
    for (raw, id) in ids.iter().zip(parsed) {
        if !members.contains(&id) {
            return Err(refused(raw));
        }
        if !kept.contains(&id) {
            kept.push(id);
        }
    }
    Ok(kept)
}

/// A member's team op with the personal call inside `TeamOp::Hub` read into its type, once, by
/// `Op::read`: the unlock check (`owner_only`), the allowlist (`hub_rule`) and the run all see
/// that one typed call, and nothing in Teams reads a field of the JSON it came as.
pub enum Op {
    /// Every op but `TeamOp::Hub`.
    Team(TeamOp),
    Hub(HubCall),
}

impl Op {
    pub fn read(op: TeamOp) -> Result<Op, String> {
        match op {
            TeamOp::Hub { call } => TeamOp::hub_call(call).map(Op::Hub),
            op => Ok(Op::Team(op)),
        }
    }
}

/// The team a `HubCall::Team` names, read as an id.
pub fn team_id(team: Option<String>) -> Result<Option<Uuid>, String> {
    team.map(|id| Uuid::parse_str(&id).map_err(|_| format!("{id} is not a team id"))).transpose()
}

/// Whether a client on a `locked` login unlocks with its password before `op`: a personal call
/// needs what it needs on the person's own account (`surface::owner_only`), and the team asks too
/// before anything that changes who is on it, its projects, groups, keys or bots, starts or steers
/// a run, answers a call or talks in its chat. A stop, giving a run back and the reads do not ask.
/// Exhaustive on purpose: a new op does not compile until it is placed here.
pub fn owner_only(op: &Op, locked: bool) -> bool {
    let team = match op {
        Op::Hub(call) => return surface::owner_only(Side::Hub, &surface::cmd_of(call), locked) || (locked && hub_rule(call).unlock),
        Op::Team(op) => match op {
            TeamOp::Rename { .. }
            | TeamOp::Invite
            | TeamOp::RevokeInvite { .. }
            | TeamOp::RemoveMember { .. }
            | TeamOp::Join { .. }
            | TeamOp::Leave
            | TeamOp::AttachProject { .. }
            | TeamOp::DetachProject { .. }
            | TeamOp::SaveGroup { .. }
            | TeamOp::JoinGroup { .. }
            | TeamOp::LeaveGroup { .. }
            | TeamOp::SaveBot { .. }
            | TeamOp::TurnBotOn { .. }
            | TeamOp::DeleteBot { .. }
            | TeamOp::SetBotCeiling { .. }
            | TeamOp::RunNow { .. }
            | TeamOp::AnswerCall { .. }
            | TeamOp::TakeOver { .. }
            | TeamOp::Chat { .. }
            | TeamOp::Hub { .. } => true,
            // A stop, giving a run back, and what only reads.
            TeamOp::List
            | TeamOp::Create { .. }
            | TeamOp::Members
            | TeamOp::Invites
            | TeamOp::Projects
            | TeamOp::Groups
            | TeamOp::Bots
            | TeamOp::BotWorkflow { .. }
            | TeamOp::Runs
            | TeamOp::Calls
            | TeamOp::GiveBack { .. }
            | TeamOp::StopRun { .. }
            | TeamOp::RunLog { .. }
            | TeamOp::AskPeople { .. }
            | TeamOp::RunModelCost { .. } => false,
        },
    };
    locked && team
}

/// Runs a person's team op, read by `Op::read`. `List`, `Create` and `Join` need no team; every
/// other op names one the person is a member of. The client's unlock check (`owner_only`) ran
/// before this on the same `op`.
pub async fn call(hub: &Arc<Hub>, person: &Arc<Account>, team: Option<Uuid>, op: Op) -> Result<Value, String> {
    let op = match op {
        Op::Hub(call) => {
            let team = team.ok_or("name the team")?;
            let (team, role) = team_scope(hub, person, team).await?;
            return run_hub_call(hub, &Scope { team, member: person.id, role }, call).await;
        }
        Op::Team(TeamOp::Hub { .. }) => return Err("a team's hub call is read by Op::read first".into()),
        Op::Team(op) => op,
    };
    match op {
        TeamOp::List => return json(membership::list(hub, person.id).await?),
        TeamOp::Create { name } => return json(membership::create(hub, person.id, &name).await?),
        TeamOp::Join { code } => return json(membership::join(hub, person.id, &code).await?),
        TeamOp::AskPeople { .. } => return Err("only a session in one of the team's boxes asks people".into()),
        TeamOp::RunModelCost { .. } => return Err("only a team's box reports what its run's model cost".into()),
        _ => {}
    }
    let team = team.ok_or("name the team")?;
    let (team, role) = team_scope(hub, person, team).await?;
    let scope = Scope { team, member: person.id, role };
    let s = &scope;
    match op {
        TeamOp::List | TeamOp::Create { .. } | TeamOp::Join { .. } | TeamOp::AskPeople { .. } | TeamOp::RunModelCost { .. } | TeamOp::Hub { .. } => unreachable!("answered above"),
        TeamOp::Rename { name } => json(membership::rename(hub, s, &name).await?),
        TeamOp::Members => json(membership::members(hub, s).await?),
        TeamOp::Invite => json(membership::invite(hub, s).await?),
        TeamOp::RevokeInvite { hint } => json(membership::revoke_invite(hub, s, &hint).await?),
        TeamOp::Invites => json(membership::invites(hub, s).await?),
        TeamOp::RemoveMember { account } => json(membership::remove_member(hub, s, &account).await?),
        TeamOp::Leave => json(membership::leave(hub, s).await?),
        TeamOp::AttachProject { key } => json(projects::attach_project(hub, s, &key).await?),
        TeamOp::DetachProject { key } => json(projects::detach_project(hub, s, &key).await?),
        TeamOp::Projects => json(projects::projects(hub, s).await?),
        TeamOp::Groups => json(groups::groups(hub, s).await?),
        TeamOp::SaveGroup { group } => json(groups::save_group(hub, s, group).await?),
        TeamOp::JoinGroup { handle } => json(groups::join_group(hub, s, &handle).await?),
        TeamOp::LeaveGroup { handle } => json(groups::leave_group(hub, s, &handle).await?),
        TeamOp::Bots => json(bots::bots(hub, s).await?),
        TeamOp::BotWorkflow { workflow } => json(bots::bot_workflow(hub, s, &workflow).await?),
        TeamOp::SaveBot { bot, connector } => json(bots::save_bot(hub, s, bot, connector).await?),
        TeamOp::TurnBotOn { workflow, on } => json(bots::turn_bot_on(hub, s, &workflow, on).await?),
        TeamOp::DeleteBot { workflow } => json(bots::delete_bot(hub, s, &workflow).await?),
        TeamOp::RunNow { workflow } => bots::run_now(hub, s, &workflow).await,
        TeamOp::Runs => json(calls::runs(hub, s).await?),
        TeamOp::Calls => json(calls::calls(hub, s).await?),
        TeamOp::AnswerCall { id, answer } => json(calls::answer_call(hub, s, &id, answer).await?),
        TeamOp::TakeOver { run } => json(calls::take_over(hub, s, &run).await?),
        TeamOp::GiveBack { run } => json(calls::give_back(hub, s, &run).await?),
        TeamOp::StopRun { run } => json(calls::stop_run(hub, s, &run).await?),
        TeamOp::SetBotCeiling { workflow, usd, minutes } => json(bots::set_bot_ceiling(hub, s, &workflow, usd, minutes).await?),
        TeamOp::RunLog { run } => json(run_log::run_log(hub, s, &run).await?),
        TeamOp::Chat { text } => json(chat::chat(hub, s, &text).await?),
    }
}

/// Runs the personal `call` on the team's account if `hub_rule` lets the member's role, held to a
/// bot's rules on the team's workflows (`bots::hub_call`). A call that sets one of the team's keys
/// answers which keys are set, never what the call answers.
async fn run_hub_call(hub: &Arc<Hub>, s: &Scope, call: HubCall) -> Result<Value, String> {
    hub_call_allowed(&call, s.role)?;
    let key = hub_rule(&call).gate == Gate::OwnerKey;
    // `bots::hub_call` boxes the recursion, as `hub_call` is what called this.
    let answer = bots::hub_call(hub, s, call).await?;
    if key {
        return json(membership::credential(hub, &s.team).await?);
    }
    Ok(answer)
}

/// Runs a machine's `HubCall::Team`: only `AskPeople` and `RunModelCost`, each from a box of a
/// team's own account; any other op is refused.
pub async fn machine_call(hub: &Arc<Hub>, account: &Arc<Account>, machine: &str, op: TeamOp) -> Result<Value, String> {
    match op {
        TeamOp::AskPeople { situation, question, choices, step } => json(calls::ask_people(hub, account, machine, situation, &question, choices, step.as_deref()).await?),
        TeamOp::RunModelCost { workflow, subject, usd } => json(calls::model_cost(hub, account, machine, &workflow, &subject, usd).await?),
        op => Err(format!("a machine may not send the team op {}", op.name())),
    }
}

/// Who runs a personal call on a team's account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Every member.
    Member,
    /// The owner only.
    Owner,
    /// The owner only, and it sets or drops one of the team's keys: it answers which keys are
    /// set (`CredentialState`), never a value.
    OwnerKey,
    /// Nobody: it belongs to a person, a machine or a session, or reads or mints a key.
    Never,
}

/// How a team takes one personal call: who runs it, and whether a locked login unlocks first
/// (beside what `surface::owner_only` asks of the call on a person's own account).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub gate: Gate,
    pub unlock: bool,
}

/// The team's rule for `call`. Members read a box's status; only the owner sets the team's keys
/// and box access and spends more on a box. A team's workflows are kept and run as its bots. Everything
/// else is refused: a team has no machines but its boxes, no files, no password, no agents and
/// no sessions of its own. Exhaustive on purpose, with no catch-all: a new `HubCall` does not
/// compile until it is placed here.
pub fn hub_rule(call: &HubCall) -> Rule {
    let (gate, unlock) = match call {
        HubCall::BoxStatus { .. } => (Gate::Member, false),
        HubCall::SetBoxAccess { .. } | HubCall::ExtendBox { .. } => (Gate::Owner, true),
        HubCall::ConnectUpstash { .. }
        | HubCall::DisconnectUpstash { .. }
        | HubCall::SetBoxClaudeToken { .. }
        | HubCall::RemoveBoxClaudeToken { .. }
        | HubCall::ConnectGithub { .. }
        | HubCall::DisconnectGithub { .. }
        | HubCall::ConnectSlack { .. }
        | HubCall::DisconnectSlack { .. }
        | HubCall::ConnectLinear { .. }
        | HubCall::DisconnectLinear { .. } => (Gate::OwnerKey, true),
        // A team's workflows change only through its bots (`TeamOp::SaveBot`, `DeleteBot`, `RunNow`),
        // it keeps no schedules yet, and its settings show its keys' hints.
        HubCall::GetSettings { .. }
        | HubCall::ListSchedules { .. }
        | HubCall::ScheduleRuns { .. }
        | HubCall::WorkflowRuns { .. }
        | HubCall::RunWorkflow { .. }
        | HubCall::SaveConnector { .. }
        | HubCall::DeleteConnector { .. }
        | HubCall::SaveSchedule { .. }
        | HubCall::PauseSchedule { .. }
        | HubCall::DeleteSchedule { .. }
        // Keys the team does not keep, and reads made with a key.
        | HubCall::LinearSuggestions { .. }
        | HubCall::SlackSuggestions { .. }
        | HubCall::GithubRepos { .. }
        | HubCall::ConnectVercel { .. }
        | HubCall::DisconnectVercel { .. }
        | HubCall::ConnectPosthog { .. }
        | HubCall::DisconnectPosthog { .. }
        | HubCall::VercelModels { .. }
        | HubCall::Classify { .. }
        | HubCall::ConnectConvex { .. }
        | HubCall::SetConvexRepos { .. }
        | HubCall::DisconnectConvex { .. }
        | HubCall::ConvexTables { .. }
        | HubCall::Convex { .. }
        | HubCall::SetRepoInsight { .. }
        | HubCall::GenerateWorkflow { .. }
        // A token minted on a machine, MCP servers and their secrets.
        | HubCall::MintBoxClaudeToken { .. }
        | HubCall::CancelBoxClaudeToken { .. }
        | HubCall::ListMcpServers { .. }
        | HubCall::SaveMcpServer { .. }
        | HubCall::EnableMcpServer { .. }
        | HubCall::RemoveMcpServer { .. }
        | HubCall::ImportMcpServer { .. }
        | HubCall::LoadMcpServers { .. }
        // Machines, pairing, the hub's invites, a box opened by hand and its answers.
        | HubCall::Machines { .. }
        | HubCall::MachineStates { .. }
        | HubCall::Place { .. }
        | HubCall::RenameMachine { .. }
        | HubCall::RemoveMachine { .. }
        | HubCall::PairCode { .. }
        | HubCall::CreateInvite { .. }
        | HubCall::Invites { .. }
        | HubCall::RevokeInvite { .. }
        | HubCall::OpenBox { .. }
        | HubCall::AnswerBox { .. }
        | HubCall::GetBoxSize { .. }
        | HubCall::SetBoxSize { .. }
        | HubCall::BoxUsage { .. }
        | HubCall::StuckBoxes { .. }
        // Projects and their files.
        | HubCall::Clone { .. }
        | HubCall::SharedProjects { .. }
        | HubCall::ShareProject { .. }
        | HubCall::SyncList { .. }
        | HubCall::SyncChanges { .. }
        | HubCall::SyncPush { .. }
        | HubCall::SyncPull { .. }
        | HubCall::SyncMarkExecutable { .. }
        // A signed-out app's one-time moves.
        | HubCall::Import { .. }
        | HubCall::ImportTastes { .. }
        // A workflow session's callbacks, which only its machine sends.
        | HubCall::Stage { .. }
        | HubCall::SlackReply { .. }
        | HubCall::LinearCreate { .. }
        | HubCall::LinearTake { .. }
        | HubCall::RunSet { .. }
        // External agents and the acts, confirmations and audit of agents.
        | HubCall::ExternalAgents { .. }
        | HubCall::AgentRequest { .. }
        | HubCall::ApproveAgent { .. }
        | HubCall::DenyAgent { .. }
        | HubCall::RevokeAgent { .. }
        | HubCall::Act { .. }
        | HubCall::Confirmations { .. }
        | HubCall::DecideConfirmation { .. }
        | HubCall::AgentAudit { .. }
        // The global orchestrator, the Capitães, tastes, delegation and its records.
        | HubCall::Relay { .. }
        | HubCall::Overview { .. }
        | HubCall::ReadCaptain { .. }
        | HubCall::RepoLook { .. }
        | HubCall::ReadSession { .. }
        | HubCall::GetTastes { .. }
        | HubCall::SetTaste { .. }
        | HubCall::ResetTasteField { .. }
        | HubCall::SetReviewer { .. }
        | HubCall::SetWorkerProfiles { .. }
        | HubCall::ProposeWorkerProfile { .. }
        | HubCall::Delegate { .. }
        | HubCall::RemoteTask { .. }
        | HubCall::RemoteEnded { .. }
        | HubCall::RemoteAsked { .. }
        | HubCall::RecordTasks { .. }
        | HubCall::DelegationStats { .. }
        | HubCall::UnclaimedEnds { .. }
        // A person's sessions, ideas and switches.
        | HubCall::SessionMarks { .. }
        | HubCall::MarkSessions { .. }
        | HubCall::ArchiveSession { .. }
        | HubCall::AutoArchive { .. }
        | HubCall::UnarchiveSession { .. }
        | HubCall::DeleteSession { .. }
        | HubCall::ListIdeas { .. }
        | HubCall::SaveIdea { .. }
        | HubCall::DeleteIdea { .. }
        | HubCall::SetReleaseProtection { .. }
        // A person's reports to pinned's issues.
        | HubCall::Reports { .. }
        | HubCall::ReportIssue { .. }
        // The password, and a team inside a team.
        | HubCall::Unlock { .. }
        | HubCall::Team { .. }
        | HubCall::Mirror { .. }
        | HubCall::Handback { .. }
        | HubCall::SetOfflineRunner { .. }
        | HubCall::ReleaseWorkflows
        | HubCall::RecordGlobalOrchestratorSave { .. }
        | HubCall::GlobalOrchestratorSave => (Gate::Never, false),
    };
    Rule { gate, unlock }
}

/// Whether a member with `role` may run the personal `call` on the team's account (`hub_rule`).
pub fn hub_call_allowed(call: &HubCall, role: Role) -> Result<(), String> {
    match (hub_rule(call).gate, role) {
        (Gate::Member, _) | (Gate::Owner | Gate::OwnerKey, Role::Owner) => Ok(()),
        (Gate::Owner | Gate::OwnerKey, Role::Member) => Err(format!("only the team's owner runs {} on the team", surface::cmd_of(call))),
        (Gate::Never, _) => Err(format!("{} does not run on a team", surface::cmd_of(call))),
    }
}

/// Sends `event` to every app connection of every member of `team` that sent a team call on that
/// connection (`Account::broadcast_team`). Another connection, a machine, and a member with no app
/// connected get nothing.
pub async fn push(hub: &Hub, team: Uuid, event: TeamEvent) -> Result<(), String> {
    let frame = ToClient::Team { team: team.to_string(), event };
    for member in db::member_ids(&hub.db, team).await? {
        if let Some(account) = hub.loaded(member) {
            account.broadcast_team(&frame);
        }
    }
    Ok(())
}

/// Runs `calls::tick` for as long as the hub runs, often enough for the policy's shortest time.
pub fn spawn_tick(hub: &Arc<Hub>) {
    let hub = Arc::downgrade(hub);
    tokio::spawn(async move {
        loop {
            let Some(strong) = hub.upgrade() else { break };
            let every = Duration::from_secs((strong.team_policy.escalate_after_secs / 4).clamp(1, 30));
            let policy = strong.team_policy.clone();
            if let Err(e) = calls::tick(&strong, &policy).await {
                eprintln!("pinned-server: the teams' calls tick failed: {e}");
            }
            drop(strong);
            tokio::time::sleep(every).await;
        }
    });
}

pub(crate) fn json<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hub_call(value: Value) -> HubCall {
        serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{value}: {e}"))
    }

    #[test]
    fn a_member_reads_a_boxs_status_only_the_owner_sets_keys_and_nobody_keeps_workflows_by_hub() {
        let box_status = json!({ "cmd": "box_status", "machine": "m1" });
        assert_eq!(hub_call_allowed(&hub_call(box_status.clone()), Role::Member), Ok(()));
        assert_eq!(hub_call_allowed(&hub_call(box_status), Role::Owner), Ok(()));
        // A team's workflows change only through its bots, it keeps no schedules, and its
        // settings show its keys' hints: refused for every role, the owner too.
        let workflows = [
            json!({ "cmd": "get_settings" }),
            json!({ "cmd": "save_connector", "connector": {
                "name": "Bugs", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
                "folder": "/w/site", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
                "instructions": "", "machine": "m1",
            } }),
            json!({ "cmd": "save_schedule", "task": {
                "id": "", "title": "Digest", "schedule": { "kind": "cron", "expr": "0 9 * * 1-5" }, "tz": "America/Sao_Paulo",
                "target": { "kind": "orchestrator" }, "action": { "kind": "prompt", "text": "Sum up" },
                "enabled": true, "created_by": { "kind": "user" }, "created_at": 0, "updated_at": 0
            } }),
            json!({ "cmd": "delete_connector", "id": "c1" }),
            json!({ "cmd": "list_schedules" }),
            json!({ "cmd": "pause_schedule", "id": "t1", "paused": true }),
            json!({ "cmd": "delete_schedule", "id": "t1" }),
            json!({ "cmd": "schedule_runs", "id": "t1" }),
            json!({ "cmd": "run_workflow", "id": "c1" }),
            json!({ "cmd": "workflow_runs" }),
        ];
        for call in workflows {
            let cmd = call["cmd"].as_str().unwrap().to_owned();
            for role in [Role::Owner, Role::Member] {
                assert_eq!(hub_call_allowed(&hub_call(call.clone()), role), Err(format!("{cmd} does not run on a team")), "{call}");
            }
        }
        let owners = [
            json!({ "cmd": "connect_upstash", "key": "k", "claude_token": "t" }),
            json!({ "cmd": "disconnect_upstash" }),
            json!({ "cmd": "set_box_claude_token", "name": "n", "token": "t" }),
            json!({ "cmd": "remove_box_claude_token", "name": "n" }),
            json!({ "cmd": "connect_github", "token": "t" }),
            json!({ "cmd": "disconnect_github" }),
            json!({ "cmd": "connect_slack", "token": "t" }),
            json!({ "cmd": "disconnect_slack" }),
            json!({ "cmd": "connect_linear", "key": "k" }),
            json!({ "cmd": "disconnect_linear" }),
            json!({ "cmd": "set_box_access", "access": serde_json::to_value(pinned_core::connectors::BoxAccess::default()).unwrap() }),
            json!({ "cmd": "extend_box", "machine": "m1", "extra_usd": 1.0 }),
        ];
        for call in owners {
            assert_eq!(hub_call_allowed(&hub_call(call.clone()), Role::Owner), Ok(()), "{call}");
            let refused = hub_call_allowed(&hub_call(call.clone()), Role::Member).unwrap_err();
            assert!(refused.contains("only the team's owner"), "{call}: {refused}");
        }
        let request = serde_json::to_value(pinned_core::delegate::parse_request(&json!({ "task": "Fix it." })).unwrap()).unwrap();
        let never = [
            json!({ "cmd": "machines" }),
            json!({ "cmd": "pair_code" }),
            json!({ "cmd": "unlock", "password": "p" }),
            json!({ "cmd": "sync_push", "project": "p", "files": [] }),
            json!({ "cmd": "import", "store": { "connectors": [] }, "linear_key": null, "slack_key": null }),
            json!({ "cmd": "delegate", "tab": "tab-1", "task": "t1", "request": request, "access": "auto" }),
            json!({ "cmd": "team", "op": { "op": "list" } }),
            json!({ "cmd": "team", "team": "t", "op": { "op": "hub", "call": { "cmd": "get_settings" } } }),
            // The Reviewer and worker profiles are a person's own way of working.
            json!({ "cmd": "set_reviewer", "patch": { "rounds": 3 } }),
            json!({ "cmd": "set_worker_profiles", "profiles": [], "drop_proposed": ["scout"] }),
            json!({ "cmd": "propose_worker_profile", "tab": "tab-1", "profile": { "name": "scout" } }),
            json!({ "cmd": "mirror", "have": "", "login": "user-token" }),
            json!({ "cmd": "handback", "changes": {}, "login": "user-token" }),
            json!({ "cmd": "set_offline_runner" }),
            json!({ "cmd": "release_workflows" }),
            json!({ "cmd": "record_global_orchestrator_save", "settings": {}, "took": [] }),
            json!({ "cmd": "global_orchestrator_save" }),
        ];
        for call in never {
            for role in [Role::Owner, Role::Member] {
                let refused = hub_call_allowed(&hub_call(call.clone()), role).unwrap_err();
                assert!(refused.contains("does not run on a team"), "{call}: {refused}");
            }
        }
    }

    #[test]
    fn classify_does_not_run_on_a_team() {
        let classify = hub_call(json!({ "cmd": "classify", "text": "The export button 500s", "question": "Is this a bug report?" }));
        for role in [Role::Owner, Role::Member] {
            assert_eq!(hub_call_allowed(&classify, role), Err("classify does not run on a team".to_owned()));
        }
    }

    fn read(call: Value) -> Result<Op, String> {
        Op::read(TeamOp::Hub { call })
    }

    #[test]
    fn a_teams_hub_call_is_read_once_from_an_object_and_its_odd_forms_get_no_way_around_the_unlock() {
        let plain = read(json!({ "cmd": "disconnect_upstash" })).unwrap();
        assert!(owner_only(&plain, true), "a locked login unlocks first");
        assert!(!owner_only(&plain, false));
        let extra = read(json!({ "cmd": "disconnect_upstash", "extra": 1, "cmd2": "get_settings" })).unwrap();
        assert!(matches!(extra, Op::Hub(HubCall::DisconnectUpstash)) && owner_only(&extra, true), "extra keys read as the plain form");
        for odd in [json!(["disconnect_upstash"]), json!(["run_workflow", "c1"]), json!({ "cmd": "Disconnect_Upstash" }), json!("disconnect_upstash")] {
            assert!(read(odd.clone()).is_err(), "{odd} is refused before any check or run");
        }
    }

    #[test]
    fn a_locked_login_unlocks_before_a_team_op_that_changes_people_keys_bots_or_workflows() {
        let group = json!({ "handle": "design" });
        let changes = [
            json!({ "op": "rename", "name": "Acme 2" }),
            json!({ "op": "invite" }),
            json!({ "op": "revoke_invite", "hint": "ABCD" }),
            json!({ "op": "remove_member", "account": "a1" }),
            json!({ "op": "join", "code": "ABCD-EFGH" }),
            json!({ "op": "leave" }),
            json!({ "op": "save_group", "group": group }),
            json!({ "op": "save_bot", "bot": { "workflow": "c1" } }),
            json!({ "op": "turn_bot_on", "workflow": "c1", "on": true }),
            json!({ "op": "hub", "call": { "cmd": "connect_upstash", "key": "k", "claude_token": "t" } }),
            json!({ "op": "hub", "call": { "cmd": "set_box_claude_token", "name": "n", "token": "t" } }),
            json!({ "op": "hub", "call": { "cmd": "disconnect_slack" } }),
            json!({ "op": "hub", "call": { "cmd": "extend_box", "machine": "m1", "extra_usd": 1.0 } }),
            json!({ "op": "attach_project", "key": "github.com/acme/site" }),
            json!({ "op": "detach_project", "key": "github.com/acme/site" }),
            json!({ "op": "join_group", "handle": "design" }),
            json!({ "op": "leave_group", "handle": "design" }),
            json!({ "op": "delete_bot", "workflow": "c1" }),
            json!({ "op": "run_now", "workflow": "c1" }),
            json!({ "op": "answer_call", "id": "k1", "answer": { "kind": { "kind": "pass_to", "handle": "design" } } }),
            json!({ "op": "take_over", "run": "r1" }),
            json!({ "op": "chat", "text": "hi" }),
        ];
        for op in changes {
            let op = Op::read(serde_json::from_value(op.clone()).unwrap()).unwrap_or_else(|e| panic!("{e}"));
            assert!(owner_only(&op, true), "locked");
            assert!(!owner_only(&op, false), "an older app's login keeps it");
        }
        let reads = [
            json!({ "op": "list" }),
            json!({ "op": "members" }),
            json!({ "op": "invites" }),
            json!({ "op": "projects" }),
            json!({ "op": "groups" }),
            json!({ "op": "bots" }),
            json!({ "op": "runs" }),
            json!({ "op": "calls" }),
            json!({ "op": "stop_run", "run": "r1" }),
            json!({ "op": "give_back", "run": "r1" }),
            json!({ "op": "run_log", "run": "r1" }),
            json!({ "op": "hub", "call": { "cmd": "box_status", "machine": "m1" } }),
        ];
        for op in reads {
            let op = Op::read(serde_json::from_value(op).unwrap()).unwrap();
            assert!(!owner_only(&op, true));
        }
        // A decision needs the password on any login, as on a person's own account.
        let decide = read(json!({ "cmd": "decide_confirmation", "id": "c1", "approve": true })).unwrap();
        assert!(owner_only(&decide, false));
    }

    #[test]
    fn an_owners_key_call_answers_only_which_keys_are_set() {
        for call in [json!({ "cmd": "connect_upstash", "key": "k", "claude_token": "t" }), json!({ "cmd": "remove_box_claude_token", "name": "n" }), json!({ "cmd": "connect_github", "token": "t" }), json!({ "cmd": "connect_linear", "key": "k" }), json!({ "cmd": "disconnect_linear" })] {
            assert_eq!(hub_rule(&hub_call(call.clone())), Rule { gate: Gate::OwnerKey, unlock: true }, "{call}");
        }
        for call in [json!({ "cmd": "mint_box_claude_token", "machine": "m1", "account": "a" }), json!({ "cmd": "load_mcp_servers" }), json!({ "cmd": "linear_suggestions" }), json!({ "cmd": "connect_posthog", "key": "k", "project": "1", "host": "us" })] {
            assert_eq!(hub_rule(&hub_call(call.clone())).gate, Gate::Never, "{call}");
        }
    }
}
