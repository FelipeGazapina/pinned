//! What a machine tells the hub about itself every 20 seconds, so a caller can pick a machine on
//! facts: how busy it is, how much disk it has left, which agent accounts it can run a worker
//! with, and how many workers it takes at once. The hub keeps the latest summary of each online
//! machine and shows it in Settings › Machines, in `projects` and in `orchestrator_capabilities`.
//!
//! A summary carries account ids and driver kinds, never a token, an email or a display name.

use crate::delegate::{ListedAccount, Target};
use crate::wire::MachineView;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// How busy a machine is. A field is None where the machine can't read it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct Load {
    /// Logical cores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    /// The one-minute load average per core, as a percent capped at 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_total_mb: Option<u64>,
    /// Memory that a new process can take without swapping: free, inactive and cached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_available_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_total_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_free_mb: Option<u64>,
}

/// One agent account of a machine, as far as a caller needs to choose it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct AccountState {
    /// The `providerInstanceId` of `orchestrator_capabilities`, such as `claude` or `bloom`.
    pub id: String,
    /// `claude`, `codex` or `cursor`.
    pub driver_kind: String,
    /// Signed in on that machine, with its CLI installed.
    pub can_run_child_task: bool,
    /// A usage limit stopped it.
    #[serde(default)]
    pub limited: bool,
    /// The model ids it offers a worker. Empty when it names none, as Codex does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// The summary a machine reports.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct MachineState {
    #[serde(default)]
    pub load: Load,
    #[serde(default)]
    pub accounts: Vec<AccountState>,
    /// Sessions running now, of any role.
    #[serde(default)]
    pub sessions: u32,
    /// Workers running now: worker sessions and delegated tasks.
    #[serde(default)]
    pub workers: u32,
    /// Workers at once that this machine allows a project, from the account's way of working.
    #[serde(default)]
    pub workers_at_once: u32,
    /// Unix seconds when the hub heard it. The hub sets it; a machine's own value is ignored.
    #[serde(default)]
    pub reported_at: i64,
}

impl AccountState {
    pub fn of(account: &ListedAccount) -> Option<Self> {
        let driver_kind = serde_json::to_value(account.provider).ok()?.as_str()?.to_owned();
        Some(Self {
            id: account.id.clone(),
            driver_kind,
            can_run_child_task: account.signed_in && account.bin_found,
            limited: account.limited,
            models: account.models.iter().map(|model| model.id.clone()).collect(),
        })
    }
}

/// Why `machine`, named `name`, can't take a worker with this `target`, from what it last
/// reported. None when it can, or when it reported nothing, so an older machine fails late as it
/// always did. Only a Cursor account's model list is exact: Claude takes aliases and any id its
/// CLI knows, and Codex takes any.
pub fn refusal(name: &str, target: &Target, state: Option<&MachineState>) -> Option<String> {
    let state = state?;
    if let Some(id) = target.account.as_deref().filter(|id| !id.is_empty()) {
        let Some(account) = state.accounts.iter().find(|account| account.id == id) else {
            return Some(format!("{name} has no {id} account; sign in there or pick another machine"));
        };
        if !account.can_run_child_task {
            return Some(format!("{id} on {name} can't run a worker: it is not signed in there, or its program is not installed; sign in there or pick another machine"));
        }
        return unlisted_model(name, target, account);
    }
    let kind = target.driver_kind.as_deref().filter(|kind| !kind.is_empty())?;
    let of_kind: Vec<&AccountState> = state.accounts.iter().filter(|account| account.driver_kind == kind && account.can_run_child_task).collect();
    if of_kind.is_empty() {
        return Some(format!("{name} has no {kind} account that can run a worker; sign in there or pick another machine"));
    }
    match of_kind.iter().find_map(|account| unlisted_model(name, target, account).is_none().then_some(())) {
        Some(()) => None,
        None => unlisted_model(name, target, of_kind[0]),
    }
}

fn unlisted_model(name: &str, target: &Target, account: &AccountState) -> Option<String> {
    let model = target.model.as_deref().filter(|model| !model.is_empty())?;
    let strict = account.driver_kind == "cursor" && !account.models.is_empty();
    let listed = account.models.iter().any(|id| id == model || id.strip_prefix(model).is_some_and(|rest| rest.starts_with('-')));
    (strict && !listed).then(|| format!("{} on {name} does not offer {model}; pick one of its models or another machine", account.id))
}

/// Puts each machine's summary into the overview `projects` shows: the whole of it on the machine,
/// and its load on each of its checkouts, so a caller sees how busy a checkout's machine is.
pub fn attach(overview: &mut crate::global_orchestrator::Overview, views: &[MachineView]) {
    let state_of = |id: &str| views.iter().find(|view| view.id == id).and_then(|view| view.state.clone());
    for machine in &mut overview.machines {
        machine.state = state_of(&machine.id);
    }
    for checkout in overview.projects.iter_mut().flat_map(|project| project.checkouts.iter_mut()) {
        checkout.load = state_of(&checkout.machine).map(|state| state.load);
    }
}

