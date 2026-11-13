//! Scheduled tasks: when, who and what (0.18.2 Ideas + Tasks, PR 1). Types and the clock only. The hub
//! scheduler and the stores come in later PRs, so nothing calls this yet.
//!
//! `next_after` is the one place that turns a [`Schedule`] and a time zone into unix time. The hub
//! evaluates with `chrono-tz`: a local time that a DST change skips runs at the first valid minute
//! after it, and a local time that happens twice runs once, at the first.

use chrono::{DateTime, Duration, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::preauth::never::NeverItem;
use crate::surface::{ActorRole, Parsed};
use crate::wire::HubCall;

/// The grace window of a task that sets none: how late a run may still be delivered and count as on time.
/// The one place this number lives; ask `Task::effective_grace`.
pub const DEFAULT_GRACE_MINS: u32 = 15;

/// The pickup window of a task that sets none: how long a delivered run may sit before it counts as
/// not picked up. The one place this number lives; ask `Task::effective_pickup`.
pub const DEFAULT_PICKUP_MINS: u32 = 10;

/// How many run times `next_runs` answers for a dry run.
pub const DRY_RUN_COUNT: usize = 3;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub schedule: Schedule,
    /// IANA name, as `America/Sao_Paulo`; set from the creator's device.
    pub tz: String,
    pub target: Target,
    pub action: Action,
    /// False means paused.
    pub enabled: bool,
    /// Minutes after `due_at` during which a run still counts as on time. None = [`DEFAULT_GRACE_MINS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grace_mins: Option<u32>,
    /// What to do with a run that was not delivered within grace. None = `OnMissed::default_for(schedule)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_missed: Option<OnMissed>,
    /// Minutes a delivered run may wait for its target to start it. None = [`DEFAULT_PICKUP_MINS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pickup_mins: Option<u32>,
    #[serde(default)]
    pub approvals: Vec<Approval>,
    #[serde(default)]
    pub fallback: Fallback,
    /// The idea it came from.
    #[serde(default)]
    pub idea: Option<String>,
    pub created_by: Actor,
    pub created_at: i64,
    pub updated_at: i64,
    /// Hub-computed, read-only.
    #[serde(default)]
    pub next_at: Option<i64>,
    #[serde(default)]
    pub last_run: Option<TaskRun>,
    /// Hub-computed, read-only: when an agent created the task and the user has not looked yet. The
    /// Inbox shows a row for it. Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fyi: Option<i64>,
}

impl Task {
    /// The grace window in minutes, with the task's own setting first.
    pub fn effective_grace(&self) -> u32 {
        self.grace_mins.unwrap_or(DEFAULT_GRACE_MINS)
    }

    /// The pickup window in minutes, with the task's own setting first.
    pub fn effective_pickup(&self) -> u32 {
        self.pickup_mins.unwrap_or(DEFAULT_PICKUP_MINS)
    }

