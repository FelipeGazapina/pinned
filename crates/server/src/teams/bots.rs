//! Bots: the team account's workflows, with who they ask and what a run may spend.
//!
//! A bot is a workflow of the team's store plus a `team_bots` row. It is a draft (off) until the
//! owner, or someone its call rules name, turns it on, and any change to it makes it a draft again.
//! Its runs always open in a box on the team's own keys and Claude token, never on a member's machine.

use super::{db, Scope};
use crate::hub::{Account, Hub};
use pinned_core::connectors::{BoxStep, Connector, Placement, Trigger};
use pinned_core::graph::{NodeKind, Watch};
use pinned_core::teams::{Bot, BotLimits, CallRules, Run, RunState, TeamEvent, TeamProject, Who};
use pinned_core::wire::HubCall;
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// What a refused run of a bot that is not on says, whichever way it was asked for.
pub const IS_A_DRAFT: &str = "this bot is a draft: it has to be turned on before it can run";
const NEEDS_KEYS: &str = "the team needs its Upstash key and Claude token before a bot can run";

fn unix_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

async fn workflows(hub: &Arc<Hub>, team: &Account) -> Result<Vec<Connector>, String> {
    Ok(hub.connectors(team).await?.get_settings().connectors)
}

async fn workflow(hub: &Arc<Hub>, team: &Account, id: &str) -> Result<Connector, String> {
    workflows(hub, team).await?.into_iter().find(|c| c.id == id).ok_or_else(|| format!("this team has no workflow {id}"))
}

/// Refuses a workflow whose trigger is not one a bot can have yet ("scheduled and GitHub-event bots
/// are not available yet"). Linear, Slack and manual triggers are fine, and today they are the only kinds.
/// The matches name every kind on purpose: a schedule or GitHub-event trigger added later does not
/// compile until it is refused here.
fn available(workflow: &Connector) -> Result<(), String> {
    match &workflow.trigger {
        Trigger::LinearLabel { .. } | Trigger::SlackMention { .. } | Trigger::SlackMessage { .. } | Trigger::Called | Trigger::Manual => {}
    }
    let triggers = workflow.graph.iter().flat_map(|g| g.nodes.iter()).filter_map(|n| match &n.kind {
        NodeKind::Trigger { watch } => Some(watch),
        _ => None,
    });
    for watch in triggers {
        match watch {
            Watch::LinearLabel { .. } | Watch::SlackMention { .. } | Watch::SlackMessage { .. } | Watch::Called | Watch::Manual => {}
        }
    }
    Ok(())
}

/// Saves the team workflow `id` with `enabled` as given, if it is not so already.
async fn set_enabled(hub: &Arc<Hub>, team: &Account, id: &str, enabled: bool) -> Result<(), String> {
    let mut stored = workflow(hub, team, id).await?;
    if stored.enabled != enabled {
        stored.enabled = enabled;
        hub.connectors(team).await?.save_connector(stored)?;
    }
    Ok(())
}

/// Puts a bot back to a draft: off in `team_bots` and its workflow disabled.
async fn draft(hub: &Arc<Hub>, scope: &Scope, id: &str) -> Result<(), String> {
    db::turn_bot_on(&hub.db, scope.team.id, id, false, scope.member).await?;
    set_enabled(hub, &scope.team, id, false).await
}

fn default_limits(hub: &Hub) -> BotLimits {
    BotLimits { usd: hub.team_policy.run_usd, minutes: hub.team_policy.run_minutes }
}

/// Every workflow of the team's account as a bot: its `team_bots` row, or `CallRules::default()`
/// and the hub's `TeamPolicy` limits when it has none, with `name` from the workflow.
pub async fn bots(hub: &Arc<Hub>, scope: &Scope) -> Result<Vec<Bot>, String> {
    let rows = db::bots(&hub.db, scope.team.id).await?;
    let mut bots = Vec::new();
    for workflow in workflows(hub, &scope.team).await? {
        let bot = match rows.iter().find(|b| b.workflow == workflow.id) {
            Some(row) => Bot { name: workflow.name.clone(), ..row.clone() },
            None => Bot { workflow: workflow.id.clone(), name: workflow.name.clone(), owner: None, calls: CallRules::default(), limits: default_limits(hub), on: false, on_by: None, once: false, off_note: None },
        };
        bots.push(bot);
    }
    Ok(bots)
}

