//! What each delegated task cost and how it ended, kept on the task so a finished one still has
//! it, and the totals a delegator reads to pick its next worker.

use super::*;
use std::collections::BTreeMap;

pub use crate::usage::TaskUsage;

impl TaskUsage {
    /// Agent spend and box spend together, when either is known.
    pub fn total_usd(&self) -> Option<f64> {
        match (self.cost_usd, self.box_usd) {
            (None, None) => None,
            (cost, boxed) => Some(cost.unwrap_or(0.0) + boxed.unwrap_or(0.0)),
        }
    }
}

/// One usage report from a child's stream: one finished turn's tokens, and the process's running
/// cost when the provider prints it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageReport {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DelegatorKind {
    GlobalOrchestrator,
    Captain,
    Worker,
    /// A thread with no role: the user's own session.
    #[default]
    Session,
}

/// The session that started a task.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct Delegator {
    pub kind: DelegatorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    /// The project's repo (`host/owner/name`), else its folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
}

impl Delegator {
    pub fn label(&self) -> String {
        let name = self.project_name.as_deref();
        match (self.kind, name) {
            (DelegatorKind::GlobalOrchestrator, _) => "Global orchestrator".into(),
            (DelegatorKind::Captain, Some(name)) => format!("Captain {name}"),
            (DelegatorKind::Captain, None) => "Captain".into(),
            (DelegatorKind::Worker, Some(name)) => format!("Worker in {name}"),
            (DelegatorKind::Worker, None) => "Worker".into(),
            (DelegatorKind::Session, Some(name)) => format!("Session in {name}"),
            (DelegatorKind::Session, None) => "Session".into(),
        }
    }

    fn key(&self) -> String {
        let kind = serde_json::to_value(self.kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        match (self.kind, &self.project) {
            (DelegatorKind::GlobalOrchestrator, _) | (_, None) => kind,
            (_, Some(project)) => format!("{kind}:{project}"),
        }
    }
}

/// Where the task ran, from its delegator's side.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunTarget {
    #[default]
    Local,
    /// On another machine of the account than its delegator's, which `machine` names.
    Machine,
    #[serde(rename = "box")]
    UpstashBox,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Done,
    Failed,
    /// pinned stopped it at one of its limits, which `limit` names.
    Limit,
    Cancelled,
}

impl TaskOutcome {
    pub const ALL: [TaskOutcome; 4] = [TaskOutcome::Done, TaskOutcome::Failed, TaskOutcome::Limit, TaskOutcome::Cancelled];

    pub fn name(self) -> &'static str {
        match self {
            TaskOutcome::Done => "done",
            TaskOutcome::Failed => "failed",
            TaskOutcome::Limit => "limit",
            TaskOutcome::Cancelled => "cancelled",
        }
    }

    fn label(self) -> &'static str {
        match self {
            TaskOutcome::Done => "Done",
            TaskOutcome::Failed => "Failed",
            TaskOutcome::Limit => "Stopped by a limit",
            TaskOutcome::Cancelled => "Cancelled",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Accepted,
    Redone,
}

impl Verdict {
    pub fn parse(text: &str) -> Result<Verdict, String> {
        match text.trim() {
            "accepted" => Ok(Verdict::Accepted),
            "redone" => Ok(Verdict::Redone),
            other => Err(format!("verdict must be accepted or redone, not {other:?}")),
        }
    }
}

/// Whether the delegator used the result. `auto` marks one pinned set from a signal: the
/// task's pull request merged, or the same delegator started a task with the same title again.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Review {
    pub verdict: Verdict,
    #[serde(default)]
    pub auto: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Unix milliseconds.
    pub at: u64,
}

