//! The waiting line of a team's runs. A bot has at most one live run, and a team at most
//! `max_live_runs` (3 unless set). A trigger's event or a Run now past that waits in its bot's line
//! (`team_run_queue`), oldest first, at most `MOST_WAITING`: one more drops the oldest. A waiting
//! event touches nothing outside pinned: its trigger's `open` was answered as consumed, so Linear
//! and Slack are left as they are. When a run ends (`calls::end_run`), `next` starts what waits,
//! the ended bot's line first, and checks each bot again: one turned off, back to a draft or
//! removed meanwhile drops its event. Every event that starts no run is counted by why
//! (`db::count_skipped`), and a bot's page shows the counts.

use super::bots::{self, Start};
use super::{calls, db};
use crate::hub::{Account, Hub};
use pinned_core::agent::Subject;
use pinned_core::connectors::{Admitted, Delivery, Event, Placement, Replies, RunsOn, Trigger, TriggerInput};
use pinned_core::teams::{Run, RunState, SkipReason};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use uuid::Uuid;

/// The most events that wait in one bot's line.
pub const MOST_WAITING: i64 = 20;
/// The most replies of a Slack thread kept with its waiting event: older ones are only counted.
pub const MOST_REPLIES: usize = 20;

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
    /// A trigger's raw event, as its poller found it before its Ask step or any graph step
    /// (`admit`): its turn runs it whole, from the start (`Connectors::granted`).
    /// The replies its Slack thread got while it waited, the newest `MOST_REPLIES`, and how many
    /// older ones were dropped.
    Event {
        kind: String,
        event: Event,
        #[serde(default)]
        replies: Vec<String>,
        #[serde(default)]
        earlier: u64,
    },
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

    /// What a member's or an older trigger's waiting run starts with; an event (`Stored::Event`)
    /// runs through the team's poller instead.
    fn start(self, workflow: &str) -> Option<Start> {
        match self {
            Stored::Member { member } => Some(Start::Member(Uuid::parse_str(&member).unwrap_or_default())),
            Stored::Trigger { kind, cwd, repo, args, subject, mentioned, account, runs_on, input } => Some(Start::Trigger {
                kind,
                work: pinned_core::connectors::Run { cwd, repo, args, connector: workflow.to_owned(), subject, mentioned, account, runs_on, input },
            }),
            Stored::Event { .. } => None,
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

/// Decides, once and before anything outside pinned happens for it, what trigger `kind`'s event
/// does: it runs now (its place held by a reserved run the cap counts), or it waits at the end of
/// its bot's line, or it is skipped (counted on the bot) for a bot that would not run it. A
/// subject the bot already runs goes to that run, as before.
pub async fn admit(hub: &Arc<Hub>, team: &Arc<Account>, kind: &str, event: Event) -> Result<Admitted, String> {
    let workflow = event.connector().to_owned();
    if let Some(reason) = skip_reason(hub, team, &workflow, true).await? {
        eprintln!("pinned-server: team {} workflow {workflow} is {}: its trigger's event is consumed", team.id, reason.as_str());
        db::count_skipped(&hub.db, team.id, &workflow, reason, 1).await?;
        return Ok(Admitted::Held);
    }
    if let Some(live) = db::live_run_for(&hub.db, team.id, &workflow, event.subject()).await? {
        let reserved = Uuid::parse_str(&live.id).map_err(|e| e.to_string())?;
        return Ok(match db::is_reserved(&hub.db, team.id, reserved).await? {
            true => Admitted::Held,
            false => Admitted::Go,
        });
    }
    let line = line(team.id);
    let _turn = line.lock().await;
    bots::ready(hub, team, &workflow).await?;
    let waiting = db::waiting_count(&hub.db, team.id, &workflow).await?;
    let busy = !db::live_runs_of(&hub.db, team.id, &workflow).await?.is_empty();
    let full = db::live_count(&hub.db, team.id).await? >= db::max_live_runs(&hub.db, team.id).await?;
    if waiting > 0 || busy || full {
        let stored = serde_json::to_value(Stored::Event { kind: kind.to_owned(), event, replies: vec![], earlier: 0 }).map_err(|e| e.to_string())?;
        let dropped = db::enqueue(&hub.db, team.id, &workflow, &stored, MOST_WAITING).await?;
        if dropped > 0 {
            eprintln!("pinned-server: team {} bot {workflow}: its line is full, so its oldest waiting event was dropped", team.id);
            db::count_skipped(&hub.db, team.id, &workflow, SkipReason::QueueFull, dropped).await?;
        }
        return Ok(Admitted::Waiting);
    }
    reserve(hub, team, kind, &event).await?;
    Ok(Admitted::Go)
}

/// Holds a place for `event`: a live run with no session yet, which the team's cap counts.
async fn reserve(hub: &Arc<Hub>, team: &Arc<Account>, kind: &str, event: &Event) -> Result<(), String> {
    let run = Run {
        id: Uuid::new_v4().to_string(),
        workflow: event.connector().to_owned(),
        state: RunState::Working,
        controller: None,
        cost_usd: 0.0,
        active_secs: 0,
        started_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64),
        ended_at: None,
        outcome: None,
        started_by: format!("trigger:{kind}"),
    };
    db::insert_reserved(&hub.db, team.id, &run, event.subject()).await
}

/// New replies of a Slack thread: kept with the thread's waiting event, the newest
/// `MOST_REPLIES`; passed on when the bot runs the thread; dropped when nothing does.
pub async fn keep_replies(hub: &Arc<Hub>, team: &Arc<Account>, delivery: &Delivery) -> Result<Replies, String> {
    let subject = Subject::SlackThread { channel: delivery.channel.clone(), thread: delivery.thread.clone() };
    if db::live_run_for(&hub.db, team.id, &delivery.connector, &subject).await?.is_some() {
        return Ok(Replies::Pass);
    }
    let line = line(team.id);
    let _turn = line.lock().await;
    let Some(waiting) = db::waiting_event_for(&hub.db, team.id, &delivery.connector, &subject).await? else {
        eprintln!("pinned-server: team {} workflow {}: new thread messages dropped: the thread has no run and no waiting event", team.id, delivery.connector);
        return Ok(Replies::Dropped);
    };
    let Ok(Stored::Event { kind, event, mut replies, mut earlier }) = serde_json::from_value(waiting.start) else {
        return Ok(Replies::Dropped);
    };
    replies.extend(delivery.messages.iter().cloned());
    let over = replies.len().saturating_sub(MOST_REPLIES);
    replies.drain(..over);
    earlier += over as u64;
    let stored = serde_json::to_value(Stored::Event { kind, event, replies, earlier }).map_err(|e| e.to_string())?;
    db::set_waiting(&hub.db, waiting.id, &stored).await?;
    Ok(Replies::Kept)
}

/// `event` with the replies its thread got while it waited, each framed as untrusted text, after
/// its first prompt: a plain run's prompt, or a graph's Slack trigger note.
pub fn with_replies(event: Event, replies: &[String], earlier: u64) -> Event {
    if replies.is_empty() && earlier == 0 {
        return event;
    }
    let mut text = vec!["Replies the thread got while this run waited to start:".to_owned()];
    if earlier > 0 {
        text.push(format!("{earlier} earlier replies not shown"));
    }
    text.extend(replies.iter().map(|reply| pinned_core::untrusted::frame("a Slack reply in this thread", reply)));
    let text = text.join("\n\n");
    match event {
        Event::Run(mut run) => {
            if let Some(last) = run.args.last_mut() {
                *last = format!("{last}\n\n{text}");
            }
            Event::Run(run)
        }
        Event::Graph { connector, subject, mut input, at } => {
            if let TriggerInput::Slack { note, .. } = &mut input {
                *note = Some(note.take().map_or(text.clone(), |n| format!("{n}\n\n{text}")));
            }
            Event::Graph { connector, subject, input, at }
        }
    }
}

/// What an admitted event's run ends with when it ends before its session opened.
pub const ENDED_UNOPENED: &str = "the workflow ended before it opened a session";

/// The admitted event of `workflow` on `subject` ended before its session opened: its reserved
/// run ends (`calls::end_run`), so what waits starts (`next`). A run whose session opened is left.
pub async fn release(hub: &Arc<Hub>, team: &Arc<Account>, workflow: &str, subject: &Subject) -> Result<(), String> {
    let Some(id) = db::reserved_run_for(&hub.db, team.id, workflow, subject).await? else { return Ok(()) };
    match calls::end_run(hub, team, id, RunState::Done, ENDED_UNOPENED, None).await {
        Err(e) if e != calls::RUN_ENDED => Err(e),
        _ => Ok(()),
    }
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
        if let Stored::Event { kind, event, replies, earlier } = stored {
            if let Some(reason) = skip_reason(hub, team, &waiting.workflow, true).await? {
                eprintln!("pinned-server: team {} bot {}: a waiting event is dropped: the bot is {}", team.id, waiting.workflow, reason.as_str());
                db::count_skipped(&hub.db, team.id, &waiting.workflow, reason, 1).await?;
                continue;
            }
            // Its place is held now, and the team's poller runs it whole, from the start.
            reserve(hub, team, &kind, &event).await?;
            hub.connectors(team).await?.granted(with_replies(event, &replies, earlier));
            continue;
        }
        let Some(start) = stored.start(&waiting.workflow) else { continue };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn slack_run(prompt: &str) -> Event {
        Event::Run(pinned_core::connectors::Run {
            cwd: PathBuf::from("/work/acme"),
            repo: None,
            args: vec!["--name".into(), "Footer".into(), prompt.into()],
            connector: "footer".into(),
            subject: Subject::SlackThread { channel: "C0ACME001".into(), thread: "1792400050.000200".into() },
            mentioned: vec![],
            account: None,
            runs_on: RunsOn::Any,
            input: String::new(),
        })
    }

    #[test]
    fn the_replies_a_waiting_run_kept_follow_its_first_prompt_each_framed() {
        let replies: Vec<String> = (6..=25).map(|n| format!("reply {n}")).collect();
        let Event::Run(run) = with_replies(slack_run("Fix the footer."), &replies, 5) else { panic!("a run") };
        let prompt = run.args.last().unwrap();
        let framed: Vec<String> = replies.iter().map(|r| pinned_core::untrusted::frame("a Slack reply in this thread", r)).collect();
        assert_eq!(prompt, &format!("Fix the footer.\n\nReplies the thread got while this run waited to start:\n\n5 earlier replies not shown\n\n{}", framed.join("\n\n")));
    }

    #[test]
    fn a_waiting_run_without_replies_keeps_its_prompt() {
        let Event::Run(run) = with_replies(slack_run("Fix the footer."), &[], 0) else { panic!("a run") };
        assert_eq!(run.args.last().unwrap(), "Fix the footer.");
    }
}