/// Project `project` was taken out of the team: every bot that is on and works in it (its
/// workflow's `repo` is the project's, by `boxes::project_key`) goes back to a draft, with a note
/// that says why. Answers the workflows it turned off.
pub async fn project_detached(hub: &Hub, team: &Account, project: &TeamProject) -> Result<Vec<String>, String> {
    let note = format!("Turned off: project {} was taken out of the team.", project.name);
    let mut off = Vec::new();
    let connectors = hub.connectors(team).await?;
    for workflow in connectors.get_settings().connectors {
        if crate::boxes::project_key(&workflow.repo).as_deref() != Some(project.key.as_str()) {
            continue;
        }
        if db::turn_bot_off_noted(&hub.db, team.id, &workflow.id, &note).await? {
            let mut stored = workflow;
            stored.enabled = false;
            off.push(stored.id.clone());
            connectors.save_connector(stored)?;
        }
    }
    Ok(off)
}

/// Every `Who` must name a member or a group of the team.
async fn check_who(hub: &Arc<Hub>, team: Uuid, calls: &CallRules) -> Result<(), String> {
    let groups = db::group_handles(&hub.db, team).await?;
    for who in [&calls.before_irreversible, &calls.in_doubt, &calls.on_limit, &calls.on_failure] {
        match who {
            Who::Owner => {}
            Who::Group { handle } => {
                if !groups.contains(handle) {
                    return Err(format!("this team has no group @{handle}"));
                }
            }
            Who::Person { account } => {
                let member = Uuid::parse_str(account).ok();
                let is_member = match member {
                    Some(member) => db::role(&hub.db, team, member).await?.is_some(),
                    None => false,
                };
                if !is_member {
                    return Err(format!("{account} is not a member of this team"));
                }
            }
        }
    }
    Ok(())
}

/// Any member. Saves who the bot asks and its limits for workflow `bot.workflow` of the team's
/// account. The bot goes back to a draft: `on` and `on_by` of `bot` are never read, and turning it
/// on again is `turn_bot_on`. A limit above the hub's team policy is the owner's to give.
pub async fn save_bot(hub: &Arc<Hub>, scope: &Scope, bot: Bot) -> Result<(), String> {
    let team = scope.team.id;
    let workflow = workflow(hub, &scope.team, &bot.workflow).await?;
    available(&workflow)?;
    check_who(hub, team, &bot.calls).await?;
    let BotLimits { usd, minutes } = bot.limits;
    if !usd.is_finite() || usd <= 0.0 || minutes == 0 {
        return Err("a run needs a limit above 0 dollars and 0 minutes".into());
    }
    let existing = db::bot(&hub.db, team, &bot.workflow).await?;
    let kept = existing.as_ref().map(|b| &b.limits);
    let policy = default_limits(hub);
    // A member may keep a limit the owner gave, not raise it.
    let above = |wants: f64, ceiling: f64, kept: Option<f64>| wants > ceiling && kept.is_none_or(|k| wants > k);
    if above(usd, policy.usd, kept.map(|l| l.usd)) || above(f64::from(minutes), f64::from(policy.minutes), kept.map(|l| f64::from(l.minutes))) {
        if scope.owner_only().is_err() {
            return Err(format!("a bot may spend at most ${} and run {} minutes unless the team's owner sets more", policy.usd, policy.minutes));
        }
    }
    let owner = existing.and_then(|b| b.owner).or_else(|| Some(scope.member.to_string()));
    let saved = Bot { name: String::new(), owner, on: false, on_by: None, off_note: None, ..bot };
    db::save_bot(&hub.db, team, &saved).await?;
    draft(hub, scope, &saved.workflow).await
}