/// One finished delegated task, as the hub keeps it for the account.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct TaskRecord {
    pub id: String,
    /// The machine whose registry holds the task. The hub stamps both from the socket.
    #[serde(default)]
    pub machine: String,
    #[serde(default)]
    pub machine_name: String,
    pub title: String,
    pub delegator: Delegator,
    pub provider: Provider,
    pub account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The model's cost tier: low, mid or high.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub target: RunTarget,
    /// Unix milliseconds.
    pub started: u64,
    pub ended: u64,
    pub usage: TaskUsage,
    pub outcome: TaskOutcome,
    /// Why it failed or stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The limit that stopped it: `time`, or `budget` once spend is enforced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    /// The account the request asked for, when a usage limit moved the task to `account`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_account: Option<String>,
    /// The model the request asked for, when a usage limit moved the task to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_model: Option<String>,
    /// Why the task ran elsewhere than asked: the limit that stopped the account it asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_because: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    /// The worker thread that showed it, on `machine`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

/// The tasks of one model, project, delegator or outcome.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct Group {
    pub key: String,
    pub label: String,
    pub runs: u32,
    pub done: u32,
    pub failed: u32,
    pub limited: u32,
    /// Tasks a usage limit moved off the account or model they asked for.
    #[serde(default)]
    pub moved: u32,
    pub cancelled: u32,
    pub accepted: u32,
    pub redone: u32,
    pub wall_ms: u64,
    pub median_wall_ms: u64,
    /// The sum over tasks whose cost is known; `priced` counts them.
    pub cost_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_cost_usd: Option<f64>,
    pub priced: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub box_usd: f64,
    pub box_ms: u64,
    /// How many tasks each limit stopped, by its name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub limits: BTreeMap<String, u32>,
}

/// One task in the slowest and most expensive lists.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct TaskBrief {
    pub id: String,
    pub title: String,
    pub machine: String,
    #[serde(default)]
    pub machine_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub delegator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub outcome: TaskOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    pub wall_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub ended: u64,
}

/// The account's delegated tasks that ended in the last `days`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct DelegationStats {
    pub days: u32,
    /// Unix milliseconds: tasks that ended from then on.
    pub since: u64,
    pub totals: Group,
    pub by_model: Vec<Group>,
    pub by_provider: Vec<Group>,
    pub by_project: Vec<Group>,
    pub by_delegator: Vec<Group>,
    pub by_outcome: Vec<Group>,
    pub slowest: Vec<TaskBrief>,
    pub priciest: Vec<TaskBrief>,
    /// Stored task records that did not read, so the totals leave them out.
    #[serde(default)]
    pub unreadable: u32,
}

