//! The waiting line of a team's runs. A bot has at most one live run, and a team at most
//! `max_live_runs` (3 unless set). A trigger's event or a Run now past that waits in its bot's line
//! (`team_run_queue`), oldest first, at most `MOST_WAITING`: one more drops the oldest. A waiting
//! event touches nothing outside pinned: its trigger's `open` was answered as consumed, so Linear
//! and Slack are left as they are. When a run ends (`calls::end_run`), `next` starts what waits,
//! the ended bot's line first, and checks each bot again: one turned off, back to a draft or
//! removed meanwhile drops its event. Every event that starts no run is counted by why
//! (`db::count_skipped`), and a bot's page shows the counts.

use super::bots::{self, Start};
use super::db;
use crate::hub::{Account, Hub};
use pinned_core::agent::Subject;
use pinned_core::connectors::{Placement, RunsOn, Trigger};
use pinned_core::teams::{Run, SkipReason};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use uuid::Uuid;

/// The most events that wait in one bot's line.
pub const MOST_WAITING: i64 = 20;

/// What `ask` did with a run.
pub enum Asked {
    Started(Run, Placement),
    /// It waits in its bot's line, `place` from the front (1 is next).
    Waiting { place: i64 },
}

/// A waiting event as `team_run_queue` keeps it.
#[derive(Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
enum Stored {
    Member { member: String },
    Trigger { kind: String, cwd: PathBuf, repo: Option<String>, args: Vec<String>, subject: Subject, mentioned: Vec<String>, account: Option<String>, runs_on: RunsOn, input: String },
}

impl Stored {
    fn of(start: &Start) -> Stored {
        match start {
            Start::Member(member) => Stored::Member { member: member.to_string() },
            Start::Trigger { kind, work } => Stored::Trigger {
                kind: kind.clone(),
                cwd: work.cwd.clone(),
                repo: work.repo.clone(),
                args: work.args.clone(),
                subject: work.subject.clone(),
                mentioned: work.mentioned.clone(),
                account: work.account.clone(),
                runs_on: work.runs_on.clone(),
                input: work.input.clone(),
            },
        }
    }

    fn start(self, workflow: &str) -> Start {
        match self {
            Stored::Member { member } => Start::Member(Uuid::parse_str(&member).unwrap_or_default()),
            Stored::Trigger { kind, cwd, repo, args, subject, mentioned, account, runs_on, input } => Start::Trigger {
                kind,
                work: pinned_core::connectors::Run { cwd, repo, args, connector: workflow.to_owned(), subject, mentioned, account, runs_on, input },
            },
        }
    }
}

/// One turn at a time per team, so two events never both take the last free place.
fn line(team: Uuid) -> Arc<tokio::sync::Mutex<()>> {
    static LINES: LazyLock<Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Mutex::default);
    LINES.lock().unwrap().entry(team).or_default().clone()
}

/// Why an event of bot `workflow` would start no run now, if it would not: its workflow is gone,
/// it is not a bot yet (a draft), it is off, or (for a trigger's event) it runs only by Run now.
pub async fn skip_reason(hub: &Arc<Hub>, team: &Account, workflow: &str, by_trigger: bool) -> Result<Option<SkipReason>, String> {
    let Some(stored) = hub.connectors(team).await?.get_settings().connectors.into_iter().find(|c| c.id == workflow) else {
        return Ok(Some(SkipReason::BotRemoved));
    };
    let Some(bot) = db::bot(&hub.db, team.id, workflow).await? else { return Ok(Some(SkipReason::Draft)) };
    if !bot.on || !stored.enabled {
        return Ok(Some(SkipReason::Off));
    }
    if by_trigger && matches!(stored.trigger, Trigger::Manual) {
        return Ok(Some(SkipReason::Manual));
    }
    Ok(None)
}

