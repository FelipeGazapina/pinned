//! The hub's scheduler: it fires the due scheduled tasks whose action is a workflow, as a Run now of
//! that workflow, and records each occurrence as a `schedule::TaskRun`. Prompt tasks are not fired yet.
//!
//! An occurrence is claimed once, by its `task_runs` row (`Db::claim_task_run`), so a restart or a
//! second hub on the database never starts it twice. A task that missed several slots runs once, for
//! its stored `next_at`, and its next run is then the first one after now.

use crate::hub::Hub;
use pinned_core::schedule::{self, Action, OnMissed, RunState, Task, TaskRun};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// How long the loop sleeps after each pass ends: a run may start this much after its slot.
pub const EVERY: Duration = Duration::from_secs(30);

/// The most due tasks one pass takes; the next pass takes the rest.
pub const BATCH: i64 = 100;

/// Runs `pass` at once, then again `EVERY` after each pass ends, for as long as the hub runs.
pub fn spawn(hub: &Arc<Hub>) {
    let hub = Arc::downgrade(hub);
    tokio::spawn(async move {
        loop {
            let Some(strong) = hub.upgrade() else { break };
            if let Err(e) = pass(&strong).await {
                eprintln!("pinned-server: the scheduler's pass failed: {e}");
            }
            drop(strong);
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// One pass: claims the occurrence of each due workflow task (at most `BATCH`), records a skip or a
/// failure at once, and starts each run in a task of its own, so a machine slow to answer holds
/// neither the other tasks nor the next pass. An occurrence another pass or hub claimed is left to it.
pub async fn pass(hub: &Arc<Hub>) -> Result<(), String> {
    let now = schedule::now();
    for (account, task, due_at) in hub.db.due_workflow_tasks(now, BATCH).await? {
        let Action::Workflow { id: workflow } = task.action.clone() else { continue };
        let (run, next_at) = decide(&task, due_at, now);
        match hub.db.claim_task_run(account, &run, next_at).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                eprintln!("pinned-server: could not claim account {account}'s task {} due at {due_at}: {e}", task.id);
                continue;
            }
        }
        if run.state != RunState::Due {
            recorded(hub, account, &run).await;
            continue;
        }
        let hub = hub.clone();
        tokio::spawn(async move {
            let by = format!("the scheduled task {:?}", task.title);
            let started = match hub.account(account).await {
                Ok(owner) => hub.run_workflow(&owner, &workflow, &by).await,
                Err(e) => Err(e),
            };
            let run = match started {
                Ok(started) => TaskRun { started_at: Some(schedule::now()), state: RunState::Done, machine: started.machine, tab: started.tab, ..run },
                Err(e) => TaskRun { state: RunState::Failed, detail: e, ..run },
            };
            match hub.db.record_task_run(account, &run).await {
                Ok(()) => recorded(&hub, account, &run).await,
                Err(e) => eprintln!("pinned-server: could not record account {account}'s task {} due at {}: {e}", run.task, run.due_at),
            }
        });
    }
    Ok(())
}

/// Logs a recorded occurrence by its ids, due time and state, never its title or detail, and sends
/// the account's apps fresh settings, so its `last_run` and `next_at` show without a reload.
async fn recorded(hub: &Hub, account: Uuid, run: &TaskRun) {
    let state = serde_json::to_value(run.state).ok();
    eprintln!("pinned-server: account {account}'s task {} due at {} is {}", run.task, run.due_at, state.as_ref().and_then(|s| s.as_str()).unwrap_or_default());
    if let Ok(owner) = hub.account(account).await {
        hub.push_settings(&owner).await;
    }
}

/// What to do with the occurrence of `task` due at `due_at`, seen at `now`: the run its claim
/// records (`due` to start it, `skipped`, or `failed` when the schedule no longer reads), and the
/// task's new `next_at`, the first run strictly after now, so a task many slots late runs once.
/// Only an explicit `on_missed: skip` skips, once past its grace window and one pass more.
pub fn decide(task: &Task, due_at: i64, now: i64) -> (TaskRun, Option<i64>) {
    let run = TaskRun { task: task.id.clone(), due_at, started_at: None, state: RunState::Due, machine: String::new(), tab: String::new(), detail: String::new() };
    let next_at = match schedule::next_after(&task.schedule, &task.tz, now) {
        Ok(next_at) => next_at,
        Err(e) => return (TaskRun { state: RunState::Failed, detail: e, ..run }, None),
    };
    let late = now - due_at;
    if task.on_missed == Some(OnMissed::Skip) && late > i64::from(task.effective_grace()) * 60 + EVERY.as_secs() as i64 {
        let detail = format!("the hub saw it {} minutes late, past its grace window of {} minutes", late / 60, task.effective_grace());
        return (TaskRun { state: RunState::Skipped, detail, ..run }, next_at);
    }
    (run, next_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinned_core::schedule::{Actor, Fallback, Schedule, Target};

    /// 2026-11-13 09:00 UTC, a slot of `daily`.
    const NINE: i64 = 1_794_560_400;
    const DAY: i64 = 86_400;

    fn daily() -> Schedule {
        Schedule::Cron { expr: "0 9 * * *".into() }
    }

    fn task(schedule: Schedule, on_missed: Option<OnMissed>, grace_mins: Option<u32>) -> Task {
        Task {
            id: "t1".into(),
            title: "Nightly".into(),
            schedule,
            tz: "UTC".into(),
            target: Target::Me,
            action: Action::Workflow { id: "c1".into() },
            enabled: true,
            grace_mins,
            on_missed,
            pickup_mins: None,
            approvals: vec![],
            fallback: Fallback::Wait,
            idea: None,
            created_by: Actor::User,
            created_at: 0,
            updated_at: 0,
            next_at: Some(NINE),
            last_run: None,
            fyi: None,
        }
    }

    #[test]
    fn an_on_time_daily_task_runs_and_its_next_run_is_the_first_slot_after_now() {
        let (run, next) = decide(&task(daily(), None, None), NINE, NINE + 5);
        assert_eq!((run.state, run.due_at, run.task.as_str()), (RunState::Due, NINE, "t1"));
        assert_eq!(next, Some(NINE + DAY));
    }

    #[test]
    fn a_daily_task_ten_days_late_runs_once_and_its_next_run_is_after_now() {
        let (run, next) = decide(&task(daily(), None, None), NINE, NINE + 10 * DAY + 3600);
        assert_eq!((run.state, run.due_at), (RunState::Due, NINE));
        assert_eq!(next, Some(NINE + 11 * DAY), "the first slot after now, not due_at + 1 day");
    }

    #[test]
    fn a_one_time_task_runs_and_has_no_next_run() {
        let (run, next) = decide(&task(Schedule::Once { at: "2026-11-13T09:00".into() }, None, None), NINE, NINE + 5);
        assert_eq!((run.state, next), (RunState::Due, None));
    }

    #[test]
    fn an_explicit_skip_past_grace_is_skipped_and_within_grace_runs() {
        let skip = task(daily(), Some(OnMissed::Skip), None);
        let (run, next) = decide(&skip, NINE, NINE + 2 * 3600);
        assert_eq!((run.state, next), (RunState::Skipped, Some(NINE + DAY)));
        assert!(run.detail.contains("grace window of 15 minutes"), "{}", run.detail);
        assert_eq!(decide(&skip, NINE, NINE + 10 * 60).0.state, RunState::Due, "within grace");
        assert_eq!(decide(&skip, NINE, NINE + 15 * 60 + 30).0.state, RunState::Due, "grace and one pass");
        assert_eq!(decide(&skip, NINE, NINE + 15 * 60 + 31).0.state, RunState::Skipped);
    }

    #[test]
    fn a_task_ten_days_late_without_an_explicit_skip_runs() {
        for on_missed in [None, Some(OnMissed::RunLate), Some(OnMissed::Ask)] {
            let (run, next) = decide(&task(daily(), on_missed, None), NINE, NINE + 10 * DAY + 3600);
            assert_eq!((run.state, next), (RunState::Due, Some(NINE + 11 * DAY)), "{on_missed:?}");
        }
    }

    #[test]
    fn a_zero_grace_skip_seen_five_seconds_late_runs() {
        assert_eq!(decide(&task(daily(), Some(OnMissed::Skip), Some(0)), NINE, NINE + 5).0.state, RunState::Due);
    }

    #[test]
    fn a_schedule_that_no_longer_reads_fails_with_no_next_run() {
        let (run, next) = decide(&Task { tz: "Mars/Base".into(), ..task(daily(), None, None) }, NINE, NINE + 5);
        assert_eq!((run.state, next), (RunState::Failed, None));
        assert!(run.detail.contains("Mars/Base"), "{}", run.detail);
    }
}