pub const STATS_DAYS: [u32; 3] = [1, 7, 30];
const TOP: usize = 5;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The totals of `records` that ended within `days` of `now` (unix ms).
pub fn aggregate(records: &[TaskRecord], now: u64, days: u32) -> DelegationStats {
    let since = now.saturating_sub(u64::from(days) * DAY_MS);
    let kept: Vec<&TaskRecord> = records.iter().filter(|r| r.ended >= since).collect();
    let group_by = |key: &dyn Fn(&TaskRecord) -> (String, String)| -> Vec<Group> {
        let mut groups: BTreeMap<String, (String, Vec<&TaskRecord>)> = BTreeMap::new();
        for record in &kept {
            let (key, label) = key(record);
            groups.entry(key).or_insert_with(|| (label, vec![])).1.push(record);
        }
        let mut out: Vec<Group> = groups.into_iter().map(|(key, (label, records))| group(key, label, &records)).collect();
        out.sort_by(|a, b| b.runs.cmp(&a.runs).then_with(|| a.key.cmp(&b.key)));
        out
    };
    let by_model = group_by(&|r| match &r.model {
        Some(model) => (model.clone(), model.clone()),
        None => (format!("default:{}", r.account), format!("{}'s default", r.account)),
    });
    let by_provider = group_by(&|r| (r.account.clone(), format!("{} ({})", r.account, r.provider.spec().name)));
    let by_project = group_by(&|r| match (&r.delegator.project, &r.delegator.project_name) {
        (Some(project), name) => (project.clone(), name.clone().unwrap_or_else(|| project.clone())),
        (None, _) => ("".into(), "No project".into()),
    });
    let by_delegator = group_by(&|r| (r.delegator.key(), r.delegator.label()));
    let mut by_outcome = group_by(&|r| (r.outcome.name().into(), r.outcome.label().into()));
    by_outcome.sort_by_key(|g| TaskOutcome::ALL.iter().position(|o| o.name() == g.key));
    let brief = |r: &&TaskRecord| TaskBrief {
        id: r.id.clone(),
        title: r.title.clone(),
        machine: r.machine.clone(),
        machine_name: r.machine_name.clone(),
        thread: r.thread.clone(),
        model: r.model.clone(),
        delegator: r.delegator.label(),
        project: r.delegator.project_name.clone().or_else(|| r.delegator.project.clone()),
        outcome: r.outcome,
        limit: r.limit.clone(),
        wall_ms: r.usage.wall_ms,
        cost_usd: r.usage.total_usd(),
        ended: r.ended,
    };
    let mut slowest = kept.clone();
    slowest.sort_by(|a, b| b.usage.wall_ms.cmp(&a.usage.wall_ms).then_with(|| a.id.cmp(&b.id)));
    let mut priciest: Vec<&TaskRecord> = kept.iter().copied().filter(|r| r.usage.total_usd().is_some_and(|usd| usd > 0.0)).collect();
    priciest.sort_by(|a, b| b.usage.total_usd().partial_cmp(&a.usage.total_usd()).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
    DelegationStats {
        days,
        since,
        totals: group("all".into(), "All tasks".into(), &kept),
        by_model,
        by_provider,
        by_project,
        by_delegator,
        by_outcome,
        slowest: slowest.iter().take(TOP).map(brief).collect(),
        priciest: priciest.iter().take(TOP).map(brief).collect(),
        unreadable: 0,
    }
}

fn group(key: String, label: String, records: &[&TaskRecord]) -> Group {
    let mut g = Group { key, label, ..Group::default() };
    let mut walls = vec![];
    let mut costs = vec![];
    for r in records {
        g.runs += 1;
        match r.outcome {
            TaskOutcome::Done => g.done += 1,
            TaskOutcome::Failed => g.failed += 1,
            TaskOutcome::Limit => g.limited += 1,
            TaskOutcome::Cancelled => g.cancelled += 1,
        }
        g.moved += u32::from(r.moved_because.is_some());
        match r.review.as_ref().map(|review| review.verdict) {
            Some(Verdict::Accepted) => g.accepted += 1,
            Some(Verdict::Redone) => g.redone += 1,
            None => {}
        }
        if let (TaskOutcome::Limit, Some(limit)) = (r.outcome, &r.limit) {
            *g.limits.entry(limit.clone()).or_default() += 1;
        }
        g.wall_ms += r.usage.wall_ms;
        walls.push(r.usage.wall_ms);
        if let Some(cost) = r.usage.cost_usd {
            g.cost_usd += cost;
            g.priced += 1;
            costs.push(cost);
        }
        g.tokens_in += r.usage.tokens_in;
        g.tokens_out += r.usage.tokens_out;
        g.box_usd += r.usage.box_usd.unwrap_or(0.0);
        g.box_ms += r.usage.box_ms.unwrap_or(0);
    }
    walls.sort_unstable();
    g.median_wall_ms = median(&walls.iter().map(|ms| *ms as f64).collect::<Vec<_>>()).map_or(0, |ms| ms.round() as u64);
    costs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    g.median_cost_usd = median(&costs);
    g
}

fn median(sorted: &[f64]) -> Option<f64> {
    match sorted.len() {
        0 => None,
        n if n % 2 == 1 => Some(sorted[n / 2]),
        n => Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2.0),
    }
}

