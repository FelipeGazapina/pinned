//! Bots: the team account's workflows, with who they ask and what a run may spend.
//!
//! A bot is a workflow of the team's store plus a `team_bots` row. It is a draft (off) until the
//! owner, or someone its call rules name, turns it on, and any change to it makes it a draft again.
//! Its runs always open in a box on the team's own keys and Claude token, never on a member's machine.

use super::{db, Scope};
use crate::hub::{Account, Hub};
use pinned_core::connectors::{BoxStep, Connector, Placement, Trigger};
use pinned_core::graph::migrate::migrate;
use pinned_core::graph::runner::{session_steps, Library, NOT_AN_ON_BOT};
use pinned_core::graph::{NodeKind, Watch};
use pinned_core::teams::{Bot, BotLimits, CallRules, Run, SavedBot, SkipReason, TeamEvent, TeamProject, Who, CEILING_TIMES};
use pinned_core::wire::HubCall;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// What a refused run of a bot that is not on says, whichever way it was asked for.
pub const IS_A_DRAFT: &str = "this bot is a draft: it has to be turned on before it can run";
const NEEDS_KEYS: &str = "the team needs its Upstash key and Claude token before a bot can run";

async fn workflows(hub: &Arc<Hub>, team: &Account) -> Result<Vec<Connector>, String> {
    Ok(hub.connectors(team).await?.get_settings().connectors)
}

async fn workflow(hub: &Arc<Hub>, team: &Account, id: &str) -> Result<Connector, String> {
    find(&workflows(hub, team).await?, id).cloned()
}

fn find<'a>(workflows: &'a [Connector], id: &str) -> Result<&'a Connector, String> {
    workflows.iter().find(|c| c.id == id).ok_or_else(|| format!("this team has no workflow {id}"))
}

/// What a bot whose workflow would reach its session more than once is refused with. On a team,
/// each later reach would type into no session or open a new run in a new box.
pub const MORE_THAN_ONE_SESSION: &str =
    "a bot's workflow may have only one session or talk step, counting the workflows it calls: a workflow with more is not available as a bot yet";

/// What a message router (`Connector::sorts_messages_at_the_check`) is refused with as a bot: it
/// would ask its model and page in Slack before the team admits the message.
pub const SORTS_AT_THE_CHECK: &str =
    "a team bot can't use a routing trigger yet: edit its steps in the workflow editor first";

/// What a workflow with an observability trigger is refused with as a bot: the team keeps no
/// PostHog key, and its poller reads no PostHog project.
pub const OBSERVABILITY_NOT_YET: &str = "a team bot can't use an observability trigger yet";

/// Refuses a workflow whose trigger is not one a bot can have yet ("GitHub-event bots are not
/// available yet"). Linear, Slack, called, manual and schedule triggers are fine; an
/// observability trigger is not (`OBSERVABILITY_NOT_YET`). The matches name every kind on
/// purpose: a GitHub-event trigger added later does not compile until it is refused here or
/// let through. Refuses a graph that holds more than one session or talk step, with those of the
/// team's `workflows` it calls (`MORE_THAN_ONE_SESSION`).
fn available(workflow: &Connector, workflows: &[Connector]) -> Result<(), String> {
    match &workflow.trigger {
        Trigger::LinearLabel { .. }
        | Trigger::SlackMention { .. }
        | Trigger::SlackMessage { .. }
        | Trigger::Called
        | Trigger::Manual
        | Trigger::LinearStatus { .. }
        | Trigger::LinearAssigned { .. }
        | Trigger::Schedule { .. } => {}
        Trigger::Observability { .. } => return Err(OBSERVABILITY_NOT_YET.into()),
    }
    if workflow.sorts_messages_at_the_check() {
        return Err(SORTS_AT_THE_CHECK.into());
    }
    let triggers = workflow.graph.iter().flat_map(|g| g.nodes.iter()).filter_map(|n| match &n.kind {
        NodeKind::Trigger { watch } => Some(watch),
        _ => None,
    });
    for watch in triggers {
        match watch {
            Watch::LinearLabel { .. }
            | Watch::SlackMention { .. }
            | Watch::SlackMessage { .. }
            | Watch::Called
            | Watch::Manual
            | Watch::LinearStatus { .. }
            | Watch::LinearAssigned { .. }
            | Watch::Schedule { .. } => {}
            Watch::Observability { .. } => return Err(OBSERVABILITY_NOT_YET.into()),
        }
    }
    let library: Library = workflows.iter().map(|c| (c.id.clone(), c.graph.clone().unwrap_or_else(|| migrate(c)))).collect();
    if session_steps(&library, &workflow.id) > 1 {
        return Err(MORE_THAN_ONE_SESSION.into());
    }
    Ok(())
}