/// A machine's summary as `orchestrator_capabilities` lists it under `machines`.
pub fn machines_json(views: &[MachineView], this: Option<&str>) -> Value {
    Value::Array(
        views
            .iter()
            .map(|view| {
                let state = view.state.as_ref();
                json!({
                    "id": view.id,
                    "name": view.name,
                    "online": view.online,
                    "isThisMachine": this == Some(view.id.as_str()),
                    "reported": state.is_some(),
                    "load": state.map(|state| &state.load),
                    "sessionsRunning": state.map(|state| state.sessions),
                    "workersRunning": state.map(|state| state.workers),
                    "workersAtOnce": state.map(|state| state.workers_at_once),
                    "providers": state.map(|state| state.accounts.iter().map(|account| json!({
                        "providerInstanceId": account.id,
                        "driverKind": account.driver_kind,
                        "canRunChildTask": account.can_run_child_task,
                        "limited": account.limited,
                        "models": account.models,
                    })).collect::<Vec<_>>()),
                })
            })
            .collect(),
    )
}

/// The one-minute load average per core as a percent, capped at 100.
fn percent_of(load: f64, cpus: u32) -> u32 {
    if cpus == 0 || !load.is_finite() || load < 0.0 {
        return 0;
    }
    (load / f64::from(cpus) * 100.0).round().min(100.0) as u32
}

/// Reads this machine's load. `disk` is a folder on the disk that holds the projects.
pub fn sample(disk: &std::path::Path) -> Load {
    let cpus = std::thread::available_parallelism().ok().and_then(|n| u32::try_from(n.get()).ok());
    let (mem_total_mb, mem_available_mb) = memory();
    let (disk_total_mb, disk_free_mb) = disk_of(disk);
    Load { cpus, cpu_percent: cpus.and_then(|cpus| load_average().map(|load| percent_of(load, cpus))), mem_total_mb, mem_available_mb, disk_total_mb, disk_free_mb }
}

#[cfg(unix)]
fn load_average() -> Option<f64> {
    let mut average = [0f64; 1];
    // SAFETY: the buffer holds the one sample asked for.
    let got = unsafe { libc::getloadavg(average.as_mut_ptr(), 1) };
    (got == 1).then_some(average[0])
}

#[cfg(not(unix))]
fn load_average() -> Option<f64> {
    None
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn disk_of(folder: &std::path::Path) -> (Option<u64>, Option<u64>) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else { return (None, None) };
    // SAFETY: `statvfs` fills the zeroed struct for a valid, NUL-ended path.
    let stats = unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut stats) != 0 {
            return (None, None);
        }
        stats
    };
    let unit = stats.f_frsize as u64;
    let mb = |blocks: u64| blocks.saturating_mul(unit) / (1024 * 1024);
    (Some(mb(stats.f_blocks as u64)), Some(mb(stats.f_bavail as u64)))
}

#[cfg(not(unix))]
fn disk_of(_folder: &std::path::Path) -> (Option<u64>, Option<u64>) {
    (None, None)
}

#[cfg(target_os = "linux")]
fn memory() -> (Option<u64>, Option<u64>) {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |key: &str| text.lines().find_map(|line| line.strip_prefix(key)?.trim().trim_end_matches("kB").trim().parse::<u64>().ok());
    (kb("MemTotal:").map(|kb| kb / 1024), kb("MemAvailable:").map(|kb| kb / 1024))
}

#[cfg(target_os = "macos")]
fn memory() -> (Option<u64>, Option<u64>) {
    let mut total: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: `hw.memsize` is one u64, and `len` says so.
    let known = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut total).cast(), &mut len, std::ptr::null_mut(), 0) } == 0;
    let out = crate::process::command("vm_stat").output().ok().and_then(|out| String::from_utf8(out.stdout).ok()).unwrap_or_default();
    (known.then_some(total / (1024 * 1024)), available_from_vm_stat(&out))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn memory() -> (Option<u64>, Option<u64>) {
    (None, None)
}

