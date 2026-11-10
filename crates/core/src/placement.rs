//! Where heavy delegated work runs. Each account keeps one policy in its way of working: machines
//! to prefer, in order, and machines to avoid. A worker in its own worktree is heavy. With no
//! target and a policy set, the caller's machine asks the hub to place a heavy worker, because
//! only the hub sees every machine's load and accounts. The hub answers with `place`, a pure
//! function over what each machine last reported.
//!
//! When no machine fits, the worker runs on the machine that asked, and the answer says why each
//! machine was skipped. Placing never fails a task.

use crate::delegate::{Isolation, Request, Target};
use crate::load::MachineState;
use serde::{Deserialize, Serialize};

/// A machine whose last report is older than this is taken as offline.
pub const STALE_SECS: i64 = 60;
/// A machine with less free disk than this takes no heavy worker.
pub const MIN_DISK_MB: u64 = 10 * 1024;
/// A machine whose one-minute load is above this percent of its cores is busy.
pub const BUSY_CPU_PERCENT: u32 = 90;
/// A machine with less memory free than this is busy.
pub const MIN_MEMORY_MB: u64 = 2 * 1024;

/// `target.machine` that keeps a worker on the caller's machine on purpose.
pub const HERE: &str = "here";
/// `target.machine` that places a worker by the policy even when it is not heavy.
pub const AUTO: &str = "auto";

/// An account's placement policy, in Settings › Way of working.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct Placement {
    /// Machine ids, best first.
    #[serde(default)]
    pub prefer: Vec<String>,
    /// Machine ids never chosen by rank. One runs a worker only as the machine that asked, when
    /// nothing else fits.
    #[serde(default)]
    pub avoid: Vec<String>,
    #[serde(default)]
    pub heavy: HeavyRule,
}

/// Which workers the policy places.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeavyRule {
    /// Workers with `isolation: "worktree"`. A shared worker works in the caller's checkout, and
    /// another machine would not see its uncommitted changes.
    #[default]
    Worktree,
    AllWorkers,
}

impl Placement {
    /// A policy with no machine in either list changes nothing.
    pub fn is_active(&self) -> bool {
        !self.prefer.is_empty() || !self.avoid.is_empty()
    }

    pub fn check(&self) -> Result<(), String> {
        if let Some(both) = self.prefer.iter().find(|id| self.avoid.contains(id)) {
            return Err(format!("machine {both} is in both Prefer and Avoid; keep it in one"));
        }
        let repeated = |ids: &[String]| ids.iter().enumerate().any(|(at, id)| ids[..at].contains(id));
        if repeated(&self.prefer) || repeated(&self.avoid) {
            return Err("a machine is listed twice; list each machine once".into());
        }
        if self.prefer.iter().chain(&self.avoid).any(|id| id.trim().is_empty()) {
            return Err("every listed machine needs an id".into());
        }
        Ok(())
    }

    /// Whether the policy places `request` when it names no target.
    pub fn is_heavy(&self, request: &Request) -> bool {
        request.heavy.unwrap_or(match self.heavy {
            HeavyRule::Worktree => request.isolation == Isolation::Worktree,
            HeavyRule::AllWorkers => true,
        })
    }
}

/// Who chose the machine a worker runs on.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum By {
    /// The request named the machine or a box.
    Explicit,
    /// The first preferred machine that fits.
    Policy,
    /// The least busy machine that fits, when no preferred one does.
    Ranked,
    /// Nothing fit, or the hub could not place it, so it runs on the machine that asked.
    Fallback,
    /// The machine that asked, with no placing: no policy, light work, or `target.machine: "here"`.
    Here,
}

/// Why a machine did not take a worker.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// It refused the start already.
    Refused,
    Avoided,
    /// Not online, or silent for more than a minute.
    Offline,
    /// Online, and reports no load: its pinned is older than placement.
    NoReport,
    NoAccount,
    NoRepo,
    LowDisk,
    Full,
    Busy,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Skip {
    pub machine: String,
    pub machine_name: String,
    pub why: Why,
    pub detail: String,
}

/// Where a worker runs and why, as the `delegate_task` answer and `task_status` show it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Placed {
    pub machine: String,
    pub machine_name: String,
    pub by: By,
    /// Empty when no policy applied, which is what every caller saw before placement.
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<Skip>,
}

