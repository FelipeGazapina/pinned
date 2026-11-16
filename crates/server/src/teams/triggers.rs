//! Triggers: what the team account's own pollers and tab wakes find (`Hub::open_found`). A trigger
//! of a team's workflow starts a run only through its bot (`bots::start_run`), and only while the
//! bot is on, through its waiting line (`queue::ask`). An event for a bot that is off, a draft, or
//! gone is consumed: one log line and a count on the bot, never queued, never replayed. A manual
//! workflow is never started here: it runs only by Run now.

use super::bots::{self, Start};
use super::{db, queue};
use crate::hub::{Account, Hub};
use pinned_core::agent::Subject;
use pinned_core::connectors::{Delivery, Placement, Run, Trigger, Work};
use pinned_core::teams::SkipReason;
use std::sync::Arc;
use uuid::Uuid;

/// Takes what a poller of team `team` found. A run starts through the bot of its workflow; new
/// thread messages go only to the bot's live run on that thread; a prompt is refused.
pub async fn fired(hub: &Arc<Hub>, team: Uuid, work: Work) -> Result<Option<Placement>, String> {
    let team = hub.account(team).await?;
    match work {
        Work::Run(run) => run_fired(hub, &team, run).await,
        Work::Delivery(delivery) => deliver(hub, &team, delivery).await,
        Work::Prompt { .. } => {
            eprintln!("pinned-server: team {}: a workflow's prompt for a live session was refused: a team's sessions are its bots' runs", team.id);
            Err("a team's workflow types into no session".into())
        }
    }
}

/// The trigger kind a run of `trigger` is started by: `trigger:<kind>`.
fn kind(trigger: &Trigger) -> &'static str {
    match trigger {
        Trigger::LinearLabel { .. } => "linear_label",
        Trigger::SlackMention { .. } => "slack_mention",
        Trigger::SlackMessage { .. } => "slack_message",
        Trigger::Called => "called",
        // Refused before a start: a manual workflow runs only by Run now.
        Trigger::Manual => "manual",
    }
}

async fn run_fired(hub: &Arc<Hub>, team: &Arc<Account>, work: Run) -> Result<Option<Placement>, String> {
    let id = work.connector.clone();
    if let Some(reason) = queue::skip_reason(hub, team, &id, true).await? {
        return consumed(hub, team, &id, reason).await;
    }
    let workflow = hub.connectors(team).await?.get_settings().connectors.into_iter().find(|c| c.id == id).ok_or("the workflow is gone")?;
    // A poller that sends a subject's run again while the bot's run on it lives starts no second
    // one: the run's tab takes it, as `place_run` answers for a person. A run whose box has not
    // placed it yet is still starting, so the subject is given back for the next check.
    if db::live_run_for(&hub.db, team.id, &id, &work.subject).await?.is_some() {
        if let Some((machine, tab)) = team.placed(&id, &work.subject) {
            return Ok(Some(Placement { machine, tab }));
        }
        return Err(format!("the bot of workflow {id} already has a live run on this {} that is still starting; the next check tries again", subject_name(&work.subject)));
    }
    let start = Start::Trigger { kind: kind(&workflow.trigger).to_owned(), work };
    match queue::ask(hub, team, &id, start).await {
        Ok(queue::Asked::Started(_, placed)) => Ok(Some(placed)),
        // Waiting: answered as consumed, so nothing outside pinned hears of it until it starts.
        Ok(queue::Asked::Waiting { .. }) => Ok(None),
        // Turned off between the check above and the start.
        Err(e) if e == bots::IS_A_DRAFT => consumed(hub, team, &id, SkipReason::Off).await,
        Err(e) => Err(e),
    }
}

/// An event of bot `id` that starts no run for `reason`: counted, and answered as consumed.
async fn consumed(hub: &Arc<Hub>, team: &Arc<Account>, id: &str, reason: SkipReason) -> Result<Option<Placement>, String> {
    eprintln!("pinned-server: team {} workflow {id} is {}: its trigger's event is consumed", team.id, reason.as_str());
    db::count_skipped(&hub.db, team.id, id, reason, 1).await?;
    Ok(None)
}

/// New messages of a Slack thread go in only while the bot has a live run on that thread, and
/// never start one.
async fn deliver(hub: &Arc<Hub>, team: &Arc<Account>, delivery: Delivery) -> Result<Option<Placement>, String> {
    let subject = Subject::SlackThread { channel: delivery.channel.clone(), thread: delivery.thread.clone() };
    if db::live_run_for(&hub.db, team.id, &delivery.connector, &subject).await?.is_none() {
        eprintln!("pinned-server: team {} workflow {}: new thread messages dropped: its bot has no live run on the thread", team.id, delivery.connector);
        return Ok(None);
    }
    hub.open_placed(team.id, Work::Delivery(delivery)).await
}

fn subject_name(subject: &Subject) -> &'static str {
    match subject {
        Subject::LinearIssue { .. } => "issue",
        Subject::SlackThread { .. } => "thread",
    }
}
