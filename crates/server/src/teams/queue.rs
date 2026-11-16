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
    /// (`admit`): when its turn comes it is granted in its row (`db::grant`), and its team's
    /// poller claims it (`claim`) and runs it whole, from the start.
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
                work: pinned_core::connectors::Run { cwd, repo, args, connector: workflow.to_owned(), subject, mentioned, account, runs_on, input, reservation: None },
            }),
            Stored::Event { .. } => None,
        }
    }
}

/// One turn at a time per team, so two events never both take the last free place: in this
/// hub, and across the hubs on one database by the team's advisory lock (`Turn`).
fn line(team: Uuid) -> Arc<tokio::sync::Mutex<()>> {
    static LINES: LazyLock<Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Mutex::default);
    LINES.lock().unwrap().entry(team).or_default().clone()
}

/// The key of team `team`'s advisory lock: what `Turn` holds while it decides.
pub const TURN_KEY: &str = "hashtextextended('pinned-team-runs:' || $1::uuid::text, 0)";

/// A team's turn to decide what runs: this hub's line, and a transaction on a connection of its
/// own holding the team's `pg_advisory_xact_lock`, so another hub on the database waits too.
/// `end` commits it; a turn dropped on an error closes its connection, which frees the lock.
/// Both waits together are bounded by `TURN_WAIT`: a turn not had by then (this hub's line, then
/// the advisory lock for what is left) is refused as `crate::db::BUSY`, never retried.
/// A turn only decides and reserves: nothing outside pinned is called while one is held.
struct Turn {
    _line: tokio::sync::OwnedMutexGuard<()>,
    held: tokio_postgres::Client,
}

/// How long a task waits for this hub's line of a team, as long as Postgres's lock timeout.
const TURN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

impl Turn {
    async fn take(hub: &Hub, team: Uuid) -> Result<Turn, String> {
        // The connection opens before the line is taken, so a slow connect never holds the line.
        let held = hub.db.own_client().await?;
        // One deadline for both waits: a task that got the line late waits on the advisory lock
        // only for what is left, so the turns queued behind another hub's are all refused in time.
        let deadline = tokio::time::Instant::now() + TURN_WAIT;
        let line = tokio::time::timeout_at(deadline, line(team).lock_owned()).await.map_err(|_| crate::db::BUSY.to_string())?;
        let left = deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis().max(1);
        held.batch_execute(&format!("begin; set local lock_timeout = {left}")).await.map_err(crate::db::err)?;
        held.execute(&format!("select pg_advisory_xact_lock({TURN_KEY})"), &[&team]).await.map_err(crate::db::err)?;
        // The turn's own statements wait on locks as every statement of the hub's does.
        held.batch_execute(&crate::db::lock_limit()).await.map_err(crate::db::err)?;
        Ok(Turn { _line: line, held })
    }