/// What the caller's machine does with a `delegate_task` request.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Runs it on the caller's machine, placed as said.
    Here(By, String),
    /// Sends it to the machine or box the request names.
    Explicit,
    /// Asks the hub to place it.
    Ask,
}

/// The request's route under `policy`. An explicit machine or box always wins.
pub fn route(policy: &Placement, request: &Request) -> Route {
    let named = request.target.machine.as_deref().map(str::trim);
    if request.target.boxed.is_some() {
        return Route::Explicit;
    }
    match named {
        Some(HERE) => Route::Here(By::Here, "target.machine is \"here\"".into()),
        Some(AUTO) if policy.is_active() => Route::Ask,
        Some(AUTO) => Route::Here(By::Here, "target.machine is \"auto\", and the account has no placement policy".into()),
        Some(_) => Route::Explicit,
        None if !policy.is_active() => Route::Here(By::Here, String::new()),
        None if policy.is_heavy(request) => Route::Ask,
        None => Route::Here(By::Here, "light work stays on the machine that asked; pass heavy: true or target.machine \"auto\" to place it".into()),
    }
}

/// What a worker needs from the machine that runs it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct Need {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// One machine of the account, as the hub sees it when it places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub state: Option<MachineState>,
    /// It has a checkout of the project, or can clone one.
    pub has_repo: bool,
}

/// Places a worker for `caller` among `machines` at unix second `now`. `exclude` are machines
/// that refused this worker already.
pub fn place(policy: &Placement, caller: &str, need: &Need, machines: &[Seen], exclude: &[String], now: i64) -> Placed {
    let mut fits: Vec<&Seen> = vec![];
    let mut skipped = vec![];
    for machine in machines {
        match skip(policy, need, machine, exclude, now) {
            Some((why, detail)) => skipped.push(Skip { machine: machine.id.clone(), machine_name: machine.name.clone(), why, detail }),
            None => fits.push(machine),
        }
    }
    let placed = |machine: &Seen, by: By, reason: &str, skipped: Vec<Skip>| Placed { machine: machine.id.clone(), machine_name: machine.name.clone(), by, reason: reason.to_owned(), skipped };
    if let Some(preferred) = policy.prefer.iter().find_map(|id| fits.iter().find(|machine| machine.id == *id)) {
        return placed(preferred, By::Policy, "first preferred machine that fits", skipped);
    }
    let mut ranked: Vec<&Seen> = fits.into_iter().filter(|machine| !policy.prefer.contains(&machine.id)).collect();
    ranked.sort_by_key(|machine| {
        let state = machine.state.clone().unwrap_or_default();
        (state.workers, state.load.cpu_percent.unwrap_or(100), std::cmp::Reverse(state.load.disk_free_mb.unwrap_or(0)))
    });
    if let Some(least) = ranked.first() {
        return placed(least, By::Ranked, "no preferred machine fits, so the least busy one that does", skipped);
    }
    let name = machines.iter().find(|machine| machine.id == caller).map_or_else(|| caller.to_owned(), |machine| machine.name.clone());
    Placed { machine: caller.to_owned(), machine_name: name, by: By::Fallback, reason: "no machine fits the policy, so it runs on the machine that asked".into(), skipped }
}

fn skip(policy: &Placement, need: &Need, machine: &Seen, exclude: &[String], now: i64) -> Option<(Why, String)> {
    let name = &machine.name;
    if exclude.contains(&machine.id) {
        return Some((Why::Refused, format!("{name} refused it")));
    }
    if policy.avoid.contains(&machine.id) {
        return Some((Why::Avoided, format!("{name} is on the Avoid list")));
    }
    if !machine.online {
        return Some((Why::Offline, format!("{name} is offline")));
    }
    let Some(state) = &machine.state else {
        return Some((Why::NoReport, format!("{name} reports no load; update pinned there")));
    };
    if now - state.reported_at > STALE_SECS {
        return Some((Why::Offline, format!("{name} has not reported for {} s", now - state.reported_at)));
    }
    if let Some(refused) = lacks_account(name, need, state) {
        return Some((Why::NoAccount, refused));
    }
    if !machine.has_repo {
        return Some((Why::NoRepo, format!("{name} has no checkout of the project and can't clone one")));
    }
    if let Some(free) = state.load.disk_free_mb.filter(|free| *free < MIN_DISK_MB) {
        return Some((Why::LowDisk, format!("{name} has {} GB of disk free; a worker needs {} GB", free / 1024, MIN_DISK_MB / 1024)));
    }
    if state.workers_at_once > 0 && state.workers >= state.workers_at_once {
        return Some((Why::Full, format!("{name} runs {} workers, and its Workers at once allows {}", state.workers, state.workers_at_once)));
    }
    if let Some(cpu) = state.load.cpu_percent.filter(|cpu| *cpu > BUSY_CPU_PERCENT) {
        return Some((Why::Busy, format!("{name}'s CPU was at {cpu}% over the last minute")));
    }
    if let Some(free) = state.load.mem_available_mb.filter(|free| *free < MIN_MEMORY_MB) {
        return Some((Why::Busy, format!("{name} has {free} MB of memory free")));
    }
    None
}

