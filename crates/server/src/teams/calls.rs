//! Runs and their calls to people: asking, answering, escalating, taking over.
//!
//! A session in a team's box asks people with `ask_people`, which answers at once with the call's
//! id: a shell command cannot wait hours. The answer, or the give-up, reaches the session later as
//! a notice typed into its tab (`tell_session`), the path a decided confirmation takes.
//!
//! The guards, each one SQL statement so two people at once cannot both win:
//! - a run waits on at most one call: only a run that works moves to `waiting` (`db::start_waiting`);
//! - the first answer wins: an answer, a stop and the give-up all close a call only while
//!   `answered_at is null` (`db::answer_call`, `db::stop_open_call`, `db::give_up_due`);
//! - one controller at a time: `db::take_over` only while `controller is null`.

use super::{db, push, Scope};
use crate::boxes::End;
use crate::hub::{Account, Hub};
use pinned_core::teams::{Answer, AnswerKind, Call, Role, Run, RunState, Situation, TeamEvent, TeamPolicy, Who};
use pinned_core::wire::{Call as MachineCall, TabView};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// How long the hub waits on a box to queue a notice, and when it tries again after a box that
/// did not take it (offline, or its tab not back yet). A repeat of the notice's id queues nothing.
const NOTICE_WAIT: Duration = Duration::from_secs(20);
const NOTICE_TRIES: [Duration; 4] = [Duration::ZERO, Duration::from_secs(3), Duration::from_secs(10), Duration::from_secs(30)];

/// The most runs `runs` lists.
const RUNS_LISTED: i64 = 50;

/// What `ask_people` answers at once: the call's id and who it asks, by handle or "the owner",
/// never an email.
///
/// `{"call":"6f1c…","asked":["@design"]}`
#[derive(Serialize, Debug, PartialEq)]
pub struct Asked {
    pub call: String,
    pub asked: Vec<String>,
}

/// The team's runs, newest first (at most 50). Any member reads them.
pub async fn runs(hub: &Arc<Hub>, scope: &Scope) -> Result<Vec<Run>, String> {
    db::runs(&hub.db, scope.team.id, RUNS_LISTED).await
}

/// The team's open calls, then the last answered. Any member reads them.
pub async fn calls(hub: &Arc<Hub>, scope: &Scope) -> Result<Vec<Call>, String> {
    db::calls(&hub.db, scope.team.id).await
}