    async fn end(self) -> Result<(), String> {
        self.held.batch_execute("commit").await.map_err(crate::db::err)
    }
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

/// Starts a run of bot `workflow` when the bot has no live run, nothing waits in its line, and
/// the team has a free place; otherwise it waits at the end of the bot's line. Refused, as
/// `bots::start_reserved` refuses, when the bot cannot run at all. The team's turn only decides
/// and reserves the run's place: its box starts once the turn is over (`bots::start_reserved`).
pub async fn ask(hub: &Arc<Hub>, team: &Arc<Account>, workflow: &str, start: Start) -> Result<Asked, String> {
    let turn = Turn::take(hub, team.id).await?;
    let decided = ask_in_turn(hub, &turn, team, workflow, start).await;
    turn.end().await?;
    let (run, work) = match decided? {
        Ok(reserved) => reserved,
        Err(place) => return Ok(Asked::Waiting { place }),
    };
    let id = Uuid::parse_str(&run.id).map_err(|e| e.to_string())?;
    let placed = bots::start_reserved(hub, team, id, work).await?;
    Ok(Asked::Started(run, placed))
}

/// `ask`'s decision, inside the team's turn: the reserved run and what it opens, or its place in line.
async fn ask_in_turn(hub: &Arc<Hub>, turn: &Turn, team: &Arc<Account>, workflow: &str, start: Start) -> Result<Result<(Run, pinned_core::connectors::Run), i64>, String> {
    bots::ready(hub, team, workflow).await?;
    let waiting = db::waiting_count(&hub.db, team.id, workflow).await?;
    let busy = !db::live_runs_of(&hub.db, team.id, workflow).await?.is_empty();
    let full = db::live_count(&hub.db, team.id).await? >= db::max_live_runs(&hub.db, team.id).await?;
    if waiting > 0 || busy || full {
        let stored = serde_json::to_value(Stored::of(&start)).map_err(|e| e.to_string())?;
        let dropped = db::enqueue(&turn.held, team.id, workflow, &stored, MOST_WAITING).await?;
        if dropped > 0 {
            eprintln!("pinned-server: team {} bot {workflow}: its line is full, so its oldest waiting event was dropped", team.id);
            db::count_skipped_on(&turn.held, team.id, workflow, SkipReason::QueueFull, dropped).await?;
        }
        return Ok(Err((waiting + 1).min(MOST_WAITING)));
    }
    let (work, started_by) = bots::work_of(&*hub.connectors(team).await?, workflow, start)?;
    let run = reserve(turn, team, workflow, started_by, &work.subject).await?;
    Ok(Ok((run, work)))
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
    let turn = Turn::take(hub, team.id).await?;
    let admitted = decide(hub, &turn, team, kind, event, &workflow).await;
    turn.end().await?;
    admitted
}

/// `admit`'s decision, inside the team's turn.
async fn decide(hub: &Arc<Hub>, turn: &Turn, team: &Arc<Account>, kind: &str, event: Event, workflow: &str) -> Result<Admitted, String> {
    let workflow = workflow.to_owned();
    bots::ready(hub, team, &workflow).await?;
    let waiting = db::waiting_count(&hub.db, team.id, &workflow).await?;
    let busy = !db::live_runs_of(&hub.db, team.id, &workflow).await?.is_empty();
    let full = db::live_count(&hub.db, team.id).await? >= db::max_live_runs(&hub.db, team.id).await?;
    if waiting > 0 || busy || full {
        let stored = serde_json::to_value(Stored::Event { kind: kind.to_owned(), event, replies: vec![], earlier: 0 }).map_err(|e| e.to_string())?;
        let dropped = db::enqueue(&turn.held, team.id, &workflow, &stored, MOST_WAITING).await?;
        if dropped > 0 {
            eprintln!("pinned-server: team {} bot {workflow}: its line is full, so its oldest waiting event was dropped", team.id);
            db::count_skipped_on(&turn.held, team.id, &workflow, SkipReason::QueueFull, dropped).await?;
        }
        return Ok(Admitted::Waiting);
    }
    let run = reserve(turn, team, &workflow, format!("trigger:{kind}"), event.subject()).await?;
    Ok(Admitted::Reserved(run.id))
}

/// Holds a place for a run of bot `workflow` on `subject` in the team's turn: a live run with no
/// session yet, which the team's cap counts. Answers the run.
async fn reserve(turn: &Turn, team: &Arc<Account>, workflow: &str, started_by: String, subject: &Subject) -> Result<Run, String> {
    let run = Run {
        id: Uuid::new_v4().to_string(),
        workflow: workflow.to_owned(),
        state: RunState::Working,
        controller: None,
        cost_usd: 0.0,
        active_secs: 0, waiting_secs: 0,
        started_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64),
        ended_at: None,
        outcome: None,
        started_by,
    };
    db::insert_reserved(&turn.held, team.id, &run, subject).await?;
    Ok(run)
}

/// New replies of a Slack thread: kept with the thread's waiting event, the newest
/// `MOST_REPLIES`; passed on when the bot runs the thread; dropped when nothing does. A thread
/// whose event was granted its place and is not claimed yet has a live (reserved) run, but its
/// replies are kept with the event, as `Replies::Kept`: no session opens for them by the
/// delivery's fallback, which would take the event's place, and the event runs once, with them,
/// when its poller claims it (`claim`). A reply to a claimed run whose session has not opened is
/// held for that run (`for_live`), and never starts a run of its own.
pub async fn keep_replies(hub: &Arc<Hub>, team: &Arc<Account>, delivery: &Delivery) -> Result<Replies, String> {
    let subject = Subject::SlackThread { channel: delivery.channel.clone(), thread: delivery.thread.clone() };
    let live = db::live_run_for(&hub.db, team.id, &delivery.connector, &subject).await?;
    let live = live.map(|run| Uuid::parse_str(&run.id).map_err(|e| e.to_string())).transpose()?;
    if let Some(run) = live {
        if db::waiting_event_for(&hub.db, team.id, &delivery.connector, &subject).await?.is_none() {
            return for_live(hub, team, run).await;
        }
    }
    let turn = Turn::take(hub, team.id).await?;
    let kept = keep_in_turn(hub, &turn, team, delivery, &subject, live).await;
    turn.end().await?;
    kept
}