/// Refuses `workflow` when one of its Call steps calls a workflow that is not an on bot of this
/// team: a draft, an off bot, or one the team does not have (deleted, or another account's).
/// Checked at save, at turn-on and when a run starts; the graph runner checks again at the step.
async fn callees_on(hub: &Arc<Hub>, team: Uuid, workflow: &Connector, workflows: &[Connector]) -> Result<(), String> {
    callees_read(hub, team, workflow, workflows).await?
}

/// `callees_on`, with a read of a bot row that failed (`Err`) apart from a refusal (`Ok(Err)`).
async fn callees_read(hub: &Arc<Hub>, team: Uuid, workflow: &Connector, workflows: &[Connector]) -> Result<Result<(), String>, String> {
    let graph = workflow.graph.clone().unwrap_or_else(|| migrate(workflow));
    for node in &graph.nodes {
        let NodeKind::Call { workflow: target } = &node.kind else { continue };
        let enabled = workflows.iter().any(|c| &c.id == target && c.enabled);
        if !enabled || !db::bot(&hub.db, team, target).await?.is_some_and(|b| b.on) {
            return Ok(Err(NOT_AN_ON_BOT.into()));
        }
    }
    Ok(Ok(()))
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

/// The most a bot's limits may be: `CEILING_TIMES` the policy's `run_usd` and `run_minutes`.
pub fn most(policy: &pinned_core::teams::TeamPolicy) -> BotLimits {
    BotLimits { usd: policy.run_usd * f64::from(CEILING_TIMES), minutes: policy.run_minutes.saturating_mul(CEILING_TIMES) }
}

/// `limits` held at the ceiling (`most`): a row stored above it, by an older hub or by hand, is
/// shown, saved again and enforced as the ceiling.
pub fn held(policy: &pinned_core::teams::TeamPolicy, limits: BotLimits) -> BotLimits {
    let most = most(policy);
    BotLimits { usd: limits.usd.min(most.usd), minutes: limits.minutes.min(most.minutes) }
}

/// Every workflow of the team's account as a bot: its `team_bots` row, or `CallRules::default()`
/// and the hub's `TeamPolicy` limits when it has none, with `name` from the workflow.
/// The team workflow of bot `id`, for its page. A member reads it; the team's settings do not.
pub async fn bot_workflow(hub: &Arc<Hub>, scope: &Scope, id: &str) -> Result<Connector, String> {
    workflow(hub, &scope.team, id).await
}

pub async fn bots(hub: &Arc<Hub>, scope: &Scope) -> Result<Vec<Bot>, String> {
    let rows = db::bots(&hub.db, scope.team.id).await?;
    let mut bots = Vec::new();
    for workflow in workflows(hub, &scope.team).await? {
        let bot = match rows.iter().find(|b| b.workflow == workflow.id) {
            Some(row) => Bot { name: workflow.name.clone(), limits: held(&hub.team_policy, row.limits.clone()), ..row.clone() },
            None => Bot { workflow: workflow.id.clone(), name: workflow.name.clone(), owner: None, calls: CallRules::default(), limits: default_limits(hub), on: false, on_by: None, once: false, off_note: None, skipped: vec![] },
        };
        let skipped = db::skipped(&hub.db, scope.team.id, &workflow.id).await?;
        bots.push(Bot { skipped, ..bot });
    }
    Ok(bots)
}

/// Project `project` was taken out of the team: every bot that is on and works in it (its
/// workflow's `repo` is the project's, by `boxes::project_key`) goes back to a draft, with a note
/// that says why, and every live run of a bot that works in it ends as `Stopped`, its box with it
/// (`calls::stop_live_runs`). Answers the workflows it turned off.
pub async fn project_detached(hub: &Hub, team: &Arc<Account>, project: &TeamProject) -> Result<Vec<String>, String> {
    let note = format!("Turned off: project {} was taken out of the team.", project.name);
    let stopped = format!("Stopped: project {} was taken out of the team.", project.name);
    let mut off = Vec::new();
    let connectors = hub.connectors(team).await?;
    for workflow in connectors.get_settings().connectors {
        if crate::boxes::project_key(&workflow.repo).as_deref() != Some(project.key.as_str()) {
            continue;
        }
        let id = workflow.id.clone();
        if db::turn_bot_off_noted(&hub.db, team.id, &workflow.id, &note).await? {
            let mut stored = workflow;
            stored.enabled = false;
            off.push(stored.id.clone());
            connectors.save_connector(stored)?;
        }
        super::calls::stop_live_runs(hub, team, &id, &stopped).await?;
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

/// Whether `id`, which the team does not keep, may name a new workflow the app made: a uuid
/// (`connectors::is_uuid`) no other account's workflow has. Else it is an unknown workflow.
async fn app_named(hub: &Arc<Hub>, team: Uuid, id: &str) -> Result<(), String> {
    if !pinned_core::connectors::is_uuid(id) || db::workflow_elsewhere(&hub.db, team, id).await? {
        return Err(format!("this team has no workflow {id}"));
    }
    Ok(())
}

/// Any member. Saves who the bot asks and its limits for workflow `bot.workflow` of the team's
/// account, and with `connector` that workflow itself, on the team's account: a new one when its
/// id is empty or a uuid the app made (`app_named`), which it keeps so the same save sent again
/// saves that workflow, else one the team already keeps (any other id is refused as an unknown
/// workflow). The bot goes back to a draft, its workflow disabled, whoever saves it: `on`
/// and `on_by` of `bot` are never read, and turning it on again is `turn_bot_on`. A limit above
/// the hub's team policy is the owner's to give. Nothing is saved when a check refuses. Answers the
/// bot's workflow.
pub async fn save_bot(hub: &Arc<Hub>, scope: &Scope, mut bot: Bot, connector: Option<Connector>) -> Result<SavedBot, String> {
    let team = scope.team.id;
    let mut workflows = workflows(hub, &scope.team).await?;
    // A new workflow: no row of its bot to turn off first.
    let mut new = false;
    let connector = match connector {
        Some(mut connector) => {
            new = !workflows.iter().any(|c| c.id == connector.id);
            if new && !connector.id.is_empty() {
                app_named(hub, team, &connector.id).await?;
            }
            if !bot.workflow.is_empty() && bot.workflow != connector.id {
                return Err(format!("this bot is for workflow {}, not the workflow it saves", bot.workflow));
            }
            connector.enabled = false;
            if !matches!(connector.runs_on, pinned_core::connectors::RunsOn::FreshBox(_)) {
                connector.runs_on = pinned_core::connectors::RunsOn::FreshBox(BoxStep::default());
            }
            bot.workflow = connector.id.clone();
            // The checks below read the workflow as it is about to be saved.
            workflows.retain(|c| c.id != connector.id);
            workflows.push(connector.clone());
            Some(connector)
        }
        None => None,
    };
    let workflow = find(&workflows, &bot.workflow)?;
    available(workflow, &workflows)?;
    callees_on(hub, team, workflow, &workflows).await?;
    check_who(hub, team, &bot.calls).await?;
    let BotLimits { usd, minutes } = bot.limits;
    if !usd.is_finite() || usd <= 0.0 || minutes == 0 {
        return Err("a run needs a limit above 0 dollars and 0 minutes".into());
    }
    let existing = match bot.workflow.is_empty() {
        true => None,
        false => db::bot(&hub.db, team, &bot.workflow).await?,
    };
    let kept = existing.as_ref().map(|b| &b.limits);
    let policy = default_limits(hub);
    // A member may keep a limit the owner gave, not raise it.
    let above = |wants: f64, ceiling: f64, kept: Option<f64>| wants > ceiling && kept.is_none_or(|k| wants > k);
    if above(usd, policy.usd, kept.map(|l| l.usd)) || above(f64::from(minutes), f64::from(policy.minutes), kept.map(|l| f64::from(l.minutes))) {
        if scope.owner_only().is_err() {
            return Err(format!("a bot may spend at most ${} and run {} minutes unless the team's owner sets more", policy.usd, policy.minutes));
        }
        // Not even the owner passes the ceiling `set_bot_ceiling` holds.
        ceiling(hub, usd, f64::from(minutes))?;
    }
    if !new {
        // Off before its workflow changes, so no trigger runs the new workflow as an on bot.
        db::turn_bot_on(&hub.db, team, &bot.workflow, false, scope.member).await?;
    }
    if let Some(connector) = connector {
        // `Hub::hub_call` on the team's account, as the team's own save: it tells the team's apps.
        let saved = Box::pin(hub.hub_call(&scope.team, HubCall::SaveConnector { connector })).await?;
        bot.workflow = saved["id"].as_str().ok_or("the saved workflow has no id")?.to_owned();
    }
    let owner = existing.and_then(|b| b.owner).or_else(|| Some(scope.member.to_string()));
    let limits = held(&hub.team_policy, bot.limits.clone());
    let saved = Bot { name: String::new(), owner, on: false, on_by: None, off_note: None, limits, ..bot };
    db::save_bot(&hub.db, team, &saved).await?;
    draft(hub, scope, &saved.workflow).await?;
    Ok(SavedBot { workflow: saved.workflow })
}

/// Whether `usd` and `minutes` are a bot's limits a team's owner may give: finite, above 0,
/// `minutes` whole, and at most `CEILING_TIMES` the hub's `TeamPolicy::run_usd` and `run_minutes`.
fn ceiling(hub: &Hub, usd: f64, minutes: f64) -> Result<BotLimits, String> {
    if !(usd.is_finite() && usd > 0.0 && minutes.is_finite() && minutes >= 1.0 && minutes.fract() == 0.0) {
        return Err("a bot's ceiling needs dollars and whole minutes above 0".into());
    }
    let policy = default_limits(hub);
    let (most_usd, most_minutes) = (policy.usd * f64::from(CEILING_TIMES), policy.minutes.saturating_mul(CEILING_TIMES));
    if usd > most_usd || minutes > f64::from(most_minutes) {
        return Err(format!("a bot's ceiling is at most ${most_usd} and {most_minutes} minutes"));
    }
    Ok(BotLimits { usd, minutes: minutes as u32 })
}

/// The team's owner only (a locked login unlocks first, `super::owner_only`): sets the
/// `BotLimits` each run of the bot of `workflow_id` may spend, at most `CEILING_TIMES` the hub's
/// policy (`ceiling`). The bot must be saved; it stays on or off as it was.
pub async fn set_bot_ceiling(hub: &Arc<Hub>, scope: &Scope, workflow_id: &str, usd: f64, minutes: f64) -> Result<(), String> {
    scope.owner_only()?;
    let limits = ceiling(hub, usd, minutes)?;
    // Only the row's limits, in one write: a save of the bot's rules at the same time stays.
    if !db::set_bot_limits(&hub.db, scope.team.id, workflow_id, &limits).await? {
        return Err("the team has no such bot".into());
    }
    Ok(())
}

/// What a bot whose stored limits are not above 0 hears when it would start.
pub const NO_LIMIT: &str = "this bot has no limit to run with: the team's owner sets one";

/// What a team's owner reads for "One box, reused" on a team: each run opens a fresh box.
pub const OWN_BOX_EACH: &str = "a team's runs each get a box of their own";

/// What a refused delete of a bot that is on, or has a live run, says.
pub const TURN_OFF_FIRST: &str = "turn the bot off first";

/// Deletes a bot that is off and has no live run: its `team_bots` row and its team workflow. Only
/// its owner (the member who answers for it) or the team's owner; anyone else is answered as if
/// the team had no such workflow. Its past runs stay listed: `team_runs` names the workflow by id
/// only.
pub async fn delete_bot(hub: &Arc<Hub>, scope: &Scope, workflow_id: &str) -> Result<(), String> {
    let team = scope.team.id;
    let unknown = format!("this team has no workflow {workflow_id}");
    let workflows = workflows(hub, &scope.team).await?;
    let workflow = find(&workflows, workflow_id)?;
    let row = db::bot(&hub.db, team, workflow_id).await?;
    if workflow.enabled || row.as_ref().is_some_and(|b| b.on) || !db::live_runs_of(&hub.db, team, workflow_id).await?.is_empty() {
        return Err(TURN_OFF_FIRST.into());
    }
    let owns = row.as_ref().and_then(|b| b.owner.as_deref()) == Some(scope.member.to_string().as_str());
    if !owns && scope.owner_only().is_err() {
        return Err(unknown);
    }
    Box::pin(hub.hub_call(&scope.team, HubCall::DeleteConnector { id: workflow_id.to_owned() })).await?;
    db::delete_bot(&hub.db, team, workflow_id).await?;
    Ok(())
}

/// Turns the bot on or off, keeping who did it (`on_by`), and enables or disables the team
/// account's workflow to match. Off is any member's. On is the owner's, a member of a group the
/// bot's call rules name, or the person one of them names, and only while the team keeps the
/// Upstash key and Claude token its boxes sign in with.
pub async fn turn_bot_on(hub: &Arc<Hub>, scope: &Scope, workflow_id: &str, on: bool) -> Result<(), String> {
    let team = scope.team.id;
    let workflows = workflows(hub, &scope.team).await?;
    let workflow = find(&workflows, workflow_id)?;
    let row = db::bot(&hub.db, team, workflow_id).await?;
    if !on {
        draft(hub, scope, workflow_id).await?;
        return Ok(());
    }
    available(workflow, &workflows)?;
    callees_on(hub, team, workflow, &workflows).await?;
    let calls = row.as_ref().map(|b| b.calls.clone()).unwrap_or_default();
    if !asked_by(hub, scope, &calls).await? {
        return Err("only the team's owner, or someone this bot asks, turns it on".into());
    }
    let credential = super::membership::credential(hub, &scope.team).await?;
    if !(credential.upstash && credential.claude) {
        return Err(NEEDS_KEYS.into());
    }
    if row.is_none() {
        let fresh = Bot { workflow: workflow_id.to_owned(), name: String::new(), owner: Some(scope.member.to_string()), calls, limits: default_limits(hub), on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
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

/// What `Hub::open_team_run` asks for. Only `start_reserved` makes one, so a team's run opens through
/// no other path.
pub struct TeamRunPass(());

/// How a team's run was asked for.
pub enum Start {
    /// A member's Run now, by the member's id.
    Member(Uuid),
    /// A trigger of the bot's workflow fired (`triggers::fired`): `kind` names it (`linear_label`,
    /// `slack_mention`, `slack_message`, `called`), and `work` is the session its poller found.
    Trigger { kind: String, work: pinned_core::connectors::Run },
}

/// Any member. Runs the bot now, as its workflow's Run now would on the team's account
/// (`start_reserved`, started by the member), or puts it in the bot's line (`queue::ask`): then the
/// reply says it waits, `{"waiting":true,"workflow":"c1","place":2,"message":"…"}`.
pub async fn run_now(hub: &Arc<Hub>, scope: &Scope, workflow_id: &str) -> Result<Value, String> {
    match super::queue::ask(hub, &scope.team, workflow_id, Start::Member(scope.member)).await? {
        super::queue::Asked::Started(run, _) => super::json(run),
        super::queue::Asked::Waiting { place } => Ok(serde_json::json!({
            "waiting": true,
            "workflow": workflow_id,
            "place": place,
            "message": format!("This bot is busy or the team has as many runs going as it may: your run waits in line (number {place}) and starts when one ends."),
        })),
    }
}

/// Whether bot `workflow_id` may start a run now: it is on, its limits are above 0 (`NO_LIMIT`),
/// its workflow is one a bot can have, and the team keeps its Upstash key and Claude token.
pub async fn ready(hub: &Arc<Hub>, team: &Account, workflow_id: &str) -> Result<(), String> {
    ready_in(hub, &TeamWorkflows::read(hub, team).await?, workflow_id).await?
}

/// A team's workflows, keys and settings as read once (`Hub::connectors`), with the team they are
/// of: what `ready_in` and the waiting line decide on, so no caller hands them another team's.
pub struct TeamWorkflows {
    team: Uuid,
    connectors: Arc<pinned_core::connectors::Connectors>,
}

impl TeamWorkflows {
    pub async fn read(hub: &Hub, team: &Account) -> Result<TeamWorkflows, String> {
        Ok(TeamWorkflows { team: team.id, connectors: hub.connectors(team).await? })
    }

    pub fn team(&self) -> Uuid {
        self.team
    }

    pub fn connectors(&self) -> &pinned_core::connectors::Connectors {
        &self.connectors
    }
}

/// `ready` with the team's workflows and keys as already read (`team`): what it reads itself is
/// only bot rows, `workflow_id`'s and those of the bots its workflow calls. A read of one that
/// failed, busy or not, is `Err`, apart from a refusal (`Ok(Err)`): it says nothing of the bot.
pub async fn ready_in(hub: &Arc<Hub>, team: &TeamWorkflows, workflow_id: &str) -> Result<Result<(), String>, String> {
    let settings = team.connectors.get_settings();
    let workflows = &settings.connectors;
    let workflow = match find(workflows, workflow_id) {
        Ok(workflow) => workflow,
        Err(e) => return Ok(Err(e)),
    };
    let bot = db::bot(&hub.db, team.team, workflow_id).await?;
    if !bot.as_ref().is_some_and(|b| b.on) || !workflow.enabled {
        return Ok(Err(IS_A_DRAFT.into()));
    }
    // A run is held by its limits (`calls::tick`): a bot without them never starts one.
    if !bot.is_some_and(|b| b.limits.usd.is_finite() && b.limits.usd > 0.0 && b.limits.minutes > 0) {
        return Ok(Err(NO_LIMIT.into()));
    }
    // Read only once the bot's own refusals are decided: a callee read that fails never hides them.
    let callees = callees_read(hub, team.team, workflow, workflows).await?;
    if let Err(e) = available(workflow, workflows).and(callees) {
        return Ok(Err(e));
    }
    let credential = super::membership::credential_in(&settings);
    if !(credential.upstash && credential.claude) {
        return Ok(Err(NEEDS_KEYS.into()));
    }
    Ok(Ok(()))
}

/// What a run of bot `workflow_id` opens, and who starts it (the member, or `trigger:<kind>`),
/// for a member's Run now or a trigger that fired. Whatever the stored workflow says, the run
/// opens in a fresh box of the team's own. Only `queue` asks, in the team's turn, before it
/// reserves the run's place: the run then starts as every team run does, by `start_reserved`.
pub fn work_of(connectors: &pinned_core::connectors::Connectors, workflow_id: &str, start: Start) -> Result<(pinned_core::connectors::Run, String), String> {
    let (mut work, started_by) = match start {
        Start::Member(member) => (connectors.run_now(workflow_id, "a member of the team, from its bots")?, member.to_string()),
        Start::Trigger { kind, work } => (work, format!("trigger:{kind}")),
    };
    // Whatever the stored workflow says, a bot never runs on a member's machine.
    if !matches!(work.runs_on, pinned_core::connectors::RunsOn::FreshBox(_)) {
        work.runs_on = pinned_core::connectors::RunsOn::FreshBox(BoxStep::default());
    }
    Ok((work, started_by))
}

/// Ends the box `machine` of run `id`, which ended while its box booted: its session is not the run's.
async fn end_box_of_ended_run(hub: &Arc<Hub>, team: &Arc<Account>, id: Uuid, machine: &str) {
    if let Ok(machine) = Uuid::parse_str(machine) {
        if let Err(e) = hub.boxes.end_machine(hub, team, machine, crate::boxes::End::Removed).await {
            eprintln!("pinned-server: the box of run {id} of team {} did not end yet: {e}", team.id);
        }
    }
}

/// The one way a team's run starts: opens the session of run `id`, whose place the team's turn
/// reserved before anything outside pinned heard of it (`queue::ask`, `queue::admit`, a waiting
/// run's turn), in a fresh box of the team's own, outside that turn. The bot must still be on and
/// the team must keep its Upstash key and Claude token. A session that does not open frees the
/// place, is counted `SkipReason::StartFailed` unless the backstop or a stop ended its run first,
/// and what waits in the bot's line goes next; the run stays listed when it had ended meanwhile.
/// The bot's `BotLimits` are held by `calls::tick`.
pub async fn start_reserved(hub: &Arc<Hub>, team: &Arc<Account>, id: Uuid, mut work: pinned_core::connectors::Run) -> Result<Placement, String> {
    let opened = async {
        ready(hub, team, &work.connector).await?;
        if !matches!(work.runs_on, pinned_core::connectors::RunsOn::FreshBox(_)) {
            work.runs_on = pinned_core::connectors::RunsOn::FreshBox(BoxStep::default());
        }
        hub.open_team_run(TeamRunPass(()), team, work.clone()).await
    };
    let placed = match opened.await {
        Ok(placed) => placed,
        Err(e) => {
            // Counted and deleted only through one conditional write on the run's row
            // (`db::fail_reserved_start`): a run still live and reserved is deleted, one its box's
            // failure ended stays listed, and either counts `StartFailed` once. A run the backstop
            // ended (counted `NeverStarted` there) or that was stopped stays as it ended, counted
            // by nobody here. No memory of this hub decides the count.
            // Each cleanup step is logged when it fails and the next still runs: above all
            // `queue::next`, so what waits in the bot's line is not left for the reaper.
            match db::fail_reserved_start(&hub.db, team.id, id, super::calls::NEVER_OPENED).await {
                Ok(true) => {
                    if let Err(e) = db::count_skipped(&hub.db, team.id, &work.connector, SkipReason::StartFailed, 1).await {
                        eprintln!("pinned-server: team {} bot {}: a failed start was not counted: {e}", team.id, work.connector);
                    }
                }
                Ok(false) => {}
                Err(e) => eprintln!("pinned-server: team {} bot {}: run {id} did not start and its place was not freed yet: {e}", team.id, work.connector),
            }
            super::queue::next(hub, team, &work.connector);
            return Err(e);
        }
    };
    // Stopped, or ended by the backstop, since its session started: it gets no box, and its box ends
    // at once. In a task of its own: the poller's check that opens this holds its subject while it
    // runs, and ending the box needs that subject to fail the runs the box held.
    // Its session works already: a write that fails (a busy row) is tried once more, and when that
    // fails too its box ends as well, so no session keeps working on a run that only holds a place.
    // A first write that failed may have committed all the same (its connection dropped after
    // it): whatever the retry answers short of opening it, the run's row is read, and a run found
    // opened on this box keeps the box. A row that does not read ends the box.
    let open = match db::open_reserved(&hub.db, team.id, id, &placed.machine).await {
        Ok(open) => Ok(open),
        Err(_) => match db::open_reserved(&hub.db, team.id, id, &placed.machine).await {
            Ok(true) => Ok(true),
            retried => match db::reservation_of(&hub.db, team.id, id).await {
                Ok(db::Reservation::Opened(Some(on))) if on == placed.machine => Ok(true),
                Ok(_) => retried,
                Err(e) => {
                    eprintln!("pinned-server: run {id} of team {}: its row was not read after its open was retried: {e}", team.id);
                    retried.and(Err(e))
                }
            },
        },
    };
    if !matches!(open, Ok(true)) {
        let (ending, of, machine) = (hub.clone(), team.clone(), placed.machine.clone());
        tokio::spawn(async move { end_box_of_ended_run(&ending, &of, id, &machine).await });
        return Err(match open {
            Err(e) => {
                eprintln!("pinned-server: run {id} of team {} did not open, so its box is ended: {e}", team.id);
                e
            }
            Ok(_) => super::calls::ENDED_WHILE_BOOTING.into(),
        });
    }
    if let Some(run) = db::run(&hub.db, team.id, id).await? {
        super::push(hub, team.id, TeamEvent::RunChanged { run }).await?;
    }
    Ok(placed)
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

/// A personal `HubCall` a member runs on the team's account (`TeamOp::Hub`), once `hub_rule`
/// let it. None of the calls that save, delete or run a workflow is let: a team's workflows
/// change and run only as its bots (`save_bot`, `delete_bot`, `run_now`).
pub async fn hub_call(hub: &Arc<Hub>, scope: &Scope, call: HubCall) -> Result<Value, String> {
    Box::pin(hub.hub_call(&scope.team, call)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinned_core::connectors::ObservabilitySource;
    use pinned_core::graph::{Edge, Graph, Node, NodeId, PortId};
    use serde_json::json;

    fn workflow(trigger: Value) -> Connector {
        serde_json::from_value(json!({
            "id": "c1", "name": "Errors", "enabled": true, "trigger": trigger, "folder": "/repo", "model": "",
            "effort": "high", "permissionMode": "plan", "worktree": false, "instructions": "",
        }))
        .unwrap()
    }

    fn graph(watch: Watch) -> Graph {
        let node = |id: &str, kind: NodeKind| Node { id: NodeId(id.into()), name: id.into(), position: [0, 0], group_name: None, kind };
        Graph {
            nodes: vec![node("trigger", NodeKind::Trigger { watch }), node("talk", NodeKind::Talk { prompt: String::new() })],
            edges: vec![Edge { from: NodeId("trigger".into()), port: PortId("out".into()), to: NodeId("talk".into()) }],
        }
    }

    #[test]
    fn a_bot_refuses_an_observability_trigger() {
        let observes = workflow(json!({ "kind": "observability", "source": { "kind": "posthog" } }));
        assert_eq!(available(&observes, std::slice::from_ref(&observes)), Err(OBSERVABILITY_NOT_YET.into()));
        let mut drawn = workflow(json!({ "kind": "manual" }));
        drawn.graph = Some(graph(Watch::Observability { source: ObservabilitySource::Posthog }));
        assert_eq!(available(&drawn, std::slice::from_ref(&drawn)), Err(OBSERVABILITY_NOT_YET.into()), "a graph's observability watch too");
        let manual = workflow(json!({ "kind": "manual" }));
        assert_eq!(available(&manual, std::slice::from_ref(&manual)), Ok(()));
    }
}