/// Starts a run of bot `workflow` (`bots::start_run`) when the bot has no live run, nothing waits
/// in its line, and the team has a free place; otherwise it waits at the end of the bot's line.
/// Refused, as `start_run` refuses, when the bot cannot run at all.
pub async fn ask(hub: &Arc<Hub>, team: &Arc<Account>, workflow: &str, start: Start) -> Result<Asked, String> {
    let line = line(team.id);
    let _turn = line.lock().await;
    bots::ready(hub, team, workflow).await?;
    let waiting = db::waiting_count(&hub.db, team.id, workflow).await?;
    let busy = !db::live_runs_of(&hub.db, team.id, workflow).await?.is_empty();
    let full = db::live_count(&hub.db, team.id).await? >= db::max_live_runs(&hub.db, team.id).await?;
    if waiting > 0 || busy || full {
        let stored = serde_json::to_value(Stored::of(&start)).map_err(|e| e.to_string())?;
        let dropped = db::enqueue(&hub.db, team.id, workflow, &stored, MOST_WAITING).await?;
        if dropped > 0 {
            eprintln!("pinned-server: team {} bot {workflow}: its line is full, so its oldest waiting event was dropped", team.id);
            db::count_skipped(&hub.db, team.id, workflow, SkipReason::QueueFull, dropped).await?;
        }
        return Ok(Asked::Waiting { place: (waiting + 1).min(MOST_WAITING) });
    }
    let (run, placed) = bots::start_run(hub, team, workflow, start).await?;
    Ok(Asked::Started(run, placed))
}

/// A run of bot `ended` ended: starts what waits while the team has a free place, in a task of its
/// own, so the end never waits on a box opening. Only `calls::end_run` calls it.
pub fn next(hub: &Hub, team: &Arc<Account>, ended: &str) {
    let Some(hub) = hub.arc() else { return };
    let (team, ended) = (team.clone(), ended.to_owned());
    tokio::spawn(async move {
        if let Err(e) = start_waiting(&hub, &team, &ended).await {
            eprintln!("pinned-server: team {}: a waiting run did not start: {e}", team.id);
        }
    });
}

async fn start_waiting(hub: &Arc<Hub>, team: &Arc<Account>, ended: &str) -> Result<(), String> {
    let line = line(team.id);
    let _turn = line.lock().await;
    loop {
        if db::live_count(&hub.db, team.id).await? >= db::max_live_runs(&hub.db, team.id).await? {
            return Ok(());
        }
        let Some(waiting) = db::next_waiting(&hub.db, team.id, ended).await? else { return Ok(()) };
        db::take_waiting(&hub.db, waiting.id).await?;
        let stored: Stored = match serde_json::from_value(waiting.start) {
            Ok(stored) => stored,
            Err(e) => {
                eprintln!("pinned-server: team {} bot {}: a waiting event was not readable and is dropped: {e}", team.id, waiting.workflow);
                continue;
            }
        };
        let start = stored.start(&waiting.workflow);
        let by_trigger = matches!(start, Start::Trigger { .. });
        if let Some(reason) = skip_reason(hub, team, &waiting.workflow, by_trigger).await? {
            eprintln!("pinned-server: team {} bot {}: a waiting event is dropped: the bot is {}", team.id, waiting.workflow, reason.as_str());
            db::count_skipped(&hub.db, team.id, &waiting.workflow, reason, 1).await?;
            continue;
        }
        let work = match &start {
            Start::Trigger { work, .. } => Some(work.clone()),
            Start::Member(_) => None,
        };
        match bots::start_run(hub, team, &waiting.workflow, start).await {
            // Its trigger's `open` was answered as consumed, so Linear and Slack hear of it only now.
            Ok(_) => {
                if let Some(work) = work {
                    let (hub, team) = (hub.clone(), team.clone());
                    tokio::spawn(async move {
                        let told = async { hub.connectors(&team).await?.started(&work).await };
                        if let Err(e) = told.await {
                            eprintln!("pinned-server: team {} bot {}: its waiting run started, but Linear or Slack was not told: {e}", team.id, work.connector);
                        }
                    });
                }
            }
            Err(e) => eprintln!("pinned-server: team {} bot {}: a waiting run did not start and is dropped: {e}", team.id, waiting.workflow),
        }
    }
}