/// Free, inactive and speculative pages of `vm_stat`'s output, in MB.
#[cfg(any(target_os = "macos", test))]
fn available_from_vm_stat(text: &str) -> Option<u64> {
    let size = text.lines().next()?.split("page size of ").nth(1)?.split_whitespace().next()?.parse::<u64>().ok()?;
    let pages = |key: &str| text.lines().find_map(|line| line.strip_prefix(key)?.trim().trim_end_matches('.').parse::<u64>().ok());
    let free = pages("Pages free:")? + pages("Pages inactive:").unwrap_or(0) + pages("Pages speculative:").unwrap_or(0);
    Some(free * size / (1024 * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, kind: &str, can: bool, models: &[&str]) -> AccountState {
        AccountState { id: id.into(), driver_kind: kind.into(), can_run_child_task: can, limited: false, models: models.iter().map(|m| (*m).into()).collect() }
    }

    fn mini() -> MachineState {
        MachineState {
            accounts: vec![account("claude", "claude", true, &["claude-opus-5-5"]), account("cursor", "cursor", true, &["grok-4.7", "composer-2.5"]), account("codex", "codex", false, &[])],
            workers_at_once: 3,
            ..Default::default()
        }
    }

    fn target(account: Option<&str>, model: Option<&str>, kind: Option<&str>) -> Target {
        Target { account: account.map(Into::into), model: model.map(Into::into), effort: None, driver_kind: kind.map(Into::into), machine: Some("mac-mini".into()), boxed: None }
    }

    #[test]
    fn a_machine_without_the_account_is_refused_with_the_reason() {
        assert_eq!(refusal("mac-mini", &target(Some("bloom"), None, None), Some(&mini())), Some("mac-mini has no bloom account; sign in there or pick another machine".into()));
    }

    #[test]
    fn an_account_that_is_there_but_cannot_run_is_refused_and_one_that_can_is_not() {
        let said = refusal("mac-mini", &target(Some("codex"), None, None), Some(&mini())).unwrap();
        assert!(said.starts_with("codex on mac-mini can't run a worker"), "{said}");
        assert_eq!(refusal("mac-mini", &target(Some("claude"), Some("opus"), None), Some(&mini())), None, "Claude takes aliases, so its model is not checked");
    }

    #[test]
    fn a_cursor_model_the_machine_lacks_is_refused_and_an_effort_suffixed_id_is_not() {
        assert_eq!(
            refusal("mac-mini", &target(Some("cursor"), Some("gpt-9"), None), Some(&mini())),
            Some("cursor on mac-mini does not offer gpt-9; pick one of its models or another machine".into())
        );
        assert_eq!(refusal("mac-mini", &target(Some("cursor"), Some("grok-4.7"), None), Some(&mini())), None);
    }

    #[test]
    fn a_driver_kind_needs_an_account_of_that_kind_that_can_run() {
        assert_eq!(refusal("mac-mini", &target(None, None, Some("codex")), Some(&mini())), Some("mac-mini has no codex account that can run a worker; sign in there or pick another machine".into()));
        assert_eq!(refusal("mac-mini", &target(None, None, Some("cursor")), Some(&mini())), None);
    }

    #[test]
    fn a_machine_that_reported_nothing_is_not_refused() {
        assert_eq!(refusal("old", &target(Some("bloom"), None, None), None), None);
        assert_eq!(refusal("mac-mini", &target(None, None, None), Some(&mini())), None, "no account named: the project's account decides there");
    }

    #[test]
    fn a_summary_round_trips_and_an_older_machines_empty_one_reads() {
        let state = MachineState { sessions: 4, workers: 2, load: Load { cpus: Some(8), cpu_percent: Some(37), disk_free_mb: Some(24_000), ..Default::default() }, ..mini() };
        let wire = serde_json::to_value(&state).unwrap();
        assert_eq!(wire["load"], json!({ "cpus": 8, "cpu_percent": 37, "disk_free_mb": 24_000 }));
        assert_eq!(wire["accounts"][2], json!({ "id": "codex", "driver_kind": "codex", "can_run_child_task": false, "limited": false }));
        assert_eq!(serde_json::from_value::<MachineState>(wire).unwrap(), state);
        assert_eq!(serde_json::from_value::<MachineState>(json!({})).unwrap(), MachineState::default());
    }

    #[test]
    fn the_wire_carries_no_credential_field() {
        let text = serde_json::to_string(&mini()).unwrap();
        for word in ["token", "email", "key", "name"] {
            assert!(!text.contains(&format!("\"{word}")), "{word} in {text}");
        }
    }

    #[test]
    fn load_is_a_percent_of_the_cores_capped_at_a_hundred() {
        assert_eq!((percent_of(2.0, 8), percent_of(30.0, 8), percent_of(1.0, 0), percent_of(f64::NAN, 4)), (25, 100, 0, 0));
    }

    #[test]
    fn vm_stat_pages_become_available_megabytes() {
        let out = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                               1000.\nPages active:                             5000.\nPages inactive:                           3000.\nPages speculative:                        1000.\n";
        assert_eq!(available_from_vm_stat(out), Some(5000 * 16384 / (1024 * 1024)));
    }

    #[test]
    fn this_machines_sample_reads_a_disk() {
        let load = sample(&std::env::temp_dir());
        assert!(load.cpus.is_some_and(|c| c > 0), "{load:?}");
        #[cfg(unix)]
        assert!(load.disk_total_mb.is_some_and(|total| total > 0) && load.disk_free_mb.is_some(), "{load:?}");
    }
}
