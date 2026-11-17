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
use crate::hub::{Account, Hub, LastTurn};
use futures_util::future::BoxFuture;
use pinned_core::agent::Subject;
use pinned_core::teams::{Answer, AnswerKind, Call, Role, Run, RunState, Situation, SkipReason, TeamEvent, TeamPolicy, Who};
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

pub(crate) const RUN_ENDED: &str = "this run has ended";

/// Why a team's box got no session: its run was stopped, or ended otherwise, while the box booted.
pub const ENDED_WHILE_BOOTING: &str = "the run ended before its box started, so its box was ended";

/// What `ask_people` answers at once.
#[derive(Serialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum Asked {
    /// The call's id and who it asks, by handle or "the owner", never an email.
    ///
    /// `{"call":"6f1c…","asked":["@design"]}`
    Call { call: String, asked: Vec<String> },
    /// The step's "Go ahead" on call `call` is used: the step runs now, and nobody is asked.
    ///
    /// `{"granted":true,"call":"6f1c…"}`
    Granted { granted: bool, call: String },
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
/// session; `PassTo` closes the call and opens one to group `handle` on the same run, with the
/// first call's clocks, while the run still waits; `TakeOver`
/// makes the member the run's controller and tells the session; `Stop` is `stop_run`. Pushes
/// `TeamEvent::CallResolved` and `RunChanged`, and answers the call as it is now.
pub async fn answer_call(hub: &Arc<Hub>, scope: &Scope, id: &str, mut answer: Answer) -> Result<Call, String> {
    let team = scope.team.id;
    let id = parse_id(id, "call")?;
    let row = db::call(&hub.db, team, id).await?.ok_or("the team has no such call")?;
    if row.closed() {
        return Err(already_answered(hub, &row).await);
    }
    // A call passed on keeps the step its "Go ahead" lets run.
    let (call, step) = (row.call, row.step);
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
    // A limit call's "Let it go on" lets the run go on to its max life: the members read that.
    let limit_choice = (call.situation == Situation::OnLimit).then_some(answer.choice.as_deref()).flatten().filter(|c| [GO_ON, STOP_IT].contains(c)).map(str::to_owned);
    if limit_choice.as_deref() == Some(GO_ON) {
        let goes_on = format!("it can now run until the {} limit", life_limit(hub.team_policy.max_life_secs));
        answer.text = if answer.text.is_empty() { goes_on } else { format!("{goes_on}. {}", answer.text) };
    }
    let Some(answered) = db::answer_call(&hub.db, team, id, &answer, outcome).await? else {
        let row = db::call(&hub.db, team, id).await?.ok_or("the team has no such call")?;
        return Err(already_answered(hub, &row).await);
    };
    push_logged(hub, team, TeamEvent::CallResolved { call: answered.clone() }).await;
    let who = name_of(hub, &answer.by).await;
    match &answer.kind {
        AnswerKind::Choice if limit_choice.as_deref() == Some(STOP_IT) => {
            end_run(hub, &scope.team, run_id, RunState::Stopped, &format!("stopped by {who}"), Some(End::Removed)).await?;
        }
        AnswerKind::Choice => {
            let said = match limit_choice {
                Some(choice) => format!("{choice}: {}", answer.text),
                None => [answer.choice.as_deref().unwrap_or_default(), answer.text.as_str()].join(" ").trim().to_owned(),
            };
            back_to_work(hub, &scope.team, run_id).await?;
            tell_session(hub, &scope.team, run_id, &format!("call-{id}"), format!("[Answer from {who} on call {id}: {said}]")).await;
        }
        AnswerKind::PassTo { handle } => {
            let next = Call { id: Uuid::new_v4().to_string(), asked: vec![Who::Group { handle: handle.clone() }], escalated: false, answer: None, ..answered.clone() };
            match db::pass_call(&hub.db, team, &next, step.as_deref(), id).await {
                Ok(Some(next)) => {
                    push_logged(hub, team, TeamEvent::CallOpened { call: next.clone() }).await;
                    notify_slack(hub, &scope.team, &next);
                }
                // Stopped, or ended otherwise, since the answer: nobody is asked on a run that ended.
                Ok(None) => return Err(format!("the call was closed, but its run has ended, so @{handle} was not asked")),
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
            db::release_go_aheads(&hub.db, team, run_id, now()).await?;
            back_to_work(hub, &scope.team, run_id).await?;
            let said = if answer.text.is_empty() { String::new() } else { format!(": {}", answer.text) };
            tell_session(hub, &scope.team, run_id, &format!("call-{id}"), format!("[{who} took over the run on call {id}{said}]")).await;
        }
        AnswerKind::Stop => {
            end_run(hub, &scope.team, run_id, RunState::Stopped, &format!("stopped by {who}"), Some(End::Removed)).await?;
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
            // A "Go ahead" was given for the run as it was; the one who took over decides anew.
            db::release_go_aheads(&hub.db, scope.team.id, run_id, now()).await?;
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
    handed_back(hub, &scope.team, run_id, scope.member).await?.ok_or_else(|| "nobody has taken this run over".into())
}

/// Gives the live run back to its bot as `give_back` does, for `member`: pushes
/// `TeamEvent::RunChanged` and types `[<who> gave the run back: continue]` into the session.
/// None when nobody controlled it or it ended.
async fn handed_back(hub: &Arc<Hub>, team: &Arc<Account>, run_id: Uuid, member: Uuid) -> Result<Option<Run>, String> {
    let Some(run) = db::give_back(&hub.db, team.id, run_id).await? else { return Ok(None) };
    push_logged(hub, team.id, TeamEvent::RunChanged { run: run.clone() }).await;
    let who = name_of(hub, &member.to_string()).await;
    tell_session(hub, team, run_id, &format!("give-back-{run_id}-{}", Uuid::new_v4()), format!("[{who} gave the run back: continue]")).await;
    Ok(Some(run))
}

/// The open calls, not escalated yet, that ask `member` now: by name, or through a group it is in
/// or is the fallback of (`asks_member`). Read before `member` goes, for `member_gone`.
pub async fn calls_asking(hub: &Arc<Hub>, team: Uuid, member: Uuid) -> Result<Vec<Call>, String> {
    let mut asking = Vec::new();
    for call in db::calls(&hub.db, team).await? {
        if call.answer.is_some() || call.escalated {
            continue;
        }
        let mut hit = false;
        for who in &call.asked {
            hit = match who {
                Who::Person { account } => *account == member.to_string(),
                Who::Group { handle } => db::asks_member(&hub.db, team, handle, member).await?,
                Who::Owner => false,
            };
            if hit {
                break;
            }
        }
        if hit {
            asking.push(call);
        }
    }
    Ok(asking)
}

/// After `member` left the team or was removed: each live run it controlled goes back to its bot
/// at once (`handed_back`), and each call of `asking` (`calls_asking`, read before it went) that
/// now asks nobody goes to the team's owner at once, as `tick`'s escalation does. Agents are never
/// asked, so none is added. Logs what it could not do: the member is gone either way.
pub async fn member_gone(hub: &Arc<Hub>, team: &Arc<Account>, member: Uuid, asking: Vec<Call>) {
    let runs = match db::runs_controlled_by(&hub.db, team.id, member).await {
        Ok(runs) => runs,
        Err(e) => return eprintln!("pinned-server: the runs {member} controlled on team {} were not given back: {e}", team.id),
    };
    for run_id in runs {
        if let Err(e) = handed_back(hub, team, run_id, member).await {
            eprintln!("pinned-server: run {run_id} of team {} was not given back: {e}", team.id);
        }
    }
    for call in asking {
        let mut reached = false;
        for who in &call.asked {
            match db::reaches_anyone(&hub.db, team.id, who).await {
                Ok(false) => {}
                Ok(true) | Err(_) => {
                    reached = true;
                    break;
                }
            }
        }
        if reached {
            continue;
        }
        let Ok(id) = parse_id(&call.id, "call") else { continue };
        match db::escalate_now(&hub.db, team.id, id).await {
            Ok(Some(call)) => push_logged(hub, team.id, TeamEvent::CallOpened { call }).await,
            Ok(None) => {}
            Err(e) => eprintln!("pinned-server: call {id} of team {} did not go to its owner: {e}", team.id),
        }
    }
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
    end_run(hub, &scope.team, run_id, RunState::Stopped, &format!("stopped by {who}"), Some(End::Removed)).await
}

/// From machine `machine` of a team's own account: the session of the team's live run in that box
/// (`db::run_on_machine`) asks the people its bot's `CallRules` name for `situation` (or the
/// run's controller, while someone took it over). The session never names people. Refused for a
/// person's machine, a team machine that is no box, a box with no live run, and a run that already
/// waits on a call. Opens the call, sets the run `Waiting`, pushes `TeamEvent::CallOpened` and
/// `RunChanged`, and answers at once with the call's id and who it asks; the answer reaches the
/// session later, typed into its tab.
///
/// `step`, the team-box hook's hash of the exact step it asks about before an irreversible one:
/// when someone answered "Go ahead" on a call of this run about the same step and question in the
/// last 10 minutes, and it was not used yet, it is used now (`db::use_go_ahead`) and the answer is
/// `Asked::Granted`, with nothing opened. Else the call opens as above and keeps the hash.
pub async fn ask_people(
    hub: &Arc<Hub>,
    team: &Arc<Account>,
    machine: &str,
    situation: Situation,
    question: &str,
    choices: Vec<String>,
    step: Option<&str>,
) -> Result<Asked, String> {
    const NOT_A_TEAM_BOX: &str = "only a session in one of a team's boxes asks people";
    if !db::is_team(&hub.db, team.id).await? {
        return Err(NOT_A_TEAM_BOX.into());
    }
    let machine_id = Uuid::parse_str(machine).map_err(|_| NOT_A_TEAM_BOX)?;
    if hub.db.box_of(team.id, machine_id).await?.is_none() {
        return Err(NOT_A_TEAM_BOX.into());
    }
    // Only the limits tick opens a run's limit call: a session never asks, so never disarms, its own.
    if situation == Situation::OnLimit {
        return Err(OWN_LIMIT.into());
    }
    let run = db::run_on_machine(&hub.db, team.id, machine).await?.ok_or("no run of the team works in this box")?;
    let run_id = parse_id(&run.id, "run")?;
    let question = question.trim();
    let step = step.filter(|s| !s.is_empty() && situation == Situation::BeforeIrreversible);
    if let Some(step) = step {
        if let Some(call) = db::use_go_ahead(&hub.db, team.id, run_id, step, question, now()).await? {
            return Ok(Asked::Granted { granted: true, call: call.to_string() });
        }
    }
    if run.state == RunState::Waiting {
        return Err(WAITS_ALREADY.into());
    }
    if question.is_empty() {
        return Err("say what you ask".into());
    }
    open_call(hub, team, run, run_id, situation, question, choices, step, false).await
}

/// What a session hears when it asks about its own limit (`Situation::OnLimit`).
pub const OWN_LIMIT: &str = "a session may not ask about its own limit: the hub does";

/// What a run that already waits on a call hears when it asks again: at most one wait at a time.
const WAITS_ALREADY: &str = "this run already waits on a call";

/// Opens a call of live run `run` for `situation`, as `ask_people` does once its checks passed,
/// and as the limits tick does for `Situation::OnLimit` (`by_hub`, the only call that counts as
/// the run's limit call): the run moves to `Waiting` in one write, so of two at once only one
/// opens a call.
#[allow(clippy::too_many_arguments)]
async fn open_call(
    hub: &Arc<Hub>,
    team: &Arc<Account>,
    run: Run,
    run_id: Uuid,
    situation: Situation,
    question: &str,
    choices: Vec<String>,
    step: Option<&str>,
    by_hub: bool,
) -> Result<Asked, String> {
    let choices: Vec<String> = choices.iter().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect();
    let named = async { Ok::<_, String>(db::bot_calls(&hub.db, team.id, &run.workflow).await?.map(|(rules, _)| rules).unwrap_or_default().who(situation).clone()) };
    // A limit call asks whom `on_limit` names even while someone took the run over, and them too.
    let asked = match &run.controller {
        Some(controller) if situation == Situation::OnLimit => {
            let (named, controller) = (named.await?, Who::Person { account: controller.clone() });
            if named == controller { vec![named] } else { vec![named, controller] }
        }
        Some(controller) => vec![Who::Person { account: controller.clone() }],
        None => vec![named.await?],
    };
    let Some(waiting) = db::start_waiting(&hub.db, team.id, run_id).await? else {
        let now = db::run(&hub.db, team.id, run_id).await?;
        return Err(match now.map(|r| r.state) {
            Some(RunState::Waiting) => WAITS_ALREADY.into(),
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
    let call = match db::insert_call(&hub.db, team.id, &call, step, by_hub).await {
        Ok(call) => call,
        Err(e) => {
            db::stop_waiting(&hub.db, team.id, run_id).await?;
            return Err(e);
        }
    };
    push_logged(hub, team.id, TeamEvent::CallOpened { call: call.clone() }).await;
    notify_slack(hub, team, &call);
    push_logged(hub, team.id, TeamEvent::RunChanged { run: waiting }).await;
    let mut shown = Vec::with_capacity(call.asked.len());
    for who in &call.asked {
        shown.push(shown_who(hub, who).await);
    }
    Ok(Asked::Call { call: call.id, asked: shown })
}

/// The most characters of a call's question a Slack notice carries.
const NOTICE_QUESTION: usize = 300;

/// The notice a call to a group posts in Slack: one line, the question without secrets and cut.
/// The bot's name and question are escaped as Slack text, so a `<!channel>` or `<@U123>` in them
/// pings nobody; only `mention`, pinned's own, stays live.
fn slack_notice(bot: &str, mention: &str, question: &str) -> String {
    let masked = pinned_core::surface::mask_secrets(question);
    let line: String = masked.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NOTICE_QUESTION).collect();
    format!("{} needs {mention}: {}. Answer in pinned.", slack_escape(bot), slack_escape(&line))
}

/// `text` as Slack shows it literally: `&`, `<` and `>` as `&amp;`, `&lt;` and `&gt;`.
fn slack_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A newly opened call that asks a group with a Slack user group tells that group in Slack, in the
/// background: one message in the channel of the call's own workflow, through the Slack bot of that
/// workflow in the team's store (`Connectors::announce_to_group`), never a member's. Nothing waits
/// on it, and a failure only logs. Answers stay in the app.
fn notify_slack(hub: &Arc<Hub>, team: &Arc<Account>, call: &Call) {
    let groups: Vec<String> = call.asked.iter().filter_map(|who| if let Who::Group { handle } = who { Some(handle.clone()) } else { None }).collect();
    if groups.is_empty() {
        return;
    }
    let (hub, team, workflow, question, call_id) = (hub.clone(), team.clone(), call.workflow.clone(), call.question.clone(), call.id.clone());
    tokio::spawn(async move {
        for handle in groups {
            let said = async {
                let slack_group = db::slack_group(&hub.db, team.id, &handle).await?;
                if !pinned_core::slack::is_group_handle(&slack_group) {
                    return Ok(false);
                }
                let connectors = hub.connectors(&team).await?;
                connectors.announce_to_group(&workflow, &slack_group, |bot, mention| slack_notice(bot, mention, &question)).await
            };
            match said.await {
                Ok(true) => {}
                Ok(false) => eprintln!("pinned-server: call {call_id} of team {} has no Slack notice for @{handle}", team.id),
                Err(e) => eprintln!("pinned-server: call {call_id} of team {} could not tell @{handle} in Slack: {e}", team.id),
            }
        }
    });
}

/// Runs every few seconds (`super::spawn_tick`), across every team. A call open longer than
/// `policy.escalate_after_secs` goes to the owner too (escalated, `TeamEvent::CallOpened` again).
/// One open longer than `policy.give_up_after_secs` closes as gave-up, with the same guard as an
/// answer, so only one of them wins; its run ends as `RunState::GaveUp`, its session is told to
/// end without the step, and its box ends. Then each run's limits are held (`limits`), and no run
/// or box is left behind (`reap`).
///
/// Each phase runs on its own: one that fails is logged with its name and the next still runs,
/// so one team's bad row holds up no other team's ends. Max life runs first, `reap` always.
pub async fn tick(hub: &Arc<Hub>, policy: &TeamPolicy) -> Result<(), String> {
    logged("max life", max_life(hub, policy).await);
    logged("escalate", escalate(hub, policy).await);
    logged("give-up", give_up(hub, policy).await);
    logged("charge", charge(hub).await);
    logged("at-limit", at_limit(hub, policy).await);
    logged("reap", reap(hub, policy).await);
    Ok(())
}

fn logged(phase: &str, done: Result<(), String>) {
    if let Err(e) = done {
        eprintln!("pinned-server: the teams' tick did not finish its {phase} phase: {e}");
    }
}

async fn escalate(hub: &Arc<Hub>, policy: &TeamPolicy) -> Result<(), String> {
    for row in db::escalate_due(&hub.db, policy.escalate_after_secs).await? {
        push_logged(hub, row.team, TeamEvent::CallOpened { call: row.call }).await;
    }
    Ok(())
}

async fn give_up(hub: &Arc<Hub>, policy: &TeamPolicy) -> Result<(), String> {
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

/// What a run that lived `TeamPolicy::max_life_secs` ends with.
pub const MAX_LIFE_REACHED: &str = "it reached the most time a run may take, waiting included";

/// The limits phases of `tick` (`max_life`, `charge`, `at_limit`):
/// - a live run whose start is `policy.max_life_secs` ago ends (`RunState::Failed`,
///   `MAX_LIFE_REACHED`) with its box, whatever it does: waiting counts toward its max life;
/// - each run at work is charged its box's rate for its active time (`db::charge_working_runs`);
///   a waiting run is not, and its wait is not active time, so a wait counts toward neither limit;
/// - a run at work that reached its bot's `BotLimits` (minutes of work, or dollars) stops working:
///   it waits on an `OnLimit` call to whom the bot's `CallRules::on_limit` names, and its session
///   is told to stop and wait. Nobody answering ends it at give-up as any call does; "Stop it"
///   ends it as `stop_run` does; "Let it go on" lets it go on, called no more for its limits
///   (one limit call per run), up to its max life.
async fn max_life(hub: &Arc<Hub>, policy: &TeamPolicy) -> Result<(), String> {
    for (team, run) in db::runs_past_max_life(&hub.db, policy.max_life_secs).await? {
        let hub = hub.clone();
        // Each on its own, as a give-up's: one box slow to end holds up no other.
        tokio::spawn(async move {
            let ended = async { end_run(&hub, &hub.account(team).await?, run, RunState::Failed, MAX_LIFE_REACHED, Some(End::Expired)).await };
            match ended.await {
                Err(e) if e != RUN_ENDED => eprintln!("pinned-server: run {run} of team {team} reached its max life but did not end yet: {e}"),
                _ => {}
            }
        });
    }
    Ok(())
}

async fn charge(hub: &Arc<Hub>) -> Result<(), String> {
    use pinned_core::wire::box_usd_per_hour;
    use pinned_core::connectors::BoxSize::{Large, Medium, Small};
    db::charge_working_runs(&hub.db, [box_usd_per_hour(Small), box_usd_per_hour(Medium), box_usd_per_hour(Large)]).await
}

async fn at_limit(hub: &Arc<Hub>, policy: &TeamPolicy) -> Result<(), String> {
    for at in db::runs_at_limit(&hub.db, &super::bots::most(policy)).await? {
        if let Err(e) = limit_reached(hub, policy, &at).await {
            eprintln!("pinned-server: run {} of team {} reached a limit but was not called for it: {e}", at.run, at.team);
        }
    }
    Ok(())
}

/// The choices of a limit call: "Let it go on" until the run's max life, or "Stop it" as `stop_run`.
const GO_ON: &str = "Let it go on";
pub(super) const STOP_IT: &str = "Stop it";

/// `secs` of `TeamPolicy::max_life_secs` as a limit reads it: "4 hour", "90 minute", "8 second".
fn life_limit(secs: u64) -> String {
    match secs {
        s if s >= 3600 && s % 3600 == 0 => format!("{} hour", s / 3600),
        s if s >= 60 && s % 60 == 0 => format!("{} minute", s / 60),
        s => format!("{s} second"),
    }
}

/// Dollars as a limit reads them: cents shown ($0.40, $5.00), or every digit a smaller limit has ($0.001).
fn dollars(usd: f64) -> String {
    let cents = usd * 100.0;
    if (cents - cents.round()).abs() < 1e-9 { format!("${usd:.2}") } else { format!("${usd}") }
}

/// Opens the `OnLimit` call of a run that reached a limit and tells its session to stop. Its
/// dollars are the box's time only: no model usage is counted.
async fn limit_reached(hub: &Arc<Hub>, policy: &TeamPolicy, at: &db::AtLimit) -> Result<(), String> {
    let team = hub.account(at.team).await?;
    let Some(run) = db::run(&hub.db, at.team, at.run).await? else { return Ok(()) };
    let what = match (at.spent, at.limits.minutes) {
        (true, _) => format!("{} of box time (model usage is not counted)", dollars(at.limits.usd)),
        (false, 1) => "1 minute of work".to_owned(),
        (false, minutes) => format!("{minutes} minutes of work"),
    };
    let question = format!("The run reached its limit: {what}. Let it go on until the {} limit, or stop it?", life_limit(policy.max_life_secs));
    let choices = vec![GO_ON.to_owned(), STOP_IT.to_owned()];
    if let Asked::Call { call, .. } = open_call(hub, &team, run, at.run, Situation::OnLimit, &question, choices, None, true).await? {
        tell_session(hub, &team, at.run, &format!("limit-{call}"), format!("[The run reached its limit: {what}. Stop and wait for the answer]")).await;
    }
    Ok(())
}

/// What a run that never had a box, or whose box went without ending it, ends with.
const NEVER_STARTED: &str = "its box never started";
const BOX_GONE: &str = "its box is gone";
/// What a reserved run whose session never opened within `TeamPolicy::reservation_limit_secs` ends with.
pub const NEVER_OPENED: &str = "it never started";

/// How long a run waits with no open call, and with no call of it closed meanwhile, before `reap`
/// moves it on. A give-up tells the session before it ends the run, which can take this long.
const STALE_WAIT: Duration = Duration::from_secs(300);

/// The reaper's half of `tick`. Every end goes through `end_run`, so no run outlives its box and
/// no box outlives its run:
/// - a running box of a team that no live run holds ends, as a stopped run's box does (`End::Removed`);
/// - a live run still without a box after the box boot wait and `RUN_WAIT` fails (`NEVER_STARTED`),
///   and one whose box has no row fails (`BOX_GONE`); a reserved run is not one of them;
/// - a reserved run whose session did not open within `policy.reservation_limit_secs` fails
///   (`NEVER_OPENED`), counted as `SkipReason::NeverStarted`, and a granted event no poller
///   claimed for it leaves its line, so it never runs without its place;
/// - a run left waiting with no open call ends as `RunState::GaveUp` when its last call gave up
///   (a give-up a restart cut short), as `RunState::Stopped` when its last call stopped it (a stop
///   or "Stop it" whose end failed), else goes back to work while its box runs, else ends (`BOX_GONE`);
/// - what waits for a team with a free place starts (`queue::next`), so an event is never left
///   waiting when the start after a run's end found the team's turn busy.
///
/// A failure is logged and leaves the rest of the pass to run; the next tick tries again.
async fn reap(hub: &Arc<Hub>, policy: &TeamPolicy) -> Result<(), String> {
    for (team, machine) in db::unheld_team_boxes(&hub.db).await? {
        let hub = hub.clone();
        // Each on its own, as a give-up's: one box slow to end holds up no other.
        tokio::spawn(async move {
            let ended = async {
                let account = hub.account(team).await?;
                hub.boxes.end_machine(&hub, &account, machine, End::Removed).await
            };
            if let Err(e) = ended.await {
                eprintln!("pinned-server: box {machine} of team {team} has no live run but did not end yet: {e}");
            }
        });
    }
    let boot = hub.boxes.policy.boot_wait + crate::hub::RUN_WAIT;
    for (team, run, never_started) in db::boxless_runs(&hub.db, boot.as_secs()).await? {
        let outcome = if never_started { NEVER_STARTED } else { BOX_GONE };
        reap_run(hub, team, run, outcome).await;
    }
    for (team, run, workflow) in db::stale_reservations(&hub.db, policy.reservation_limit_secs).await? {
        let held = hub.hold_backstop.lock().unwrap().take();
        if let Some((read, go)) = held {
            read.notify_one();
            go.notified().await;
        }
        end_reservation(hub, team, run, &workflow, policy.reservation_limit_secs).await;
    }
    for (team, run, box_runs, last) in db::stale_waiting(&hub.db, STALE_WAIT.as_secs()).await? {
        if let Some(db::LastCall::StoppedBy(by)) = last {
            // A person stopped it but its end failed (a busy write): it ends now, never back to work.
            let hub = hub.clone();
            tokio::spawn(async move {
                let ended = async {
                    let who = name_of(&hub, &by).await;
                    end_run(&hub, &hub.account(team).await?, run, RunState::Stopped, &format!("stopped by {who}"), box_runs.then_some(End::Removed)).await
                };
                match ended.await {
                    Err(e) if e != RUN_ENDED => eprintln!("pinned-server: run {run} of team {team} was stopped but did not end yet: {e}"),
                    _ => {}
                }
            });
            continue;
        }
        if let Some(db::LastCall::GaveUp(call)) = last {
            // Its call gave up but the run never ended (a restart): it ends now, never back to work.
            let hub = hub.clone();
            tokio::spawn(async move {
                let box_end = box_runs.then_some(End::Expired);
                let ended = async { end_run(&hub, &hub.account(team).await?, run, RunState::GaveUp, &format!("nobody answered call {call}"), box_end).await };
                match ended.await {
                    Err(e) if e != RUN_ENDED => eprintln!("pinned-server: run {run} of team {team} gave up but did not end yet: {e}"),
                    _ => {}
                }
            });
            continue;
        }
        if !box_runs {
            reap_run(hub, team, run, BOX_GONE).await;
            continue;
        }
        let moved = async { back_to_work(hub, &hub.account(team).await?, run).await };
        if let Err(e) = moved.await {
            eprintln!("pinned-server: run {run} of team {team} waits on no call but did not go back to work: {e}");
        }
    }
    for (team, workflow) in db::startable_waiting(&hub.db).await? {
        match hub.account(team).await {
            Ok(account) => super::queue::next(hub, &account, &workflow),
            Err(e) => eprintln!("pinned-server: team {team} has a free place but what waits did not start: {e}"),
        }
    }
    Ok(())
}

/// Ends a run whose box is not there as `RunState::Failed` with `outcome`, leaving one that ended meanwhile.
async fn reap_run(hub: &Arc<Hub>, team: Uuid, run: Uuid, outcome: &str) {
    let ended = async { end_run(hub, &hub.account(team).await?, run, RunState::Failed, outcome, None).await };
    match ended.await {
        Err(e) if e != RUN_ENDED => eprintln!("pinned-server: run {run} of team {team} has no box but did not end yet: {e}"),
        _ => {}
    }
}

/// Ends reserved run `run` of bot `workflow`, whose session never opened within `secs`, as
/// `RunState::Failed` (`NEVER_OPENED`) and counts it, after taking out an unclaimed granted event
/// that holds its place. Only through one write on the run's row (`db::end_stale_reservation`):
/// a run that opened, ended, or whose granted event a poller claimed meanwhile (which holds its
/// place anew) is left as it is, and counted by nobody here.
async fn end_reservation(hub: &Arc<Hub>, team: Uuid, run: Uuid, workflow: &str, secs: u64) {
    let ended = async {
        db::drop_grant(&hub.db, team, run).await?;
        match end_run_if(hub, &hub.account(team).await?, run, RunState::Failed, NEVER_OPENED, None, Some(secs)).await {
            Ok(_) => {}
            Err(e) if e == RUN_ENDED => return Ok(()),
            Err(e) => return Err(e),
        }
        // Counted here only: this write ended the run, so a box still booting for it and failing
        // after finds it ended and counts no failed start on top (`db::fail_reserved_start`).
        db::count_skipped(&hub.db, team, workflow, SkipReason::NeverStarted, 1).await
    };
    if let Err(e) = ended.await {
        eprintln!("pinned-server: reserved run {run} of team {team} never opened but did not end yet: {e}");
    }
}

/// Ends the run of a call nobody answered: the session hears it first, then its box ends.
async fn gave_up(hub: &Arc<Hub>, team: Uuid, call: &Call) -> Result<(), String> {
    let account = hub.account(team).await?;
    let run_id = parse_id(&call.run, "run")?;
    let text = format!("[Nobody answered call {} in time: end without doing the step]", call.id);
    tell_session_now(hub, &account, run_id, &format!("call-{}", call.id), text).await;
    end_run(hub, &account, run_id, RunState::GaveUp, &format!("nobody answered call {}", call.id), Some(End::Expired)).await.map(drop)
}

/// Every way a run ends goes through here: `stop_run`, an answer that stops it, the give-up, and
/// its box ending for any reason (`box_ended`). Moves the run to the ended `state` (refused when it
/// had ended), takes out of its line a granted event no poller claimed for it (`db::drop_grant`),
/// closes its open call as stopped, pushes both, ends its box for `box_end` (None when
/// the box is what ended), and discards a bot made to run once (`bots::run_ended`). A box that
/// does not end now is logged: the reaper still ends it at its max life.
pub async fn end_run(hub: &Hub, team: &Arc<Account>, run_id: Uuid, state: RunState, outcome: &str, box_end: Option<End>) -> Result<Run, String> {
    end_run_if(hub, team, run_id, state, outcome, box_end, None).await
}

/// `end_run`, whose state write, when `stale_after` is set, ends only a run still reserved whose
/// place was held at least that many seconds ago (`db::end_stale_reservation`): the backstop's.
/// The rest follows only when that write ended the run; otherwise `RUN_ENDED`.
async fn end_run_if(hub: &Hub, team: &Arc<Account>, run_id: Uuid, state: RunState, outcome: &str, box_end: Option<End>, stale_after: Option<u64>) -> Result<Run, String> {
    let current = db::run(&hub.db, team.id, run_id).await?.ok_or("the team has no such run")?;
    let controller = current.controller.as_deref().and_then(|c| Uuid::parse_str(c).ok());
    let ended = match stale_after {
        None => db::set_run_state(&hub.db, team.id, run_id, state, controller, Some(outcome)).await?,
        Some(secs) => db::end_stale_reservation(&hub.db, team.id, run_id, state, controller, outcome, secs).await?,
    };
    if !ended {
        return Err(RUN_ENDED.into());
    }
    // A granted event no poller claimed for it never runs now: a claim takes only a live run's.
    if let Err(e) = db::drop_grant(&hub.db, team.id, run_id).await {
        eprintln!("pinned-server: run {run_id} of team {} ended but its granted event stays in line: {e}", team.id);
    }
    // The run has ended: each step after the state write is logged when it fails and the rest
    // still runs, so the caller hears that the run ended, as it did.
    let closed = Answer { by: team.id.to_string(), choice: None, text: outcome.to_owned(), kind: AnswerKind::Stop, at: now() };
    match db::stop_open_call(&hub.db, team.id, run_id, &closed).await {
        Ok(Some(call)) => push_logged(hub, team.id, TeamEvent::CallResolved { call }).await,
        Ok(None) => {}
        Err(e) => eprintln!("pinned-server: run {run_id} of team {} ended but its open call stays open: {e}", team.id),
    }
    let run = match db::run(&hub.db, team.id, run_id).await {
        Ok(Some(run)) => run,
        read => {
            if let Err(e) = read {
                eprintln!("pinned-server: run {run_id} of team {} ended but was not read again: {e}", team.id);
            }
            let ended_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            Run { state, outcome: Some(outcome.to_owned()), ended_at: Some(ended_at), ..current }
        }
    };
    push_logged(hub, team.id, TeamEvent::RunChanged { run: run.clone() }).await;
    if let Some(why) = box_end {
        match db::run_box(&hub.db, team.id, run_id).await {
            Ok(machine) => {
                if let Some(machine) = machine.and_then(|m| Uuid::parse_str(&m).ok()) {
                    if let Err(e) = hub.boxes.end_machine(hub, team, machine, why).await {
                        eprintln!("pinned-server: the box of run {run_id} of team {} did not end yet: {e}", team.id);
                    }
                }
            }
            Err(e) => eprintln!("pinned-server: the box of run {run_id} of team {} was not found to end: {e}", team.id),
        }
    }
    if let Err(e) = super::bots::run_ended(hub, team.id, &run).await {
        eprintln!("pinned-server: run {run_id} of team {} ended but its run-once bot stays: {e}", team.id);
    }
    super::queue::next(hub, team, &run.workflow);
    Ok(run)
}

/// Ends every live run of bot `workflow` as `Stopped` with `outcome`, ending its box
/// (`End::Removed`). A run that ends meanwhile is left as it ended.
pub async fn stop_live_runs(hub: &Hub, team: &Arc<Account>, workflow: &str, outcome: &str) -> Result<(), String> {
    for run_id in db::live_runs_of(&hub.db, team.id, workflow).await? {
        match end_run(hub, team, run_id, RunState::Stopped, outcome, Some(End::Removed)).await {
            Err(e) if e != RUN_ENDED => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// The box `machine` of `account` ends for `why` (`boxes::Boxes` calls it on every end, before
/// the box goes), its session's last turn having stood as `turn`. When it is a team's box whose run
/// has not ended, the run ends with it, so no run stays `waiting` or `working` once its box is gone;
/// the state follows `ended_by_box`. Boxed: ending a run can end its box, which comes back here.
pub fn box_ended<'a>(hub: &'a Hub, account: &'a Account, machine: Uuid, why: &'a End, turn: LastTurn) -> BoxFuture<'a, Result<(), String>> {
    Box::pin(async move {
        let Some(run) = db::run_on_machine(&hub.db, account.id, &machine.to_string()).await? else { return Ok(()) };
        let team = hub.account(account.id).await?;
        let (state, outcome) = ended_by_box(why, turn);
        match end_run(hub, &team, parse_id(&run.id, "run")?, state, &outcome, None).await {
            Err(e) if e != RUN_ENDED => Err(e),
            _ => Ok(()),
        }
    })
}

/// The box `machine` of `account` failed to boot or to start its session (`boxes::Boxes::drive`).
/// When it is a team's box, its live run ends as `box_ended` ends it for `End::Failed`, so it does
/// not stay working: the run the box claimed, else the one with no box yet that it was opened for
/// (the team's workflow `workflow` and `subject`).
pub async fn box_failed(hub: &Hub, account: &Account, machine: Uuid, workflow: &str, subject: &Subject, error: &str) -> Result<(), String> {
    if !account.team {
        return Ok(());
    }
    let run = match db::run_on_machine(&hub.db, account.id, &machine.to_string()).await? {
        Some(run) => run,
        None => match db::live_run_for(&hub.db, account.id, workflow, subject).await? {
            Some(run) if db::run_box(&hub.db, account.id, parse_id(&run.id, "run")?).await?.is_none() => run,
            _ => return Ok(()),
        },
    };
    let team = hub.account(account.id).await?;
    let (state, outcome) = ended_by_box(&End::Failed(error.to_owned()), LastTurn::Unknown);
    match end_run(hub, &team, parse_id(&run.id, "run")?, state, &outcome, None).await {
        Err(e) if e != RUN_ENDED => Err(e),
        _ => Ok(()),
    }
}

/// How a run ends when its box ends first. `Done` only when the session ended its own turn:
/// - `End::Done`: it opened its pull request and stopped working → Done.
/// - `End::TabClosed`, `Exited` or `Idle` after its last turn ended (Stop hook, `LastTurn::Ended`) → Done.
/// - `End::TabClosed` that a person closed before its turn ended (`LastTurn::ClosedByPerson`) → Stopped.
/// - `End::TabClosed` that went away mid-turn, or whose turn the hub cannot tell → Failed.
/// - `End::Exited` or `Idle` mid-turn, or whose turn the hub cannot tell → Failed.
/// - `End::Removed` (someone removed its box) or `Disconnected` (the team's Upstash Box) → Stopped.
/// - `End::Expired` (max life or budget), `Offline`, `Vanished` (deleted outside pinned),
///   `NeverStarted` (its session stopped at a startup screen), `Failed` → Failed.
fn ended_by_box(why: &End, turn: LastTurn) -> (RunState, String) {
    const FINISHED: &str = "its session finished";
    match (why, turn) {
        (End::Done, _) => (RunState::Done, FINISHED.into()),
        (End::TabClosed | End::Exited | End::Idle, LastTurn::Ended) => (RunState::Done, FINISHED.into()),
        (End::TabClosed, LastTurn::ClosedByPerson) => (RunState::Stopped, "its session's tab was closed before it finished".into()),
        (End::TabClosed, _) => (RunState::Failed, "its session's tab went away before it finished".into()),
        (End::Exited, _) => (RunState::Failed, "its session exited before it finished".into()),
        (End::Idle, _) => (RunState::Failed, "its session sat idle before it finished".into()),
        (End::Removed, _) => (RunState::Stopped, "its box was removed".into()),
        (End::Disconnected, _) => (RunState::Stopped, "the team's Upstash Box was disconnected".into()),
        (End::Expired, _) => (RunState::Failed, "its box ran out of time or budget".into()),
        (End::Offline, _) => (RunState::Failed, "its box went offline".into()),
        (End::Vanished, _) => (RunState::Failed, "its box was deleted outside pinned".into()),
        (End::NeverStarted, _) => (RunState::Failed, "its session never started: it stopped at a startup screen".into()),
        (End::Failed(e), _) => (RunState::Failed, format!("its box failed: {e}")),
    }
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

async fn push_logged(hub: &Hub, team: Uuid, event: TeamEvent) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row of `ended_by_box`'s rule: Done only when the session ended its own turn.
    #[test]
    fn a_run_is_done_only_when_its_session_ended_its_own_turn() {
        let state = |why: End, turn| ended_by_box(&why, turn).0;
        assert_eq!(state(End::Done, LastTurn::Unknown), RunState::Done, "it opened its pull request and stopped");
        for why in [End::TabClosed, End::Exited, End::Idle] {
            assert_eq!(state(why.clone(), LastTurn::Ended), RunState::Done, "{why:?} after its turn ended");
            assert_eq!(state(why.clone(), LastTurn::Unknown), RunState::Failed, "{why:?} mid-turn or untold");
        }
        assert_eq!(state(End::TabClosed, LastTurn::ClosedByPerson), RunState::Stopped, "a person closed it mid-turn");
        assert_eq!(state(End::Exited, LastTurn::ClosedByPerson), RunState::Failed, "only a tab is closed by a person");
        assert_eq!(state(End::Removed, LastTurn::Ended), RunState::Stopped);
        assert_eq!(state(End::Disconnected, LastTurn::Ended), RunState::Stopped);
        for why in [End::Expired, End::Offline, End::Vanished, End::Failed("x".into())] {
            assert_eq!(state(why.clone(), LastTurn::Ended), RunState::Failed, "{why:?}");
        }
    }
}