/// A member asked by the call (a `Who::Person` naming it, a member of a `Who::Group` now, or the
/// group's fallback while it has no member) or the team's owner answers it; the first answer
/// wins and any later one is refused with the first. The hub sets `answer.by` and `answer.at`.
/// `Choice` (one of the call's choices, when it has some) types the choice and text into the
/// session; `PassTo` closes the call and opens one to group `handle` on the same run; `TakeOver`
/// makes the member the run's controller and tells the session; `Stop` is `stop_run`. Pushes
/// `TeamEvent::CallResolved` and `RunChanged`, and answers the call as it is now.
pub async fn answer_call(hub: &Arc<Hub>, scope: &Scope, id: &str, mut answer: Answer) -> Result<Call, String> {
    let team = scope.team.id;
    let id = parse_id(id, "call")?;
    let row = db::call(&hub.db, team, id).await?.ok_or("the team has no such call")?;
    if row.closed() {
        return Err(already_answered(hub, &row).await);
    }
    let call = row.call;
    if !asks(hub, scope, &call.asked).await? {
        return Err("this call is not for you".into());
    }
    let run_id = parse_id(&call.run, "run")?;
    let run = db::run(&hub.db, team, run_id).await?.ok_or("the call's run is gone")?;
    let outcome = match &mut answer.kind {
        AnswerKind::Choice => {
            answer.choice = answer.choice.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(str::to_owned);
            match &answer.choice {
                Some(choice) if !call.choices.is_empty() && !call.choices.contains(choice) => return Err(format!("choose one of: {}", call.choices.join(", "))),
                None if !call.choices.is_empty() => return Err(format!("choose one of: {}", call.choices.join(", "))),
                None if answer.text.trim().is_empty() => return Err("say what the bot should do".into()),
                _ => "answered",
            }
        }
        AnswerKind::PassTo { handle } => {
            *handle = handle.trim().trim_start_matches('@').to_owned();
            if !db::group_exists(&hub.db, team, handle).await? {
                return Err(format!("the team has no group @{handle}"));
            }
            "passed"
        }
        AnswerKind::TakeOver => {
            if let Some(other) = run.controller.as_deref().filter(|c| *c != scope.member.to_string()) {
                return Err(format!("{} already took over", name_of(hub, other).await));
            }
            "taken-over"
        }
        AnswerKind::Stop => "stopped",
    };
    answer.by = scope.member.to_string();
    answer.at = now();
    answer.text = answer.text.trim().to_owned();
    let Some(answered) = db::answer_call(&hub.db, team, id, &answer, outcome).await? else {
        let row = db::call(&hub.db, team, id).await?.ok_or("the team has no such call")?;
        return Err(already_answered(hub, &row).await);
    };
    push_logged(hub, team, TeamEvent::CallResolved { call: answered.clone() }).await;
    let who = name_of(hub, &answer.by).await;
    match &answer.kind {
        AnswerKind::Choice => {
            let said = [answer.choice.as_deref().unwrap_or_default(), answer.text.as_str()].join(" ").trim().to_owned();
            back_to_work(hub, &scope.team, run_id).await?;
            tell_session(hub, &scope.team, run_id, &format!("call-{id}"), format!("[Answer from {who} on call {id}: {said}]")).await;
        }
        AnswerKind::PassTo { handle } => {
            let next = Call { id: Uuid::new_v4().to_string(), asked: vec![Who::Group { handle: handle.clone() }], escalated: false, answer: None, ..answered.clone() };
            match db::insert_call(&hub.db, team, &next).await {
                Ok(next) => push_logged(hub, team, TeamEvent::CallOpened { call: next }).await,
                Err(e) => {
                    // The run cannot wait on a call that was never opened.
                    back_to_work(hub, &scope.team, run_id).await?;
                    return Err(format!("the call was closed but @{handle} could not be asked: {e}"));
                }
            }
        }
        AnswerKind::TakeOver => {
            if db::take_over(&hub.db, team, run_id, scope.member).await?.is_none() && run.controller.is_none() {
                eprintln!("pinned-server: run {run_id} of team {team} changed while {} took it over", scope.member);
            }
            back_to_work(hub, &scope.team, run_id).await?;
            let said = if answer.text.is_empty() { String::new() } else { format!(": {}", answer.text) };
            tell_session(hub, &scope.team, run_id, &format!("call-{id}"), format!("[{who} took over the run on call {id}{said}]")).await;
        }
        AnswerKind::Stop => {
            end_run(hub, &scope.team, run_id, RunState::Stopped, &format!("stopped by {who}"), End::Removed).await?;
        }
    }
    let row = db::call(&hub.db, team, id).await?.ok_or("the team has no such call")?;
    Ok(row.call)
}

/// Someone who may steer the run (`may_steer`) takes over a run that has not ended:
/// `RunState::TakenOver` with the member as `controller`, or still `Waiting` while its call is
/// open, which its answer moves on. One controller at a time. Pushes `TeamEvent::RunChanged`.
pub async fn take_over(hub: &Arc<Hub>, scope: &Scope, run: &str) -> Result<Run, String> {
    let (run_id, run) = live_run(hub, scope, run).await?;
    may_steer(hub, scope, run_id, &run).await?;
    match db::take_over(&hub.db, scope.team.id, run_id, scope.member).await? {
        Some(run) => {
            push_logged(hub, scope.team.id, TeamEvent::RunChanged { run: run.clone() }).await;
            Ok(run)
        }
        None => {
            let run = db::run(&hub.db, scope.team.id, run_id).await?.ok_or("the team has no such run")?;
            match run.controller {
                Some(controller) if !run.state.ended() => Err(format!("{} already took over", name_of(hub, &controller).await)),
                _ => Err("this run has ended".into()),
            }
        }
    }
}

/// The run's controller (or the team's owner) gives it back to its bot: `RunState::Working`, or
/// still `Waiting` while its call is open. Types `[<who> gave the run back: continue]` into the
/// session and pushes `TeamEvent::RunChanged`.
pub async fn give_back(hub: &Arc<Hub>, scope: &Scope, run: &str) -> Result<Run, String> {
    let (run_id, run) = live_run(hub, scope, run).await?;
    let Some(controller) = run.controller.as_deref() else { return Err("nobody has taken this run over".into()) };
    if controller != scope.member.to_string() && scope.role != Role::Owner {
        return Err(format!("only {}, who took it over, or the team's owner gives it back", name_of(hub, controller).await));
    }
    let run = db::give_back(&hub.db, scope.team.id, run_id).await?.ok_or("nobody has taken this run over")?;
    push_logged(hub, scope.team.id, TeamEvent::RunChanged { run: run.clone() }).await;
    let who = name_of(hub, &scope.member.to_string()).await;
    tell_session(hub, &scope.team, run_id, &format!("give-back-{run_id}-{}", Uuid::new_v4()), format!("[{who} gave the run back: continue]")).await;
    Ok(run)
}