impl DelegationStats {
    /// The few numbers a delegator weighs when it picks a worker, per model: how many ran, how
    /// many finished, how long and how much the middle one took, and which limits stopped them.
    pub fn brief(&self) -> Value {
        let row = |g: &Group| {
            let mut row = json!({
                "runs": g.runs,
                "doneRate": rate(g.done, g.runs),
                "medianMinutes": round(g.median_wall_ms as f64 / 60_000.0, 1),
            });
            if let Some(cost) = g.median_cost_usd {
                row["medianUsd"] = json!(round(cost, 2));
            }
            if g.limited > 0 {
                row["stoppedByLimit"] = json!(g.limits);
            }
            if g.moved > 0 {
                row["movedByLimit"] = json!(g.moved);
            }
            if g.accepted + g.redone > 0 {
                row["accepted"] = json!(g.accepted);
                row["redone"] = json!(g.redone);
            }
            row
        };
        let models: serde_json::Map<String, Value> = self.by_model.iter().map(|g| (g.label.clone(), row(g))).collect();
        let outcomes: serde_json::Map<String, Value> = self.by_outcome.iter().map(|g| (g.key.clone(), json!(g.runs))).collect();
        let mut brief = json!({
            "days": self.days,
            "all": row(&self.totals),
            "costUsd": round(self.totals.cost_usd + self.totals.box_usd, 2),
            "byModel": models,
            "outcomes": outcomes,
        });
        if self.totals.box_usd > 0.0 {
            brief["boxUsd"] = json!(round(self.totals.box_usd, 2));
        }
        if self.unreadable > 0 {
            brief["unreadableRecords"] = json!(self.unreadable);
        }
        brief
    }
}

