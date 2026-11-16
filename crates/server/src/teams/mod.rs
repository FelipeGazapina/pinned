//! pinned Teams on the hub. A team is an account of kind `team` with no login; a member reaches
//! it only through `call`, which checks the membership (`team_scope`) on every call and hands the
//! op the team's account. An op never sees the member's own account, only its id (`Scope::member`).
//!
//! Each op is one function in one file: membership.rs, projects.rs, groups.rs, bots.rs, calls.rs,
//! chat.rs. Their SQL lives in db.rs, one function per query.

pub mod bots;
pub mod calls;
pub mod chat;
pub mod db;
pub mod groups;
pub mod membership;
pub mod projects;

use crate::hub::{Account, Hub};
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

/// Runs a person's `HubCall::Team`. `List`, `Create` and `Join` need no team; every other op
/// names one the person is a member of. The client's own unlock check ran before this, on the
/// `Team` call and on the personal call inside `TeamOp::Hub`.
pub async fn call(hub: &Arc<Hub>, person: &Arc<Account>, team: Option<Uuid>, op: TeamOp) -> Result<Value, String> {
    match op {
        TeamOp::List => return json(membership::list(hub, person.id).await?),
        TeamOp::Create { name } => return json(membership::create(hub, person.id, &name).await?),
        TeamOp::Join { code } => return json(membership::join(hub, person.id, &code).await?),
        TeamOp::AskPeople { .. } => return Err("only a session in one of the team's boxes asks people".into()),
        _ => {}
    }
    let team = team.ok_or("name the team")?;
    let (team, role) = team_scope(hub, person, team).await?;
    let scope = Scope { team, member: person.id, role };
    let s = &scope;
    match op {
        TeamOp::List | TeamOp::Create { .. } | TeamOp::Join { .. } | TeamOp::AskPeople { .. } => unreachable!("answered above"),
        TeamOp::Rename { name } => json(membership::rename(hub, s, &name).await?),
        TeamOp::Members => json(membership::members(hub, s).await?),
        TeamOp::Invite => json(membership::invite(hub, s).await?),
        TeamOp::RevokeInvite { hint } => json(membership::revoke_invite(hub, s, &hint).await?),
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
        TeamOp::SaveBot { bot } => json(bots::save_bot(hub, s, bot).await?),
        TeamOp::TurnBotOn { workflow, on } => json(bots::turn_bot_on(hub, s, &workflow, on).await?),
        TeamOp::RunNow { workflow } => json(bots::run_now(hub, s, &workflow).await?),
        TeamOp::Runs => json(calls::runs(hub, s).await?),
        TeamOp::Calls => json(calls::calls(hub, s).await?),
        TeamOp::AnswerCall { id, answer } => json(calls::answer_call(hub, s, &id, answer).await?),
        TeamOp::TakeOver { run } => json(calls::take_over(hub, s, &run).await?),
        TeamOp::GiveBack { run } => json(calls::give_back(hub, s, &run).await?),
        TeamOp::StopRun { run } => json(calls::stop_run(hub, s, &run).await?),
        TeamOp::Chat { text } => json(chat::chat(hub, s, &text).await?),
        TeamOp::Hub { call } => {
            let call = TeamOp::hub_call(call)?;
            hub_call_allowed(&call, s.role)?;
            // A bot's rules hold on the team's workflows: `bots::hub_call` boxes the recursion, as `hub_call` called this.
            bots::hub_call(hub, s, call).await
        }
    }
}

/// Runs a machine's `HubCall::Team`: only `AskPeople`, from a machine of a team's own account.
pub async fn machine_call(hub: &Arc<Hub>, account: &Arc<Account>, machine: &str, op: TeamOp) -> Result<Value, String> {
    match op {
        TeamOp::AskPeople { situation, question, choices, step } => json(calls::ask_people(hub, account, machine, situation, &question, choices, step.as_deref()).await?),
        op => Err(format!("a machine may not send the team op {}", op.name())),
    }
}

/// Whether a member with `role` may run the personal `call` on the team's account. Members
/// read the settings, keep workflows and schedules and run them; only the owner sets the team's
/// keys and box access and spends more on a box. Everything else, such as machines, pairing,
/// imports, unlocking, delegation and sync, is refused: a team has no machines of its own but
/// its boxes, no files and no password.
pub fn hub_call_allowed(call: &HubCall, role: Role) -> Result<(), String> {
    let member = matches!(
        call,
        HubCall::GetSettings
            | HubCall::SaveConnector { .. }
            | HubCall::DeleteConnector { .. }
            | HubCall::ListSchedules
            | HubCall::SaveSchedule { .. }
            | HubCall::PauseSchedule { .. }
            | HubCall::DeleteSchedule { .. }
            | HubCall::ScheduleRuns { .. }
            | HubCall::RunWorkflow { .. }
            | HubCall::BoxStatus { .. }
    );
    let owner = matches!(
        call,
        HubCall::ConnectUpstash { .. }
            | HubCall::DisconnectUpstash
            | HubCall::SetBoxClaudeToken { .. }
            | HubCall::RemoveBoxClaudeToken { .. }
            | HubCall::ConnectGithub { .. }
            | HubCall::DisconnectGithub
            | HubCall::ConnectSlack { .. }
            | HubCall::DisconnectSlack { .. }
            | HubCall::SetBoxAccess { .. }
            | HubCall::ExtendBox { .. }
    );
    let cmd = pinned_core::surface::cmd_of(call);
    match (member, owner, role) {
        (true, _, _) | (_, true, Role::Owner) => Ok(()),
        (_, true, Role::Member) => Err(format!("only the team's owner runs {cmd} on the team")),
        _ => Err(format!("{cmd} does not run on a team")),
    }
}

/// Sends `event` to every connected app of every member of `team`. An app that never listed its
/// teams ignores it; a member with no app connected gets nothing.
pub async fn push(hub: &Hub, team: Uuid, event: TeamEvent) -> Result<(), String> {
    let frame = ToClient::Team { team: team.to_string(), event };
    for member in db::member_ids(&hub.db, team).await? {
        if let Some(account) = hub.loaded(member) {
            account.broadcast(&frame);
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
    fn a_member_runs_workflows_and_only_the_owner_sets_keys() {
        let members = [
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
            json!({ "cmd": "box_status", "machine": "m1" }),
        ];
        for call in members {
            assert_eq!(hub_call_allowed(&hub_call(call.clone()), Role::Member), Ok(()), "{call}");
            assert_eq!(hub_call_allowed(&hub_call(call.clone()), Role::Owner), Ok(()), "{call}");
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
        ];
        for call in never {
            for role in [Role::Owner, Role::Member] {
                let refused = hub_call_allowed(&hub_call(call.clone()), role).unwrap_err();
                assert!(refused.contains("does not run on a team"), "{call}: {refused}");
            }
        }
    }
}