/// Replies for the thread's live run `run`: passed on to its session once it opened. A run still
/// reserved was claimed (or admitted) and its box is still opening its session, so they are held
/// for it (`Replies::Later`): nothing takes them now, the thread's cursor stays, and the next check
/// gives them to that session, once. Passed on now, the delivery's fallback would start the run's
/// place a second time.
async fn for_live(hub: &Arc<Hub>, team: &Arc<Account>, run: Uuid) -> Result<Replies, String> {
    Ok(match db::is_reserved(&hub.db, team.id, run).await? {
        true => Replies::Later,
        false => Replies::Pass,
    })
}

/// `keep_replies` in the team's turn; `live` is the thread's live run. A granted event a poller
/// claims meanwhile is gone from its line, and its run takes the replies (`for_live`).
async fn keep_in_turn(hub: &Arc<Hub>, turn: &Turn, team: &Arc<Account>, delivery: &Delivery, subject: &Subject, live: Option<Uuid>) -> Result<Replies, String> {
    let subject = subject.clone();
    let Some(waiting) = db::waiting_event_for(&hub.db, team.id, &delivery.connector, &subject).await? else {
        if let Some(run) = live {
            return for_live(hub, team, run).await;
        }
        // Replies held for a run (`for_live`) whose place then ended before its session opened
        // have no run to reach: they are dropped once, logged and counted on the bot.
        if db::last_run_never_opened(&hub.db, team.id, &delivery.connector, &subject).await? {
            eprintln!("pinned-server: team {} workflow {}: {} new thread messages dropped: the thread's run ended before its session opened", team.id, delivery.connector, delivery.messages.len());
            db::count_skipped_on(&turn.held, team.id, &delivery.connector, SkipReason::RepliesDropped, delivery.messages.len() as u64).await?;
            return Ok(Replies::Dropped);
        }
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
    if !db::set_waiting(&turn.held, waiting.id, &stored).await? {
        return match live {
            Some(run) => for_live(hub, team, run).await,
            None => Ok(Replies::Dropped),
        };
    }
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
    text.extend(replies.iter().map(|reply| pinned_core::untrusted::frame(pinned_core::untrusted::SLACK_REPLY, reply)));
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

/// The team's poller claims granted event `id` (`db::grant`) right before it runs it from the
/// start with its place already held: only the claim that deleted the row gets the event, so of
/// two hubs exactly one runs it. A hub that stops before the claim leaves the row granted, and the
/// next hub's poller runs it once. A hub that stops after the claim and before the event's session
/// opens does not run it again: its reserved run (`reserved_at`) is left, and ended by the hub's
/// backstop for reservations that never opened.
/// Answers the event with the reserved run whose place it now holds, the only place it starts in.
pub async fn claim(hub: &Arc<Hub>, team: &Arc<Account>, id: i64) -> Result<Option<(Event, Option<String>)>, String> {
    let Some((start, run)) = db::claim(&hub.db, team.id, id).await? else { return Ok(None) };
    match serde_json::from_value(start) {
        Ok(Stored::Event { event, replies, earlier, .. }) => Ok(Some((with_replies(event, &replies, earlier), run.map(|r| r.to_string())))),
        _ => {
            eprintln!("pinned-server: team {}: a granted event was not readable and is dropped", team.id);
            Ok(None)
        }
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
    let turn = Turn::take(hub, team.id).await?;
    let mut taken = Taken::default();
    let decided = take_and_start(hub, &turn, team, ended, &mut taken).await;
    turn.end().await?;
    // What the turn reserved and committed starts even when a step after a reservation failed:
    // left, a reserved run would hold its place until the backstop ends it as never started.
    // A grant is in Postgres once the turn committed: the team's poller looks now.
    if taken.granted {
        match hub.connectors(team).await {
            Ok(connectors) => connectors.wake(),
            Err(e) => eprintln!("pinned-server: team {}: a granted event waits for its poller's next check: {e}", team.id),
        }
    }
    // Each reserved run's box starts outside the turn, on its own, so one slow boot holds up no other.
    for (id, work, told) in taken.starts {
        // A turn whose statement failed rolled back with it: only a run still reserved starts.
        match db::is_reserved(&hub.db, team.id, id).await {
            Ok(false) => continue,
            Ok(true) => {}
            Err(e) => eprintln!("pinned-server: team {}: reserved run {id} starts unchecked: {e}", team.id),
        }
        let (hub, team) = (hub.clone(), team.clone());
        tokio::spawn(async move {
            let workflow = work.connector.clone();
            match bots::start_reserved(&hub, &team, id, work).await {
                // Its trigger's `open` was answered as consumed, so Linear and Slack hear of it only now.
                Ok(_) => {
                    if let Some(work) = told {
                        let told = async { hub.connectors(&team).await?.started(&work).await };
                        if let Err(e) = told.await {
                            eprintln!("pinned-server: team {} bot {}: its waiting run started, but Linear or Slack was not told: {e}", team.id, work.connector);
                        }
                    }
                }
                // Counted as `SkipReason::StartFailed` by `start_reserved`, unless it was stopped.
                Err(e) => eprintln!("pinned-server: team {} bot {workflow}: a waiting run did not start and is dropped: {e}", team.id),
            }
        });
    }
    decided
}

/// What `take_and_start` decided in the team's turn: whether it granted an event a place, and the
/// reserved runs to start once the turn is over, each with the trigger's work to report as started.
#[derive(Default)]
struct Taken {
    granted: bool,
    starts: Vec<(Uuid, pinned_core::connectors::Run, Option<pinned_core::connectors::Run>)>,
}

/// `start_waiting`'s takes, inside the team's turn: each waiting event or run that has a free
/// place gets it reserved, and nothing outside pinned is called.
async fn take_and_start(hub: &Arc<Hub>, turn: &Turn, team: &Arc<Account>, ended: &str, taken: &mut Taken) -> Result<(), String> {
    loop {
        if db::live_count_on(&turn.held, team.id).await? >= db::max_live_runs(&hub.db, team.id).await? {
            return Ok(());
        }
        let Some(waiting) = db::next_waiting(&turn.held, team.id, ended).await? else { return Ok(()) };
        let stored: Stored = match serde_json::from_value(waiting.start) {
            Ok(stored) => stored,
            Err(e) => {
                eprintln!("pinned-server: team {} bot {}: a waiting event was not readable and is dropped: {e}", team.id, waiting.workflow);
                db::take_waiting(&turn.held, waiting.id).await?;
                continue;
            }
        };
        if let Stored::Event { kind, event, .. } = stored {
            if let Some(reason) = skip_reason(hub, team, &waiting.workflow, true).await? {
                eprintln!("pinned-server: team {} bot {}: a waiting event is dropped: the bot is {}", team.id, waiting.workflow, reason.as_str());
                db::take_waiting(&turn.held, waiting.id).await?;
                db::count_skipped_on(&turn.held, team.id, &waiting.workflow, reason, 1).await?;
                continue;
            }
            // Its place is held now and its row granted, in one transaction (the turn's): the
            // team's poller claims it and runs it whole, from the start.
            let run = reserve(turn, team, &waiting.workflow, format!("trigger:{kind}"), event.subject()).await?;
            db::grant(&turn.held, waiting.id, Uuid::parse_str(&run.id).map_err(|e| e.to_string())?).await?;
            taken.granted = true;
            continue;
        }
        // Every read on the shared connection comes before the take: one that fails (a busy read)
        // returns before the row is taken, so the run still waits, counted by nobody. Only what
        // the take is written with (`turn.held`: its counts and its reservation) rolls back with a
        // turn that fails after it, and the turn commits whatever this returns.
        let Some(start) = stored.start(&waiting.workflow) else {
            db::take_waiting(&turn.held, waiting.id).await?;
            continue;
        };
        let by_trigger = matches!(start, Start::Trigger { .. });
        let told = match &start {
            Start::Trigger { work, .. } => Some(work.clone()),
            Start::Member(_) => None,
        };
        let read = match skip_reason(hub, team, &waiting.workflow, by_trigger).await? {
            Some(reason) => Err(reason),
            None => Ok(match bots::ready(hub, team, &waiting.workflow).await {
                Ok(()) => bots::work_of(&*hub.connectors(team).await?, &waiting.workflow, start),
                // A busy read says nothing of the bot: the run still waits.
                Err(e) if e == crate::db::BUSY => return Err(e),
                Err(e) => Err(e),
            }),
        };
        db::take_waiting(&turn.held, waiting.id).await?;
        let reserved = match read {
            Ok(reserved) => reserved,
            Err(reason) => {
                eprintln!("pinned-server: team {} bot {}: a waiting event is dropped: the bot is {}", team.id, waiting.workflow, reason.as_str());
                db::count_skipped_on(&turn.held, team.id, &waiting.workflow, reason, 1).await?;
                continue;
            }
        };
        let (work, started_by) = match reserved {
            Ok(reserved) => reserved,
            Err(e) => {
                eprintln!("pinned-server: team {} bot {}: a waiting run did not start and is dropped: {e}", team.id, waiting.workflow);
                db::count_skipped_on(&turn.held, team.id, &waiting.workflow, SkipReason::StartFailed, 1).await?;
                continue;
            }
        };
        let run = reserve(turn, team, &waiting.workflow, started_by, &work.subject).await?;
        taken.starts.push((Uuid::parse_str(&run.id).map_err(|e| e.to_string())?, work, told));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slack_run(prompt: &str) -> Event {
        Event::Run(pinned_core::connectors::Run {
            reservation: None,
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