/// Someone who may steer the run (`may_steer`) stops a run that has not ended: `RunState::Stopped`,
/// its open call closed as stopped, and its box ended. Pushes the changes.
pub async fn stop_run(hub: &Arc<Hub>, scope: &Scope, run: &str) -> Result<Run, String> {
    let (run_id, run) = live_run(hub, scope, run).await?;
    may_steer(hub, scope, run_id, &run).await?;
    let stop = Answer { by: scope.member.to_string(), choice: None, text: String::new(), kind: AnswerKind::Stop, at: now() };
    if let Some(call) = db::stop_open_call(&hub.db, scope.team.id, run_id, &stop).await? {
        push_logged(hub, scope.team.id, TeamEvent::CallResolved { call }).await;
    }
    let who = name_of(hub, &stop.by).await;
    end_run(hub, &scope.team, run_id, RunState::Stopped, &format!("stopped by {who}"), End::Removed).await
}

/// From machine `machine` of a team's own account: the session of the team's live run in that box
/// (`db::run_on_machine`) asks the people its bot's `CallRules` name for `situation` (or the
/// run's controller, while someone took it over). The session never names people. Refused for a
/// person's machine, a team machine that is no box, a box with no live run, and a run that already
/// waits on a call. Opens the call, sets the run `Waiting`, pushes `TeamEvent::CallOpened` and
/// `RunChanged`, and answers at once with the call's id and who it asks; the answer reaches the
/// session later, typed into its tab.
pub async fn ask_people(hub: &Arc<Hub>, team: &Arc<Account>, machine: &str, situation: Situation, question: &str, choices: Vec<String>) -> Result<Asked, String> {
    const NOT_A_TEAM_BOX: &str = "only a session in one of a team's boxes asks people";
    if !db::is_team(&hub.db, team.id).await? {
        return Err(NOT_A_TEAM_BOX.into());
    }
    let machine_id = Uuid::parse_str(machine).map_err(|_| NOT_A_TEAM_BOX)?;
    if hub.db.box_of(team.id, machine_id).await?.is_none() {
        return Err(NOT_A_TEAM_BOX.into());
    }
    let run = db::run_on_machine(&hub.db, team.id, machine).await?.ok_or("no run of the team works in this box")?;
    if run.state == RunState::Waiting {
        return Err("this run already waits on a call".into());
    }
    let question = question.trim();
    if question.is_empty() {
        return Err("say what you ask".into());
    }
    let choices: Vec<String> = choices.iter().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect();
    let run_id = parse_id(&run.id, "run")?;
    let asked = match &run.controller {
        Some(controller) => vec![Who::Person { account: controller.clone() }],
        None => {
            let rules = db::bot_calls(&hub.db, team.id, &run.workflow).await?.map(|(rules, _)| rules).unwrap_or_default();
            vec![rules.who(situation).clone()]
        }
    };
    let Some(waiting) = db::start_waiting(&hub.db, team.id, run_id).await? else {
        let now = db::run(&hub.db, team.id, run_id).await?;
        return Err(match now.map(|r| r.state) {
            Some(RunState::Waiting) => "this run already waits on a call".into(),
            _ => "this run has ended".into(),
        });
    };
    let call = Call {
        id: Uuid::new_v4().to_string(),
        run: run.id.clone(),
        workflow: run.workflow.clone(),
        situation,
        asked,
        question: question.to_owned(),
        choices,
        created_at: 0,
        escalated: false,
        answer: None,
    };
    let call = match db::insert_call(&hub.db, team.id, &call).await {
        Ok(call) => call,
        Err(e) => {
            db::stop_waiting(&hub.db, team.id, run_id).await?;
            return Err(e);
        }
    };
    push_logged(hub, team.id, TeamEvent::CallOpened { call: call.clone() }).await;
    push_logged(hub, team.id, TeamEvent::RunChanged { run: waiting }).await;
    let mut shown = Vec::with_capacity(call.asked.len());
    for who in &call.asked {
        shown.push(shown_who(hub, who).await);
    }
    Ok(Asked { call: call.id, asked: shown })
}