/// Why the machine named `name` can't run the worker on the account and model it needs, from its
/// last report.
pub fn lacks_account(name: &str, need: &Need, state: &MachineState) -> Option<String> {
    let target = Target { account: need.account.clone(), model: need.model.clone(), effort: None, driver_kind: need.driver_kind.clone(), machine: None, boxed: None };
    if let Some(refused) = crate::load::refusal(name, &target, Some(state)) {
        return Some(refused);
    }
    let id = need.account.as_deref()?;
    state.accounts.iter().any(|account| account.id == id && account.limited).then(|| format!("{id} on {name} is stopped by a usage limit"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::{AccountState, Load};

    const NOW: i64 = 1_000_000;

    fn state(workers: u32, cpu: u32, disk_gb: u64) -> MachineState {
        MachineState {
            load: Load { cpus: Some(8), cpu_percent: Some(cpu), mem_total_mb: Some(16_384), mem_available_mb: Some(8_192), disk_total_mb: Some(500_000), disk_free_mb: Some(disk_gb * 1024) },
            accounts: vec![AccountState { id: "claude".into(), driver_kind: "claude".into(), can_run_child_task: true, limited: false, models: vec![] }],
            sessions: workers,
            workers,
            workers_at_once: 5,
            reported_at: NOW - 10,
        }
    }

    fn seen(id: &str, state: MachineState) -> Seen {
        Seen { id: id.into(), name: id.into(), online: true, state: Some(state), has_repo: true }
    }

    fn policy(prefer: &[&str], avoid: &[&str]) -> Placement {
        Placement { prefer: prefer.iter().map(|id| (*id).into()).collect(), avoid: avoid.iter().map(|id| (*id).into()).collect(), heavy: HeavyRule::Worktree }
    }

    /// `(machine, by, [(skipped machine, why)])`, which each case states literally.
    fn outcome(placed: &Placed) -> (&str, By, Vec<(&str, Why)>) {
        (placed.machine.as_str(), placed.by, placed.skipped.iter().map(|skip| (skip.machine.as_str(), skip.why)).collect())
    }

    fn fleet() -> Vec<Seen> {
        vec![seen("book", state(1, 30, 200)), seen("mini", state(2, 40, 24)), seen("vps", state(0, 5, 80))]
    }

    #[test]
    fn the_resolver_places_each_case_as_the_table_says() {
        let need = Need { account: Some("claude".into()), ..Need::default() };
        let offline = |id: &str| Seen { online: false, state: None, ..seen(id, state(0, 0, 100)) };
        let stale = Seen { state: Some(MachineState { reported_at: NOW - 61, ..state(0, 0, 100) }), ..seen("mini", state(0, 0, 100)) };
        let no_account = seen("mini", MachineState { accounts: vec![], ..state(0, 0, 100) });
        let limited = seen("mini", MachineState { accounts: vec![AccountState { limited: true, ..state(0, 0, 100).accounts[0].clone() }], ..state(0, 0, 100) });
        let no_repo = Seen { has_repo: false, ..seen("mini", state(0, 0, 100)) };
        let low_disk = seen("mini", state(0, 0, 9));
        let full = seen("mini", MachineState { workers_at_once: 2, ..state(2, 0, 100) });
        let hot = seen("mini", state(0, 91, 100));
        let short_of_memory = seen("mini", MachineState { load: Load { mem_available_mb: Some(2_000), ..state(0, 0, 100).load }, ..state(0, 0, 100) });
        let unreported = Seen { state: None, ..seen("mini", state(0, 0, 100)) };
        let vps = seen("vps", state(3, 50, 50));
        let cases: Vec<(&str, Placement, Vec<Seen>, (&str, By, Vec<(&str, Why)>))> = vec![
            ("the first preferred machine wins over a less busy one", policy(&["mini", "vps"], &[]), fleet(), ("mini", By::Policy, vec![])),
            ("an offline preferred machine is skipped", policy(&["mini", "vps"], &[]), vec![seen("book", state(0, 0, 100)), offline("mini"), vps.clone()], ("vps", By::Policy, vec![("mini", Why::Offline)])),
            ("a machine silent for over a minute is offline", policy(&["mini", "vps"], &[]), vec![stale, vps.clone()], ("vps", By::Policy, vec![("mini", Why::Offline)])),
            ("a machine without the account is skipped", policy(&["mini", "vps"], &[]), vec![no_account, vps.clone()], ("vps", By::Policy, vec![("mini", Why::NoAccount)])),
            ("a machine whose account is limited is skipped", policy(&["mini", "vps"], &[]), vec![limited, vps.clone()], ("vps", By::Policy, vec![("mini", Why::NoAccount)])),
            ("a machine without the repo is skipped", policy(&["mini", "vps"], &[]), vec![no_repo, vps.clone()], ("vps", By::Policy, vec![("mini", Why::NoRepo)])),
            ("a machine under 10 GB of disk is skipped", policy(&["mini", "vps"], &[]), vec![low_disk, vps.clone()], ("vps", By::Policy, vec![("mini", Why::LowDisk)])),
            ("a full machine is skipped", policy(&["mini", "vps"], &[]), vec![full, vps.clone()], ("vps", By::Policy, vec![("mini", Why::Full)])),
            ("a machine over 90% CPU is busy", policy(&["mini", "vps"], &[]), vec![hot, vps.clone()], ("vps", By::Policy, vec![("mini", Why::Busy)])),
            ("a machine under 2 GB of memory is busy", policy(&["mini", "vps"], &[]), vec![short_of_memory, vps.clone()], ("vps", By::Policy, vec![("mini", Why::Busy)])),
            ("a machine that reports nothing is skipped", policy(&["mini", "vps"], &[]), vec![unreported, vps.clone()], ("vps", By::Policy, vec![("mini", Why::NoReport)])),
            ("with no preferred machine fitting, the least busy one wins", policy(&["mini"], &[]), vec![seen("book", state(1, 30, 200)), offline("mini"), seen("vps", state(0, 5, 80))], ("vps", By::Ranked, vec![("mini", Why::Offline)])),
            ("the avoided caller is never ranked", policy(&[], &["book"]), vec![seen("book", state(0, 0, 900)), seen("mini", state(4, 80, 24))], ("mini", By::Ranked, vec![("book", Why::Avoided)])),
            ("the avoided caller is the fallback when nothing else fits", policy(&["mini"], &["book"]), vec![seen("book", state(0, 0, 900)), offline("mini")], ("book", By::Fallback, vec![("book", Why::Avoided), ("mini", Why::Offline)])),
            ("a machine that refused is not asked again", policy(&["mini", "vps"], &[]), fleet(), ("vps", By::Policy, vec![("mini", Why::Refused)])),
        ];
        for (case, policy, machines, expected) in cases {
            let exclude = if case.contains("refused") { vec!["mini".to_owned()] } else { vec![] };
            let placed = place(&policy, "book", &need, &machines, &exclude, NOW);
            assert_eq!(outcome(&placed), expected, "{case}");
        }
    }

    #[test]
    fn load_ties_break_on_workers_then_cpu_then_free_disk() {
        let rank = |machines: Vec<Seen>| place(&policy(&[], &["x"]), "x", &Need::default(), &machines, &[], NOW).machine;
        assert_eq!(rank(vec![seen("a", state(2, 10, 100)), seen("b", state(1, 90, 20))]), "b", "fewer workers first");
        assert_eq!(rank(vec![seen("a", state(1, 50, 100)), seen("b", state(1, 20, 20))]), "b", "then lower CPU");
        assert_eq!(rank(vec![seen("a", state(1, 20, 20)), seen("b", state(1, 20, 100))]), "b", "then more free disk");
    }

    #[test]
    fn nothing_fitting_answers_the_caller_with_every_reason() {
        let machines = vec![Seen { online: false, state: None, ..seen("mini", state(0, 0, 100)) }, seen("vps", state(0, 0, 5)), seen("book", state(0, 95, 100))];
        let placed = place(&policy(&["mini", "vps"], &[]), "book", &Need::default(), &machines, &[], NOW);
        assert_eq!(
            placed,
            Placed {
                machine: "book".into(),
                machine_name: "book".into(),
                by: By::Fallback,
                reason: "no machine fits the policy, so it runs on the machine that asked".into(),
                skipped: vec![
                    Skip { machine: "mini".into(), machine_name: "mini".into(), why: Why::Offline, detail: "mini is offline".into() },
                    Skip { machine: "vps".into(), machine_name: "vps".into(), why: Why::LowDisk, detail: "vps has 5 GB of disk free; a worker needs 10 GB".into() },
                    Skip { machine: "book".into(), machine_name: "book".into(), why: Why::Busy, detail: "book's CPU was at 95% over the last minute".into() },
                ],
            }
        );
        assert_eq!(
            serde_json::to_value(&placed.skipped[0]).unwrap(),
            serde_json::json!({ "machine": "mini", "machineName": "mini", "why": "offline", "detail": "mini is offline" })
        );
    }

    fn request(args: serde_json::Value) -> Request {
        crate::delegate::parse_request(&args).unwrap()
    }

    #[test]
    fn the_route_follows_the_target_the_policy_and_the_weight() {
        let set = policy(&["mini"], &[]);
        let worktree = serde_json::json!({ "task": "t", "isolation": "worktree" });
        let light = "light work stays on the machine that asked; pass heavy: true or target.machine \"auto\" to place it";
        let cases = [
            ("an empty policy keeps everything here", Placement::default(), worktree.clone(), Route::Here(By::Here, String::new())),
            ("a heavy worker is placed", set.clone(), worktree.clone(), Route::Ask),
            ("a shared worker is light", set.clone(), serde_json::json!({ "task": "t" }), Route::Here(By::Here, light.into())),
            ("heavy: true places a shared worker", set.clone(), serde_json::json!({ "task": "t", "heavy": true }), Route::Ask),
            ("heavy: false keeps a worktree worker here", set.clone(), serde_json::json!({ "task": "t", "isolation": "worktree", "heavy": false }), Route::Here(By::Here, light.into())),
            ("an explicit machine wins", set.clone(), serde_json::json!({ "task": "t", "isolation": "worktree", "target": { "machine": "vps" } }), Route::Explicit),
            ("a box wins", set.clone(), serde_json::json!({ "task": "t", "isolation": "worktree", "target": { "box": true } }), Route::Explicit),
            ("here keeps it here", set.clone(), serde_json::json!({ "task": "t", "isolation": "worktree", "target": { "machine": "here" } }), Route::Here(By::Here, "target.machine is \"here\"".into())),
            ("auto places light work", set.clone(), serde_json::json!({ "task": "t", "target": { "machine": "auto" } }), Route::Ask),
            (
                "auto with no policy stays here",
                Placement::default(),
                serde_json::json!({ "task": "t", "target": { "machine": "auto" } }),
                Route::Here(By::Here, "target.machine is \"auto\", and the account has no placement policy".into()),
            ),
            ("every worker is heavy under AllWorkers", Placement { heavy: HeavyRule::AllWorkers, ..set.clone() }, serde_json::json!({ "task": "t" }), Route::Ask),
        ];
        for (case, policy, args, expected) in cases {
            assert_eq!(route(&policy, &request(args)), expected, "{case}");
        }
    }

    #[test]
    fn a_policy_lists_each_machine_once_and_in_one_list() {
        assert_eq!(policy(&["mini"], &["mini"]).check(), Err("machine mini is in both Prefer and Avoid; keep it in one".into()));
        assert_eq!(policy(&["mini", "mini"], &[]).check(), Err("a machine is listed twice; list each machine once".into()));
        assert_eq!(policy(&["mini"], &["book"]).check(), Ok(()));
        assert!(!Placement::default().is_active() && policy(&[], &["book"]).is_active());
    }

    #[test]
    fn a_policy_reads_from_an_empty_object_and_round_trips() {
        assert_eq!(serde_json::from_value::<Placement>(serde_json::json!({})).unwrap(), Placement::default());
        let set = Placement { heavy: HeavyRule::AllWorkers, ..policy(&["mini"], &["book"]) };
        let wire = serde_json::to_value(&set).unwrap();
        assert_eq!(wire, serde_json::json!({ "prefer": ["mini"], "avoid": ["book"], "heavy": "all_workers" }));
        assert_eq!(serde_json::from_value::<Placement>(wire).unwrap(), set);
    }
}