/// Turns the bot on or off, keeping who did it (`on_by`), and enables or disables the team
/// account's workflow to match. Off is any member's. On is the owner's, a member of a group the
/// bot's call rules name, or the person one of them names, and only while the team keeps the
/// Upstash key and Claude token its boxes sign in with.
pub async fn turn_bot_on(hub: &Arc<Hub>, scope: &Scope, workflow_id: &str, on: bool) -> Result<(), String> {
    let team = scope.team.id;
    let workflow = workflow(hub, &scope.team, workflow_id).await?;
    let row = db::bot(&hub.db, team, workflow_id).await?;
    if !on {
        draft(hub, scope, workflow_id).await?;
        return Ok(());
    }
    available(&workflow)?;
    let calls = row.as_ref().map(|b| b.calls.clone()).unwrap_or_default();
    if !asked_by(hub, scope, &calls).await? {
        return Err("only the team's owner, or someone this bot asks, turns it on".into());
    }
    let credential = super::membership::credential(hub, &scope.team).await?;
    if !(credential.upstash && credential.claude) {
        return Err(NEEDS_KEYS.into());
    }
    if row.is_none() {
        let fresh = Bot { workflow: workflow_id.to_owned(), name: String::new(), owner: Some(scope.member.to_string()), calls, limits: default_limits(hub), on: false, on_by: None, once: false, off_note: None };
        db::save_bot(&hub.db, team, &fresh).await?;
    }
    set_enabled(hub, &scope.team, workflow_id, true).await?;
    if let Err(e) = db::turn_bot_on(&hub.db, team, workflow_id, true, scope.member).await {
        let _ = set_enabled(hub, &scope.team, workflow_id, false).await;
        return Err(e);
    }
    Ok(())
}

