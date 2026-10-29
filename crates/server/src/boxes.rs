//! The Upstash Boxes workflow runs live in: one box per issue or thread, booted as a node of the
//! account, given exactly that run, and deleted with its machine once the session ends. The `boxes`
//! table is the only source of truth. Every step checks it before acting, so the first pass after a
//! restart finishes whatever the last hub left halfway.

use crate::hub::{Account, Hub, RUN_WAIT};
use futures_util::future::BoxFuture;
use pinned_core::account::Credentials;
use pinned_core::agent::Subject;
use pinned_core::connectors::{BoxSize, BoxStep, Run};
use pinned_core::upstash::{self, BoxId, Exec, Keys, Listed, NewBox};
use pinned_core::wire::{Call, RunSpec};
use pinned_core::github;
use pinned_core::machine;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const DIR: &str = "/workspace/home/.pinned";
const BOOTSTRAP: &str = include_str!("box-bootstrap.sh");
const START: &str = include_str!("box-start.sh");
/// The bootstrap output a failure quotes, from its end.
const TAIL: usize = 600;

/// The Box API as the lifecycle uses it. `Upstash` forwards to `pinned_core::upstash`; tests pass a
/// fake whose exec of the bootstrap starts an in-process node.
pub trait BoxApi: Send + Sync {
    fn create<'a>(&'a self, key: &'a str, new: NewBox) -> BoxFuture<'a, Result<BoxId, String>>;
    fn list<'a>(&'a self, key: &'a str, label: &'a str) -> BoxFuture<'a, Result<Vec<Listed>, String>>;
    fn write<'a>(&'a self, key: &'a str, id: &'a BoxId, path: &'a str, content: String) -> BoxFuture<'a, Result<(), String>>;
    fn exec<'a>(&'a self, key: &'a str, id: &'a BoxId, script: String) -> BoxFuture<'a, Result<Exec, String>>;
    fn delete<'a>(&'a self, key: &'a str, id: &'a BoxId) -> BoxFuture<'a, Result<(), String>>;
}

pub struct Upstash;

impl BoxApi for Upstash {
    fn create<'a>(&'a self, key: &'a str, new: NewBox) -> BoxFuture<'a, Result<BoxId, String>> {
        Box::pin(async move { upstash::create(key, &new).await })
    }
    fn list<'a>(&'a self, key: &'a str, label: &'a str) -> BoxFuture<'a, Result<Vec<Listed>, String>> {
        Box::pin(upstash::list(key, label))
    }
    fn write<'a>(&'a self, key: &'a str, id: &'a BoxId, path: &'a str, content: String) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { upstash::write(key, id, path, &content).await })
    }
    fn exec<'a>(&'a self, key: &'a str, id: &'a BoxId, script: String) -> BoxFuture<'a, Result<Exec, String>> {
        Box::pin(async move { upstash::exec(key, id, &script).await })
    }
    fn delete<'a>(&'a self, key: &'a str, id: &'a BoxId) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(upstash::delete(key, id))
    }
}

/// When a box's session counts as ended, and how often the hub looks.
#[derive(Clone, Debug)]
pub struct Policy {
    /// Creating the box, booting its node and handing it the run.
    pub boot_wait: Duration,
    /// With the session not working.
    pub idle: Duration,
    /// With the session not working after it opened its pull request, which is its work done.
    pub done: Duration,
    /// With claude exited; restarting it from a client brings it back.
    pub exited: Duration,
    /// With no tab for the run, which covers the time from the run to the node's first tabs.
    pub tab_gone: Duration,
    pub offline: Duration,
    /// From the row's creation, whatever the session does.
    pub max_life: Duration,
    /// For a workflow after one of its boxes failed to boot, so a boot that always fails doesn't
    /// pay for a box every check.
    pub cooldown: Duration,
    pub tick: Duration,
    /// Between looks for boxes of this hub that no row names.
    pub sweep: Duration,
}

impl Default for Policy {
    fn default() -> Policy {
        let minutes = |m: u64| Duration::from_secs(m * 60);
        Policy {
            boot_wait: minutes(6),
            idle: minutes(10),
            done: minutes(2),
            exited: minutes(5),
            tab_gone: minutes(2),
            offline: minutes(10),
            max_life: minutes(4 * 60),
            cooldown: minutes(10),
            tick: Duration::from_secs(60),
            sweep: minutes(10),
        }
    }
}

