//! A signed-in machine's copy of its account: kept in step with the hub, used when the hub can't
//! answer, and handed back when it can again.

use crate::account_copy::{self, Changes, Copy, Edit, Item, Mirrored, Note, RunSet, Taken, Version};
use crate::agent::Subject;
use crate::connectors::{self, Connectors, FileStorage, RunsOn, Work};
use crate::link::{self, Reach};
use crate::local_hub::{self, AccountSide, LocalHub};
use crate::machine::Machine;
use crate::taste::{self, Tastes};
use crate::wire::{HubCall, MachineId};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

const STATE_FILE: &str = "standby.json";

/// Who finds and fires the account's triggers, as this machine sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Authority {
    /// The hub. This machine keeps its copy current.
    #[default]
    Hub,
    /// This machine, since unix millis `at`, until it hands back.
    Here { at: u64 },
}

/// The login this machine signed in with.
pub struct Signer {
    pub machine: MachineId,
    pub name: String,
    pub login: String,
}

/// What the app shows about the copy.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub copied: bool,
    pub heard_at: Option<u64>,
    pub runner: Option<MachineId>,
    pub machine_name: String,
    pub authority: Authority,
    pub gone_for: Option<Duration>,
    pub notes: Vec<Note>,
    pub holds_changes: bool,
    pub blind: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CopyMeta {
    tag: String,
    #[serde(default)]
    runner: Option<MachineId>,
    at: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct State {
    #[serde(default)]
    copy: Option<CopyMeta>,
    #[serde(default)]
    base: BTreeMap<Item, Version>,
    #[serde(default)]
    edited: BTreeMap<Item, u64>,
    #[serde(default)]
    run_sets: Vec<RunSet>,
    #[serde(default)]
    progressed: bool,
    #[serde(default)]
    notes: Vec<Note>,
    #[serde(default)]
    heard_at: Option<u64>,
    #[serde(default)]
    stopping_seen: bool,
    #[serde(default)]
    authority: Authority,
    #[serde(default)]
    changes_id: uuid::Uuid,
}

enum Step {
    Again,
    Wait,
}

trait Drive: Send + Sync {
    fn call(&self, call: HubCall) -> Result<Value, String>;
    fn online(&self) -> bool;
    fn reach(&self) -> Reach;
}

impl Drive for link::Handle {
    fn call(&self, call: HubCall) -> Result<Value, String> {
        self.call_hub(call)
    }
    fn online(&self) -> bool {
        link::Handle::online(self)
    }
    fn reach(&self) -> Reach {
        link::Handle::reach(self)
    }
}

pub struct Standby {
    dir: PathBuf,
    signer: Signer,
    machine: Arc<Machine>,
    connectors: Arc<Connectors>,
    state: Mutex<State>,
    authority_tx: watch::Sender<Authority>,
    reach: Mutex<Reach>,
    heard: Mutex<Option<tokio::time::Instant>>,
    saw_hello: AtomicBool,
    blind: AtomicBool,
    on_check: Mutex<Option<Arc<dyn Fn(&connectors::Check) + Send + Sync>>>,
    watching: Mutex<Option<tokio::task::JoinHandle<()>>>,
    runtime: tokio::runtime::Runtime,
}

impl Standby {
    /// Opens the copy kept under `dir` for this sign-in, or an empty one. An edit that reached
    /// the store without a stamp is stamped here, so a crash between the two writes still hands
    /// the edit back.
    pub fn open(dir: PathBuf, signer: Signer, machine: Arc<Machine>, exe: String) -> Standby {
        let _ = std::fs::create_dir_all(&dir);
        let mut state = read_state(&dir);
        let connectors = Arc::new(Connectors::new(FileStorage::new(dir.clone()), exe));
        let current = items_of(&connectors, &dir);
        stamp_diff(&mut state.edited, &state.base, &current, connectors::unix_millis());
        write_state(&dir, &state);
        let (authority_tx, _) = watch::channel(state.authority);
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("standby runtime");
        Standby {
            dir,
            signer,
            machine,
            connectors,
            state: Mutex::new(state),
            authority_tx,
            reach: Mutex::new(Reach::Unreachable),
            heard: Mutex::new(None),
            saw_hello: AtomicBool::new(false),
            blind: AtomicBool::new(false),
            on_check: Mutex::new(None),
            watching: Mutex::new(None),
            runtime,
        }
    }

    /// Keeps the copy current, takes over when the hub's address says no hub is there, and hands
    /// back once the hub answers. Runs until the link stops.
    pub fn follow(self: &Arc<Self>, link: link::Handle, on_check: Box<dyn Fn(&connectors::Check) + Send + Sync>) {
        *self.on_check.lock().unwrap() = Some(Arc::from(on_check));
        let standby = Arc::clone(self);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("standby follow");
            runtime.block_on(standby.run(link));
        });
    }

    /// Takes over on the owner's word while the hub's address says no hub is there.
    pub fn take_over(&self) -> Result<(), String> {
        if self.state.lock().unwrap().copy.is_none() {
            return Err("this Mac has no copy of the account".into());
        }
        match *self.reach.lock().unwrap() {
            Reach::Up => Err("the hub is answering".into()),
            Reach::Gone { .. } => {
                self.become_here();
                Ok(())
            }
            Reach::Unreachable | Reach::Refused => Err("this Mac can't tell whether the hub is off".into()),
        }
    }

    pub fn authority(&self) -> watch::Receiver<Authority> {
        self.authority_tx.subscribe()
    }

    pub fn view(&self) -> View {
        let state = self.state.lock().unwrap();
        let gone_for = match *self.reach.lock().unwrap() {
            Reach::Gone { since } => Some(since.elapsed()),
            _ => None,
        };
        View {
            copied: state.copy.is_some(),
            heard_at: state.heard_at,
            runner: state.copy.as_ref().and_then(|copy| copy.runner.clone()),
            machine_name: self.signer.name.clone(),
            authority: state.authority,
            gone_for,
            notes: state.notes.clone(),
            holds_changes: holding(&state),
            blind: self.blind.load(Ordering::SeqCst),
        }
    }

    pub fn dismiss_notes(&self) {
        let mut state = self.state.lock().unwrap();
        state.notes.clear();
        write_state(&self.dir, &state);
    }

    /// Deletes the copy and its keys.
    pub fn forget(&self) {
        if let Some(task) = self.watching.lock().unwrap().take() {
            task.abort();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
        *self.state.lock().unwrap() = State::default();
        let _ = self.authority_tx.send(Authority::Hub);
    }

    pub fn connectors(&self) -> &Arc<Connectors> {
        &self.connectors
    }

    async fn run(self: Arc<Self>, link: link::Handle) {
        let mut online = link.state();
        let mut reach = link.reach_watch();
        let mut stopped = link.stopped();
        loop {
            if *stopped.borrow() {
                break;
            }
            if matches!(self.step(&link).await, Step::Again) {
                continue;
            }
            let pause = self.pause();
            tokio::select! {
                biased;
                result = stopped.changed() => {
                    if result.is_err() || *stopped.borrow() {
                        break;
                    }
                }
                result = online.changed() => {
                    if result.is_err() {
                        break;
                    }
                }
                result = reach.changed() => {
                    if result.is_err() {
                        break;
                    }
                }
                _ = tokio::time::sleep(pause) => {}
            }
        }
        self.stop_watcher().await;
    }

    async fn step(&self, pipe: &impl Drive) -> Step {
        self.note_reach(pipe.reach(), pipe.online());
        if !pipe.online() {
            self.blind.store(false, Ordering::SeqCst);
        }
        if self.blind.load(Ordering::SeqCst) {
            return Step::Wait;
        }
        if pipe.online() && self.holds_changes() {
            return match self.hand_back(pipe).await {
                Ok(()) => Step::Again,
                Err(error) if link::is_unanswered(&error) && pipe.online() && error != link::OFFLINE => Step::Again,
                Err(error) => {
                    if !link::is_unanswered(&error) && !link::is_unknown_command(&error) {
                        eprintln!("pinned: account copy: {error}");
                    }
                    Step::Wait
                }
            };
        }
        if pipe.online() {
            return match self.mirror(pipe).await {
                Ok(true) => Step::Again,
                Ok(false) => Step::Wait,
                Err(error) if link::is_unanswered(&error) => Step::Wait,
                Err(error) => {
                    eprintln!("pinned: account copy: {error}");
                    Step::Wait
                }
            };
        }
        self.consider_takeover();
        if matches!(self.authority_now(), Authority::Here { .. }) {
            self.start_watcher();
        }
        Step::Wait
    }

    fn pause(&self) -> Duration {
        if let Reach::Gone { since } = *self.reach.lock().unwrap() {
            let elapsed = since.elapsed();
            if elapsed < account_copy::TAKEOVER_AFTER {
                return (account_copy::TAKEOVER_AFTER - elapsed).max(Duration::from_millis(50));
            }
        }
        Duration::from_secs(30)
    }

    fn consider_takeover(&self) {
        let reach = self.reach.lock().unwrap().clone();
        let (authority, runner_is_me, stopping_seen) = {
            let state = self.state.lock().unwrap();
            let runner_is_me = state.copy.as_ref().and_then(|copy| copy.runner.as_deref()) == Some(self.signer.machine.as_str());
            (state.authority, runner_is_me, state.stopping_seen)
        };
        let heard = *self.heard.lock().unwrap();
        if due_to_take_over(&reach, authority, runner_is_me, heard, stopping_seen, self.saw_hello.load(Ordering::SeqCst)) {
            self.become_here();
        }
    }

    fn become_here(&self) {
        {
            let mut state = self.state.lock().unwrap();
            if !matches!(state.authority, Authority::Here { .. }) {
                state.authority = Authority::Here { at: connectors::unix_millis() };
                write_state(&self.dir, &state);
                let _ = self.authority_tx.send(state.authority);
            }
        }
        self.start_watcher();
    }

    fn authority_now(&self) -> Authority {
        self.state.lock().unwrap().authority
    }

    async fn hand_back(&self, pipe: &impl Drive) -> Result<(), String> {
        let was_here = matches!(self.authority_now(), Authority::Here { .. });
        if was_here {
            self.stop_watcher().await;
        }
        let result = pipe.call(HubCall::Handback { changes: self.changes(), login: self.signer.login.clone() });
        let classified = self.classify_handback(result);
        if classified.is_err() && was_here && !matches!(self.authority_now(), Authority::Hub) {
            self.start_watcher();
        }
        classified
    }

    fn classify_handback(&self, result: Result<Value, String>) -> Result<(), String> {
        match result {
            Ok(value) => {
                let taken: Taken = serde_json::from_value(value).map_err(|error| format!("the hub's handback: {error}"))?;
                self.install(&taken.copy, false)?;
                let mut state = self.state.lock().unwrap();
                state.edited.clear();
                state.run_sets.clear();
                state.progressed = false;
                state.notes.extend(taken.notes);
                state.authority = Authority::Hub;
                state.changes_id = uuid::Uuid::nil();
                write_state(&self.dir, &state);
                drop(state);
                let _ = self.authority_tx.send(Authority::Hub);
                Ok(())
            }
            Err(error) if link::is_unknown_command(&error) => {
                self.note_older_hub();
                self.blind.store(true, Ordering::SeqCst);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn note_older_hub(&self) {
        let mut state = self.state.lock().unwrap();
        let listed = state.edited.keys().map(|item| item.as_str().to_owned()).collect::<Vec<_>>().join(", ");
        if let Some(item) = state.edited.keys().next().cloned().or_else(|| state.base.keys().next().cloned()) {
            state.notes.push(Note {
                item,
                text: format!("the hub is an older version and did not take back what changed here while it was off: {listed}"),
            });
        }
        state.edited.clear();
        state.run_sets.clear();
        state.progressed = false;
        state.authority = Authority::Hub;
        state.changes_id = uuid::Uuid::nil();
        write_state(&self.dir, &state);
        drop(state);
        let _ = self.authority_tx.send(Authority::Hub);
    }

    async fn mirror(&self, pipe: &impl Drive) -> Result<bool, String> {
        let have = self.state.lock().unwrap().copy.as_ref().map(|copy| copy.tag.clone()).unwrap_or_default();
        match pipe.call(HubCall::Mirror { have, login: self.signer.login.clone() }) {
            Ok(value) => {
                let mirrored: Mirrored = serde_json::from_value(value).map_err(|error| format!("the hub's copy: {error}"))?;
                self.apply_mirrored(mirrored)?;
                Ok(true)
            }
            Err(error) if link::is_unknown_command(&error) => {
                self.blind.store(true, Ordering::SeqCst);
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn apply_mirrored(&self, mirrored: Mirrored) -> Result<(), String> {
        match mirrored {
            Mirrored::Copy { copy, stopping } => {
                if self.holds_changes() {
                    return Ok(());
                }
                self.install(&copy, stopping)
            }
            Mirrored::Unchanged { stopping } => {
                let mut state = self.state.lock().unwrap();
                state.heard_at = Some(connectors::unix_millis());
                state.stopping_seen = stopping;
                write_state(&self.dir, &state);
                *self.heard.lock().unwrap() = Some(tokio::time::Instant::now());
                Ok(())
            }
        }
    }

    fn install(&self, copy: &Copy, stopping: bool) -> Result<(), String> {
        self.connectors.take_copy(copy)?;
        let taste_items: BTreeMap<_, _> = copy.items.iter().filter(|(item, _)| item.is_taste()).map(|(item, version)| (item.clone(), version.clone())).collect();
        if !taste_items.is_empty() {
            let tastes = Tastes::from_copy_items(&taste_items)?;
            taste::save(&tastes, Some(&self.dir))?;
        }
        let mut state = self.state.lock().unwrap();
        state.base = items_of(&self.connectors, &self.dir);
        state.copy = Some(CopyMeta { tag: copy.tag(), runner: copy.runner.clone(), at: connectors::unix_millis() });
        state.heard_at = Some(connectors::unix_millis());
        state.stopping_seen = stopping;
        write_state(&self.dir, &state);
        *self.heard.lock().unwrap() = Some(tokio::time::Instant::now());
        Ok(())
    }

    fn changes(&self) -> Changes {
        let current = self.current_items();
        let progress = self.connectors.copy_parts().1;
        let runs = self.connectors.own_runs();
        let mut state = self.state.lock().unwrap();
        if holding(&state) && state.changes_id.is_nil() {
            state.changes_id = uuid::Uuid::new_v4();
            write_state(&self.dir, &state);
        }
        let edits = state
            .edited
            .iter()
            .map(|(item, at)| (item.clone(), Edit { at: *at, base: state.base.get(item).cloned(), ours: current.get(item).cloned() }))
            .collect();
        let took_over_at = match state.authority {
            Authority::Here { at } => Some(at),
            Authority::Hub => None,
        };
        Changes { id: state.changes_id, edits, progress, runs, run_sets: state.run_sets.clone(), took_over_at }
    }

    fn current_items(&self) -> BTreeMap<Item, Version> {
        items_of(&self.connectors, &self.dir)
    }

    fn has_copy(&self) -> bool {
        self.state.lock().unwrap().copy.is_some()
    }

    pub fn holds_changes(&self) -> bool {
        holding(&self.state.lock().unwrap())
    }

    fn start_watcher(&self) {
        let mut slot = self.watching.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let on_check = self.on_check.lock().unwrap().clone().unwrap_or_else(|| Arc::new(|_: &connectors::Check| {}));
        let connectors = Arc::clone(&self.connectors);
        let machine = Arc::clone(&self.machine);
        let place = self.signer.machine.clone();
        *slot = Some(self.runtime.spawn(async move {
            connectors
                .watch(
                    move |work| {
                        let machine = Arc::clone(&machine);
                        let place = place.clone();
                        async move { take_here(machine, place, work).await }
                    },
                    move |check| on_check(check),
                )
                .await;
        }));
    }

    async fn stop_watcher(&self) {
        if let Some(task) = self.watching.lock().unwrap().take() {
            task.abort();
            let _ = task.await;
        }
    }

    pub(crate) fn note_reach(&self, reach: Reach, online: bool) {
        if online || matches!(reach, Reach::Up) {
            self.saw_hello.store(true, Ordering::SeqCst);
        }
        let stored = if online {
            reach
        } else {
            match reach {
                Reach::Gone { since } => Reach::Gone { since },
                Reach::Refused => Reach::Refused,
                Reach::Up | Reach::Unreachable => Reach::Unreachable,
            }
        };
        *self.reach.lock().unwrap() = stored;
    }

    async fn dispatch(&self, call: HubCall) -> Option<Result<Value, String>> {
        if !self.has_copy() {
            return None;
        }
        let keeping = matches!(&call, HubCall::RunSet { connector, subject, .. } if !self.connectors.has_run(connector, subject));
        let callback = is_callback(&call);
        let before = self.current_items();
        let place = self.signer.machine.clone();
        let hub = LocalHub {
            connectors: &self.connectors,
            machine: &self.machine,
            place_as: &place,
            taste_dir: Some(&self.dir),
            account: Some(self),
        };
        let answered = hub.answer(call).await;
        self.stamp_after(&before);
        let result = match answered {
            local_hub::Answer::HubOnly => return None,
            local_hub::Answer::Done(result) => result,
        };
        if callback && !keeping && result.is_ok() {
            let mut state = self.state.lock().unwrap();
            state.progressed = true;
            write_state(&self.dir, &state);
        }
        Some(result)
    }

    fn stamp_after(&self, before: &BTreeMap<Item, Version>) {
        let after = self.current_items();
        let mut state = self.state.lock().unwrap();
        let before_len = state.edited.len();
        stamp_diff(&mut state.edited, before, &after, connectors::unix_millis());
        if state.edited.len() != before_len {
            write_state(&self.dir, &state);
        }
    }
}

impl AccountSide for Standby {
    fn keep_run_set(&self, set: RunSet) {
        let mut state = self.state.lock().unwrap();
        state.run_sets.push(set);
        write_state(&self.dir, &state);
    }
}

impl link::Local for Standby {
    fn answer(&self, call: &HubCall) -> Option<Result<Value, String>> {
        if !self.has_copy() {
            return None;
        }
        let handle = self.runtime.handle().clone();
        handle.block_on(self.dispatch(call.clone()))
    }

    fn holds_changes(&self) -> bool {
        Standby::holds_changes(self)
    }

    fn own_callback(&self, call: &HubCall) -> bool {
        if !self.has_copy() {
            return false;
        }
        match call {
            HubCall::Stage { connector, subject, .. } | HubCall::RunSet { connector, subject, .. } => self.connectors.has_run(connector, subject),
            HubCall::LinearCreate { connector, subject: Some(issue), .. } => self.connectors.has_run(connector, &Subject::LinearIssue { issue: issue.clone() }),
            HubCall::SlackReply { connector, channel, thread, .. }
            | HubCall::LinearCreate { connector, channel, thread, .. }
            | HubCall::LinearTake { connector, channel, thread, .. } => {
                self.connectors.has_run(connector, &Subject::SlackThread { channel: channel.clone(), thread: thread.clone() })
            }
            _ => false,
        }
    }
}

fn holding(state: &State) -> bool {
    matches!(state.authority, Authority::Here { .. }) || !state.edited.is_empty() || !state.run_sets.is_empty() || state.progressed
}

fn items_of(connectors: &Connectors, dir: &Path) -> BTreeMap<Item, Version> {
    let (mut items, _, _) = connectors.copy_parts();
    if let Some(tastes) = taste::load(Some(dir)) {
        items.extend(tastes.copy_items());
    }
    items
}

/// Stamps every item whose version differs from `base`. A stamp already recorded keeps its time.
fn stamp_diff(edited: &mut BTreeMap<Item, u64>, base: &BTreeMap<Item, Version>, current: &BTreeMap<Item, Version>, now: u64) {
    for (item, version) in current {
        if base.get(item) != Some(version) {
            edited.entry(item.clone()).or_insert(now);
        }
    }
    for item in base.keys() {
        if !current.contains_key(item) {
            edited.entry(item.clone()).or_insert(now);
        }
    }
}

fn is_callback(call: &HubCall) -> bool {
    matches!(
        call,
        HubCall::Stage { .. } | HubCall::SlackReply { .. } | HubCall::LinearCreate { .. } | HubCall::LinearTake { .. } | HubCall::RunSet { .. }
    )
}

fn read_state(dir: &Path) -> State {
    match std::fs::read(dir.join(STATE_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => State::default(),
    }
}

fn write_state(dir: &Path, state: &State) {
    let _ = std::fs::create_dir_all(dir);
    let partial = dir.join(format!("{STATE_FILE}.{}.{:?}", std::process::id(), std::thread::current().id()));
    let Ok(bytes) = serde_json::to_vec_pretty(state) else { return };
    if std::fs::write(&partial, bytes).is_err() {
        return;
    }
    let _ = std::fs::rename(&partial, dir.join(STATE_FILE));
}

async fn take_here(machine: Arc<Machine>, place: String, work: Work) -> Result<Option<connectors::Placement>, String> {
    match work {
        Work::Run(run) if matches!(run.runs_on, RunsOn::FreshBox(_)) => Err("Upstash Boxes need the hub, which is off".into()),
        Work::Run(run) => {
            let spec = run.spec();
            let tab = connectors::off_worker(move || machine.run(&spec)).await?;
            Ok(Some(connectors::Placement { machine: place, tab }))
        }
        Work::Delivery(delivery) => {
            if machine.deliver(&delivery.env(), &delivery.text()) { Ok(None) } else { Err("no session answers that thread".into()) }
        }
        Work::Prompt { env, text } => {
            if machine.deliver(&env, &text) { Ok(None) } else { Err("no session answers that thread".into()) }
        }
    }
}

/// Whether the runner should start watching. `heard` and `since` share one clock, so a paused
/// test moves both. `Gone` before this process has seen the hub does not count.
pub(crate) fn due_to_take_over(
    reach: &Reach,
    authority: Authority,
    runner_is_me: bool,
    heard: Option<tokio::time::Instant>,
    stopping_seen: bool,
    saw_hello: bool,
) -> bool {
    if !saw_hello || !runner_is_me || !matches!(authority, Authority::Hub) {
        return false;
    }
    let Reach::Gone { since } = reach else { return false };
    if since.elapsed() < account_copy::TAKEOVER_AFTER {
        return false;
    }
    stopping_seen || fresh(heard, *since)
}

fn fresh(heard: Option<tokio::time::Instant>, since: tokio::time::Instant) -> bool {
    match heard {
        Some(heard) if heard >= since => true,
        Some(heard) => since.saturating_duration_since(heard) <= account_copy::FRESH,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::Connector;
    use crate::machine::refuse_real_home;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-standby-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn machine_in(dir: &Path) -> Arc<Machine> {
        let launch = crate::machine::Launch {
            exe: dir.join("pinned"),
            bins: BTreeMap::new(),
            path_env: None,
            settings: dir.join("claude-settings.json"),
            mcp_config: None,
            reminders: false,
            plugin_dir: dir.join("plugin"),
            tutorial_helper: dir.join("helper"),
            sock: dir.join("sock"),
            vector_search: false,
            first_run_wait: Duration::from_millis(1),
            starting_limit: Duration::from_secs(1),
            claude_home: Some(dir.join("claude")),
        };
        Machine::new(launch, Some(dir.join("machine")), None, |_| {})
    }

    fn workflow(instructions: &str) -> Connector {
        serde_json::from_value(json!({
            "name": "Triage",
            "enabled": true,
            "trigger": {"kind": "manual"},
            "folder": "/work",
            "model": "",
            "effort": "medium",
            "permissionMode": "acceptEdits",
            "worktree": false,
            "instructions": instructions,
            "graph": {
                "nodes": [
                    {"id": "t", "name": "Trigger", "position": [0, 0], "type": "trigger", "watch": {"kind": "manual"}},
                    {"id": "s", "name": "Do", "position": [1, 0], "type": "session"}
                ],
                "edges": [{"from": "t", "port": "out", "to": "s"}]
            }
        }))
        .unwrap()
    }

    fn opened(dir: PathBuf) -> Standby {
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let machine = machine_in(&home);
        let signer = Signer { machine: machine.listed_id(), name: "MacBook".into(), login: "user-token".into() };
        Standby::open(dir.join("copy"), signer, machine, home.join("pinned").display().to_string())
    }

    struct Script {
        online: AtomicBool,
        reach: Mutex<Reach>,
        answers: Mutex<Vec<Result<Value, String>>>,
        calls: Mutex<Vec<HubCall>>,
    }

    impl Drive for Script {
        fn call(&self, call: HubCall) -> Result<Value, String> {
            self.calls.lock().unwrap().push(call);
            self.answers.lock().unwrap().pop().unwrap_or_else(|| Err(link::OFFLINE.into()))
        }
        fn online(&self) -> bool {
            self.online.load(Ordering::SeqCst)
        }
        fn reach(&self) -> Reach {
            self.reach.lock().unwrap().clone()
        }
    }

    #[test]
    fn an_edit_written_but_not_stamped_is_handed_back() {
        refuse_real_home();
        let dir = scratch("unstamped");
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let machine = machine_in(&home);
        let copy = dir.join("copy");
        std::fs::create_dir_all(&copy).unwrap();
        let connectors = Connectors::new(FileStorage::new(copy.clone()), home.join("pinned").display().to_string());
        let saved = connectors.save_connector(workflow("one")).unwrap();
        let (items, _, _) = connectors.copy_parts();
        let item = Item::workflow(&saved.id);
        write_state(
            &copy,
            &State {
                copy: Some(CopyMeta { tag: "base".into(), runner: Some(machine.listed_id()), at: 1 }),
                base: items,
                edited: BTreeMap::from([(Item::taste("placement"), 10)]),
                ..State::default()
            },
        );
        let mut again = saved.clone();
        again.instructions = "two".into();
        connectors.save_connector(again).unwrap();
        drop(connectors);

        let standby = Standby::open(
            copy,
            Signer { machine: machine.listed_id(), name: "MacBook".into(), login: "user-token".into() },
            machine,
            home.join("pinned").display().to_string(),
        );
        let changes = standby.changes();
        let edit = changes.edits.get(&item).expect("the unstamped edit");
        assert_eq!(edit.base.as_ref().unwrap().value["instructions"], json!("one"));
        assert_eq!(edit.ours.as_ref().unwrap().value["instructions"], json!("two"));
        assert_eq!(standby.state.lock().unwrap().edited.get(&Item::taste("placement")), Some(&10));
        assert_eq!(standby.changes().id, changes.id);
        assert!(!changes.id.is_nil());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stamp_already_recorded_keeps_its_time() {
        refuse_real_home();
        let dir = scratch("kept-stamp");
        let standby = opened(dir.clone());
        let saved = standby.connectors.save_connector(workflow("one")).unwrap();
        let item = Item::workflow(&saved.id);
        {
            let mut state = standby.state.lock().unwrap();
            state.copy = Some(CopyMeta { tag: "base".into(), runner: Some(standby.signer.machine.clone()), at: 1 });
            state.base = standby.current_items();
            state.edited.insert(item.clone(), 10);
            write_state(&standby.dir, &state);
        }
        let mut again = saved;
        again.instructions = "two".into();
        standby.connectors.save_connector(again).unwrap();
        let current = standby.current_items();
        let mut state = standby.state.lock().unwrap();
        let base = state.base.clone();
        stamp_diff(&mut state.edited, &base, &current, 99);
        assert_eq!(state.edited.get(&item), Some(&10));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_taken_handback_clears_the_edits_and_an_older_hub_keeps_them_as_a_note() {
        refuse_real_home();
        let dir = scratch("handback");
        let standby = opened(dir.clone());
        let saved = standby.connectors.save_connector(workflow("one")).unwrap();
        let item = Item::workflow(&saved.id);
        {
            let mut state = standby.state.lock().unwrap();
            state.copy = Some(CopyMeta { tag: "base".into(), runner: Some(standby.signer.machine.clone()), at: 1 });
            state.base = standby.current_items();
            state.edited.insert(item.clone(), 10);
            write_state(&standby.dir, &state);
        }
        let first = standby.changes().id;
        assert!(standby.classify_handback(Err(link::OFFLINE.into())).is_err());
        assert_eq!(standby.state.lock().unwrap().edited.get(&item), Some(&10));
        assert_eq!(standby.changes().id, first);

        let copy = Copy {
            items: standby.current_items(),
            progress: account_copy::Progress::default(),
            fixed: json!({}),
            runner: Some(standby.signer.machine.clone()),
        };
        standby.classify_handback(Ok(serde_json::to_value(Taken { notes: vec![], copy }).unwrap())).unwrap();
        assert!(standby.state.lock().unwrap().edited.is_empty());
        assert!(standby.changes().id.is_nil());
        assert_eq!(standby.authority_now(), Authority::Hub);

        standby.state.lock().unwrap().edited.insert(item.clone(), 11);
        standby.classify_handback(Err("this hub runs pinned 0.18.7, which can't answer that".into())).unwrap();
        let state = standby.state.lock().unwrap();
        assert!(state.edited.is_empty());
        assert_eq!(state.authority, Authority::Hub);
        assert!(state.notes[0].text.contains("older version"));
        assert!(standby.blind.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mirror_writes_tastes_and_does_not_cover_an_edit() {
        refuse_real_home();
        let dir = scratch("taste");
        let standby = opened(dir.clone());
        let mut items = BTreeMap::new();
        items.insert(Item::taste("captains/parallel"), Version { value: json!(2), secrets: BTreeMap::new() });
        let copy = Copy {
            items: items.clone(),
            progress: account_copy::Progress::default(),
            fixed: json!({}),
            runner: Some(standby.signer.machine.clone()),
        };
        standby.apply_mirrored(Mirrored::Copy { copy: Box::new(copy), stopping: false }).unwrap();
        assert_eq!(taste::load(Some(&standby.dir)).unwrap().captains.parallel, 2);
        assert!(standby.dir.join("taste.json").is_file());
        standby.state.lock().unwrap().edited.insert(Item::taste("captains/parallel"), 4);
        items.insert(Item::taste("captains/parallel"), Version { value: json!(9), secrets: BTreeMap::new() });
        let replacement = Copy {
            items,
            progress: account_copy::Progress::default(),
            fixed: json!({}),
            runner: Some(standby.signer.machine.clone()),
        };
        standby.apply_mirrored(Mirrored::Copy { copy: Box::new(replacement), stopping: true }).unwrap();
        assert_eq!(taste::load(Some(&standby.dir)).unwrap().captains.parallel, 2);
        assert_eq!(standby.state.lock().unwrap().edited.get(&Item::taste("captains/parallel")), Some(&4));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn while_the_copy_holds_an_edit_the_first_call_is_the_handback() {
        refuse_real_home();
        let dir = scratch("order");
        let standby = opened(dir.clone());
        let saved = standby.connectors.save_connector(workflow("one")).unwrap();
        {
            let mut state = standby.state.lock().unwrap();
            state.copy = Some(CopyMeta { tag: "have".into(), runner: Some(standby.signer.machine.clone()), at: 1 });
            state.edited.insert(Item::workflow(&saved.id), 10);
        }
        let copy = Copy {
            items: standby.current_items(),
            progress: account_copy::Progress::default(),
            fixed: json!({}),
            runner: Some(standby.signer.machine.clone()),
        };
        let taken = serde_json::to_value(Taken { notes: vec![], copy }).unwrap();
        let script = Script {
            online: AtomicBool::new(true),
            reach: Mutex::new(Reach::Up),
            answers: Mutex::new(vec![Ok(taken)]),
            calls: Mutex::new(vec![]),
        };
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(standby.step(&script));
        let calls = script.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls.first(), Some(HubCall::Handback { .. })));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn take_over_runs_here_only_when_the_address_says_the_hub_is_gone() {
        refuse_real_home();
        let dir = scratch("takeover");
        let standby = opened(dir.clone());
        standby.state.lock().unwrap().copy = Some(CopyMeta { tag: "x".into(), runner: Some(standby.signer.machine.clone()), at: 1 });
        standby.note_reach(Reach::Up, true);
        assert_eq!(standby.take_over().unwrap_err(), "the hub is answering");
        standby.note_reach(Reach::Unreachable, false);
        assert_eq!(standby.take_over().unwrap_err(), "this Mac can't tell whether the hub is off");
        standby.note_reach(Reach::Gone { since: tokio::time::Instant::now() }, false);
        standby.take_over().unwrap();
        assert!(matches!(standby.authority_now(), Authority::Here { .. }));
        standby.forget();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn a_fresh_runner_is_due_after_five_minutes_and_not_before_a_hello() {
        let heard = tokio::time::Instant::now();
        let since = tokio::time::Instant::now();
        tokio::time::advance(account_copy::TAKEOVER_AFTER).await;
        let reach = Reach::Gone { since };
        assert!(!due_to_take_over(&reach, Authority::Hub, true, Some(heard), false, false));
        assert!(due_to_take_over(&reach, Authority::Hub, true, Some(heard), false, true));
        let stale = tokio::time::Instant::now();
        tokio::time::advance(account_copy::FRESH + Duration::from_secs(1)).await;
        let later = Reach::Gone { since: tokio::time::Instant::now() };
        tokio::time::advance(account_copy::TAKEOVER_AFTER).await;
        assert!(!due_to_take_over(&later, Authority::Hub, true, Some(stale), false, true));
        assert!(due_to_take_over(&later, Authority::Hub, true, Some(stale), true, true));
        assert!(!due_to_take_over(&later, Authority::Hub, false, Some(stale), true, true));
    }
}