fn rate(part: u32, whole: u32) -> f64 {
    if whole == 0 { 0.0 } else { round(f64::from(part) / f64::from(whole), 2) }
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// The period `delegation_stats` reads: one of 1, 7 or 30 days, 7 when not given.
pub fn stats_days(asked: Option<u64>) -> Result<u32, String> {
    match asked {
        None => Ok(7),
        Some(days) => STATS_DAYS.iter().copied().find(|d| u64::from(*d) == days).ok_or_else(|| format!("days must be 1, 7 or 30, not {days}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 30 * DAY_MS;

    fn record(id: &str, model: &str, project: &str, kind: DelegatorKind, outcome: TaskOutcome, minutes: u64, cost: Option<f64>) -> TaskRecord {
        TaskRecord {
            id: id.into(),
            machine: "m1".into(),
            machine_name: "Mac mini".into(),
            title: format!("task {id}"),
            delegator: Delegator { kind, tab: Some("tab-1".into()), project: Some(format!("github.com/acme/{project}")), project_name: Some(project.into()) },
            provider: Provider::Claude,
            account: "claude".into(),
            model: Some(model.into()),
            tier: None,
            effort: None,
            target: RunTarget::Local,
            started: BASE,
            ended: BASE + minutes * 60_000,
            usage: TaskUsage { tokens_in: 100, tokens_out: 10, cost_usd: cost, wall_ms: minutes * 60_000, box_usd: None, box_ms: None },
            outcome,
            reason: None,
            limit: (outcome == TaskOutcome::Limit).then(|| "time".into()),
            asked_account: None,
            asked_model: None,
            moved_because: None,
            review: None,
            thread: None,
        }
    }

    #[test]
    fn totals_break_down_by_model_project_delegator_and_outcome() {
        let mut records = vec![
            record("a", "claude-opus-5-5", "site", DelegatorKind::Captain, TaskOutcome::Done, 10, Some(1.0)),
            record("b", "claude-opus-5-5", "site", DelegatorKind::Captain, TaskOutcome::Limit, 120, Some(4.0)),
            record("c", "claude-opus-5-5", "pinned", DelegatorKind::GlobalOrchestrator, TaskOutcome::Limit, 120, Some(5.0)),
            record("d", "glm-5.3", "pinned", DelegatorKind::Worker, TaskOutcome::Failed, 3, None),
            record("e", "glm-5.3", "pinned", DelegatorKind::Captain, TaskOutcome::Cancelled, 5, Some(0.5)),
        ];
        records[0].review = Some(Review { verdict: Verdict::Accepted, auto: false, note: None, at: 0 });
        records[3].review = Some(Review { verdict: Verdict::Redone, auto: true, note: None, at: 0 });
        let mut old = record("z", "glm-5.3", "site", DelegatorKind::Captain, TaskOutcome::Done, 1, Some(9.0));
        old.ended = 0;
        records.push(old);
        let now = BASE + 200 * 60_000;
        let stats = aggregate(&records, now, 7);

        let t = &stats.totals;
        assert_eq!((t.runs, t.done, t.failed, t.limited, t.cancelled, t.accepted, t.redone), (5, 1, 1, 2, 1, 1, 1), "z ended before the period");
        assert_eq!((t.cost_usd, t.priced, t.median_cost_usd), (10.5, 4, Some(2.5)));
        assert_eq!((t.wall_ms, t.median_wall_ms), (258 * 60_000, 10 * 60_000));

        let opus = &stats.by_model[0];
        assert_eq!((opus.key.as_str(), opus.runs, opus.limited, opus.limits.get("time")), ("claude-opus-5-5", 3, 2, Some(&2)));
        assert_eq!(stats.by_model[1].key, "glm-5.3");
        assert_eq!(stats.by_project.iter().map(|g| (g.label.as_str(), g.runs)).collect::<Vec<_>>(), [("pinned", 3), ("site", 2)]);
        assert_eq!(
            stats.by_delegator.iter().map(|g| (g.label.as_str(), g.runs)).collect::<Vec<_>>(),
            [("Captain site", 2), ("Captain pinned", 1), ("Global orchestrator", 1), ("Worker in pinned", 1)]
        );
        assert_eq!(stats.by_outcome.iter().map(|g| (g.key.as_str(), g.runs)).collect::<Vec<_>>(), [("done", 1), ("failed", 1), ("limit", 2), ("cancelled", 1)]);
        assert_eq!(stats.slowest.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["b", "c", "a", "e", "d"]);
        assert_eq!(stats.priciest.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["c", "b", "a", "e"], "d has no price");

        let brief = stats.brief();
        assert_eq!(brief["byModel"]["claude-opus-5-5"], json!({ "runs": 3, "doneRate": 0.33, "medianMinutes": 120.0, "medianUsd": 4.0, "stoppedByLimit": { "time": 2 }, "accepted": 1, "redone": 0 }));
        assert_eq!(brief["byModel"]["glm-5.3"], json!({ "runs": 2, "doneRate": 0.0, "medianMinutes": 4.0, "medianUsd": 0.5, "accepted": 0, "redone": 1 }));
        assert_eq!(brief["outcomes"], json!({ "done": 1, "failed": 1, "limit": 2, "cancelled": 1 }));
        assert_eq!(brief["costUsd"], json!(10.5));
    }

    #[test]
    fn box_spend_counts_toward_the_total_and_the_period_is_one_of_three() {
        let mut boxed = record("x", "claude-sonnet-5-5", "site", DelegatorKind::Captain, TaskOutcome::Done, 30, Some(0.4));
        boxed.usage.box_usd = Some(0.6);
        boxed.usage.box_ms = Some(40 * 60_000);
        let stats = aggregate(&[boxed], BASE + 31 * 60_000, 1);
        assert_eq!((stats.totals.box_usd, stats.totals.box_ms), (0.6, 40 * 60_000));
        assert_eq!(stats.priciest[0].cost_usd, Some(1.0));
        assert_eq!(stats_days(None), Ok(7));
        assert_eq!(stats_days(Some(30)), Ok(30));
        assert_eq!(stats_days(Some(3)), Err("days must be 1, 7 or 30, not 3".into()));
    }
}