/// The hub's record that one run lives in one box. It exists before its box does and goes only
/// after the box and its machine are gone.
#[derive(Clone, Debug)]
pub struct BoxRow {
    /// 16 hex characters; the box carries the label `p:<id>`.
    pub id: String,
    pub account: Uuid,
    /// Chosen with the row, so a crash at any point knows which machine to remove.
    pub machine: Uuid,
    pub upstash: Option<BoxId>,
    pub connector: String,
    pub subject: Subject,
    /// Without the GitHub token, which dispatch adds back from the account's key.
    pub spec: RunSpec,
    pub size: BoxSize,
    pub phase: Phase,
    /// Since the row was written, by Postgres's clock.
    pub age: Duration,
}

impl BoxRow {
    pub fn remains(&self) -> Remains {
        Remains { id: self.id.clone(), machine: self.machine, upstash: self.upstash.clone(), why: format!("{:?}", self.phase) }
    }
}

/// What deleting a row's box and machine takes, which even a row this hub can't read gives, as
/// one a hub with another run shape wrote.
#[derive(Clone, Debug)]
pub struct Remains {
    pub id: String,
    pub machine: Uuid,
    pub upstash: Option<BoxId>,
    /// Why it ends, for the log.
    pub why: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Creating,
    Booting,
    Running,
    Ending(End),
}

#[derive(Clone, Debug, PartialEq)]
pub enum End {
    TabClosed,
    Exited,
    Idle,
    /// The session opened its pull request and stopped.
    Done,
    Offline,
    Expired,
    /// Someone removed its machine.
    Removed,
    /// Someone deleted the box outside pinned.
    Vanished,
    Disconnected,
    Failed(String),
}

const ENDS: [(End, &str); 9] = [
    (End::TabClosed, "tab-closed"),
    (End::Exited, "exited"),
    (End::Idle, "idle"),
    (End::Done, "done"),
    (End::Offline, "offline"),
    (End::Expired, "expired"),
    (End::Removed, "removed"),
    (End::Vanished, "vanished"),
    (End::Disconnected, "disconnected"),
];
const FAILED: &str = "failed: ";

impl Phase {
    /// The `phase` and `reason` columns.
    pub fn stored(&self) -> (&'static str, Option<String>) {
        match self {
            Phase::Creating => ("creating", None),
            Phase::Booting => ("booting", None),
            Phase::Running => ("running", None),
            Phase::Ending(End::Failed(why)) => ("ending", Some(format!("{FAILED}{why}"))),
            Phase::Ending(end) => ("ending", ENDS.iter().find(|(e, _)| e == end).map(|(_, name)| name.to_string())),
        }
    }

    pub fn load(phase: &str, reason: Option<String>) -> Phase {
        match phase {
            "creating" => Phase::Creating,
            "booting" => Phase::Booting,
            "running" => Phase::Running,
            _ => {
                let reason = reason.unwrap_or_default();
                let end = match reason.strip_prefix(FAILED) {
                    Some(why) => End::Failed(why.to_owned()),
                    None => ENDS.iter().find(|(_, name)| *name == reason).map_or_else(|| End::Failed(reason.clone()), |(e, _)| e.clone()),
                };
                Phase::Ending(end)
            }
        }
    }
}

/// What the hub's live state shows of a running row's session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seen {
    Offline,
    NoTab,
    /// `quiet`: the session is not working.
    Tab { running: bool, quiet: bool },
}

/// When each condition of a running row began, by the hub's clock. A restart resets them, and
/// `max_life` still bounds what a box costs.
#[derive(Default, Debug)]
struct Timers {
    quiet_since: Option<Instant>,
    exited_since: Option<Instant>,
    gone_since: Option<Instant>,
    offline_since: Option<Instant>,
    /// The session reported its pull request opened.
    done: bool,
}

/// How far `boot` brought a row.
enum Boot {
    Online,
    /// Another hub process moved the row on, as during a deploy, so this one stops driving it.
    Moved,
}

