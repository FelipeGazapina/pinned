//! Runs and their calls to people: asking, answering, escalating, taking over.

use super::Scope;
use crate::hub::{Account, Hub};
use pinned_core::teams::{Answer, Call, Run, Situation, TeamPolicy};
use std::sync::Arc;

/// The team's runs, newest first (at most 50). Any member reads them.
pub async fn runs(_hub: &Arc<Hub>, _scope: &Scope) -> Result<Vec<Run>, String> {
    Err("not built yet".into())
}

/// The team's open calls, then the last answered. Any member reads them.
pub async fn calls(_hub: &Arc<Hub>, _scope: &Scope) -> Result<Vec<Call>, String> {
    Err("not built yet".into())
}

/// A member asked by the call (a `Who::Person` naming it, a member of a `Who::Group`, or the
/// owner for `Who::Owner` and once escalated) answers it, once. The hub sets `answer.by` and
/// `answer.at`. `Choice` sends the choice and text to the session; `PassTo` asks group `handle`
/// instead; `TakeOver` is `take_over`; `Stop` is `stop_run`. Pushes `TeamEvent::CallResolved`
/// and answers the call as it is now.
pub async fn answer_call(_hub: &Arc<Hub>, _scope: &Scope, _id: &str, _answer: Answer) -> Result<Call, String> {
    Err("not built yet".into())
}

/// A member takes over a run that has not ended: `RunState::TakenOver` with the member as
/// `controller`. Pushes `TeamEvent::RunChanged`.
pub async fn take_over(_hub: &Arc<Hub>, _scope: &Scope, _run: &str) -> Result<Run, String> {
    Err("not built yet".into())
}

/// The run's controller (or the owner) gives it back to its bot: `RunState::Working`.
pub async fn give_back(_hub: &Arc<Hub>, _scope: &Scope, _run: &str) -> Result<Run, String> {
    Err("not built yet".into())
}

/// Any member stops a run that has not ended: its session stops, `RunState::Stopped`.
pub async fn stop_run(_hub: &Arc<Hub>, _scope: &Scope, _run: &str) -> Result<Run, String> {
    Err("not built yet".into())
}

/// From machine `machine` of the team's own account (`db::is_team`, else refused): the session of
/// the run on that machine (`db::run_on_machine`) asks the people its bot's `CallRules` name for
/// `situation`. The session never names people. Opens the call (`db::insert_call`), sets the run
/// `Waiting`, pushes `TeamEvent::CallOpened`, and answers once a person answers (the `Answer`), or
/// with an error once the run gave up (`tick`).
pub async fn ask_people(_hub: &Arc<Hub>, _team: &Arc<Account>, _machine: &str, _situation: Situation, _question: &str, _choices: Vec<String>) -> Result<Answer, String> {
    Err("not built yet".into())
}

/// Runs every few seconds (`super::spawn_tick`): a call open longer than
/// `policy.escalate_after_secs` goes to the owner too (escalated); one open longer than
/// `policy.give_up_after_secs` ends its run as `RunState::GaveUp`; a run past its bot's
/// `BotLimits` asks `Situation::OnLimit`. Each change pushes its `TeamEvent`.
/// Does nothing until built, so the hub's log stays quiet.
pub async fn tick(_hub: &Arc<Hub>, _policy: &TeamPolicy) -> Result<(), String> {
    Ok(())
}