/// Runs every few seconds (`super::spawn_tick`), across every team. A call open longer than
/// `policy.escalate_after_secs` goes to the owner too (escalated, `TeamEvent::CallOpened` again).
/// One open longer than `policy.give_up_after_secs` closes as gave-up, with the same guard as an
/// answer, so only one of them wins; its run ends as `RunState::GaveUp`, its session is told to
/// end without the step, and its box ends. Run limits are not checked here.
pub async fn tick(hub: &Arc<Hub>, policy: &TeamPolicy) -> Result<(), String> {
    for row in db::escalate_due(&hub.db, policy.escalate_after_secs).await? {
        push_logged(hub, row.team, TeamEvent::CallOpened { call: row.call }).await;
    }
    for row in db::give_up_due(&hub.db, policy.give_up_after_secs).await? {
        let (team, call) = (row.team, row.call);
        push_logged(hub, team, TeamEvent::CallResolved { call: call.clone() }).await;
        let hub = hub.clone();
        // Each on its own, so one box slow to take its notice or to end holds up no other.
        tokio::spawn(async move {
            if let Err(e) = gave_up(&hub, team, &call).await {
                eprintln!("pinned-server: run {} of team {team} gave up but did not end cleanly: {e}", call.run);
            }
        });
    }
    Ok(())
}

/// Ends the run of a call nobody answered: the session hears it first, then its box ends.
async fn gave_up(hub: &Arc<Hub>, team: Uuid, call: &Call) -> Result<(), String> {
    let account = hub.account(team).await?;
    let run_id = parse_id(&call.run, "run")?;
    let text = format!("[Nobody answered call {} in time: end without doing the step]", call.id);
    tell_session_now(hub, &account, run_id, &format!("call-{}", call.id), text).await;
    end_run(hub, &account, run_id, RunState::GaveUp, &format!("nobody answered call {}", call.id), End::Expired).await.map(drop)
}

/// Moves the run to the ended `state`, pushes it, and ends its box for `why`. A box that does not
/// end now is logged: the reaper still ends it at its max life.
async fn end_run(hub: &Arc<Hub>, team: &Arc<Account>, run_id: Uuid, state: RunState, outcome: &str, why: End) -> Result<Run, String> {
    let current = db::run(&hub.db, team.id, run_id).await?.ok_or("the team has no such run")?;
    let controller = current.controller.as_deref().and_then(|c| Uuid::parse_str(c).ok());
    if !db::set_run_state(&hub.db, team.id, run_id, state, controller, Some(outcome)).await? {
        return Err("this run has ended".into());
    }
    let run = db::run(&hub.db, team.id, run_id).await?.ok_or("the team has no such run")?;
    push_logged(hub, team.id, TeamEvent::RunChanged { run: run.clone() }).await;
    if let Some(machine) = db::run_box(&hub.db, team.id, run_id).await?.and_then(|m| Uuid::parse_str(&m).ok()) {
        if let Err(e) = hub.boxes.end_machine(hub, team, machine, why).await {
            eprintln!("pinned-server: the box of run {run_id} of team {} did not end yet: {e}", team.id);
        }
    }
    Ok(run)
}

/// Moves a waiting run back to work and pushes it; a run that no longer waits stays as it is.
async fn back_to_work(hub: &Arc<Hub>, team: &Arc<Account>, run_id: Uuid) -> Result<(), String> {
    if let Some(run) = db::stop_waiting(&hub.db, team.id, run_id).await? {
        push_logged(hub, team.id, TeamEvent::RunChanged { run }).await;
    }
    Ok(())
}

/// The run, while it has not ended.
async fn live_run(hub: &Arc<Hub>, scope: &Scope, run: &str) -> Result<(Uuid, Run), String> {
    let run_id = parse_id(run, "run")?;
    let run = db::run(&hub.db, scope.team.id, run_id).await?.ok_or("the team has no such run")?;
    if run.state.ended() {
        return Err("this run has ended".into());
    }
    Ok((run_id, run))
}

/// Who takes over or stops a run: the team's owner, its bot's owner, and whoever its open call asks.
async fn may_steer(hub: &Arc<Hub>, scope: &Scope, run_id: Uuid, run: &Run) -> Result<(), String> {
    if scope.role == Role::Owner {
        return Ok(());
    }
    if let Some((_, Some(owner))) = db::bot_calls(&hub.db, scope.team.id, &run.workflow).await? {
        if owner == scope.member {
            return Ok(());
        }
    }
    if let Some(call) = db::open_call(&hub.db, scope.team.id, run_id).await? {
        if asks(hub, scope, &call.asked).await? {
            return Ok(());
        }
    }
    Err("only the people its call asks, its bot's owner or the team's owner do this".into())
}