/// Whether a running row's session has ended. Every end rule lives here.
fn verdict(age: Duration, seen: Seen, t: &mut Timers, now: Instant, p: &Policy) -> Option<End> {
    // `None` is a condition this look can't see, as the session's while its node is offline: its
    // timer neither starts nor restarts, so a dropped link doesn't buy a quiet box more time.
    let (offline, gone, exited, quiet) = match seen {
        Seen::Offline => (Some(true), None, None, None),
        Seen::NoTab => (Some(false), Some(true), None, None),
        Seen::Tab { running, quiet } => (Some(false), Some(false), Some(!running), Some(quiet)),
    };
    let held = |since: &mut Option<Instant>, on: Option<bool>| match on {
        Some(false) => {
            *since = None;
            None
        }
        Some(true) => Some(now.duration_since(*since.get_or_insert(now))),
        None => since.map(|at| now.duration_since(at)),
    };
    let quiet = held(&mut t.quiet_since, quiet);
    let ended = [
        (held(&mut t.offline_since, offline).is_some_and(|h| h >= p.offline), End::Offline),
        (held(&mut t.gone_since, gone).is_some_and(|h| h >= p.tab_gone), End::TabClosed),
        (held(&mut t.exited_since, exited).is_some_and(|h| h >= p.exited), End::Exited),
        (t.done && quiet.is_some_and(|h| h >= p.done), End::Done),
        (quiet.is_some_and(|h| h >= p.idle), End::Idle),
    ];
    if age >= p.max_life {
        return Some(End::Expired);
    }
    ended.into_iter().find(|(over, _)| *over).map(|(_, end)| end)
}

/// The label every box of this hub carries, so hubs sharing one Box key never sweep each other's.
pub fn hub_label(public_url: &str) -> String {
    let digest = Sha256::digest(public_url.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("hub:{}", &hex[..12])
}

fn row_label(row: &str) -> String {
    format!("p:{row}")
}

fn new_row_id() -> String {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn quoted(text: &str) -> String {
    text.replace('\'', "'\\''")
}

/// The bootstrap the hub runs in a new box. It carries no secret.
pub fn bootstrap_script(hub: &str) -> String {
    BOOTSTRAP.replace("{{hub}}", &quoted(hub))
}

/// The script that starts the box's node on every container start. It carries no secret.
/// `repo` is the run's repository, whose clone it marks trusted for claude.
pub fn start_script(login: &str, repo: Option<&str>) -> String {
    let clones = serde_json::to_string(&repo.and_then(machine::repo_name).into_iter().collect::<Vec<_>>()).unwrap();
    START.replace("{{login}}", &quoted(login)).replace("{{clones}}", &quoted(&clones))
}

/// The secrets the node and its sessions run with, as `start.sh` sources them.
fn box_env(keys: &Keys, github: Option<&str>) -> String {
    let mut env = format!("CLAUDE_CODE_OAUTH_TOKEN='{}'\n", quoted(&keys.claude_token));
    if let Some(token) = github {
        env += &format!("GH_TOKEN='{}'\n", quoted(token));
    }
    env
}

/// `Box · <the session's name>`, as the run names its tab.
fn machine_name(spec: &RunSpec) -> String {
    let named = spec.args.iter().position(|a| a == "--name").and_then(|i| spec.args.get(i + 1));
    let name = format!("Box · {}", named.map_or("pinned", String::as_str));
    name.chars().take(100).collect()
}

fn tail(exec: &Exec) -> String {
    let text = format!("{}{}", exec.output, exec.error);
    let text = text.trim();
    let start = text.char_indices().rev().nth(TAIL - 1).map_or(0, |(i, _)| i);
    text[start..].to_owned()
}

/// Marks a row as driven while it lives, so an open and a reap never drive one row at once.
struct Driving<'a> {
    rows: &'a Mutex<HashSet<String>>,
    id: String,
}

impl Drop for Driving<'_> {
    fn drop(&mut self) {
        self.rows.lock().unwrap().remove(&self.id);
    }
}

pub struct Boxes {
    api: Arc<dyn BoxApi>,
    pub policy: Policy,
    /// Every box this hub creates carries it.
    pub label: String,
    driving: Mutex<HashSet<String>>,
    timers: Mutex<HashMap<String, Timers>>,
    cooling: Mutex<HashMap<(Uuid, String), Instant>>,
    swept: Mutex<HashMap<Uuid, Instant>>,
}

impl Boxes {
    pub fn new(api: Arc<dyn BoxApi>, policy: Policy, public_url: &str) -> Boxes {
        Boxes {
            api,
            policy,
            label: hub_label(public_url),
            driving: Mutex::default(),
            timers: Mutex::default(),
            cooling: Mutex::default(),
            swept: Mutex::default(),
        }
    }

    /// Checks the Box key with Upstash before the account keeps it.
    pub async fn check_key(&self, key: &str) -> Result<(), String> {
        self.api.list(key, &self.label).await.map(drop)
    }