    /// The missed-run policy, with the task's own setting first.
    pub fn effective_on_missed(&self) -> OnMissed {
        self.on_missed.unwrap_or_else(|| OnMissed::default_for(&self.schedule))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Schedule {
    /// A wall-clock date and time in the task's zone: `2026-11-13T09:00`.
    Once { at: String },
    /// Five-field cron, evaluated in the task's zone.
    Cron { expr: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Target {
    /// The user: a notification on every device, with no agent run.
    Me,
    /// The global Orchestrator, on the machine that hosts it.
    Orchestrator,
    /// A project's Captain, by repo key; an empty machine means wherever `send_to_captain` would pick.
    Captain {
        project: String,
        #[serde(default)]
        machine: String,
    },
    /// An existing conversation, by Claude session id, so it survives tab restarts.
    Session {
        session: String,
        #[serde(default)]
        machine: String,
    },
    /// A new worker in a project, placed like `delegate_task`.
    Worker {
        project: String,
        #[serde(default)]
        model: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Action {
    /// Text the target receives as one prompt.
    Prompt { text: String },
    /// Run now of a saved workflow. The target is ignored: the workflow's own `runs_on` decides.
    Workflow { id: String },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OnMissed {
    /// The run becomes `missed` and the user decides: Run now or Skip.
    Ask,
    /// Record the run as `skipped`; the next occurrence comes as usual.
    Skip,
    /// Run as soon as a path opens, whenever that is.
    RunLate,
}

impl OnMissed {
    /// The policy of a task that sets none: Ask for a one-time task, Skip for a repeating one.
    /// The only place these defaults are written.
    pub fn default_for(schedule: &Schedule) -> OnMissed {
        match schedule {
            Schedule::Once { .. } => OnMissed::Ask,
            Schedule::Cron { .. } => OnMissed::Skip,
        }
    }
}

/// Where a run goes when its target's machine is not there.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Fallback {
    /// Wait within the grace window. A box costs money, so this is the default.
    #[default]
    Wait,
    /// Open a box, for the targets that allow it.
    Box,
}

/// A step the task may take without asking: that item, in that repo, when the command contains the text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Approval {
    pub item: NeverItem,
    pub must_contain: String,
    pub repo: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct TaskRun {
    pub task: String,
    pub due_at: i64,
    #[serde(default)]
    pub started_at: Option<i64>,
    pub state: RunState,
    #[serde(default)]
    pub machine: String,
    #[serde(default)]
    pub tab: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    Due,
    Delivered,
    Queued,
    Held,
    Done,
    Failed,
    Missed,
    Skipped,
    Lost,
}

/// Who created a task or an idea: the user, or an agent by the part it plays.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Actor {
    User,
    Agent {
        role: ActorRole,
        #[serde(default)]
        machine: String,
        #[serde(default)]
        tab: String,
        #[serde(default)]
        session_title: String,
    },
}

fn parse_tz(tz: &str) -> Result<Tz, String> {
    tz.trim().parse::<Tz>().map_err(|_| {
        format!("unknown time zone \"{tz}\": use an IANA name such as America/Sao_Paulo")
    })
}

/// Checks the schedule and the zone without computing a time, for a save that should refuse early.
pub fn validate(schedule: &Schedule, tz: &str) -> Result<(), String> {
    parse_tz(tz)?;
    match schedule {
        Schedule::Once { at } => parse_local(at).map(|_| ()),
        Schedule::Cron { expr } => parse_cron(expr).map(|_| ()),
    }
}

fn parse_local(at: &str) -> Result<NaiveDateTime, String> {
    let at = at.trim();
    NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| format!("\"{at}\" is not a date and time: write it as 2026-11-13T09:00"))
}

fn parse_cron(expr: &str) -> Result<croner::Cron, String> {
    let fields = expr.split_whitespace().count();
    if fields != 5 {
        return Err(format!(
            "cron \"{}\" has {fields} fields: write five (minute hour day-of-month month day-of-week)",
            expr.trim()
        ));
    }
    expr.parse::<croner::Cron>()
        .map_err(|e| format!("cron \"{}\" is not valid: {e}", expr.trim()))
}

/// A wall-clock time in `tz` as unix time. A time that happens twice gives the first; a time a DST
/// change skipped gives the first valid minute after it.
fn resolve_local(tz: Tz, mut local: NaiveDateTime) -> Result<i64, String> {
    // The longest gap in the tz database is a whole day (Samoa, 2011).
    for _ in 0..=24 * 60 {
        if let Some(t) = tz.from_local_datetime(&local).earliest() {
            return Ok(t.timestamp());
        }
        local += Duration::minutes(1);
    }
    Err(format!("{local} cannot be placed in {tz}"))
}

/// The first run strictly after unix time `after`, in the zone `tz`. None when there is none: a
/// one-time schedule already past, or a cron that never matches again.
pub fn next_after(schedule: &Schedule, tz: &str, after: i64) -> Result<Option<i64>, String> {
    let tz = parse_tz(tz)?;
    match schedule {
        Schedule::Once { at } => {
            let t = resolve_local(tz, parse_local(at)?)?;
            Ok((t > after).then_some(t))
        }
        Schedule::Cron { expr } => {
            let cron = parse_cron(expr)?;
            let from: DateTime<Tz> = tz
                .timestamp_opt(after, 0)
                .single()
                .ok_or_else(|| format!("{after} is not a time"))?;
            // croner reports no further match as an error, so that is None here. A pattern it
            // cannot parse was refused above.
            Ok(cron.find_next_occurrence(&from, false).ok().map(|t| t.timestamp()))
        }
    }
}

/// The next [`DRY_RUN_COUNT`] run times after `after`, for `save_schedule`'s dry run.
pub fn next_runs(schedule: &Schedule, tz: &str, after: i64) -> Result<Vec<i64>, String> {
    let mut out = Vec::with_capacity(DRY_RUN_COUNT);
    let mut at = after;
    while out.len() < DRY_RUN_COUNT {
        match next_after(schedule, tz, at)? {
            Some(t) => {
                out.push(t);
                at = t;
            }
            None => break,
        }
    }
    Ok(out)
}

/// One run time as the person reads it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct NextRun {
    /// Unix seconds.
    pub at: i64,
    /// In the task's zone: "Fri 13 Nov 09:00 -03".
    pub local: String,
}

/// What `save_schedule` answers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct SaveAnswer {
    /// The task as stored; None for a dry run, which stores nothing.
    pub task: Option<Task>,
    /// The next three run times in the task's zone.
    pub next: Vec<NextRun>,
}

/// `next_runs` with each time written in the zone `tz`, for the answer to a save.
pub fn next_run_times(schedule: &Schedule, tz: &str, after: i64) -> Result<Vec<NextRun>, String> {
    let zone = parse_tz(tz)?;
    Ok(next_runs(schedule, tz, after)?
        .into_iter()
        .map(|at| NextRun { at, local: zone.timestamp_opt(at, 0).single().map(|t| t.format("%a %d %b %H:%M %Z").to_string()).unwrap_or_default() })
        .collect())
}

// ---- What the hub lets a save do (0.18.2 PR 4) ----

/// The shortest interval an agent's cron may repeat at (design 4.4). The user has no such floor.
pub const AGENT_MIN_INTERVAL_MINS: i64 = 15;

/// The most tasks one account may hold that agents created (design 4.4).
pub const AGENT_MAX_TASKS: usize = 20;

/// Whether `Target::Worker` may be saved. PR 6 (held steps) flips this: until a run's never-list hits
/// can be held, a task may not start workers.
pub const WORKER_TARGETS_OPEN: bool = false;

/// Whether a task an agent creates waits for the user's OK. It does not: it is active at once and the
/// user gets a notice (`Task::fyi`, an Inbox row, an audit row). The one place that rule lives:
/// setting this to true makes `save_schedule` a Confirm for every new task an agent creates.
pub const AGENT_TASK_NEEDS_OK: bool = false;

/// Why `save_schedule` from an agent waits for the user, or None when it runs at once. Approvals
/// always wait; a new task waits only while [`AGENT_TASK_NEEDS_OK`] is true.
pub fn save_needs_ok(task: &Task) -> Option<String> {
    if !task.approvals.is_empty() {
        let list: Vec<String> = task.approvals.iter().map(|a| format!("{} containing \"{}\"", a.item.label(), a.must_contain)).collect();
        return Some(format!("task \"{}\" would run without asking for: {}", task.title, list.join("; ")));
    }
    (AGENT_TASK_NEEDS_OK && task.id.trim().is_empty()).then(|| format!("an agent wants to schedule \"{}\"", task.title))
}

/// The argument check of an agent's `save_schedule`: what needs no stored state, [`agent_floor`].
pub fn agent_check(call: &Parsed) -> Result<(), String> {
    let Parsed::Hub(HubCall::SaveSchedule { task, .. }) = call else { return Ok(()) };
    agent_floor(task)
}

/// An agent's cron may not repeat more often than [`AGENT_MIN_INTERVAL_MINS`]. The hub checks it
/// again on every save an agent makes, whatever path the save came by.
pub fn agent_floor(task: &Task) -> Result<(), String> {
    let Schedule::Cron { .. } = task.schedule else { return Ok(()) };
    if let Some(gap) = shortest_gap(&task.schedule, &task.tz, now())? {
        if gap < AGENT_MIN_INTERVAL_MINS * 60 {
            return Err(format!(
                "that schedule repeats every {} minutes: an agent may not schedule a task more often than every {AGENT_MIN_INTERVAL_MINS} minutes",
                (gap / 60).max(1)
            ));
        }
    }
    Ok(())
}

/// Whether `call` is one of the schedule calls, which the hub answers with the caller's `Actor`.
pub fn is_schedule_call(call: &HubCall) -> bool {
    matches!(
        call,
        HubCall::ListSchedules | HubCall::SaveSchedule { .. } | HubCall::PauseSchedule { .. } | HubCall::DeleteSchedule { .. } | HubCall::ScheduleRuns { .. }
    )
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The shortest gap in seconds between two runs of `schedule` after `from`, over its next 400 runs.
/// None for a one-time schedule or one with fewer than two runs.
pub fn shortest_gap(schedule: &Schedule, tz: &str, from: i64) -> Result<Option<i64>, String> {
    let mut at = from;
    let mut last: Option<i64> = None;
    let mut gap: Option<i64> = None;
    for _ in 0..400 {
        let Some(t) = next_after(schedule, tz, at)? else { break };
        if let Some(before) = last {
            gap = Some(gap.map_or(t - before, |g| g.min(t - before)));
        }
        last = Some(t);
        at = t;
    }
    Ok(gap)
}

/// Whether the text names a tag: "tag", "refs/tags", or a version such as v0.18.2.
fn covers_tag(text: &str) -> bool {
    let text = text.to_lowercase();
    text.contains("tag") || text.split(|c: char| !c.is_alphanumeric() && c != '.').any(|w| w.len() > 1 && w.starts_with('v') && w[1..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Whether an approval needs tag and main protection to exist before it may be granted.
fn needs_protection(approval: &Approval) -> bool {
    matches!(approval.item, NeverItem::Release | NeverItem::Deploy | NeverItem::PushToMain) || covers_tag(&approval.must_contain)
}

/// What a save must satisfy whoever sends it, the user's app or an agent. `protected` is the
/// account's release-protection switch: tag and main protection exist.
pub fn check_save(task: &Task, protected: bool) -> Result<(), String> {
    if task.title.trim().is_empty() {
        return Err("a task needs a title".into());
    }
    validate(&task.schedule, &task.tz)?;
    match &task.action {
        Action::Prompt { text } if text.trim().is_empty() => return Err("a prompt task needs its text".into()),
        Action::Workflow { id } if id.trim().is_empty() => return Err("a workflow task needs the workflow's id".into()),
        _ => {}
    }
    if matches!(task.target, Target::Worker { .. }) && !WORKER_TARGETS_OPEN {
        return Err("a task cannot start a new worker yet: pick Me, the Orchestrator, a Captain or a session. Workers wait for held steps, so a run can stop for the user".into());
    }
    for approval in &task.approvals {
        if approval.must_contain.trim().is_empty() {
            return Err("an approval needs the text the command must contain: \"any release\" is not allowed".into());
        }
        if matches!(approval.item, NeverItem::Secrets | NeverItem::SelfGrant | NeverItem::Payment) {
            return Err(format!("{} can never be approved ahead of time: it always stops for the user", approval.item.label()));
        }
        if !protected && needs_protection(approval) {
            return Err("pre-approving a release needs tag and main protection; until Felipe turns that on, a release always stops for him".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cron(expr: &str) -> Schedule {
        Schedule::Cron { expr: expr.into() }
    }
    fn once(at: &str) -> Schedule {
        Schedule::Once { at: at.into() }
    }
    /// A wall-clock time in a zone as unix time.
    fn ts(tz: &str, local: &str) -> i64 {
        resolve_local(tz.parse().unwrap(), parse_local(local).unwrap()).unwrap()
    }
    /// Unix time as local text in a zone, with the offset, for readable assertions.
    fn show(tz: &str, t: i64) -> String {
        let tz: Tz = tz.parse().unwrap();
        tz.timestamp_opt(t, 0).unwrap().format("%Y-%m-%dT%H:%M%:z").to_string()
    }
    fn next(s: &Schedule, tz: &str, after: i64) -> String {
        show(tz, next_after(s, tz, after).unwrap().expect("a next run"))
    }

    const SP: &str = "America/Sao_Paulo";
    const NY: &str = "America/New_York";

    #[test]
    fn once_in_the_future_and_past() {
        let s = once("2026-11-13T09:00");
        let t = ts(SP, "2026-11-13T09:00");
        assert_eq!(next_after(&s, SP, t - 1).unwrap(), Some(t));
        assert_eq!(next_after(&s, SP, t).unwrap(), None, "strictly after");
        assert_eq!(next_after(&s, SP, t + 3600).unwrap(), None);
        // 09:00 in Sao Paulo (UTC-3 in November 2026) is 12:00 UTC.
        assert_eq!(t, 1_794_571_200);
        assert_eq!(next_runs(&s, SP, t - 1).unwrap(), vec![t]);
    }

    #[test]
    fn cron_daily_and_weekdays() {
        let after = ts(NY, "2026-11-13T09:00"); // a Friday
        assert_eq!(next(&cron("0 9 * * *"), NY, after), "2026-11-14T09:00-05:00");
        let weekdays = cron("0 9 * * 1-5");
        assert_eq!(next(&weekdays, NY, after), "2026-11-16T09:00-05:00", "skips the weekend");
        let runs = next_runs(&weekdays, NY, after).unwrap();
        let shown: Vec<_> = runs.iter().map(|t| show(NY, *t)).collect();
        assert_eq!(
            shown,
            ["2026-11-16T09:00-05:00", "2026-11-17T09:00-05:00", "2026-11-18T09:00-05:00"]
        );
    }

    #[test]
    fn tomorrow_nine_across_midnight_in_a_non_utc_zone() {
        // 23:30 on the 12th in Sao Paulo is already the 13th in UTC.
        let now = ts(SP, "2026-11-12T23:30");
        assert_eq!(show("UTC", now), "2026-11-13T02:30+00:00");
        // An agent turns "tomorrow 09:00" into the local date it computed in the creator's zone.
        let s = once("2026-11-13T09:00");
        assert_eq!(next(&s, SP, now), "2026-11-13T09:00-03:00");
        assert_eq!(show("UTC", next_after(&s, SP, now).unwrap().unwrap()), "2026-11-13T12:00+00:00");
        // The same cron at 09:00 local after 23:30 local lands on the next local day.
        assert_eq!(next(&cron("0 9 * * *"), SP, now), "2026-11-13T09:00-03:00");
    }

    #[test]
    fn dst_gap_runs_at_first_valid_minute_after() {
        // Sao Paulo, 2018-11-04: clocks jump 00:00 -> 01:00, so 00:30 does not exist.
        let after = ts(SP, "2018-11-03T12:00");
        assert_eq!(next(&cron("30 0 * * *"), SP, after), "2018-11-04T01:00-02:00");
        assert_eq!(next(&once("2018-11-04T00:30"), SP, after), "2018-11-04T01:00-02:00");
        // The day after it is back to 00:30.
        let run = next_after(&cron("30 0 * * *"), SP, after).unwrap().unwrap();
        assert_eq!(next(&cron("30 0 * * *"), SP, run), "2018-11-05T00:30-02:00");
        // New York, 2018-03-11: 02:00 -> 03:00.
        let after = ts(NY, "2018-03-10T12:00");
        assert_eq!(next(&cron("30 2 * * *"), NY, after), "2018-03-11T03:00-04:00");
        assert_eq!(next(&once("2018-03-11T02:30"), NY, after), "2018-03-11T03:00-04:00");
    }

    #[test]
    fn dst_overlap_runs_once_at_the_first() {
        // Sao Paulo, 2018-02-17 23:00 happens twice (clocks go back at 00:00 on the 18th).
        let after = ts(SP, "2018-02-17T12:00");
        let c = cron("30 23 * * *");
        let first = next_after(&c, SP, after).unwrap().unwrap();
        assert_eq!(show(SP, first), "2018-02-17T23:30-02:00");
        // The repeated 23:30 (-03:00) is an hour later; the next run is the following day instead.
        assert_eq!(next(&c, SP, first), "2018-02-18T23:30-03:00");
        assert_eq!(show(SP, ts(SP, "2018-02-17T23:30")), "2018-02-17T23:30-02:00", "once takes the first");
        assert_eq!(next(&once("2018-02-17T23:30"), SP, after), "2018-02-17T23:30-02:00");
        // New York, 2018-11-04: 01:30 happens twice.
        let after = ts(NY, "2018-11-03T12:00");
        let c = cron("30 1 * * *");
        let first = next_after(&c, NY, after).unwrap().unwrap();
        assert_eq!(show(NY, first), "2018-11-04T01:30-04:00");
        assert_eq!(next(&c, NY, first), "2018-11-05T01:30-05:00");
        assert_eq!(next(&once("2018-11-04T01:30"), NY, after), "2018-11-04T01:30-04:00");
    }

    #[test]
    fn weekly_cron_keeps_its_local_time_across_dst() {
        let runs = |s: &Schedule, after: i64| -> Vec<String> {
            next_runs(s, NY, after).unwrap().iter().map(|t| show(NY, *t)).collect()
        };
        // Sundays at 09:00 across both changes: the local time stays, the UTC offset moves.
        // New York falls back on Sunday 2026-11-01 and springs forward on Sunday 2027-03-14.
        let sunday_nine = cron("0 9 * * 0");
        assert_eq!(
            runs(&sunday_nine, ts(NY, "2026-10-20T12:00")),
            ["2026-10-25T09:00-04:00", "2026-11-01T09:00-05:00", "2026-11-08T09:00-05:00"]
        );
        assert_eq!(
            runs(&sunday_nine, ts(NY, "2027-03-01T12:00")),
            ["2027-03-07T09:00-05:00", "2027-03-14T09:00-04:00", "2027-03-21T09:00-04:00"]
        );
        // A weekly time inside the change itself: the repeated 01:30 runs once, the skipped 02:30 at
        // 03:00, and the following weeks are back at the set time.
        assert_eq!(
            runs(&cron("30 1 * * 0"), ts(NY, "2026-10-31T12:00")),
            ["2026-11-01T01:30-04:00", "2026-11-08T01:30-05:00", "2026-11-15T01:30-05:00"]
        );
        assert_eq!(
            runs(&cron("30 2 * * 0"), ts(NY, "2027-03-13T12:00")),
            ["2027-03-14T03:00-04:00", "2027-03-21T02:30-04:00", "2027-03-28T02:30-04:00"]
        );
    }

    #[test]
    fn leap_day() {
        let c = cron("0 9 29 2 *");
        let after = ts("UTC", "2026-03-01T00:00");
        assert_eq!(next(&c, "UTC", after), "2028-02-29T09:00+00:00");
        let runs = next_runs(&c, "UTC", after).unwrap();
        let shown: Vec<_> = runs.iter().map(|t| show("UTC", *t)).collect();
        assert_eq!(shown, ["2028-02-29T09:00+00:00", "2032-02-29T09:00+00:00", "2036-02-29T09:00+00:00"]);
    }

    #[test]
    fn invalid_input_is_refused_with_a_readable_error() {
        let e = next_after(&cron("not a cron"), "UTC", 0).unwrap_err();
        assert!(e.contains("5 fields") || e.contains("five"), "{e}");
        let e = next_after(&cron("61 * * * *"), "UTC", 0).unwrap_err();
        assert!(e.contains("61 * * * *") && e.contains("not valid"), "{e}");
        let e = next_after(&cron("0 0 9 * * *"), "UTC", 0).unwrap_err();
        assert!(e.contains("6 fields"), "six fields are refused: {e}");
        let e = next_after(&cron("0 9 * * *"), "Mars/Olympus", 0).unwrap_err();
        assert!(e.contains("Mars/Olympus") && e.contains("IANA"), "{e}");
        let e = next_after(&once("tomorrow 9am"), "UTC", 0).unwrap_err();
        assert!(e.contains("2026-11-13T09:00"), "{e}");
        assert!(validate(&cron("0 9 * * *"), SP).is_ok());
        assert!(validate(&cron("0 9 * * *"), "Nowhere").is_err());
        assert!(validate(&once("2026-13-45T99:99"), SP).is_err());
    }

    #[test]
    fn serde_shapes() {
        assert_eq!(
            serde_json::to_string(&cron("0 9 * * 1-5")).unwrap(),
            r#"{"kind":"cron","expr":"0 9 * * 1-5"}"#
        );
        assert_eq!(serde_json::to_string(&Target::Me).unwrap(), r#"{"kind":"me"}"#);
        assert_eq!(serde_json::to_string(&OnMissed::RunLate).unwrap(), r#""run-late""#);
        assert_eq!(serde_json::to_string(&RunState::Lost).unwrap(), r#""lost""#);
        let a = Approval { item: NeverItem::PushToMain, must_contain: "v0.18.2".into(), repo: "r".into() };
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            r#"{"item":"push-to-main","must_contain":"v0.18.2","repo":"r"}"#
        );
    }

    fn sample() -> Task {
        Task {
            id: "t1".into(),
            title: "Release pinned 0.18.2".into(),
            schedule: once("2026-11-13T09:00"),
            tz: SP.into(),
            target: Target::Captain { project: "github.com/FelipeGazapina/pinned".into(), machine: String::new() },
            action: Action::Prompt { text: "release".into() },
            enabled: true,
            grace_mins: Some(30),
            on_missed: Some(OnMissed::RunLate),
            pickup_mins: None,
            fyi: None,
            approvals: vec![Approval {
                item: NeverItem::Release,
                must_contain: "v0.18.2".into(),
                repo: "github.com/FelipeGazapina/pinned".into(),
            }],
            fallback: Fallback::Box,
            idea: Some("i1".into()),
            created_by: Actor::Agent {
                role: ActorRole::Captain,
                machine: "pinned-mac".into(),
                tab: "t".into(),
                session_title: "release".into(),
            },
            created_at: 1,
            updated_at: 2,
            next_at: Some(3),
            last_run: Some(TaskRun {
                task: "t1".into(),
                due_at: 3,
                started_at: None,
                state: RunState::Missed,
                machine: String::new(),
                tab: String::new(),
                detail: String::new(),
            }),
        }
    }

    #[test]
    fn task_round_trip() {
        let t = sample();
        let back: Task = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
        assert_eq!(back.effective_grace(), 30);
        assert_eq!(back.effective_on_missed(), OnMissed::RunLate);
    }

    #[test]
    fn older_json_without_the_optional_fields_takes_the_defaults() {
        let json = r#"{
            "id":"t1","title":"Digest","schedule":{"kind":"cron","expr":"0 9 * * 1-5"},"tz":"America/Sao_Paulo",
            "target":{"kind":"orchestrator"},"action":{"kind":"prompt","text":"digest"},"enabled":true,
            "created_by":{"kind":"user"},"created_at":1,"updated_at":2}"#;
        let t: Task = serde_json::from_str(json).unwrap();
        assert_eq!(t.grace_mins, None);
        assert_eq!(t.on_missed, None);
        assert!(t.approvals.is_empty());
        assert_eq!(t.fallback, Fallback::Wait);
        assert_eq!((&t.idea, t.next_at, &t.last_run), (&None, None, &None));
        assert_eq!(t.effective_grace(), DEFAULT_GRACE_MINS);
        assert_eq!(t.effective_grace(), 15);
        assert_eq!(t.effective_on_missed(), OnMissed::Skip);
        // Unset options stay off the wire, so an old reader of a new task sees the same shape.
        let out = serde_json::to_string(&t).unwrap();
        assert!(!out.contains("grace_mins") && !out.contains("on_missed"), "{out}");
    }

    #[test]
    fn effective_defaults_follow_the_schedule_kind_and_yield_to_the_task() {
        let mut t = sample();
        t.grace_mins = None;
        t.on_missed = None;
        assert_eq!(t.effective_grace(), DEFAULT_GRACE_MINS);
        assert_eq!(t.effective_on_missed(), OnMissed::Ask, "once");
        t.schedule = cron("0 9 * * *");
        assert_eq!(t.effective_on_missed(), OnMissed::Skip, "cron");
        t.on_missed = Some(OnMissed::Ask);
        t.grace_mins = Some(0);
        assert_eq!(t.effective_on_missed(), OnMissed::Ask);
        assert_eq!(t.effective_grace(), 0, "zero is a setting, not a missing one");
    }

    #[test]
    fn actor_round_trip() {
        let user: Actor = serde_json::from_str(r#"{"kind":"user"}"#).unwrap();
        assert_eq!(user, Actor::User);
        let agent: Actor = serde_json::from_str(r#"{"kind":"agent","role":"captain"}"#).unwrap();
        assert!(matches!(agent, Actor::Agent { role: ActorRole::Captain, .. }));
    }

    fn plain() -> Task {
        Task { approvals: vec![], target: Target::Me, schedule: cron("0 9 * * 1-5"), ..sample() }
    }

    #[test]
    fn pickup_default_lives_in_one_place() {
        let mut t = sample();
        assert_eq!(t.effective_pickup(), DEFAULT_PICKUP_MINS);
        t.pickup_mins = Some(2);
        assert_eq!(t.effective_pickup(), 2);
        let old: Task = serde_json::from_str(&serde_json::to_string(&plain()).unwrap()).unwrap();
        assert_eq!(old.pickup_mins, None);
    }

    #[test]
    fn a_save_is_checked_whoever_sends_it() {
        assert_eq!(check_save(&plain(), false), Ok(()));
        assert!(check_save(&Task { title: " ".into(), ..plain() }, false).is_err());
        assert!(check_save(&Task { tz: "Mars/Base".into(), ..plain() }, false).unwrap_err().contains("time zone"));
        let worker = Task { target: Target::Worker { project: "p".into(), model: String::new() }, ..plain() };
        assert!(check_save(&worker, true).unwrap_err().contains("new worker"));
    }

    #[test]
    fn release_approvals_need_protection() {
        let approval = |item, text: &str| Approval { item, must_contain: text.into(), repo: "r".into() };
        for item in [NeverItem::Release, NeverItem::Deploy, NeverItem::PushToMain] {
            let t = Task { approvals: vec![approval(item, "x")], ..plain() };
            assert!(check_save(&t, false).unwrap_err().contains("tag and main protection"), "{item:?}");
            assert_eq!(check_save(&t, true), Ok(()), "{item:?} with protection");
        }
        let tag = Task { approvals: vec![approval(NeverItem::NetworkInstall, "git push origin v0.18.2")], ..plain() };
        assert!(check_save(&tag, false).is_err(), "covers a tag");
        assert_eq!(check_save(&tag, true), Ok(()));
        let harmless = Task { approvals: vec![approval(NeverItem::NetworkInstall, "npm install left-pad")], ..plain() };
        assert_eq!(check_save(&harmless, false), Ok(()));
        for item in [NeverItem::Secrets, NeverItem::SelfGrant, NeverItem::Payment] {
            assert!(check_save(&Task { approvals: vec![approval(item, "x")], ..plain() }, true).is_err(), "{item:?} never");
        }
        assert!(check_save(&Task { approvals: vec![approval(NeverItem::Release, " ")], ..plain() }, true).is_err());
    }

    #[test]
    fn an_agent_may_not_repeat_faster_than_every_15_minutes() {
        let save = |expr: &str| Parsed::Hub(HubCall::SaveSchedule { task: Task { schedule: cron(expr), ..plain() }, dry_run: false });
        assert!(agent_check(&save("*/5 * * * *")).unwrap_err().contains("every 5 minutes"));
        assert!(agent_check(&save("0,10 9 * * *")).is_err(), "two runs ten minutes apart");
        assert_eq!(agent_check(&save("*/15 * * * *")), Ok(()));
        assert_eq!(agent_check(&save("0 9 * * 1-5")), Ok(()));
        assert_eq!(agent_check(&Parsed::Hub(HubCall::ListSchedules)), Ok(()));
    }

    #[test]
    fn approvals_make_an_agent_save_wait_and_a_plain_one_run() {
        assert!(save_needs_ok(&sample()).unwrap().contains("release"));
        assert_eq!(save_needs_ok(&Task { id: String::new(), ..plain() }), None, "an agent's new task needs no OK");
    }

    #[test]
    fn next_run_times_are_written_in_the_task_zone() {
        let runs = next_run_times(&once("2026-11-13T09:00"), SP, 0).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].local, "Fri 13 Nov 09:00 -03");
    }
}