/// Whether the caller is the team's owner, a member of a group `calls` names, or a person it names.
async fn asked_by(hub: &Arc<Hub>, scope: &Scope, calls: &CallRules) -> Result<bool, String> {
    if scope.owner_only().is_ok() {
        return Ok(true);
    }
    for who in [&calls.before_irreversible, &calls.in_doubt, &calls.on_limit, &calls.on_failure] {
        match who {
            Who::Owner => {}
            Who::Person { account } => {
                if *account == scope.member.to_string() {
                    return Ok(true);
                }
            }
            Who::Group { handle } => {
                if db::in_group(&hub.db, scope.team.id, handle, scope.member).await? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// What `Hub::open_team_run` asks for. Only `start_run` makes one, so a team's run opens through
/// no other path.
pub struct TeamRunPass(());

/// How a team's run was asked for.
pub enum Start {
    /// A member's Run now, by the member's id.
    Member(Uuid),
    /// A trigger of the bot's workflow fired (`triggers::fired`): `kind` names it (`linear_label`,
    /// `slack_mention`, `slack_message`, `called`), and `work` is the session its poller found.
    Trigger { kind: &'static str, work: pinned_core::connectors::Run },
}

/// Any member. Runs the bot now, as its workflow's Run now would on the team's account
/// (`start_run`, started by the member).
pub async fn run_now(hub: &Arc<Hub>, scope: &Scope, workflow_id: &str) -> Result<Run, String> {
    start_run(hub, &scope.team, workflow_id, Start::Member(scope.member)).await.map(|(run, _)| run)
}

/// The one way a team's run starts, from a member's Run now or a trigger that fired. The bot must
/// be on and the team must keep its Upstash key and Claude token. Whatever the stored workflow
/// says, the run opens in a fresh box of the team's own. Records the run (`db::insert_run`, started
/// by the member or `trigger:<kind>`, with the subject its box opens for, so the box sets its
/// machine on the run before the session starts: `db::claim_run_box`) and pushes
/// `TeamEvent::RunChanged`. Answers the run and where it opened. The bot's `BotLimits` are held by
/// `calls::tick`.
pub async fn start_run(hub: &Arc<Hub>, team: &Arc<Account>, workflow_id: &str, start: Start) -> Result<(Run, Placement), String> {
    let workflow = workflow(hub, team, workflow_id).await?;
    let on = db::bot(&hub.db, team.id, workflow_id).await?.is_some_and(|b| b.on);
    if !on || !workflow.enabled {
        return Err(IS_A_DRAFT.into());
    }
    available(&workflow)?;
    let credential = super::membership::credential(hub, team).await?;
    if !(credential.upstash && credential.claude) {
        return Err(NEEDS_KEYS.into());
    }
    let (mut work, started_by) = match start {
        Start::Member(member) => (hub.connectors(team).await?.run_now(workflow_id, "a member of the team, from its bots")?, member.to_string()),
        Start::Trigger { kind, work } => (work, format!("trigger:{kind}")),
    };
    // Whatever the stored workflow says, a bot never runs on a member's machine.
    if !matches!(work.runs_on, pinned_core::connectors::RunsOn::FreshBox(_)) {
        work.runs_on = pinned_core::connectors::RunsOn::FreshBox(BoxStep::default());
    }
    let run = Run {
        id: Uuid::new_v4().to_string(),
        workflow: workflow_id.to_owned(),
        state: RunState::Working,
        controller: None,
        cost_usd: 0.0,
        active_secs: 0,
        started_at: unix_secs(),
        ended_at: None,
        outcome: None,
        started_by,
    };
    let id = Uuid::parse_str(&run.id).map_err(|e| e.to_string())?;
    db::insert_run(&hub.db, team.id, &run, None).await?;
    // The box sets its machine on the run (`db::claim_run_box`) before it starts the session.
    if let Err(e) = db::set_run_subject(&hub.db, team.id, id, &work.subject).await {
        db::delete_run(&hub.db, team.id, id).await?;
        return Err(e);
    }
    let placed = match hub.open_team_run(TeamRunPass(()), team, work).await {
        Ok(placed) => placed,
        Err(e) => {
            db::delete_run(&hub.db, team.id, id).await?;
            return Err(e);
        }
    };
    db::set_run_box(&hub.db, team.id, id, &placed.machine).await?;
    super::push(hub, team.id, TeamEvent::RunChanged { run: run.clone() }).await?;
    Ok((run, placed))
}

/// A run of the team ended (`run.state.ended()`): a bot made to run once is discarded, its row and
/// its workflow. `calls::end_run`, which every end of a run goes through, calls it.
pub async fn run_ended(hub: &Hub, team: Uuid, run: &Run) -> Result<(), String> {
    if !run.state.ended() {
        return Ok(());
    }
    let Some(bot) = db::bot(&hub.db, team, &run.workflow).await?.filter(|b| b.once) else { return Ok(()) };
    let account = hub.account(team).await?;
    hub.connectors(&account).await?.delete_connector(&bot.workflow)?;
    db::delete_bot(&hub.db, team, &bot.workflow).await?;
    Ok(())
}

/// A personal `HubCall` a member runs on the team's account (`TeamOp::Hub`). Where it would let a
/// member act around a bot's rules it is held to them:
/// - `SaveConnector` always stores the workflow to run in a box, keeps its stored `enabled` (a new
///   workflow is saved off), and puts a bot back to a draft;
/// - `RunWorkflow` runs only a bot that is on, and records the run (`run_now`);
/// - `DeleteConnector` of a bot deletes its row too.
pub async fn hub_call(hub: &Arc<Hub>, scope: &Scope, call: HubCall) -> Result<Value, String> {
    let team = scope.team.id;
    match call {
        HubCall::SaveConnector { mut connector } => {
            let stored = match connector.id.is_empty() {
                true => None,
                false => workflows(hub, &scope.team).await?.into_iter().find(|c| c.id == connector.id),
            };
            let is_bot = match &stored {
                Some(stored) => db::bot(&hub.db, team, &stored.id).await?.is_some(),
                None => false,
            };
            connector.enabled = !is_bot && stored.as_ref().is_some_and(|c| c.enabled);
            if !matches!(connector.runs_on, pinned_core::connectors::RunsOn::FreshBox(_)) {
                connector.runs_on = pinned_core::connectors::RunsOn::FreshBox(BoxStep::default());
            }
            let saved = Box::pin(hub.hub_call(&scope.team, HubCall::SaveConnector { connector })).await?;
            if is_bot {
                db::turn_bot_on(&hub.db, team, saved["id"].as_str().unwrap_or_default(), false, scope.member).await?;
            }
            Ok(saved)
        }
        HubCall::DeleteConnector { id } => {
            let gone = Box::pin(hub.hub_call(&scope.team, HubCall::DeleteConnector { id: id.clone() })).await?;
            db::delete_bot(&hub.db, team, &id).await?;
            Ok(gone)
        }
        HubCall::RunWorkflow { id } => super::json(run_now(hub, scope, &id).await?),
        call => Box::pin(hub.hub_call(&scope.team, call)).await,
    }
}