    fn drive_guard(&self, id: &str) -> Option<Driving<'_>> {
        self.driving.lock().unwrap().insert(id.to_owned()).then(|| Driving { rows: &self.driving, id: id.to_owned() })
    }

    /// `Hub::open` for a box run. A run whose issue or thread has a box already goes to it.
    /// Otherwise the run gets a new row, and a box booted for it within `boot_wait`. On failure the
    /// row ends, and the answer says why, which reaches the workflow's check line.
    pub async fn open(&self, hub: &Hub, account: &Arc<Account>, run: Run, step: BoxStep) -> Result<(), String> {
        let keys = hub.connectors(account).await?.box_keys().ok_or("Upstash Box is not connected: connect it in pinned's Connectors settings")?;
        let cooled = self.cooling.lock().unwrap().get(&(account.id, run.connector.clone())).copied();
        if let Some(left) = cooled.and_then(|at| self.policy.cooldown.checked_sub(at.elapsed())) {
            let minutes = left.as_secs().div_ceil(60).max(1);
            return Err(format!("Upstash Box: this workflow's last box failed to boot, so pinned tries again in {minutes} min"));
        }
        let mut spec = run.spec();
        spec.token = None;
        let fresh = BoxRow {
            id: new_row_id(),
            account: account.id,
            machine: Uuid::new_v4(),
            upstash: None,
            connector: run.connector,
            subject: run.subject,
            spec: spec.clone(),
            size: step.size,
            phase: Phase::Creating,
            age: Duration::ZERO,
        };
        // Driven before it is written, so a reap that reads it first leaves it alone.
        let driving = self.drive_guard(&fresh.id);
        let (row, _) = hub.db.reserve_box(&fresh).await?;
        if let Phase::Ending(_) = row.phase {
            return Err("Upstash Box: the last box for this is still being deleted; the next check tries again".into());
        }
        let driving = if row.id == fresh.id { driving } else { self.drive_guard(&row.id) };
        let Some(_driving) = driving else {
            return Err("Upstash Box: its box is still booting; the next check tries again".into());
        };
        // The run asked for now, which may carry newer thread messages than the one the row was made with.
        self.drive(hub, account, row, &spec, &keys).await.map_err(|e| format!("Upstash Box: {e}"))
    }

    /// Brings the row to running: its box's node online within `boot_wait`, then `spec` answered
    /// by it within `RUN_WAIT`, which leaves a slow clone its time. A row another hub process moved
    /// is left to that process.
    async fn drive(&self, hub: &Hub, account: &Arc<Account>, row: BoxRow, spec: &RunSpec, keys: &Keys) -> Result<(), String> {
        if row.phase == Phase::Running {
            return self.dispatch(hub, account, &row, spec).await;
        }
        let machine = row.machine.to_string();
        let booted = if row.phase == Phase::Booting && account.is_online(&machine) {
            Ok(Boot::Online)
        } else {
            // A row resumed after a restart keeps some time, since its age counts the hub's downtime.
            let left = self.policy.boot_wait.saturating_sub(row.age).max(self.policy.boot_wait / 2);
            match tokio::time::timeout(left, self.boot(hub, account, &row, keys)).await {
                Ok(booted) => booted,
                Err(_) => Err(format!("the box did not boot within {} s", left.as_secs())),
            }
        };
        let booted = match booted {
            Ok(Boot::Moved) => return Err(format!("box {} is driven by another hub; the next check tries again", row.id)),
            Ok(Boot::Online) => self.dispatch(hub, account, &row, spec).await,
            Err(e) => Err(e),
        };
        if booted.is_ok() {
            // Another hub may have moved it on already, which is just as good.
            hub.db.set_box_phase(&row.id, &Phase::Booting, &Phase::Running).await?;
        }
        if let Err(e) = &booted {
            eprintln!("pinned-server: box {} of account {} failed: {e}", row.id, account.id);
            self.cooling.lock().unwrap().insert((account.id, row.connector.clone()), Instant::now());
            let failed = Phase::Ending(End::Failed(e.clone()));
            for from in [Phase::Creating, Phase::Booting] {
                if hub.db.set_box_phase(&row.id, &from, &failed).await? {
                    let mut row = row.clone();
                    row.phase = failed.clone();
                    if let Err(e) = self.end(hub, account, &row.remains(), Some(keys)).await {
                        eprintln!("pinned-server: could not delete box {}: {e}", row.id);
                    }
                    break;
                }
            }
        }
        booted
    }

    /// Creating → booting, until the box's node is online. Each step checks what is done already:
    /// the box, by the row or by its label; the node, by whether it is online.
    async fn boot(&self, hub: &Hub, account: &Arc<Account>, row: &BoxRow, keys: &Keys) -> Result<Boot, String> {
        let key = keys.api_key.as_str();
        let mut upstash = row.upstash.clone();
        if row.phase == Phase::Creating {
            let (id, made) = match upstash {
                Some(id) => (id, false),
                None => match self.api.list(key, &row_label(&row.id)).await?.into_iter().next() {
                    Some(listed) => (listed.id, false),
                    None => {
                        let new = NewBox {
                            name: format!("pinned-{}", row.id),
                            labels: vec![self.label.clone(), row_label(&row.id)],
                            size: row.size,
                            init_command: format!("sh {DIR}/start.sh || true"),
                        };
                        (self.api.create(key, new).await?, true)
                    }
                },
            };
            if !hub.db.box_created(&row.id, &id).await? {
                // The row ended or another hub created its own box meanwhile, so nothing else
                // knows of this one; once its key is disconnected no sweep can find it either.
                if made {
                    self.api.delete(key, &id).await?;
                }
                return Ok(Boot::Moved);
            }
            upstash = Some(id);
        }
        let id = upstash.ok_or_else(|| format!("box {} is booting without a box", row.id))?;
        let machine = row.machine.to_string();
        if !account.is_online(&machine) {
            self.boot_node(hub, account, row, &id, keys).await?;
            while !account.is_online(&machine) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Ok(Boot::Online)
    }

    /// Gives the box's machine a fresh token, writes what its node signs in and runs with, and
    /// runs the bootstrap, which starts the node.
    async fn boot_node(&self, hub: &Hub, account: &Arc<Account>, row: &BoxRow, id: &BoxId, keys: &Keys) -> Result<(), String> {
        let key = keys.api_key.as_str();
        let connectors = hub.connectors(account).await?;
        let token = hub.db.put_machine(row.machine, account.id, &machine_name(&row.spec), "linux").await?;
        hub.push_machines(account).await;
        let creds = Credentials {
            hub: hub.public_url.clone(),
            email: account.email.clone(),
            user_token: String::new(),
            machine_id: row.machine.to_string(),
            machine_token: token,
        };
        let login = connectors.get_settings().github.map_or_else(|| "pinned".to_owned(), |g| g.login);
        // Nothing promises that a file write makes the folders above it.
        let made = self.api.exec(key, id, format!("mkdir -p {DIR}/bin")).await?;
        if made.exit_code != 0 {
            return Err(format!("could not make {DIR}: {}", tail(&made)));
        }
        self.api.write(key, id, &format!("{DIR}/account.json"), serde_json::to_string_pretty(&creds).unwrap()).await?;
        self.api.write(key, id, &format!("{DIR}/box.env"), box_env(keys, connectors.github_token().as_deref())).await?;
        self.api.write(key, id, &format!("{DIR}/start.sh"), start_script(&login, row.spec.repo.as_deref())).await?;
        let exec = self.api.exec(key, id, bootstrap_script(&hub.public_url)).await?;
        if exec.exit_code != 0 {
            return Err(format!("the bootstrap failed: {}", tail(&exec)));
        }
        Ok(())
    }

    /// Hands the run to the row's machine, unless its session there already works on it. The
    /// machine answers a run sent again with the session it started.
    async fn dispatch(&self, hub: &Hub, account: &Arc<Account>, row: &BoxRow, spec: &RunSpec) -> Result<(), String> {
        let machine = row.machine.to_string();
        if account.working_on(&row.connector, &row.subject, |id| *id == machine) {
            return Ok(());
        }
        let mut spec = spec.clone();
        spec.token = hub.connectors(account).await?.github_token().filter(|_| spec.repo.as_deref().is_some_and(github::hosts));
        hub.call_machine(account, &machine, Call::Run { run: spec }, RUN_WAIT).await.map(drop).map_err(String::from)
    }

    /// Ending → gone, each step a no-op once done: the machine, so its node can't sign in again,
    /// then the box, then the row. Without keys the row stays, to be retried once they are back.
    async fn end(&self, hub: &Hub, account: &Arc<Account>, row: &Remains, keys: Option<&Keys>) -> Result<(), String> {
        let machine = row.machine.to_string();
        hub.db.remove_machine(account.id, row.machine).await?;
        account.went_offline(&machine, None);
        hub.push_machines(account).await;
        let keys = keys.ok_or_else(|| format!("Upstash Box is not connected, so box {} stays until it is", row.upstash.as_ref().map_or(&row.id, |b| &b.0)))?;
        let key = keys.api_key.as_str();
        match &row.upstash {
            Some(id) => self.api.delete(key, id).await?,
            None => {
                for listed in self.api.list(key, &row_label(&row.id)).await? {
                    self.api.delete(key, &listed.id).await?;
                }
            }
        }
        hub.db.delete_box(&row.id).await?;
        self.timers.lock().unwrap().remove(&row.id);
        eprintln!("pinned-server: deleted box {} of account {} ({})", row.id, account.id, row.why);
        Ok(())
    }

    /// Ends the row for `why`, unless it is ending already, and deletes its box.
    async fn finish(&self, hub: &Hub, account: &Arc<Account>, mut row: BoxRow, why: End, keys: Option<&Keys>) -> Result<(), String> {
        if !matches!(row.phase, Phase::Ending(_)) {
            let ending = Phase::Ending(why);
            if !hub.db.set_box_phase(&row.id, &row.phase, &ending).await? {
                return Err(format!("box {} changed while pinned ended it; it ends on the next check", row.id));
            }
            row.phase = ending;
        }
        self.end(hub, account, &row.remains(), keys).await
    }

    /// The session for `subject` opened its pull request, so its box ends once it stops working.
    pub async fn work_done(&self, hub: &Hub, account: &Account, connector: &str, subject: &Subject) -> Result<(), String> {
        if let Some(row) = hub.db.box_for(account.id, connector, subject).await? {
            self.timers.lock().unwrap().entry(row).or_default().done = true;
        }
        Ok(())
    }

    /// Removing a box's machine deletes the box too. Answers whether `machine` ran in one.
    pub async fn end_for_machine(&self, hub: &Hub, account: &Arc<Account>, machine: Uuid) -> Result<bool, String> {
        let Some(row) = hub.db.box_of(account.id, machine).await? else { return Ok(false) };
        let keys = hub.connectors(account).await?.box_keys();
        self.finish(hub, account, row, End::Removed, keys.as_ref()).await?;
        Ok(true)
    }

    /// Deletes every box of the account, before its Box key goes.
    pub async fn end_all(&self, hub: &Hub, account: &Arc<Account>) -> Result<(), String> {
        let keys = hub.connectors(account).await?.box_keys();
        for row in hub.db.boxes(account.id).await? {
            match row {
                Ok(row) => self.finish(hub, account, row, End::Disconnected, keys.as_ref()).await?,
                Err(remains) => self.end(hub, account, &remains, keys.as_ref()).await?,
            }
        }
        Ok(())
    }

    /// One pass over the account's rows: boots rows nobody drives, as after a restart, ends running
    /// rows whose session ended, finishes ending rows, and every `sweep` deletes this hub's boxes
    /// that no row names.
    pub async fn reap(&self, hub: &Arc<Hub>, account: &Arc<Account>) {
        if let Err(e) = self.reap_once(hub, account).await {
            eprintln!("pinned-server: could not look after account {}'s boxes: {e}", account.id);
        }
    }

    async fn reap_once(&self, hub: &Arc<Hub>, account: &Arc<Account>) -> Result<(), String> {
        let keys = hub.connectors(account).await?.box_keys();
        let due = self.swept.lock().unwrap().get(&account.id).is_none_or(|at| at.elapsed() >= self.policy.sweep);
        // Listed before the rows are read, so every box listed has its row among them.
        let listed = match &keys {
            Some(keys) if due => match self.api.list(&keys.api_key, &self.label).await {
                Ok(listed) => {
                    self.swept.lock().unwrap().insert(account.id, Instant::now());
                    Some(listed)
                }
                // Only the sweep needs the list; the rows' own steps still run.
                Err(e) => {
                    eprintln!("pinned-server: could not list account {}'s boxes: {e}", account.id);
                    None
                }
            },
            _ => None,
        };
        let rows = hub.db.boxes(account.id).await?;
        let now = Instant::now();
        for row in rows {
            let row = match row {
                Ok(row) => row,
                Err(remains) => {
                    if let Err(e) = self.end(hub, account, &remains, keys.as_ref()).await {
                        eprintln!("pinned-server: could not end a box of account {}: {e}", account.id);
                    }
                    continue;
                }
            };
            if self.driving.lock().unwrap().contains(&row.id) {
                continue;
            }
            let ended = match &row.phase {
                Phase::Creating | Phase::Booting => match &keys {
                    Some(keys) => {
                        let (hub, account, keys) = (hub.clone(), account.clone(), keys.clone());
                        tokio::spawn(async move {
                            let Some(_driving) = hub.boxes.drive_guard(&row.id) else { return };
                            if let Err(e) = hub.boxes.drive(&hub, &account, row.clone(), &row.spec, &keys).await {
                                eprintln!("pinned-server: could not finish booting a box of account {}: {e}", account.id);
                            }
                        });
                        continue;
                    }
                    None => Some(End::Disconnected),
                },
                Phase::Running => {
                    let gone = listed.as_ref().zip(row.upstash.as_ref()).is_some_and(|(listed, id)| !listed.iter().any(|b| b.id == *id));
                    let seen = account.seen(&row.machine.to_string(), &row.connector, &row.subject);
                    let mut timers = self.timers.lock().unwrap();
                    let verdict = verdict(row.age, seen, timers.entry(row.id.clone()).or_default(), now, &self.policy);
                    if gone { Some(End::Vanished) } else { verdict }
                }
                Phase::Ending(_) => None,
            };
            let finished = match ended {
                Some(why) => self.finish(hub, account, row, why, keys.as_ref()).await,
                None if matches!(row.phase, Phase::Ending(_)) => self.end(hub, account, &row.remains(), keys.as_ref()).await,
                None => Ok(()),
            };
            if let Err(e) = finished {
                eprintln!("pinned-server: could not end a box of account {}: {e}", account.id);
            }
        }
        if let (Some(listed), Some(keys)) = (listed, &keys) {
            for orphan in listed {
                let row = orphan.labels.iter().find_map(|l| l.strip_prefix("p:"));
                if let Some(row) = row {
                    if hub.db.box_exists(row).await? {
                        continue;
                    }
                }
                eprintln!("pinned-server: deleting box {} that no row of this hub names", orphan.id);
                self.api.delete(&keys.api_key, &orphan.id).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn policy() -> Policy {
        Policy { idle: 20 * MIN, exited: 20 * MIN, tab_gone: 20 * MIN, offline: 20 * MIN, max_life: 20 * MIN, ..Policy::default() }
    }

    /// The verdict after `seen` held for `held`, judged at the start and at the end.
    fn after(seen: Seen, held: Duration, age: Duration) -> Option<End> {
        let (start, mut timers) = (Instant::now(), Timers::default());
        assert_eq!(verdict(Duration::ZERO, seen, &mut timers, start, &policy()), None);
        verdict(age, seen, &mut timers, start + held, &policy())
    }

    #[test]
    fn each_end_rule_fires_once_its_condition_held_for_its_limit() {
        let quiet = Seen::Tab { running: true, quiet: true };
        for (seen, end) in [
            (Seen::Offline, End::Offline),
            (Seen::NoTab, End::TabClosed),
            (Seen::Tab { running: false, quiet: true }, End::Exited),
            (quiet, End::Idle),
        ] {
            assert_eq!(after(seen, 19 * MIN, MIN), None, "{seen:?} for 19 of 20 minutes");
            assert_eq!(after(seen, 21 * MIN, MIN), Some(end), "{seen:?} for 21 of 20 minutes");
        }
        assert_eq!(after(Seen::Tab { running: true, quiet: false }, 21 * MIN, MIN), None, "a working session never idles");
    }

    #[test]
    fn max_life_ends_a_box_whatever_its_session_does() {
        let working = Seen::Tab { running: true, quiet: false };
        let mut timers = Timers::default();
        assert_eq!(verdict(19 * MIN, working, &mut timers, Instant::now(), &policy()), None);
        assert_eq!(verdict(21 * MIN, working, &mut timers, Instant::now(), &policy()), Some(End::Expired));
    }

    #[test]
    fn working_resets_quiet() {
        let p = policy();
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p), None);
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: false }, &mut t, start + 15 * MIN, &p), None, "working again");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 16 * MIN, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 35 * MIN, &p), None, "quiet for 19 minutes since it worked");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 37 * MIN, &p), Some(End::Idle));
    }

    #[test]
    fn a_box_whose_pull_request_is_open_ends_soon_after_its_session_stops() {
        let p = Policy { done: 2 * MIN, ..policy() };
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 3 * MIN, &p), None, "quiet without a pull request waits out idle");
        t.done = true;
        assert_eq!(verdict(MIN, Seen::Tab { running: true, quiet: false }, &mut t, start + 4 * MIN, &p), None, "still working");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 5 * MIN, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 6 * MIN, &p), None, "quiet for 1 of 2 minutes");
        assert_eq!(verdict(MIN, quiet, &mut t, start + 7 * MIN, &p), Some(End::Done));
    }

    #[test]
    fn a_dropped_link_does_not_restart_the_quiet_clock() {
        let p = policy();
        let (start, mut t) = (Instant::now(), Timers::default());
        let quiet = Seen::Tab { running: true, quiet: true };
        assert_eq!(verdict(MIN, quiet, &mut t, start, &p), None);
        assert_eq!(verdict(MIN, Seen::Offline, &mut t, start + 10 * MIN, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 19 * MIN, &p), None);
        assert_eq!(verdict(MIN, quiet, &mut t, start + 21 * MIN, &p), Some(End::Idle));
    }

    #[test]
    fn phases_survive_the_database() {
        for phase in [
            Phase::Creating,
            Phase::Booting,
            Phase::Running,
            Phase::Ending(End::Idle),
            Phase::Ending(End::TabClosed),
            Phase::Ending(End::Done),
            Phase::Ending(End::Failed("the bootstrap failed: curl: (22) 404".into())),
        ] {
            let (stored, reason) = phase.stored();
            assert_eq!(Phase::load(stored, reason), phase);
        }
    }

    #[test]
    fn a_hub_labels_its_boxes_within_upstashs_20_characters() {
        let label = hub_label("https://pinned-hub-production.up.railway.app");
        assert_eq!(label.len(), 16);
        assert!(label.starts_with("hub:"));
        assert_ne!(label, hub_label("http://127.0.0.1:8797"));
        assert_eq!(row_label(&new_row_id()).len(), 18);
    }

    #[test]
    fn the_scripts_a_box_runs_carry_no_secret() {
        let keys = Keys { api_key: "box_SECRETKEY".into(), claude_token: "sk-ant-oat01-SECRETCLAUDE".into() };
        let scripts = [bootstrap_script("https://hub.example"), start_script("felipe", Some("acme/app"))];
        for script in &scripts {
            for secret in ["box_SECRETKEY", "SECRETCLAUDE", "ghp_SECRETGITHUB", "machine-token"] {
                assert!(!script.contains(secret), "{secret} in {script}");
            }
            assert!(!script.contains("{{"), "{script}");
        }
        assert!(scripts[0].contains("curl -fsSL 'https://hub.example/download/pinned-server-aarch64-unknown-linux-musl'"));
        assert!(scripts[1].contains("git config --global user.email 'felipe@users.noreply.github.com'"));
        assert!(scripts[1].contains(r#"const clones = ["app"].map"#), "{}", scripts[1]);
        assert!(start_script("felipe", None).contains("const clones = [].map"));
        assert_eq!(
            box_env(&keys, Some("ghp_SECRETGITHUB")),
            "CLAUDE_CODE_OAUTH_TOKEN='sk-ant-oat01-SECRETCLAUDE'\nGH_TOKEN='ghp_SECRETGITHUB'\n"
        );
        assert_eq!(start_script("o'brien", None).lines().find(|l| l.contains("user.name")), Some("git config --global user.name 'o'\\''brien'"));
    }

    #[test]
    fn a_box_machine_is_named_after_its_session() {
        let spec = |args: &[&str]| RunSpec { folder: "/repo".into(), repo: None, args: args.iter().map(|a| a.to_string()).collect(), env: vec![], token: None, account: None };
        assert_eq!(machine_name(&spec(&["--effort", "high", "--name", "ENG-142 Fix login", "prompt"])), "Box · ENG-142 Fix login");
        assert_eq!(machine_name(&spec(&["prompt"])), "Box · pinned");
        assert_eq!(machine_name(&spec(&["--name", &"x".repeat(200)])).chars().count(), 100);
    }

    #[test]
    fn a_failed_bootstrap_quotes_the_end_of_its_output() {
        let long = Exec { exit_code: 1, output: format!("{}\nlast line", "x".repeat(2000)), error: "curl: (22) 404\n".into() };
        let quoted = tail(&long);
        assert_eq!(quoted.chars().count(), TAIL);
        assert!(quoted.ends_with("last linecurl: (22) 404"));
    }
}