/// Whether `asked` takes in the member now: the team's owner always, a `Who::Person` naming it,
/// or a `Who::Group` it is in (or whose fallback it is while the group has no member).
async fn asks(hub: &Arc<Hub>, scope: &Scope, asked: &[Who]) -> Result<bool, String> {
    if scope.role == Role::Owner {
        return Ok(true);
    }
    for who in asked {
        let hit = match who {
            Who::Person { account } => *account == scope.member.to_string(),
            Who::Group { handle } => db::asks_member(&hub.db, scope.team.id, handle, scope.member).await?,
            Who::Owner => false,
        };
        if hit {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `<who> already answered: <their answer>`, for an answer that came second.
async fn already_answered(hub: &Arc<Hub>, row: &db::CallRow) -> String {
    let Some(answer) = &row.call.answer else {
        return match row.outcome.as_deref() {
            Some("gave-up") => "nobody answered this call in time: its run gave up".into(),
            _ => "this call is closed".into(),
        };
    };
    let said = match &answer.kind {
        AnswerKind::Choice => [answer.choice.as_deref().unwrap_or_default(), answer.text.as_str()].join(" ").trim().to_owned(),
        AnswerKind::PassTo { handle } => format!("passed it to @{handle}"),
        AnswerKind::TakeOver => "took over the run".into(),
        AnswerKind::Stop => "stopped the run".into(),
    };
    format!("{} already answered: {said}", name_of(hub, &answer.by).await)
}

/// A member as others see them: the local part of their email, never the whole address.
async fn name_of(hub: &Arc<Hub>, account: &str) -> String {
    let email = match Uuid::parse_str(account) {
        Ok(id) => db::email(&hub.db, id).await.ok().flatten(),
        Err(_) => None,
    };
    email.and_then(|e| e.split('@').next().map(str::to_owned)).filter(|n| !n.is_empty()).unwrap_or_else(|| "a former member".into())
}

/// Who a call asks, as the session hears it: `@design`, `the owner`, or a member's name.
async fn shown_who(hub: &Arc<Hub>, who: &Who) -> String {
    match who {
        Who::Person { account } => name_of(hub, account).await,
        who => who.to_string(),
    }
}

/// Types `text` into the session of the run's box, in the background, trying again while the box
/// does not take it. `id` makes a repeat queue nothing.
async fn tell_session(hub: &Arc<Hub>, team: &Arc<Account>, run_id: Uuid, id: &str, text: String) {
    let (hub, team, id) = (hub.clone(), team.clone(), id.to_owned());
    tokio::spawn(async move { tell_session_now(&hub, &team, run_id, &id, text).await });
}

/// Types `text` into the session of the run's box as `Call::Notice`, the queue a decided
/// confirmation's notice takes, typed once the session is idle. Logs when it never could.
async fn tell_session_now(hub: &Arc<Hub>, team: &Arc<Account>, run_id: Uuid, id: &str, text: String) {
    let place = async {
        let machine = db::run_box(&hub.db, team.id, run_id).await?.ok_or("the run has no box")?;
        let run = db::run(&hub.db, team.id, run_id).await?.ok_or("the run is gone")?;
        Ok::<_, String>((machine, run.workflow))
    };
    let (machine, workflow) = match place.await {
        Ok(found) => found,
        Err(e) => return eprintln!("pinned-server: run {run_id} of team {} was not told of {id}: {e}", team.id),
    };
    let mut last = String::from("its box had no session");
    for wait in NOTICE_TRIES {
        tokio::time::sleep(wait).await;
        let Some(tab) = session_tab(team, &machine, &workflow) else { continue };
        let call = MachineCall::Notice { tab: tab.id.clone(), session: tab.session.clone(), text: text.clone(), id: id.to_owned() };
        match hub.call_machine(team, &machine, call, NOTICE_WAIT).await {
            Ok(Value::Bool(true)) => return,
            Ok(_) => last = "its box had no such tab".into(),
            Err(e) => last = String::from(e),
        }
    }
    eprintln!("pinned-server: run {run_id} of team {} was not told of {id}: {last}", team.id);
}

/// The run's session in its box: the tab of its workflow, else the box's only tab.
pub(super) fn session_tab(team: &Account, machine: &str, workflow: &str) -> Option<TabView> {
    let tabs = team.tabs_of(machine).unwrap_or_default();
    let found = tabs.iter().find(|t| t.connector.as_deref() == Some(workflow)).or(match tabs.as_slice() {
        [only] => Some(only),
        _ => None,
    });
    found.cloned()
}

async fn push_logged(hub: &Arc<Hub>, team: Uuid, event: TeamEvent) {
    if let Err(e) = push(hub, team, event).await {
        eprintln!("pinned-server: team {team}'s members did not hear of a change: {e}");
    }
}

fn parse_id(id: &str, what: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id.trim()).map_err(|_| format!("the team has no such {what}"))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}
