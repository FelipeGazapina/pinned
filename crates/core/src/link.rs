//! The binary installs rustls's ring provider before spawning the link.

use crate::account::Credentials;
use crate::agent::Message;
use crate::machine::{Machine, ViewerId};
use crate::{skills, sync};
use crate::wire::{Bytes, Call, HubCall, MachineEvent, MachineFrame, MachineView, Reply, TabId, ToMachine};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};
use tokio::net::TcpStream;
use tokio::sync::{mpsc as channel, watch};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message as Frame;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const PING: Duration = Duration::from_secs(20);
/// A hub silent this long, pongs included, is gone even if the socket hasn't said so.
const SILENT: Duration = Duration::from_secs(60);
/// A dial still unanswered by then sits on a half-open connection TCP itself never times out.
const DIAL_WAIT: Duration = Duration::from_secs(15);
const HELLO_WAIT: Duration = Duration::from_secs(15);
const CALL_WAIT: Duration = Duration::from_secs(60);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A connection that stayed up this long was healthy, so the one that follows its end starts over
/// at `BACKOFF_MIN` instead of continuing the climb of the failed attempts before it.
const BACKOFF_STABLE: Duration = Duration::from_secs(30);
/// Every wait varies by up to this fraction either way, so machines a hub restart dropped together
/// don't all come back at the same moment.
const BACKOFF_JITTER: f64 = 0.25;
const SYNC_FIRST: Duration = Duration::from_secs(5);
const SYNC_EVERY: Duration = Duration::from_secs(20);
/// After the hub refuses a push at the account's limit, the account's pushes that grow what the hub
/// holds wait this long, twice as long after each refused probe, up to `PUSH_HOLD_MAX`.
const PUSH_HOLD_MIN: Duration = SYNC_EVERY;
const PUSH_HOLD_MAX: Duration = Duration::from_secs(10 * 60);
/// The close code the hub sends when it doesn't know the token a socket signed in with.
pub const UNAUTHORIZED: u16 = 4001;
/// How long a box's node is refused with `UNAUTHORIZED`, with no hello accepted in between, before
/// the node ends its sessions and exits. The node leaves on a refusal that arrives at or after
/// this span from the first one; a hub that is merely unreachable never makes it leave. A live box
/// can be refused for a while: a re-boot gives the box a new token before the bootstrap kills the
/// old node (up to 4 minutes), and sign-in can be slow. There is no setting for it.
pub const BOX_REFUSED_FOR: Duration = Duration::from_secs(15 * 60);

/// The span `Shared::note` measures refusals against. It is `BOX_REFUSED_FOR` everywhere but in
/// the tests that run `run` against a real socket and need it short.
#[derive(Clone, Copy)]
struct RefusedSpan(Duration);

impl Default for RefusedSpan {
    fn default() -> Self {
        RefusedSpan(BOX_REFUSED_FOR)
    }
}

pub struct Link {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
    thread: std::thread::JoinHandle<()>,
    syncing: std::thread::JoinHandle<()>,
}

struct Outgoing {
    text: String,
    wrote: Option<mpsc::Sender<()>>,
}

#[derive(Default)]
struct Shared {
    out: Mutex<Option<channel::UnboundedSender<Outgoing>>>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>,
    next_call: AtomicU64,
    subscribed: Mutex<HashSet<(TabId, ViewerId)>>,
    sync_now: AtomicBool,
    refusals: watch::Sender<Refusals>,
    refused_span: RefusedSpan,
}

impl Shared {
    /// Records how a connection attempt went in the window `Link::refused_for` waits on.
    fn note(&self, outcome: Outcome) {
        let now = tokio::time::Instant::now();
        self.refusals.send_if_modified(|refusals| {
            let before = *refusals;
            refusals.note(outcome, now, self.refused_span.0);
            *refusals != before
        });
    }

    fn send(&self, frame: &MachineFrame) -> bool {
        let out = self.out.lock().unwrap().clone();
        out.is_some_and(|out| out.send(Outgoing { text: serde_json::to_string(frame).unwrap(), wrote: None }).is_ok())
    }

    fn send_flushed(&self, frame: &MachineFrame) -> bool {
        let Some(out) = self.out.lock().unwrap().clone() else { return false };
        let (wrote, written) = mpsc::channel();
        if out.send(Outgoing { text: serde_json::to_string(frame).unwrap(), wrote: Some(wrote) }).is_err() {
            return false;
        }
        // A timeout means the frame is still queued. False would emit it again, and a newer phase can pass the older one.
        match written.recv_timeout(Duration::from_secs(5)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => false,
        }
    }

    /// Asks the hub, blocking up to 60 seconds for the answer. Fails at once while offline.
    fn call(&self, call: HubCall) -> Result<Value, String> {
        let id = self.next_call.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, answer);
        if !self.send(&MachineFrame::Call { id, call }) {
            self.pending.lock().unwrap().remove(&id);
            return Err(OFFLINE.into());
        }
        match answered.recv_timeout(CALL_WAIT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(HUB_TIMEOUT.into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(DROPPED.into()),
        }
    }

    fn online(&self) -> bool {
        self.out.lock().unwrap().is_some()
    }

    fn disconnected(&self, machine: &Machine) {
        *self.out.lock().unwrap() = None;
        self.pending.lock().unwrap().clear();
        for (tab, viewer) in self.subscribed.lock().unwrap().drain() {
            machine.detach(&tab, &viewer);
        }
        machine.drop_remote_thread_watchers();
    }
}

/// Connects `machine` to the hub `creds` names, and keeps it connected until `Link::stop`.
/// `version` is the binary's, which the hub logs.
pub fn spawn(creds: Credentials, machine: Arc<Machine>, version: &str) -> Link {
    let shared = Arc::new(Shared::default());
    let flushing = shared.clone();
    machine.set_flusher(Arc::new(move |event| flushing.send_flushed(&MachineFrame::Event { event: event.clone() })));
    let (stop, stopped) = watch::channel(false);
    let (synced, synced_machine, sync_stopped) = (shared.clone(), machine.clone(), stopped.clone());
    let syncing = std::thread::spawn(move || sync_projects(&synced, &synced_machine, &sync_stopped));
    let (waking, sync_thread) = (shared.clone(), syncing.thread().clone());
    machine.set_sync_waker(Arc::new(move || {
        waking.sync_now.store(true, Ordering::SeqCst);
        sync_thread.unpark();
    }));
    let calling = shared.clone();
    machine.set_hub(Arc::new(move |call| calling.call(call)));
    let events = shared.clone();
    let listener = machine.subscribe(Box::new(move |event| {
        events.send(&MachineFrame::Event { event: event.clone() });
    }));
    let (link, version) = (shared.clone(), version.to_owned());
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("could not start the hub link's runtime");
        runtime.block_on(run(link, creds, machine, version, stopped, listener));
    });
    Link { shared, stop, thread, syncing }
}

/// What a hub call answers while the link is down.
pub const OFFLINE: &str = "not connected to the hub";
/// What a hub call that waited out `CALL_WAIT` fails with: the hub may still be working on it.
pub const HUB_TIMEOUT: &str = "the hub did not answer within 60 seconds";
const DROPPED: &str = "the connection to the hub dropped";

/// Whether a hub call failed for want of an answer, not because the hub refused it: it may have
/// run anyway, and asking again is safe only for a call the hub answers the same way twice.
pub fn is_unanswered(error: &str) -> bool {
    matches!(error, HUB_TIMEOUT | OFFLINE | DROPPED)
}

impl Link {
    /// Asks the hub, blocking up to 60 seconds for the answer. Fails at once while offline.
    pub fn call_hub(&self, call: HubCall) -> Result<Value, String> {
        self.shared.call(call)
    }

    /// Passes a connector session's callback to the hub and answers what the session prints.
    /// Every other socket message is the machine's own business.
    pub fn forward(&self, message: Message) -> Result<String, String> {
        let call = match message {
            Message::Stage { connector, subject, stage } => HubCall::Stage { connector, subject, stage },
            Message::SlackReply { connector, channel, thread, text } => HubCall::SlackReply { connector, channel, thread, text },
            Message::LinearCreate { connector, channel, thread, title, description, assignee } => {
                HubCall::LinearCreate { connector, channel, thread, title, description, assignee }
            }
            Message::LinearTake { connector, channel, thread, issue } => HubCall::LinearTake { connector, channel, thread, issue },
            Message::RunSet { connector, subject, name, value } => HubCall::RunSet { connector, subject, name, value },
            Message::AskPeople { situation, question, choices, step } => {
                HubCall::Team { team: None, op: crate::teams::TeamOp::AskPeople { situation, question, choices, step } }
            }
            _ => return Err("only a workflow session's callbacks go to the hub".into()),
        };
        match self.call_hub(call)? {
            Value::String(text) => Ok(text),
            other => Ok(other.to_string()),
        }
    }

    pub fn online(&self) -> bool {
        self.shared.online()
    }

    /// Resolves when the hub refuses this machine's token (close code `UNAUTHORIZED`) at or after
    /// `BOX_REFUSED_FOR` from the first refusal, with no hello accepted in between. It never
    /// resolves while the hub accepts the machine, and nothing else, such as a dropped connection
    /// or a hub that does not answer, starts the span or ends it: it takes a fresh refusal.
    pub fn refused_for(&self) -> impl std::future::Future<Output = ()> {
        wait_refused(self.shared.refusals.subscribe())
    }

    pub fn stop(self) {
        let _ = self.stop.send(true);
        self.syncing.thread().unpark();
        let _ = self.thread.join();
        let _ = self.syncing.join();
    }
}

impl sync::Hub for Shared {
    fn list(&self, project: &str) -> Result<Vec<sync::Entry>, String> {
        decode(self.call(HubCall::SyncList { project: project.into() })?)
    }
    fn push(&self, project: &str, files: Vec<sync::File>) -> Result<(), String> {
        self.call(HubCall::SyncPush { project: project.into(), files }).map(drop)
    }
    fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<sync::File>, String> {
        decode(self.call(HubCall::SyncPull { project: project.into(), paths })?)
    }
    fn mark_executable(&self, project: &str, repairs: Vec<sync::ModeRepair>) -> Result<(), String> {
        self.call(HubCall::SyncMarkExecutable { project: project.into(), repairs }).map(drop)
    }
}

impl sync::Hub for Link {
    fn list(&self, project: &str) -> Result<Vec<sync::Entry>, String> {
        sync::Hub::list(self.shared.as_ref(), project)
    }
    fn push(&self, project: &str, files: Vec<sync::File>) -> Result<(), String> {
        sync::Hub::push(self.shared.as_ref(), project, files)
    }
    fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<sync::File>, String> {
        sync::Hub::pull(self.shared.as_ref(), project, paths)
    }
    fn mark_executable(&self, project: &str, repairs: Vec<sync::ModeRepair>) -> Result<(), String> {
        sync::Hub::mark_executable(self.shared.as_ref(), project, repairs)
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("the hub sent something pinned can't read: {e}"))
}

/// Keeps the machine's projects in step with the account's while the link runs.
fn sync_projects(shared: &Shared, machine: &Machine, stopped: &watch::Receiver<bool>) {
    let file = machine.sync_file().map(Path::to_path_buf);
    Syncer::default().run(shared, machine, file.as_deref(), stopped);
}

/// Tells the hub how busy this machine is and which accounts it has. Blocks while it reads the
/// tastes and the catalogs, so only a thread of its own calls it.
fn report_state(shared: &Shared, machine: &Machine) {
    if shared.online() {
        shared.send(&MachineFrame::Event { event: MachineEvent::State { state: machine.report_state() } });
    }
}

#[derive(Default)]
struct Syncer {
    state: Option<sync::State>,
    reported_unreadable: bool,
    shared_projects: HashSet<String>,
    reported_old_hub: bool,
    backoff: PushBackoff,
}

/// Holds back the machine's pushes once the hub refused one at a sync limit. The usual cause is
/// hub-wide, the space the whole hub has left, and an account's own limit only where the hub sets
/// one; either way the hub refuses a push that grows past it whichever project sends it, so one
/// hold covers every project and plugin share. Until it ends, a sync sends only what does not grow
/// the hub's copy (see `sync::sync_shrinking`); when the wait is over, the next cycle is a probe
/// that sends everything. A probe cycle in which no sync is refused at the limit and one came back
/// Ok ends the hold, whatever other errors its syncs had; one whose every sync errored leaves it as
/// it is, to probe again; one in which any is refused doubles it, whatever the other projects did. The hold is not restarted when the files to send change. The project lists and
/// pulls every cycle all the same.
///
/// A cycle runs `begin`, then a sync of each project and plugin share, then `finish`.
struct PushBackoff {
    limit: Option<Limit>,
    draw: fn() -> f64,
    round: Round,
}

/// What the syncs of the cycle in progress did.
#[derive(Default)]
struct Round {
    /// The hold is on, and its wait is over: this cycle is a probe.
    probe: bool,
    /// The hold is on, and its wait is not over: this cycle only shrinks.
    held: bool,
    refused: bool,
    /// A sync of the cycle came back Ok: it reached the hub, and pushed or had nothing to push.
    synced: bool,
    /// A sync of the cycle never ran, so the cycle says nothing of the hub's room.
    unclean: bool,
}

struct Limit {
    /// The hold before jitter: `PUSH_HOLD_MIN` after the first refusal, doubled after each refused
    /// probe cycle, up to `PUSH_HOLD_MAX`.
    wait: Duration,
    /// The end of the wait: the jittered hold, never past `PUSH_HOLD_MAX`.
    until: Instant,
    /// The hub's refusal that set the hold or last doubled it, which the app's notice names.
    refusal: String,
}

impl Default for PushBackoff {
    fn default() -> Self {
        Self::with_draw(draw)
    }
}

impl PushBackoff {
    /// `draw` is the jitter source: a draw in `0.0..1.0` for each hold.
    fn with_draw(draw: fn() -> f64) -> Self {
        PushBackoff { limit: None, draw, round: Round::default() }
    }

    /// Starts a cycle: with the hold on and its wait over it is a probe.
    fn begin(&mut self, now: Instant) {
        let on = self.limit.is_some();
        let shrinking = self.shrinking(now);
        self.round = Round { probe: on && !shrinking, held: on && shrinking, ..Round::default() };
    }

    /// Ends the cycle: the hold ends when a probe cycle ran every sync, none was refused at the
    /// limit and at least one came back Ok. A sync that failed for another reason says nothing of
    /// the hub's room, so it does not keep the hold; but a probe whose every sync failed, perhaps
    /// before it pushed, has not asked the hub anything, so the hold stays as it is.
    fn finish(&mut self) {
        let round = std::mem::take(&mut self.round);
        if round.probe && !round.refused && round.synced && !round.unclean {
            self.limit = None;
        }
    }

    /// Marks the cycle as one that left a project or plugin share unsynced.
    fn skipped(&mut self) {
        self.round.unclean = true;
    }

    fn sync(&mut self, hub: &impl sync::Hub, state: &mut sync::State, checkout: &sync::Checkout, now: Instant) -> Result<sync::Synced, String> {
        let shrinking = self.shrinking(now);
        let done = if shrinking { sync::sync_shrinking(hub, state, checkout, SystemTime::now()) } else { sync::sync(hub, state, checkout, SystemTime::now()) };
        self.record(done, now)
    }

    fn sync_plugins(&mut self, hub: &impl sync::Hub, state: &mut sync::State, share: &sync::PluginShare, now: Instant) -> Result<sync::Synced, String> {
        let shrinking = self.shrinking(now);
        let done = if shrinking { sync::sync_plugins_shrinking(hub, state, share, SystemTime::now()) } else { sync::sync_plugins(hub, state, share, SystemTime::now()) };
        self.record(done, now)
    }

    /// Whether the hold is on at `now`: pushes send only what does not grow the hub's copy.
    fn shrinking(&self, now: Instant) -> bool {
        self.limit.as_ref().is_some_and(|limit| now < limit.until)
    }

    /// Notes what a sync did. The first refusal of a cycle sets the hold, or doubles it when the
    /// cycle is a probe; a refusal of a cycle that only shrinks leaves the hold as it is.
    fn record(&mut self, done: Result<sync::Synced, String>, now: Instant) -> Result<sync::Synced, String> {
        if done.is_ok() {
            self.round.synced = true;
        }
        match &done {
            Ok(sync::Synced { push: sync::Push::Refused { error }, .. }) => {
                let first = !self.round.refused;
                self.round.refused = true;
                if first && !self.round.held {
                    let wait = match &self.limit {
                        Some(limit) => (limit.wait * 2).min(PUSH_HOLD_MAX),
                        None => PUSH_HOLD_MIN,
                    };
                    self.limit = Some(Limit { wait, until: now + jittered(wait, (self.draw)()).min(PUSH_HOLD_MAX), refusal: error.clone() });
                }
            }
            _ => {}
        }
        done
    }

    /// Whether the hub's limit holds the account's pushes, probe due or not.
    #[cfg(test)]
    fn limited(&self) -> bool {
        self.limit.is_some()
    }

    /// The refusal the app's notice names while the hold is on, `None` once it ends.
    fn refusal(&self) -> Option<String> {
        self.limit.as_ref().map(|limit| limit.refusal.clone())
    }

    /// A user action ends the hold, so the next sync is a probe.
    fn end(&mut self) {
        self.limit = None;
    }
}

impl Syncer {
    fn run(mut self, shared: &Shared, machine: &Machine, file: Option<&Path>, stopped: &watch::Receiver<bool>) {
        let mut wait = SYNC_FIRST;
        'run: loop {
            let until = Instant::now() + wait;
            while let Some(left) = until.checked_duration_since(Instant::now()) {
                if *stopped.borrow() || stopped.has_changed().is_err() {
                    break 'run;
                }
                if shared.sync_now.load(Ordering::SeqCst) && shared.online() {
                    break;
                }
                std::thread::park_timeout(left);
            }
            wait = SYNC_EVERY;
            if shared.online() {
                report_state(shared, machine);
                match file {
                    Some(file) => self.cycle(shared, shared, machine, file),
                    None => shared.sync_now.store(false, Ordering::SeqCst),
                }
            }
        }
        if let Some(state) = self.state.as_mut() {
            if let Err(e) = state.flush() {
                eprintln!("pinned: {e}");
            }
        }
        machine.hold_sync_state(false);
        machine.hold_sync_limit(None);
    }

    fn cycle(&mut self, shared: &Shared, hub: &impl sync::Hub, machine: &Machine, file: &Path) {
        // A user action ends every hold, so the refused pushes go out in this cycle.
        if shared.sync_now.swap(false, Ordering::SeqCst) {
            self.backoff.end();
        }
        if self.state.is_none() {
            match sync::State::load(file, &crate::machine::home().join(".claude")) {
                Ok(loaded) => {
                    self.state = Some(loaded);
                    machine.hold_sync_state(true);
                }
                Err(e) => {
                    if !self.reported_unreadable {
                        eprintln!("pinned: could not sync: {e}. Start sync over from Settings, Machines in the app.");
                        self.reported_unreadable = true;
                    }
                    return;
                }
            }
        }
        let Some(state) = self.state.as_mut() else { return };
        let checkouts = machine.checkouts();
        let shares = machine.plugin_shares();
        // With nothing left on the machine to send, the account's hold has nothing to hold.
        if checkouts.is_empty() && shares.is_empty() {
            self.backoff.end();
        }
        self.backoff.begin(Instant::now());
        for checkout in checkouts {
            if !shared.online() {
                self.backoff.skipped();
                break;
            }
            let key = checkout.project.key.clone();
            if !self.shared_projects.contains(&key) && shared.call(HubCall::ShareProject { project: checkout.project.clone() }).is_ok() {
                self.shared_projects.insert(key.clone());
            }
            match self.backoff.sync(hub, state, &checkout, Instant::now()) {
                Ok(done) => {
                    machine.skills_changed(&skills::pulled_by_project_sync(&checkout, &done.pulled));
                    if !done.pushed.is_empty() || !done.pulled.is_empty() {
                        eprintln!("pinned: synced {key}: {} up, {} down", done.pushed.len(), done.pulled.len())
                    }
                    if let sync::Push::Refused { error, .. } = &done.push {
                        eprintln!("pinned: could not send {key}'s changes: {error}");
                    }
                }
                Err(e) => eprintln!("pinned: could not sync {key}: {e}"),
            }
        }
        for share in shares {
            if !shared.online() {
                self.backoff.skipped();
                break;
            }
            match self.backoff.sync_plugins(hub, state, &share, Instant::now()) {
                Ok(done) => {
                    self.reported_old_hub = false;
                    machine.skills_changed(&skills::pulled_by_plugin_sync(&share, &done.pulled));
                    if !done.pushed.is_empty() || !done.pulled.is_empty() {
                        eprintln!("pinned: synced plugins: {} up, {} down", done.pushed.len(), done.pulled.len());
                    }
                    if let sync::Push::Refused { error, .. } = &done.push {
                        eprintln!("pinned: could not send the plugins' changes: {error}");
                    }
                }
                Err(e) if sync::old_hub(&e) => {
                    if !self.reported_old_hub {
                        eprintln!("pinned: this hub does not share plugins yet: {e}");
                        self.reported_old_hub = true;
                    }
                    break;
                }
                Err(e) => eprintln!("pinned: could not sync plugins: {e}"),
            }
        }
        self.backoff.finish();
        machine.hold_sync_limit(self.backoff.refusal());
        if let Err(e) = state.flush() {
            eprintln!("pinned: {e}");
        }
    }
}

/// How long the link waits before dialing again: it doubles from `BACKOFF_MIN` to `BACKOFF_MAX`
/// with each attempt that fails, and starts over after a connection that held.
#[derive(Debug)]
struct Backoff {
    next: Duration,
}

impl Backoff {
    fn new() -> Self {
        Backoff { next: BACKOFF_MIN }
    }

    /// Notes that a connection ended after `up` of being accepted by the hub, or None if the hub
    /// never accepted it. A stable one, or one the hub closed on purpose, resets the climb.
    fn ended(&mut self, up: Option<Duration>, closed_cleanly: bool) {
        if closed_cleanly || up.is_some_and(|up| up >= BACKOFF_STABLE) {
            self.next = BACKOFF_MIN;
        }
    }

    /// The wait before the next dial, varied by `BACKOFF_JITTER` around the current step and never
    /// past `BACKOFF_MAX`. `unit` is a draw in `0.0..1.0`.
    fn wait(&mut self, unit: f64) -> Duration {
        let wait = jittered(self.next, unit).min(BACKOFF_MAX);
        self.next = (self.next * 2).min(BACKOFF_MAX);
        wait
    }
}

/// `step` varied by `BACKOFF_JITTER` either way: `unit` is a draw in `0.0..1.0`, and 0.0 is the shortest.
fn jittered(step: Duration, unit: f64) -> Duration {
    let spread = 1.0 + BACKOFF_JITTER * (2.0 * unit.clamp(0.0, 1.0) - 1.0);
    step.mul_f64(spread)
}

/// A draw in `0.0..1.0` from the standard library's randomly seeded hasher.
fn draw() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let bits = std::collections::hash_map::RandomState::new().build_hasher().finish();
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

async fn run(shared: Arc<Shared>, creds: Credentials, machine: Arc<Machine>, version: String, mut stopped: watch::Receiver<bool>, listener: u64) {
    let mut backoff = Backoff::new();
    loop {
        let mut accepted = None;
        let ended = tokio::select! {
            ended = connection(&shared, &creds, &machine, &version, &mut accepted) => Some(ended),
            _ = stopped.changed() => None,
        };
        shared.disconnected(&machine);
        if let Some(Err(fault)) = &ended {
            shared.note(fault.outcome());
        }
        let up = accepted.map(|at: Instant| at.elapsed());
        match ended {
            None => break,
            Some(Ok(())) => {
                eprintln!("pinned: disconnected from {}", creds.hub);
                backoff.ended(up, true);
            }
            Some(Err(Fault::Transport(e))) => {
                backoff.ended(up, false);
                eprintln!("pinned: hub link: {e}; retrying");
            }
            Some(Err(Fault::Refused(e) | Fault::Unauthorized(e))) => {
                backoff.ended(up, false);
                eprintln!("pinned: hub link: {e}; retrying");
                crate::boot::refuse();
            }
        }
        let wait = backoff.wait(draw());
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stopped.changed() => break,
        }
    }
    machine.unsubscribe(listener);
}

/// One connection, from dialing to its end. Ok means the hub closed it with a close frame, as it
/// does when it restarts. `accepted` is set to the moment the hub accepted the machine.
async fn connection(shared: &Arc<Shared>, creds: &Credentials, machine: &Arc<Machine>, version: &str, accepted: &mut Option<Instant>) -> Result<(), Fault> {
    let url = ws_url(&creds.hub).map_err(Fault::Transport)?;
    let (mut sink, mut stream) = dial(&url).await.map_err(Fault::Transport)?.split();
    let auth = MachineFrame::Auth {
        token: creds.machine_token.clone(),
        name: crate::account::hostname(),
        os: crate::account::os(),
        version: version.into(),
    };
    sink.send(text(&auth)).await.map_err(transport)?;
    let hello = tokio::time::timeout(HELLO_WAIT, stream.next()).await.map_err(|_| Fault::Transport("the hub did not answer the sign in".into()))?;
    let hello = accept_hello(hello)?;
    *accepted = Some(Instant::now());
    shared.note(Outcome::Accepted);
    machine.set_hub_id(hello.id, hello.name);
    if crate::boot::commit_when_ready().await == crate::boot::AfterHello::Leaving {
        return std::future::pending().await;
    }

    let (out, mut incoming) = channel::unbounded_channel();
    *shared.out.lock().unwrap() = Some(out.clone());
    let calls = in_order(shared.clone(), machine.clone());
    eprintln!("pinned: connected to {}", creds.hub);
    let (sharing, reporting) = (machine.clone(), shared.clone());
    std::thread::spawn(move || {
        sharing.share_tastes_again();
        sharing.send_records();
        sharing.warm_statuses();
        report_state(&reporting, &sharing);
    });
    shared.send(&MachineFrame::Event { event: MachineEvent::Tabs { tabs: machine.tabs() } });
    for (account, usage) in machine.usage() {
        shared.send(&MachineFrame::Event { event: MachineEvent::Usage { usage, account } });
    }
    shared.send(&MachineFrame::Event { event: MachineEvent::Update { status: machine.update_snapshot() } });
    if let Some(on) = machine.background() {
        shared.send(&MachineFrame::Event { event: MachineEvent::Background { on } });
    }

    let mut ping = tokio::time::interval(PING);
    let mut heard = Instant::now();
    loop {
        tokio::select! {
            Some(outgoing) = incoming.recv() => {
                sink.send(Frame::Text(outgoing.text.into())).await.map_err(transport)?;
                if let Some(wrote) = outgoing.wrote {
                    let _ = wrote.send(());
                }
            }
            frame = stream.next() => {
                heard = Instant::now();
                match frame {
                    Some(Ok(Frame::Text(t))) => answer(shared, machine, &out, &calls, &t, version),
                    Some(Ok(Frame::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(transport(e)),
                }
            }
            _ = ping.tick() => {
                if heard.elapsed() > SILENT {
                    return Err(Fault::Transport("the hub stopped answering".into()));
                }
                sink.send(Frame::Ping(Default::default())).await.map_err(transport)?;
            }
        }
    }
}

/// Opens the machine's socket to the hub at `url`, giving up once the TCP, TLS and WebSocket
/// handshakes together take longer than `DIAL_WAIT`.
async fn dial(url: &str) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, String> {
    let config = WebSocketConfig::default().max_message_size(Some(sync::MESSAGE_LIMIT)).max_frame_size(Some(sync::MESSAGE_LIMIT));
    let connecting = tokio_tungstenite::connect_async_with_config(url, Some(config), false);
    let (socket, _) = tokio::time::timeout(DIAL_WAIT, connecting)
        .await
        .map_err(|_| format!("could not connect to {url}: no answer within {}s", DIAL_WAIT.as_secs()))?
        .map_err(|e| format!("could not connect to {url}: {e}"))?;
    Ok(socket)
}

/// A thread that answers the connection's calls one after another, so input typed fast reaches
/// the session in order. It ends with the connection.
fn in_order(shared: Arc<Shared>, machine: Arc<Machine>) -> mpsc::Sender<(u64, Call)> {
    let (calls, queue) = mpsc::channel::<(u64, Call)>();
    std::thread::spawn(move || {
        for (id, call) in queue {
            finish_call(&shared, &machine, id, call);
        }
    });
    calls
}

enum Fault {
    Transport(String),
    /// The hub answered the sign in with something that is not a hello.
    Refused(String),
    /// The hub closed the socket with `UNAUTHORIZED`: it does not know the token.
    Unauthorized(String),
}

impl Fault {
    /// What this fault means for the span `Link::refused_for` measures. Only `UNAUTHORIZED` counts.
    fn outcome(&self) -> Outcome {
        match self {
            Fault::Unauthorized(_) => Outcome::Unauthorized,
            Fault::Refused(_) | Fault::Transport(_) => Outcome::Other,
        }
    }
}

/// How a connection attempt went, as far as `Refusals` is concerned.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    /// The hub accepted the machine: its hello arrived.
    Accepted,
    /// The hub closed the socket with `UNAUTHORIZED`.
    Unauthorized,
    /// Anything else: a transport fault, another close code, a frame that is not a hello.
    Other,
}

/// Since when the hub has refused this machine's token without accepting it in between, and
/// whether a refusal has since arrived at or after the deadline.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Refusals {
    since: Option<tokio::time::Instant>,
    due: bool,
}

impl Refusals {
    /// Records `outcome` at `now`. True when `outcome` is a refusal that arrives at or after
    /// `span` from the first one with no accepted hello in between, and from then on `due`
    /// holds. An accepted hello starts the span over, the first refusal opens it, and nothing
    /// else touches it: the time passing, a transport fault, another close code or a frame that
    /// is not a hello is not a refusal, so it never makes this true.
    fn note(&mut self, outcome: Outcome, now: tokio::time::Instant, span: Duration) -> bool {
        match outcome {
            Outcome::Accepted => {
                *self = Refusals::default();
                false
            }
            Outcome::Unauthorized => {
                let since = *self.since.get_or_insert(now);
                let fresh = now >= since + span;
                self.due |= fresh;
                fresh
            }
            Outcome::Other => false,
        }
    }
}

async fn wait_refused(mut seen: watch::Receiver<Refusals>) {
    loop {
        if seen.borrow_and_update().due {
            return;
        }
        if seen.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

fn transport(error: tokio_tungstenite::tungstenite::Error) -> Fault {
    Fault::Transport(error.to_string())
}

/// This machine's id, from the hub's hello.
fn accept_hello(hello: Option<Result<Frame, tokio_tungstenite::tungstenite::Error>>) -> Result<MachineView, Fault> {
    if let Some(Ok(Frame::Text(t))) = &hello {
        if let Ok(ToMachine::Hello { machine }) = serde_json::from_str::<ToMachine>(t) {
            return Ok(machine);
        }
    }
    match hello {
        Some(Ok(Frame::Close(Some(close)))) if u16::from(close.code) == UNAUTHORIZED => {
            Err(Fault::Unauthorized("the hub does not know this machine's token: pair it again".into()))
        }
        Some(Ok(frame @ (Frame::Text(_) | Frame::Binary(_)))) => Err(Fault::Refused(format!("the hub answered the sign in with {frame:?}"))),
        other => Err(Fault::Transport(format!("the hub answered the sign in with {other:?}"))),
    }
}

fn finish_call(shared: &Shared, machine: &Machine, id: u64, call: Call) {
    let (result, armed) = machine.finish_for_restart(call);
    let frame = MachineFrame::Reply(Reply::new(id, result));
    if armed {
        let _ = shared.send_flushed(&frame);
        machine.restart_now();
    } else {
        shared.send(&frame);
    }
}

fn queue(out: &channel::UnboundedSender<Outgoing>, frame: &MachineFrame) {
    let _ = out.send(Outgoing { text: serde_json::to_string(frame).unwrap(), wrote: None });
}

/// The refusal `answer` sends when this binary does not know the command.
pub fn is_unknown_command(err: &str) -> bool {
    err.contains("can't answer that")
}

fn answer(shared: &Arc<Shared>, machine: &Arc<Machine>, out: &channel::UnboundedSender<Outgoing>, calls: &mpsc::Sender<(u64, Call)>, frame: &str, version: &str) {
    match serde_json::from_str::<ToMachine>(frame) {
        // The hub alone gets the token, in its reply: no other caller of this machine does.
        Ok(ToMachine::Call { id, call: Call::SetupToken { account } }) => {
            let (shared, machine) = (shared.clone(), machine.clone());
            std::thread::spawn(move || shared.send(&MachineFrame::Reply(Reply::new(id, machine.setup_token(&account).map(Value::String)))));
        }
        Ok(ToMachine::Call { id, call }) if call.may_wait() => {
            let (shared, machine) = (shared.clone(), machine.clone());
            std::thread::spawn(move || finish_call(&shared, &machine, id, call));
        }
        Ok(ToMachine::Call { id, call }) => {
            let _ = calls.send((id, call));
        }
        Ok(ToMachine::Reply(reply)) => {
            if let Some(waiting) = shared.pending.lock().unwrap().remove(&reply.id) {
                let _ = waiting.send(reply.result());
            }
        }
        Ok(ToMachine::Subscribe { tab, viewer }) => {
            let (out, id, to) = (out.clone(), tab.clone(), viewer.clone());
            let sink = move |bytes: &[u8]| {
                let frame = MachineFrame::Output { viewer: to.clone(), tab: id.clone(), data: Bytes(bytes.to_vec()) };
                queue(&out, &frame);
            };
            if machine.attach(&tab, viewer.clone(), Box::new(sink)).is_ok() {
                shared.subscribed.lock().unwrap().insert((tab, viewer));
            }
        }
        Ok(ToMachine::Unsubscribe { tab, viewer }) => {
            machine.detach(&tab, &viewer);
            shared.subscribed.lock().unwrap().remove(&(tab, viewer));
        }
        Ok(ToMachine::WatchThreads { viewer, tabs }) => machine.set_watched_threads(&viewer, &tabs),
        Ok(ToMachine::Hello { .. }) => {}
        Err(e) => {
            let value: Value = serde_json::from_str(frame).unwrap_or_default();
            if let (Some("call"), Some(id)) = (value["type"].as_str(), value["id"].as_u64()) {
                let reply = MachineFrame::Reply(Reply::new(id, Err(format!("this machine runs pinned {version}, which can't answer that: {e}"))));
                queue(out, &reply);
            }
        }
    }
}

fn text(frame: &MachineFrame) -> Frame {
    Frame::Text(serde_json::to_string(frame).unwrap().into())
}

fn ws_url(hub: &str) -> Result<String, String> {
    let hub = hub.trim().trim_end_matches('/');
    if let Some(rest) = hub.strip_prefix("https://") {
        Ok(format!("wss://{rest}/ws/machine"))
    } else if let Some(rest) = hub.strip_prefix("http://") {
        Ok(format!("ws://{rest}/ws/machine"))
    } else {
        Err(format!("{hub} is not an http or https URL"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn waits(backoff: &mut Backoff, count: usize) -> Vec<u64> {
        (0..count).map(|_| backoff.wait(0.5).as_secs()).collect()
    }

    #[test]
    fn failed_attempts_double_the_wait_up_to_the_cap() {
        let mut backoff = Backoff::new();
        assert_eq!(waits(&mut backoff, 8), [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn a_connection_that_held_for_the_stable_period_starts_the_wait_over() {
        let mut backoff = Backoff::new();
        waits(&mut backoff, 4);
        backoff.ended(Some(BACKOFF_STABLE - Duration::from_millis(1)), false);
        assert_eq!(backoff.wait(0.5).as_secs(), 16, "a connection a hair short of stable keeps the climb");
        backoff.ended(Some(BACKOFF_STABLE), false);
        assert_eq!(waits(&mut backoff, 3), [1, 2, 4]);
    }

    #[test]
    fn a_connection_the_hub_never_accepted_keeps_the_climb() {
        let mut backoff = Backoff::new();
        waits(&mut backoff, 3);
        backoff.ended(None, false);
        assert_eq!(backoff.wait(0.5).as_secs(), 8);
    }

    #[test]
    fn a_close_frame_from_the_hub_resets_the_wait() {
        let mut backoff = Backoff::new();
        waits(&mut backoff, 5);
        backoff.ended(Some(Duration::from_secs(1)), true);
        assert_eq!(waits(&mut backoff, 2), [1, 2]);
    }

    #[test]
    fn jitter_stays_within_a_quarter_of_the_step_and_under_the_cap() {
        for step in 0..8 {
            let nominal = (BACKOFF_MIN * 2u32.pow(step)).min(BACKOFF_MAX);
            let (mut low, mut high) = (Backoff::new(), Backoff::new());
            low.next = nominal;
            high.next = nominal;
            let (low, high) = (low.wait(0.0), high.wait(1.0));
            assert_eq!(low, nominal.mul_f64(0.75), "step {step}");
            assert_eq!(high, nominal.mul_f64(1.25).min(BACKOFF_MAX), "step {step}");
            assert!(high <= BACKOFF_MAX);
        }
    }

    #[test]
    fn draws_cover_the_unit_interval_and_vary() {
        let draws: Vec<f64> = (0..200).map(|_| draw()).collect();
        assert!(draws.iter().all(|d| (0.0..1.0).contains(d)));
        assert!(draws.iter().any(|d| *d < 0.25) && draws.iter().any(|d| *d > 0.75));
    }

    #[test]
    fn the_machine_socket_follows_the_hubs_scheme() {
        assert_eq!(ws_url("https://pinned.example.com/").as_deref(), Ok("wss://pinned.example.com/ws/machine"));
        assert_eq!(ws_url("http://127.0.0.1:8787").as_deref(), Ok("ws://127.0.0.1:8787/ws/machine"));
        assert!(ws_url("pinned.example.com").is_err());
    }

    #[test]
    fn an_empty_state_file_stops_sync_until_the_machine_resets_it() {
        let dir = std::env::temp_dir().join(format!("pinned-link-{}-reset", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sync.json");
        std::fs::write(&file, b"").unwrap();
        let machine = Machine::new(crate::machine::tests::launch(), Some(dir.clone()), None, |_| {});
        let shared = Shared::default();
        let (out, _frames) = channel::unbounded_channel();
        *shared.out.lock().unwrap() = Some(out);
        let mut syncer = Syncer::default();
        syncer.cycle(&shared, &shared, &machine, &file);
        assert_eq!(std::fs::read(&file).unwrap(), b"", "a stopped sync leaves the file as it was");
        let why = format!("the sync state {} is corrupt (EOF while parsing a value at line 1 column 0)", file.display());
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::String(why)));
        assert_eq!(machine.call(Call::ResetSync), Ok(Value::Null));
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::Null));
        syncer.cycle(&shared, &shared, &machine, &file);
        let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved, serde_json::json!({ "hashes": {}, "agreed": {}, "claude_homes": {} }));
        assert_eq!(std::fs::read(dir.join("sync.json.damaged")).unwrap(), b"");
        let _ = std::fs::remove_dir_all(dir);
    }

    type Fixture = (std::path::PathBuf, std::path::PathBuf, Arc<Machine>, Shared, channel::UnboundedReceiver<Outgoing>);

    fn stopped_sync_fixture(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("pinned-link-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sync.json");
        let machine = Machine::new(crate::machine::tests::launch(), Some(dir.clone()), None, |_| {});
        let shared = Shared::default();
        let (out, frames) = channel::unbounded_channel();
        *shared.out.lock().unwrap() = Some(out);
        (dir, file, machine, shared, frames)
    }

    #[test]
    fn a_state_file_that_breaks_while_the_loop_holds_its_state_still_reads_as_syncing() {
        let (dir, file, machine, shared, _frames) = stopped_sync_fixture("held");
        let mut syncer = Syncer::default();
        syncer.cycle(&shared, &shared, &machine, &file);
        std::fs::write(&file, b"").unwrap();
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::Null));
        syncer.cycle(&shared, &shared, &machine, &file);
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::Null), "an idle cycle keeps the state it holds and doesn't rewrite the file");
        assert_eq!(std::fs::read(&file).unwrap(), b"");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn once_the_sync_loop_ends_the_state_file_decides_again() {
        let (dir, file, machine, shared, _frames) = stopped_sync_fixture("ended");
        let mut syncer = Syncer::default();
        syncer.cycle(&shared, &shared, &machine, &file);
        std::fs::write(&file, b"").unwrap();
        let (_stop, stopped) = watch::channel(true);
        syncer.run(&shared, &machine, Some(&file), &stopped);
        let why = format!("the sync state {} is corrupt (EOF while parsing a value at line 1 column 0)", file.display());
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::String(why)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_sync_loop_ends_once_its_link_is_dropped() {
        let (dir, file, machine, shared, _frames) = stopped_sync_fixture("dropped");
        let (stop, stopped) = watch::channel(false);
        drop(stop);
        let (ended, done) = mpsc::channel();
        std::thread::spawn(move || {
            Syncer::default().run(&shared, &machine, Some(&file), &stopped);
            let _ = ended.send(());
        });
        assert_eq!(done.recv_timeout(Duration::from_secs(10)), Ok(()));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn creds(hub: String) -> Credentials {
        Credentials { hub, email: "ana@acme.dev".into(), user_token: String::new(), machine_id: "m-1".into(), machine_token: "t".into() }
    }

    #[test]
    fn a_link_stopped_between_cycles_does_not_wait_for_the_next() {
        let (dir, _file, machine, _shared, _frames) = stopped_sync_fixture("link-idle");
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let link = spawn(creds(format!("http://{closed}")), machine.clone(), "0.0.0-test");
        std::thread::sleep(Duration::from_millis(200));
        let stopping = Instant::now();
        link.stop();
        assert!(stopping.elapsed() < Duration::from_secs(2), "stop waited {:?} for the sync loop's next cycle", stopping.elapsed());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn a_hub_that_answers_no_call(calls: mpsc::Sender<String>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async move {
                let (socket, _) = tokio::net::TcpListener::from_std(listener).unwrap().accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
                socket.next().await;
                let hello = r#"{"type":"hello","machine":{"id":"m-1","name":"mac","os":"macos","online":true,"last_seen":0}}"#;
                socket.send(Frame::Text(hello.into())).await.unwrap();
                while let Some(Ok(frame)) = socket.next().await {
                    if let Frame::Text(text) = frame {
                        let sent: Value = serde_json::from_str(&text).unwrap();
                        if sent["type"] == "call" {
                            let _ = calls.send(sent["call"]["cmd"].as_str().unwrap().to_owned());
                        }
                    }
                }
            });
        });
        url
    }

    #[test]
    fn a_link_stopped_inside_a_cycle_saves_the_checkout_in_hand_and_lets_go() {
        let (dir, file, machine, _shared, _frames) = stopped_sync_fixture("link-stop");
        for name in ["one", "two"] {
            let app = dir.join(name);
            std::fs::create_dir_all(app.join(".claude/skills")).unwrap();
            for args in [&["init", "-q"][..], &["remote", "add", "origin", &format!("git@github.com:acme/{name}.git")]] {
                assert!(std::process::Command::new("git").arg("-C").arg(&app).args(args).status().unwrap().success());
            }
            std::fs::write(app.join(".claude/skills/skill.md"), vec![0u8; 16 << 20]).unwrap();
            machine.open_project(app);
        }
        let (calls, called) = mpsc::channel();
        let link = spawn(creds(a_hub_that_answers_no_call(calls)), machine.clone(), "0.0.0-test");
        assert_eq!(called.recv_timeout(Duration::from_secs(30)).as_deref(), Ok("share_project"));
        link.stop();
        let saved: Value = serde_json::from_slice(&std::fs::read(&file).expect("the stopped loop saved its state")).unwrap();
        let hashed: Vec<String> = saved["hashes"].as_object().unwrap().keys().map(|path| Path::new(path).file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(hashed, ["skill.md"]);
        std::fs::write(&file, b"").unwrap();
        let why = format!("the sync state {} is corrupt (EOF while parsing a value at line 1 column 0)", file.display());
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::String(why)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_state_file_restored_by_hand_reads_as_syncing_before_the_next_cycle() {
        let (dir, file, machine, shared, _frames) = stopped_sync_fixture("restored");
        std::fs::write(&file, b"").unwrap();
        Syncer::default().cycle(&shared, &shared, &machine, &file);
        assert!(matches!(machine.call(Call::SyncStopped), Ok(Value::String(_))));
        std::fs::write(&file, br#"{"hashes":{},"agreed":{},"claude_homes":{}}"#).unwrap();
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::Null));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_reset_leaves_a_readable_state_file_alone() {
        let dir = std::env::temp_dir().join(format!("pinned-link-{}-keep", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sync.json");
        let saved = r#"{"hashes":{},"agreed":{"github.com/acme/app /work/app":{"skills/a.md":"abc"}},"claude_homes":{}}"#;
        std::fs::write(&file, saved).unwrap();
        let machine = Machine::new(crate::machine::tests::launch(), Some(dir.clone()), None, |_| {});
        assert_eq!(machine.call(Call::ResetSync), Ok(Value::Null));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
        assert_eq!(dir.join("sync.json.damaged").exists(), false);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The paused clock jumps to the next timer whenever the dial waits on the socket, so
    /// neither the dial's wait nor the outer guard costs real time.
    #[tokio::test(start_paused = true)]
    async fn a_dial_the_hub_never_answers_ends_in_an_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _held = listener.accept().await;
            std::future::pending::<()>().await;
        });
        let dialed = tokio::time::timeout(Duration::from_secs(60), dial(&url)).await.expect("the dial never ended");
        let err = dialed.expect_err("the dial succeeded although the hub never answered");
        assert!(err.starts_with(&format!("could not connect to {url}: ")), "{err}");
    }

    #[test]
    fn a_hub_that_reads_the_sign_in_and_says_no_is_a_refusal() {
        let close = |code: u16| Some(Ok(Frame::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame { code: code.into(), reason: "".into() }))));
        assert!(matches!(accept_hello(close(UNAUTHORIZED)), Err(Fault::Unauthorized(_))));
        assert!(matches!(accept_hello(Some(Ok(Frame::Text(r#"{"type":"nope"}"#.into())))), Err(Fault::Refused(_))));
        assert!(matches!(accept_hello(close(1001)), Err(Fault::Transport(_))));
        assert!(matches!(accept_hello(None), Err(Fault::Transport(_))));
    }

    fn close_frame(code: u16) -> Option<Result<Frame, tokio_tungstenite::tungstenite::Error>> {
        Some(Ok(Frame::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame { code: code.into(), reason: "".into() }))))
    }

    fn hello_frame() -> Option<Result<Frame, tokio_tungstenite::tungstenite::Error>> {
        let hello = ToMachine::Hello {
            machine: MachineView { id: "m1".into(), name: "box".into(), os: "linux".into(), online: true, last_seen: 0, upstash_box: None, version: String::new(), update: None, state: None, background: None },
        };
        Some(Ok(Frame::Text(serde_json::to_string(&hello).unwrap().into())))
    }

    /// What `run` and `connection` do with one answer to the sign in.
    fn answer(shared: &Shared, hello: Option<Result<Frame, tokio_tungstenite::tungstenite::Error>>) {
        match accept_hello(hello) {
            Ok(_) => shared.note(Outcome::Accepted),
            Err(fault) => shared.note(fault.outcome()),
        }
    }

    const MINUTE: Duration = Duration::from_secs(60);

    /// Feeds `hello` every 30 seconds for 60 virtual minutes and reports whether the refusal span
    /// ever ran out. The clock is paused, so the hour costs no real time.
    async fn trips_within_an_hour(hello: impl Fn() -> Option<Result<Frame, tokio_tungstenite::tungstenite::Error>>) -> bool {
        let shared = Shared::default();
        let waiting = tokio::spawn(wait_refused(shared.refusals.subscribe()));
        for _ in 0..120 {
            answer(&shared, hello());
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        let tripped = waiting.is_finished();
        waiting.abort();
        tripped
    }

    #[test]
    fn the_refused_span_is_fifteen_minutes() {
        assert_eq!(BOX_REFUSED_FOR, 15 * MINUTE);
    }

    #[test]
    fn the_refused_span_ends_exactly_at_fifteen_minutes_of_unauthorized() {
        let t0 = tokio::time::Instant::now();
        let mut refusals = Refusals::default();
        assert!(!refusals.note(Outcome::Unauthorized, t0, BOX_REFUSED_FOR));
        assert!(!refusals.note(Outcome::Unauthorized, t0 + BOX_REFUSED_FOR - Duration::from_secs(1), BOX_REFUSED_FOR));
        assert!(refusals.note(Outcome::Unauthorized, t0 + BOX_REFUSED_FOR, BOX_REFUSED_FOR));
    }

    #[test]
    fn an_accepted_hello_starts_the_refused_span_over() {
        let t0 = tokio::time::Instant::now();
        let mut refusals = Refusals::default();
        refusals.note(Outcome::Unauthorized, t0, BOX_REFUSED_FOR);
        assert!(!refusals.note(Outcome::Accepted, t0 + 14 * MINUTE, BOX_REFUSED_FOR));
        assert_eq!(refusals, Refusals::default());
        assert!(!refusals.note(Outcome::Unauthorized, t0 + 14 * MINUTE, BOX_REFUSED_FOR));
        assert!(!refusals.note(Outcome::Unauthorized, t0 + 28 * MINUTE, BOX_REFUSED_FOR));
        assert!(refusals.note(Outcome::Unauthorized, t0 + 29 * MINUTE, BOX_REFUSED_FOR));
    }

    #[test]
    fn other_outcomes_neither_start_nor_reset_the_refused_span() {
        let t0 = tokio::time::Instant::now();
        let mut refusals = Refusals::default();
        for minute in 0..120 {
            assert!(!refusals.note(Outcome::Other, t0 + minute * MINUTE, BOX_REFUSED_FOR));
        }
        assert_eq!(refusals, Refusals::default());
        refusals.note(Outcome::Unauthorized, t0, BOX_REFUSED_FOR);
        for minute in [10, 15, 60] {
            assert!(!refusals.note(Outcome::Other, t0 + minute * MINUTE, BOX_REFUSED_FOR), "only a refusal can be the one that ends the wait");
        }
        assert!(!refusals.due, "a transport fault made the span run out");
        assert!(refusals.note(Outcome::Unauthorized, t0 + 60 * MINUTE, BOX_REFUSED_FOR), "a transport fault reset the span");
    }

    #[test]
    fn only_an_unauthorized_close_is_counted_as_a_refusal() {
        let outcome = |hello| accept_hello(hello).map(|_| Outcome::Accepted).unwrap_or_else(|fault| fault.outcome());
        assert_eq!(outcome(close_frame(UNAUTHORIZED)), Outcome::Unauthorized);
        assert_eq!(outcome(close_frame(1001)), Outcome::Other);
        assert_eq!(outcome(Some(Ok(Frame::Text(r#"{"type":"nope"}"#.into())))), Outcome::Other);
        assert_eq!(outcome(Some(Ok(Frame::Binary(vec![1, 2].into())))), Outcome::Other);
        assert_eq!(outcome(None), Outcome::Other);
        assert_eq!(Fault::Transport("the hub did not answer the sign in".into()).outcome(), Outcome::Other);
        assert_eq!(outcome(hello_frame()), Outcome::Accepted);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hub_that_refuses_for_fifteen_minutes_ends_the_wait_and_not_before() {
        let shared = Shared::default();
        let started = tokio::time::Instant::now();
        let waiting = tokio::spawn(wait_refused(shared.refusals.subscribe()));
        while started.elapsed() < BOX_REFUSED_FOR {
            answer(&shared, close_frame(UNAUTHORIZED));
            tokio::time::sleep(Duration::from_millis(1)).await;
            assert!(!waiting.is_finished(), "ended after {:?}", started.elapsed());
            tokio::time::sleep(Duration::from_secs(30) - Duration::from_millis(1)).await;
        }
        assert_eq!(started.elapsed(), BOX_REFUSED_FOR);
        assert!(!waiting.is_finished(), "ended with no refusal at the deadline");
        answer(&shared, close_frame(UNAUTHORIZED));
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(waiting.is_finished(), "still waiting after a refusal at {:?}", started.elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_stays_open_after_fifteen_minutes_without_another_refusal_and_a_refusal_after_the_deadline_ends_it() {
        let shared = Shared::default();
        let started = tokio::time::Instant::now();
        let waiting = tokio::spawn(wait_refused(shared.refusals.subscribe()));
        answer(&shared, close_frame(UNAUTHORIZED));
        tokio::time::sleep(BOX_REFUSED_FOR).await;
        assert!(!waiting.is_finished(), "ended at the deadline with no refusal since the first one");
        for _ in 0..8 {
            answer(&shared, Some(Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed)));
            tokio::time::sleep(30 * MINUTE).await;
            assert!(!waiting.is_finished(), "a hub that does not answer ended the wait after {:?}", started.elapsed());
        }
        answer(&shared, close_frame(UNAUTHORIZED));
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(waiting.is_finished(), "a refusal after the deadline left the wait open");
    }

    #[tokio::test(start_paused = true)]
    async fn a_hello_after_fourteen_minutes_of_refusals_keeps_the_wait_open() {
        let shared = Shared::default();
        let waiting = tokio::spawn(wait_refused(shared.refusals.subscribe()));
        for _ in 0..2 {
            for _ in 0..28 {
                answer(&shared, close_frame(UNAUTHORIZED));
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            assert!(!waiting.is_finished(), "ended after 14 minutes of refusals");
            answer(&shared, hello_frame());
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert!(!waiting.is_finished(), "ended with the hub accepting the machine");
        waiting.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_frame_that_is_not_a_hello_never_ends_the_wait() {
        assert!(!trips_within_an_hour(|| Some(Ok(Frame::Text(r#"{"type":"nope"}"#.into())))).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_transport_error_never_ends_the_wait() {
        let error = || Some(Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed));
        assert!(!trips_within_an_hour(error).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_1001_close_never_ends_the_wait() {
        assert!(!trips_within_an_hour(|| close_frame(1001)).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_sign_in_the_hub_never_answers_never_ends_the_wait() {
        let shared = Shared::default();
        let waiting = tokio::spawn(wait_refused(shared.refusals.subscribe()));
        for _ in 0..240 {
            let fault = Fault::Transport("the hub did not answer the sign in".into());
            shared.note(fault.outcome());
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
        assert!(!waiting.is_finished());
        waiting.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_hub_restart_closes_with_1001_and_the_new_hub_accepts_the_machine() {
        let shared = Shared::default();
        let waiting = tokio::spawn(wait_refused(shared.refusals.subscribe()));
        for _ in 0..120 {
            answer(&shared, close_frame(1001));
            tokio::time::sleep(Duration::from_secs(10)).await;
            answer(&shared, hello_frame());
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
        assert!(!waiting.is_finished());
        waiting.abort();
    }

    /// A hub on a real socket that reads each sign in and closes the socket with `code` instead of
    /// a hello, and counts the sign ins it read.
    fn a_hub_that_closes_every_sign_in(code: u16, sign_ins: Arc<std::sync::atomic::AtomicUsize>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                loop {
                    let (socket, _) = listener.accept().await.unwrap();
                    let Ok(mut socket) = tokio_tungstenite::accept_async(socket).await else { continue };
                    socket.next().await;
                    sign_ins.fetch_add(1, Ordering::SeqCst);
                    let close = tokio_tungstenite::tungstenite::protocol::CloseFrame { code: code.into(), reason: "".into() };
                    let _ = socket.send(Frame::Close(Some(close))).await;
                }
            });
        });
        url
    }

    /// Runs the link's own `run` loop against `hub`, with the span cut to `span` so the test costs
    /// seconds and not 15 minutes. True when `refused_for` resolved, false once the hub has read
    /// `sign_ins_to_watch` sign ins and it still had not. `BOX_REFUSED_FOR` stays the production value.
    async fn the_link_against(hub: String, span: Duration, sign_ins: &std::sync::atomic::AtomicUsize, sign_ins_to_watch: usize) -> bool {
        let dir = std::env::temp_dir().join(format!("pinned-link-{}-socket-{}", std::process::id(), sign_ins_to_watch));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let machine = Machine::new(crate::machine::tests::launch(), Some(dir.clone()), None, |_| {});
        let shared = Arc::new(Shared { refused_span: RefusedSpan(span), ..Shared::default() });
        let (_stop, stopped) = watch::channel(false);
        let watched = async {
            while sign_ins.load(Ordering::SeqCst) < sign_ins_to_watch {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        let ended = tokio::select! {
            _ = run(shared.clone(), creds(hub), machine, "0.0.0-test".into(), stopped, 0) => panic!("the link's run loop ended"),
            _ = wait_refused(shared.refusals.subscribe()) => true,
            _ = watched => false,
            _ = tokio::time::sleep(Duration::from_secs(60)) => panic!("the link did not settle in 60 s"),
        };
        let _ = std::fs::remove_dir_all(dir);
        ended
    }

    #[tokio::test]
    async fn a_hub_that_closes_every_sign_in_with_4001_ends_the_wait_on_the_first_refusal_after_the_span() {
        let sign_ins = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hub = a_hub_that_closes_every_sign_in(UNAUTHORIZED, sign_ins.clone());
        assert!(the_link_against(hub, Duration::from_millis(200), &sign_ins, usize::MAX).await);
        assert!(sign_ins.load(Ordering::SeqCst) >= 2, "the first refusal alone ended the wait");
    }

    #[tokio::test]
    async fn a_hub_that_closes_every_sign_in_with_1001_never_ends_the_wait() {
        let sign_ins = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hub = a_hub_that_closes_every_sign_in(1001, sign_ins.clone());
        assert!(!the_link_against(hub, Duration::from_millis(200), &sign_ins, 3).await);
        assert!(sign_ins.load(Ordering::SeqCst) >= 3);
    }

    /// What the fake hub on the test's runtime does with a connection.
    #[derive(Clone, Copy)]
    enum Answer {
        /// Reads the sign in and closes the socket with this code instead of a hello.
        Close(u16),
        /// Takes the socket and never reads it, as a hub too busy to look at the sign in.
        Silent,
        /// Reads the sign in, answers it with a hello and closes the socket with 1001, as a hub
        /// that accepts the machine and then restarts.
        Hello,
    }

    /// A hub on a real socket, spawned on the test's own runtime so the paused clock moves only once
    /// both ends are idle. It answers its `n`th connection, from 0, with `answer(n)`, and counts the
    /// connections it answered.
    async fn a_hub_on_this_runtime(answer: impl Fn(usize) -> Answer + Send + 'static) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = answered.clone();
        tokio::spawn(async move {
            let mut unread = Vec::new();
            for n in 0.. {
                let (socket, _) = listener.accept().await.unwrap();
                let Ok(mut socket) = tokio_tungstenite::accept_async(socket).await else { continue };
                match answer(n) {
                    Answer::Silent => unread.push(socket),
                    Answer::Hello => {
                        socket.next().await;
                        if let Some(Ok(hello)) = hello_frame() {
                            let _ = socket.send(hello).await;
                        }
                        let close = tokio_tungstenite::tungstenite::protocol::CloseFrame { code: 1001.into(), reason: "".into() };
                        let _ = socket.send(Frame::Close(Some(close))).await;
                    }
                    Answer::Close(code) => {
                        socket.next().await;
                        let close = tokio_tungstenite::tungstenite::protocol::CloseFrame { code: code.into(), reason: "".into() };
                        let _ = socket.send(Frame::Close(Some(close))).await;
                    }
                }
                counted.fetch_add(1, Ordering::SeqCst);
            }
        });
        (url, answered)
    }

    /// Runs the link's own `run` loop against `hub` with the production span, for at most
    /// `for_at_most` on the paused clock. How long `refused_for` took to resolve, or None if it did not.
    async fn the_wait_against(hub: String, for_at_most: Duration, name: &str) -> Option<Duration> {
        let dir = std::env::temp_dir().join(format!("pinned-link-{}-paused-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let machine = Machine::new(crate::machine::tests::launch(), Some(dir.clone()), None, |_| {});
        let shared = Arc::new(Shared::default());
        let (_stop, stopped) = watch::channel(false);
        let started = tokio::time::Instant::now();
        let ended = tokio::select! {
            _ = run(shared.clone(), creds(hub), machine, "0.0.0-test".into(), stopped, 0) => panic!("the link's run loop ended"),
            _ = wait_refused(shared.refusals.subscribe()) => Some(started.elapsed()),
            _ = tokio::time::sleep(for_at_most) => None,
        };
        let _ = std::fs::remove_dir_all(dir);
        ended
    }

    #[tokio::test(start_paused = true)]
    async fn a_hub_that_never_reads_the_sign_in_for_twenty_minutes_never_ends_the_wait() {
        let (hub, answered) = a_hub_on_this_runtime(|_| Answer::Silent).await;
        assert_eq!(the_wait_against(hub, 20 * MINUTE, "silent").await, None);
        let answered = answered.load(Ordering::SeqCst);
        assert!(answered >= 10, "the link dialed only {answered} times in 20 minutes");
    }

    #[tokio::test(start_paused = true)]
    async fn a_hub_that_closes_every_sign_in_with_1013_for_twenty_minutes_never_ends_the_wait() {
        let (hub, answered) = a_hub_on_this_runtime(|_| Answer::Close(1013)).await;
        assert_eq!(the_wait_against(hub, 20 * MINUTE, "1013").await, None);
        let answered = answered.load(Ordering::SeqCst);
        assert!(answered >= 10, "the hub read only {answered} sign ins in 20 minutes");
    }

    #[tokio::test(start_paused = true)]
    async fn a_hub_that_answers_every_sign_in_with_4001_for_fifteen_minutes_ends_the_wait_on_the_next_one() {
        let (hub, answered) = a_hub_on_this_runtime(|_| Answer::Close(UNAUTHORIZED)).await;
        let ended = the_wait_against(hub, 20 * MINUTE, "4001").await.expect("15 minutes of unknown token and one more did not end the wait");
        assert!(ended >= BOX_REFUSED_FOR, "ended after {ended:?}, before the span");
        assert!(ended <= BOX_REFUSED_FOR + BACKOFF_MAX + Duration::from_secs(1), "ended after {ended:?}, long after the first refusal past the span");
        let answered = answered.load(Ordering::SeqCst);
        assert!(answered >= 10, "the hub read only {answered} sign ins");
    }

    /// A hub outage: the hub says "unknown token" once, then for 20 minutes it either never reads
    /// the sign in or closes it with 1013. The node stays.
    #[tokio::test(start_paused = true)]
    async fn a_hub_outage_after_one_4001_never_ends_the_wait() {
        let answer = |n: usize| match n {
            0 => Answer::Close(UNAUTHORIZED),
            n if n % 2 == 1 => Answer::Silent,
            _ => Answer::Close(1013),
        };
        let (hub, answered) = a_hub_on_this_runtime(answer).await;
        assert_eq!(the_wait_against(hub, 20 * MINUTE + BOX_REFUSED_FOR, "outage").await, None);
        let answered = answered.load(Ordering::SeqCst);
        assert!(answered >= 10, "the hub answered only {answered} sign ins");
    }

    /// How a hub on the test's runtime answered each connection: when, since `t0`, and with which
    /// close code (0 for a hello).
    type Answers = Arc<Mutex<Vec<(Duration, u16)>>>;

    fn noted(answers: &Answers, t0: tokio::time::Instant, answer: Answer) -> Answer {
        let code = match answer {
            Answer::Close(code) => code,
            Answer::Silent => u16::MAX,
            Answer::Hello => 0,
        };
        answers.lock().unwrap().push((t0.elapsed(), code));
        answer
    }

    /// Through the real `run` and the production span: 4001 for 10 minutes, one accepted hello,
    /// then 4001 again. The hello starts the span over, so the wait ends on the first 4001 at or
    /// after 15 minutes from the first one that follows the hello, not 15 minutes from the first
    /// one ever. Without `shared.note(Outcome::Accepted)` in `connection` it ends at about 15 minutes.
    #[tokio::test(start_paused = true)]
    async fn a_hello_between_two_runs_of_4001_starts_the_wait_over_through_run() {
        let t0 = tokio::time::Instant::now();
        let answers = Answers::default();
        let hello_given = AtomicBool::new(false);
        let noting = answers.clone();
        let (hub, _) = a_hub_on_this_runtime(move |_| {
            let hello_now = t0.elapsed() >= 10 * MINUTE && !hello_given.swap(true, Ordering::SeqCst);
            noted(&noting, t0, if hello_now { Answer::Hello } else { Answer::Close(UNAUTHORIZED) })
        })
        .await;
        let ended = the_wait_against(hub, 40 * MINUTE, "hello").await.expect("4001, a hello and 4001 again never ended the wait");
        let answers = answers.lock().unwrap();
        let mut after_the_hello = answers.iter().skip_while(|(_, code)| *code != 0).skip(1);
        let (first, _) = after_the_hello.next().expect("the hub never answered after the hello");
        assert!(answers.iter().filter(|(_, code)| *code == 0).count() == 1);
        assert!(ended >= *first + BOX_REFUSED_FOR, "ended after {ended:?}, less than 15 minutes after the first 4001 that followed the hello at {first:?}");
        assert!(ended <= *first + BOX_REFUSED_FOR + BACKOFF_MAX + Duration::from_secs(1), "ended after {ended:?}, long after the first 4001 past the span from {first:?}");
    }

    /// Through the real `run` and the production span: one 4001, then a hub that reads no sign in
    /// for 16 minutes, then 4001 again. The silence neither ends the wait nor starts the span over,
    /// so the wait ends on the first 4001 at or after 15 minutes from the first one: the one that
    /// ends the silence.
    #[tokio::test(start_paused = true)]
    async fn a_4001_then_silence_then_4001_ends_the_wait_on_the_first_4001_past_the_span_through_run() {
        let t0 = tokio::time::Instant::now();
        let answers = Answers::default();
        let noting = answers.clone();
        let (hub, _) = a_hub_on_this_runtime(move |n| {
            let silent = n > 0 && t0.elapsed() < 16 * MINUTE;
            noted(&noting, t0, if silent { Answer::Silent } else { Answer::Close(UNAUTHORIZED) })
        })
        .await;
        let ended = the_wait_against(hub, 40 * MINUTE, "silence").await.expect("4001, silence and 4001 again never ended the wait");
        let answers = answers.lock().unwrap();
        let (first, _) = answers[0];
        let (last, code) = *answers.iter().rfind(|(_, code)| *code == UNAUTHORIZED).unwrap();
        assert_eq!(code, UNAUTHORIZED);
        assert!(answers.iter().filter(|(_, code)| *code == UNAUTHORIZED).count() == 2, "{answers:?}");
        assert!(last >= first + BOX_REFUSED_FOR, "the 4001 that ended it came at {last:?}, before the span from {first:?}");
        assert!(ended >= last && ended <= last + Duration::from_secs(1), "ended at {ended:?}, not on the first 4001 past the span at {last:?}");
    }

    struct ArmsRestart {
        restarted: Arc<std::sync::atomic::AtomicBool>,
    }

    impl crate::machine::SelfUpdate for ArmsRestart {
        fn install(&self, machine: &Machine) -> crate::wire::UpdateStatus {
            let status = crate::wire::UpdateStatus::Restarting { to: "1.2.0".into() };
            let restarted = self.restarted.clone();
            machine.schedule_restart(Box::new(move || restarted.store(true, Ordering::SeqCst)));
            machine.report(status.clone());
            status
        }
    }

    #[test]
    fn an_offline_flush_returns_without_waiting() {
        let shared = Shared::default();
        assert!(!shared.send_flushed(&MachineFrame::Event { event: MachineEvent::Update { status: None } }));
    }

    #[test]
    fn a_queued_flush_counts_once_the_writer_confirms_and_not_when_the_writer_drops_it() {
        let frame = MachineFrame::Event { event: MachineEvent::Update { status: None } };
        let shared = Shared::default();
        let (out, mut incoming) = channel::unbounded_channel();
        *shared.out.lock().unwrap() = Some(out);
        let writer = std::thread::spawn(move || {
            let outgoing = incoming.blocking_recv().unwrap();
            outgoing.wrote.unwrap().send(()).unwrap();
        });
        assert!(shared.send_flushed(&frame));
        writer.join().unwrap();

        let (out, mut incoming) = channel::unbounded_channel();
        *shared.out.lock().unwrap() = Some(out);
        let writer = std::thread::spawn(move || {
            let outgoing = incoming.blocking_recv().unwrap();
            drop(outgoing.wrote);
        });
        assert!(!shared.send_flushed(&frame));
        writer.join().unwrap();
    }

    #[test]
    fn a_restart_waits_until_the_reply_is_written() {
        use crate::machine::Launch;
        use std::collections::BTreeMap;
        use std::sync::atomic::AtomicBool;

        let restarted = Arc::new(AtomicBool::new(false));
        let machine = Machine::new(
            Launch {
                exe: "pinned".into(),
                bins: BTreeMap::new(),
                path_env: None,
                settings: "settings".into(),
                mcp_config: None,
                reminders: false,
                plugin_dir: "plugin".into(),
                tutorial_helper: "tutorial".into(),
                sock: "sock".into(),
                vector_search: false,
                first_run_wait: std::time::Duration::from_secs(60),
                starting_limit: std::time::Duration::from_secs(600),
                claude_home: None,
            },
            None,
            None,
            |_| {},
        );
        machine.set_installer(Arc::new(ArmsRestart { restarted: restarted.clone() }));
        let shared = Arc::new(Shared::default());
        let (out, mut incoming) = channel::unbounded_channel();
        *shared.out.lock().unwrap() = Some(out);
        let at_write = restarted.clone();
        let writer = std::thread::spawn(move || {
            let outgoing = incoming.blocking_recv().unwrap();
            assert!(!at_write.load(Ordering::SeqCst));
            outgoing.wrote.unwrap().send(()).unwrap();
            outgoing.text
        });
        finish_call(&shared, &machine, 7, Call::UpdateSelf);
        let text = writer.join().unwrap();
        let frame: Value = serde_json::from_str(&text).unwrap();
        assert!(restarted.load(Ordering::SeqCst));
        assert_eq!(frame["id"], 7);
        assert_eq!(frame["ok"], serde_json::json!({ "state": "restarting", "to": "1.2.0" }));
    }

    struct HoldRestart {
        started: Mutex<Option<std::sync::mpsc::Sender<()>>>,
        release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        restarted: Arc<AtomicBool>,
    }

    impl crate::machine::SelfUpdate for HoldRestart {
        fn install(&self, machine: &Machine) -> crate::wire::UpdateStatus {
            let restarted = self.restarted.clone();
            machine.schedule_restart(Box::new(move || restarted.store(true, Ordering::SeqCst)));
            machine.report(crate::wire::UpdateStatus::Restarting { to: "1.2.0".into() });
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            if let Some(release) = self.release.lock().unwrap().take() {
                let _ = release.recv();
            }
            crate::wire::UpdateStatus::Restarting { to: "1.2.0".into() }
        }
    }

    struct LetGo(Option<std::sync::mpsc::Sender<()>>);

    impl Drop for LetGo {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[test]
    fn a_different_call_does_not_restart_an_update() {
        use crate::machine::Launch;
        use std::collections::BTreeMap;
        use std::sync::atomic::AtomicBool;

        let restarted = Arc::new(AtomicBool::new(false));
        let machine = Machine::new(
            Launch {
                exe: "pinned".into(),
                bins: BTreeMap::new(),
                path_env: None,
                settings: "settings".into(),
                mcp_config: None,
                reminders: false,
                plugin_dir: "plugin".into(),
                tutorial_helper: "tutorial".into(),
                sock: "sock".into(),
                vector_search: false,
                first_run_wait: std::time::Duration::from_secs(60),
                starting_limit: std::time::Duration::from_secs(600),
                claude_home: None,
            },
            None,
            None,
            |_| {},
        );
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let _let_go = LetGo(Some(release_tx));
        machine.set_installer(Arc::new(HoldRestart {
            started: Mutex::new(Some(started_tx)),
            release: Mutex::new(Some(release_rx)),
            restarted: restarted.clone(),
        }));
        let shared = Arc::new(Shared::default());
        let (out, mut incoming) = channel::unbounded_channel();
        *shared.out.lock().unwrap() = Some(out);
        let writer = std::thread::spawn(move || {
            while let Some(outgoing) = incoming.blocking_recv() {
                if let Some(wrote) = outgoing.wrote {
                    let _ = wrote.send(());
                }
            }
        });
        let updating = machine.clone();
        let shared_update = shared.clone();
        let update = std::thread::spawn(move || finish_call(&shared_update, &updating, 7, Call::UpdateSelf));
        started_rx.recv().unwrap();
        finish_call(&shared, &machine, 8, Call::Tabs);
        assert!(!restarted.load(Ordering::SeqCst), "tabs restarted the update");
        drop(_let_go);
        update.join().unwrap();
        assert!(restarted.load(Ordering::SeqCst), "the update call did not restart");
        drop(shared);
        writer.join().unwrap();
    }

    /// A hub at the account's limit while `room` is false: it refuses any push that grows the
    /// account, and counts the calls it gets.
    #[derive(Default)]
    struct FullHub {
        files: std::cell::RefCell<std::collections::BTreeMap<String, sync::File>>,
        room: std::cell::Cell<bool>,
        lists: std::cell::Cell<usize>,
        pushes: std::cell::Cell<usize>,
        /// What the hub refuses a growing push with; the account's 1024 MB unless a test says.
        refusal: std::cell::RefCell<Option<String>>,
    }

    impl sync::Hub for FullHub {
        fn list(&self, _: &str) -> Result<Vec<sync::Entry>, String> {
            self.lists.set(self.lists.get() + 1);
            Ok(self.files.borrow().values().map(|f| f.entry.clone()).collect())
        }
        fn push(&self, _: &str, files: Vec<sync::File>) -> Result<(), String> {
            self.pushes.set(self.pushes.get() + 1);
            let replaced: u64 = files.iter().filter_map(|f| self.files.borrow().get(&f.entry.path).map(|old| old.entry.size)).sum();
            let incoming: u64 = files.iter().map(|f| f.entry.size).sum();
            if !self.room.get() && incoming > replaced {
                let account = "the account's synced projects would pass 1024 MB; nothing is deleted, the push is refused";
                return Err(self.refusal.borrow().clone().unwrap_or_else(|| account.into()));
            }
            for file in files {
                self.files.borrow_mut().insert(file.entry.path.clone(), file);
            }
            Ok(())
        }
        fn pull(&self, _: &str, paths: Vec<String>) -> Result<Vec<sync::File>, String> {
            Ok(paths.iter().filter_map(|p| self.files.borrow().get(p).cloned()).collect())
        }
        fn mark_executable(&self, _: &str, _: Vec<sync::ModeRepair>) -> Result<(), String> {
            Ok(())
        }
    }

    fn full_checkout(name: &str) -> (std::path::PathBuf, sync::Checkout, std::path::PathBuf) {
        project_checkout("github.com/acme/app", name)
    }

    fn project_checkout(key: &str, name: &str) -> (std::path::PathBuf, sync::Checkout, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pinned-link-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = sync::Project { key: key.into(), name: "app".into(), repo: format!("git@github.com:{}.git", key.trim_start_matches("github.com/")) };
        let checkout = sync::Checkout { project, path: dir.join("app"), history: None, claude_home: Some(dir.join(".claude")) };
        let notes = sync::claude_dir(&dir.join(".claude"), &checkout.path).join("memory/notes.md");
        std::fs::create_dir_all(notes.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&checkout.path).unwrap();
        std::fs::write(&notes, "notes written at the limit").unwrap();
        (dir, checkout, notes)
    }

    /// Runs a sync cycle every `SYNC_EVERY` from `start`, calling `before` ahead of each, and
    /// returns the seconds at which a push reached the hub.
    fn pushed_at(hub: &FullHub, checkout: &sync::Checkout, cycles: u32, mut before: impl FnMut(u64)) -> Vec<u64> {
        let (mut backoff, mut state) = (PushBackoff::with_draw(|| 0.5), sync::State::default());
        let start = Instant::now();
        let mut at = vec![];
        for cycle in 0..cycles {
            let seconds = (SYNC_EVERY * cycle).as_secs();
            before(seconds);
            let pushes = hub.pushes.get();
            backoff.begin(start + SYNC_EVERY * cycle);
            let _ = backoff.sync(hub, &mut state, checkout, start + SYNC_EVERY * cycle);
            backoff.finish();
            if hub.pushes.get() > pushes {
                at.push(seconds);
            }
        }
        at
    }

    #[test]
    fn a_refused_push_waits_twice_as_long_each_time_up_to_ten_minutes_while_the_project_lists_every_cycle() {
        let (dir, checkout, _) = full_checkout("full-hub");
        let hub = FullHub::default();
        let at = pushed_at(&hub, &checkout, 400, |_| {});
        assert_eq!(at, [0, 20, 60, 140, 300, 620, 1220, 1820, 2420, 3020, 3620, 4220, 4820, 5420, 6020, 6620, 7220, 7820]);
        assert_eq!(hub.lists.get(), 400, "the project still lists and pulls every cycle");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_changed_file_waits_out_the_account_hold_and_the_probe_the_hub_takes_ends_it() {
        // Refusals at 0, 20 and 60 s hold 20, 40 and 80 s. The edits at 40 and 120 s grow nothing
        // sent during the hold and change nothing about it; the probe at 140 s is taken.
        let (dir, checkout, notes) = full_checkout("full-hub-reset");
        let hub = FullHub::default();
        let at = pushed_at(&hub, &checkout, 8, |seconds| match seconds {
            40 => std::fs::write(&notes, "notes edited at second 40").unwrap(),
            100 => hub.room.set(true),
            120 => std::fs::write(&notes, "notes edited again at second 120").unwrap(),
            _ => {}
        });
        assert_eq!(at, [0, 20, 60, 140]);
        assert_eq!(hub.files.borrow()["claude/memory/notes.md"].entry.sha, sync::sha(b"notes edited again at second 120"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn append(path: &std::path::Path, line: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap().write_all(line.as_bytes()).unwrap();
    }

    #[test]
    fn a_held_history_waits_for_a_probe_the_hub_takes_and_then_merges_both_sides() {
        let (dir, mut mac, _) = full_checkout("held-history-mac");
        let (mini_dir, mut mini, _) = full_checkout("held-history-mini");
        mac.history = Some(dir.join("history.jsonl"));
        mini.history = Some(mini_dir.join("history.jsonl"));
        let turn = |session: &str, ended: u64| format!("{{\"session\":\"{session}\",\"ended\":{ended}}}\n");
        let (mac_history, mini_history) = (mac.history.clone().unwrap(), mini.history.clone().unwrap());
        append(&mac_history, &turn("both", 1));
        let hub = FullHub::default();
        hub.room.set(true);
        let (mut mac_state, mut mini_state) = (sync::State::default(), sync::State::default());
        sync::sync(&hub, &mut mac_state, &mac, SystemTime::now()).unwrap();
        sync::sync(&hub, &mut mini_state, &mini, SystemTime::now()).unwrap();
        hub.room.set(false);
        append(&mac_history, &turn("mac", 2));

        let (mut backoff, start, mut at) = (PushBackoff::with_draw(|| 0.5), Instant::now(), vec![]);
        for cycle in 0..16 {
            let seconds = (SYNC_EVERY * cycle).as_secs();
            append(&mini_history, &turn("mini", 1000 + seconds));
            hub.room.set(true);
            assert_eq!(sync::sync(&hub, &mut mini_state, &mini, SystemTime::now()).unwrap().push, sync::Push::Done);
            hub.room.set(false);
            if seconds == 200 {
                append(&mac_history, &turn("mac", 3));
            }
            let pushes = hub.pushes.get();
            backoff.begin(start + SYNC_EVERY * cycle);
            backoff.sync(&hub, &mut mac_state, &mac, start + SYNC_EVERY * cycle).unwrap();
            backoff.finish();
            if hub.pushes.get() > pushes {
                at.push(seconds);
            }
        }
        assert_eq!(at, [0, 20, 60, 140, 300], "the hold keeps the history back, the hub's turns and mac's own alike, until the probe the hub refuses at 300 s");
        assert_eq!(std::fs::read_to_string(&mac_history).unwrap(), [turn("both", 1), turn("mac", 2), turn("mac", 3)].concat(), "the history is not merged while the hub refuses it");

        hub.room.set(true);
        assert_eq!(sync::sync(&hub, &mut mac_state, &mac, SystemTime::now()).unwrap().pushed, vec![sync::HISTORY]);
        let merged = std::fs::read_to_string(&mac_history).unwrap();
        assert!(merged.contains(&turn("mac", 3)) && merged.contains(&turn("mini", 1300)), "taken, the merge lands here too: {merged}");
        assert_eq!(hub.files.borrow()[sync::HISTORY].entry.sha, sync::sha(merged.as_bytes()));
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(mini_dir));
    }

    #[test]
    fn a_taken_push_ends_the_hold_so_a_later_refusal_waits_twenty_seconds_again() {
        let (dir, checkout, notes) = full_checkout("taken-ends-hold");
        let hub = FullHub::default();
        // The hub has room from 60 s, the probe there is taken and ends the hold; the next refusal, at
        // 80 s, holds 20 s, so the probe after it is at 100 s.
        let at = pushed_at(&hub, &checkout, 6, |seconds| match seconds {
            40 => std::fs::write(&notes, "b").unwrap(),
            60 => hub.room.set(true),
            80 => {
                hub.room.set(false);
                std::fs::write(&notes, "notes written at the limit").unwrap();
            }
            _ => {}
        });
        assert_eq!(at, [0, 20, 60, 80, 100]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A hub for the whole account: while `room` is false it refuses a push that grows what it
    /// holds, across every project, the way the hub's limit does. `busy` refuses every push with an
    /// error that is not the limit's. It counts the push calls it gets.
    #[derive(Default)]
    struct AccountHub {
        files: std::cell::RefCell<std::collections::BTreeMap<(String, String), sync::File>>,
        room: std::cell::Cell<bool>,
        busy: std::cell::Cell<bool>,
        pushes: std::cell::Cell<usize>,
    }

    impl sync::Hub for AccountHub {
        fn list(&self, project: &str) -> Result<Vec<sync::Entry>, String> {
            Ok(self.files.borrow().iter().filter(|((p, _), _)| p == project).map(|(_, f)| f.entry.clone()).collect())
        }
        fn push(&self, project: &str, files: Vec<sync::File>) -> Result<(), String> {
            self.pushes.set(self.pushes.get() + 1);
            if self.busy.get() {
                return Err("the hub is restarting".into());
            }
            let key = |path: &str| (project.to_owned(), path.to_owned());
            let replaced: u64 = files.iter().filter_map(|f| self.files.borrow().get(&key(&f.entry.path)).map(|old| old.entry.size)).sum();
            let incoming: u64 = files.iter().map(|f| f.entry.size).sum();
            if !self.room.get() && incoming > replaced {
                return Err(format!("{}1024 MB; nothing is deleted, the push is refused", sync::LIMIT_REFUSAL));
            }
            for file in files {
                let key = key(&file.entry.path);
                if file.data.is_some() {
                    self.files.borrow_mut().insert(key, file);
                } else {
                    self.files.borrow_mut().remove(&key);
                }
            }
            Ok(())
        }
        fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<sync::File>, String> {
            Ok(paths.iter().filter_map(|p| self.files.borrow().get(&(project.to_owned(), p.clone())).cloned()).collect())
        }
        fn mark_executable(&self, _: &str, _: Vec<sync::ModeRepair>) -> Result<(), String> {
            Ok(())
        }
    }

    /// Text whose length grows with `seconds`, so a file rewritten later is a bigger one on the hub.
    fn grown(seconds: u64) -> String {
        (0..seconds * 10 + 10).map(|i| format!("{:08x}", (i as u32).wrapping_mul(2654435761) ^ seconds as u32)).collect()
    }

    /// Runs one sync of each checkout every `SYNC_EVERY` for `cycles` cycles, calling `before` ahead
    /// of each, and returns the push calls the hub took, as (second, calls) for each cycle that took any.
    fn account_pushes(hub: &AccountHub, backoff: &mut PushBackoff, checkouts: &[&sync::Checkout], cycles: u32, mut before: impl FnMut(u64)) -> Vec<(u64, usize)> {
        let (mut state, start) = (sync::State::default(), Instant::now());
        let mut calls = vec![];
        for cycle in 0..cycles {
            let seconds = (SYNC_EVERY * cycle).as_secs();
            before(seconds);
            let pushes = hub.pushes.get();
            backoff.begin(start + SYNC_EVERY * cycle);
            for checkout in checkouts {
                let _ = backoff.sync(hub, &mut state, checkout, start + SYNC_EVERY * cycle);
            }
            backoff.finish();
            if hub.pushes.get() > pushes {
                calls.push((seconds, hub.pushes.get() - pushes));
            }
        }
        calls
    }

    #[test]
    fn a_limit_refusal_holds_the_account_to_eight_push_calls_in_thirty_minutes_at_the_shortest_jitter() {
        // The first hold is `PUSH_HOLD_MIN`, 20 s, and at the shortest jitter (draw 0.0) it is 0.75 of
        // that: 15 s. Each refused probe doubles the step and the hold is 0.75 of that: 30, 60, 120,
        // 240 s, then 600 s (640 s capped), 450 s at the shortest. A probe is the first cycle at or after
        // the hold ends, and cycles come every 20 s, so the probes fall at 0, 20, 60, 120, 240, 480,
        // 940 and 1400 s: eight in the 1800 s. The next would be at 1860 s.
        let (dir, checkout, notes) = full_checkout("account-limit");
        let hub = AccountHub::default();
        let mut backoff = PushBackoff::with_draw(|| 0.0);
        let calls = account_pushes(&hub, &mut backoff, &[&checkout], 90, |seconds| std::fs::write(&notes, grown(seconds)).unwrap());
        assert_eq!(calls, [(0, 1), (20, 1), (60, 1), (120, 1), (240, 1), (480, 1), (940, 1), (1400, 1)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_limit_refusal_holds_the_other_projects_of_the_account_too() {
        let (dir, app, app_notes) = project_checkout("github.com/acme/app", "account-app");
        let (other_dir, site, site_notes) = project_checkout("github.com/acme/site", "account-site");
        let hub = AccountHub::default();
        let mut backoff = PushBackoff::with_draw(|| 0.0);
        let calls = account_pushes(&hub, &mut backoff, &[&app, &site], 90, |seconds| {
            std::fs::write(&app_notes, grown(seconds)).unwrap();
            std::fs::write(&site_notes, grown(seconds)).unwrap();
        });
        assert_eq!(calls, [(0, 1), (20, 1), (60, 1), (120, 1), (240, 1), (480, 1), (940, 1), (1400, 1)], "the site's push waits with the app's, one call per probe");
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(other_dir));
    }

    fn first_retry(draw: fn() -> f64, name: &str) -> u64 {
        let (dir, checkout, notes) = full_checkout(name);
        let hub = AccountHub::default();
        let mut backoff = PushBackoff::with_draw(draw);
        let calls = account_pushes(&hub, &mut backoff, &[&checkout], 3, |seconds| std::fs::write(&notes, grown(seconds)).unwrap());
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(calls[0], (0, 1), "the first push is refused");
        calls[1].0
    }

    #[test]
    fn a_limit_hold_varies_its_first_wait_by_the_jitter_either_way() {
        // 20 s is `PUSH_HOLD_MIN`; `BACKOFF_JITTER` puts it at 15 s, 20 s or 25 s. The first probe is the
        // first cycle at or after the hold: 20 s for 15 s or 20 s, 40 s for 25 s.
        assert_eq!(first_retry(|| 0.0, "jitter-short"), 20);
        assert_eq!(first_retry(|| 0.5, "jitter-middle"), 20);
        assert_eq!(first_retry(|| 1.0, "jitter-long"), 40);
    }

    #[test]
    fn a_push_that_only_deletes_goes_out_while_the_account_is_held() {
        let (dir, checkout, notes) = full_checkout("held-deletes");
        let old = notes.with_file_name("old.md");
        let hub = AccountHub::default();
        hub.room.set(true);
        let mut backoff = PushBackoff::with_draw(|| 0.0);
        let calls = account_pushes(&hub, &mut backoff, &[&checkout], 4, |seconds| match seconds {
            0 => {
                std::fs::write(&old, "old notes").unwrap();
                std::fs::write(&notes, grown(0)).unwrap();
            }
            20 => {
                hub.room.set(false);
                std::fs::write(&notes, grown(20)).unwrap();
            }
            60 => std::fs::remove_file(&old).unwrap(),
            _ => {}
        });
        // 0 s taken; 20 s refused; 40 s the probe is refused again, the hold now 30 s; 60 s is inside it,
        // and the only thing sent is the deletion, which the hub takes because it grows nothing.
        assert_eq!(calls, [(0, 1), (20, 1), (40, 1), (60, 1)], "at 60 s the deletion alone goes out, once");
        assert!(!hub.files.borrow().contains_key(&("github.com/acme/app".to_owned(), "claude/memory/old.md".to_owned())), "the hub took the deletion");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn once_the_hub_has_room_the_next_probe_is_taken_and_syncing_resumes_every_cycle() {
        let (dir, checkout, notes) = full_checkout("room-again");
        let hub = AccountHub::default();
        let mut backoff = PushBackoff::with_draw(|| 0.0);
        let calls = account_pushes(&hub, &mut backoff, &[&checkout], 10, |seconds| {
            if seconds == 100 {
                hub.room.set(true);
            }
            std::fs::write(&notes, grown(seconds)).unwrap();
        });
        // Probes refused at 0, 20 and 60 s (holds of 15, 30 and 60 s); the hub has room from 100 s, so
        // the probe at 120 s is taken, and every cycle after it sends the changed file at once.
        assert_eq!(calls, [(0, 1), (20, 1), (60, 1), (120, 1), (140, 1), (160, 1), (180, 1)]);
        assert_eq!(hub.files.borrow()[&("github.com/acme/app".to_owned(), "claude/memory/notes.md".to_owned())].entry.sha, sync::sha(grown(180).as_bytes()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_refusal_that_is_not_the_limit_holds_nothing_and_the_next_cycle_sends_again() {
        let (dir, checkout, notes) = full_checkout("busy-hub");
        let hub = AccountHub::default();
        hub.busy.set(true);
        let mut backoff = PushBackoff::with_draw(|| 0.0);
        let mut state = sync::State::default();
        let start = Instant::now();
        let mut errors = vec![];
        for cycle in 0..4 {
            std::fs::write(&notes, grown(SYNC_EVERY.as_secs() * cycle as u64)).unwrap();
            backoff.begin(start + SYNC_EVERY * cycle);
            errors.push(backoff.sync(&hub, &mut state, &checkout, start + SYNC_EVERY * cycle).err());
            backoff.finish();
        }
        assert_eq!(errors, vec![Some("the hub is restarting".to_owned()); 4], "each refusal is an error of the sync, as before");
        assert_eq!(hub.pushes.get(), 4, "a refusal that is not the limit is not held: every cycle sends again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_user_action_sends_a_refused_project_again_at_once_instead_of_after_its_hold() {
        let (dir, file, machine, shared, _frames) = stopped_sync_fixture("wake");
        let app = dir.join("app");
        std::fs::create_dir_all(app.join(".claude/skills")).unwrap();
        for args in [&["init", "-q"][..], &["remote", "add", "origin", "git@github.com:acme/app.git"]] {
            assert!(std::process::Command::new("git").arg("-C").arg(&app).args(args).status().unwrap().success());
        }
        std::fs::write(app.join(".claude/skills/skill.md"), "a skill").unwrap();
        machine.open_project(app);
        let hub = FullHub::default();
        let mut syncer = Syncer { shared_projects: HashSet::from(["github.com/acme/app".to_owned()]), ..Syncer::default() };
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(hub.pushes.get(), 1, "the hub refuses the first push");
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(hub.pushes.get(), 1, "the refused push waits out its hold");
        shared.sync_now.store(true, Ordering::SeqCst);
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(hub.pushes.get(), 2, "a user action sends the refused push again at once");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A project at the account's limit, open on a machine whose sync loop has the state file.
    fn refused_project(name: &str) -> (std::path::PathBuf, std::path::PathBuf, Arc<Machine>, Shared, std::path::PathBuf) {
        let (dir, file, machine, shared, _frames) = stopped_sync_fixture(name);
        let app = dir.join("app");
        std::fs::create_dir_all(app.join(".claude/skills")).unwrap();
        for args in [&["init", "-q"][..], &["remote", "add", "origin", "git@github.com:acme/app.git"]] {
            assert!(std::process::Command::new("git").arg("-C").arg(&app).args(args).status().unwrap().success());
        }
        std::fs::write(app.join(".claude/skills/skill.md"), "a skill").unwrap();
        machine.open_project(app.clone());
        (dir, file, machine, shared, app)
    }

    #[test]
    fn a_refused_push_shows_the_limit_notice_until_a_push_is_taken() {
        let (dir, file, machine, shared, _app) = refused_project("limit-notice");
        let hub = FullHub::default();
        let mut syncer = Syncer { shared_projects: HashSet::from(["github.com/acme/app".to_owned()]), ..Syncer::default() };
        assert_eq!(machine.call(Call::SyncLimit), Ok(Value::Null), "nothing refused yet");
        syncer.cycle(&shared, &hub, &machine, &file);
        let notice = "Sync stopped: your synced projects would pass 1024 MB. Nothing was lost; this machine keeps everything.";
        assert_eq!(machine.call(Call::SyncLimit), Ok(Value::String(notice.into())), "the refused push shows the notice");
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(machine.call(Call::SyncLimit), Ok(Value::String(notice.into())), "a held push keeps the notice");
        hub.room.set(true);
        shared.sync_now.store(true, Ordering::SeqCst);
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(machine.call(Call::SyncLimit), Ok(Value::Null), "a push the hub takes clears it");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_push_the_full_hub_refuses_shows_that_the_hub_is_out_of_space() {
        let (dir, file, machine, shared, _app) = refused_project("limit-hub-full");
        let hub = FullHub::default();
        *hub.refusal.borrow_mut() = Some("the account's synced projects would pass the space the hub has left; nothing is deleted, the push is refused".into());
        let mut syncer = Syncer { shared_projects: HashSet::from(["github.com/acme/app".to_owned()]), ..Syncer::default() };
        syncer.cycle(&shared, &hub, &machine, &file);
        let paused = "Sync paused: the hub is out of space for synced projects. Nothing was lost; this machine keeps everything.";
        assert_eq!(machine.call(Call::SyncLimit), Ok(Value::String(paused.into())));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_refused_project_that_leaves_the_machine_clears_the_limit_notice() {
        let (dir, file, machine, shared, app) = refused_project("limit-gone");
        let hub = FullHub::default();
        let mut syncer = Syncer { shared_projects: HashSet::from(["github.com/acme/app".to_owned()]), ..Syncer::default() };
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(machine.call(Call::SyncLimit), Ok(Value::String("Sync stopped: your synced projects would pass 1024 MB. Nothing was lost; this machine keeps everything.".into())));
        std::fs::remove_dir_all(app.join(".git")).unwrap();
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(machine.call(Call::SyncLimit), Ok(Value::Null), "a project no longer on the machine holds no notice");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A syncer whose hold is over: the next cycle is a probe.
    fn probe_due(syncer: &mut Syncer) {
        syncer.backoff.limit.as_mut().expect("a hold to end").until = Instant::now();
    }

    fn syncer_of(keys: &[&str]) -> Syncer {
        Syncer { shared_projects: keys.iter().map(|key| (*key).to_owned()).collect(), backoff: PushBackoff::with_draw(|| 0.5), ..Syncer::default() }
    }

    /// A git checkout in `dir` with a remote, as a project the machine syncs.
    fn init_project(dir: &Path, folder: &str, remote: &str) -> std::path::PathBuf {
        let app = dir.join(folder);
        std::fs::create_dir_all(app.join(".claude/skills")).unwrap();
        for args in [&["init", "-q"][..], &["remote", "add", "origin", remote]] {
            assert!(std::process::Command::new("git").arg("-C").arg(&app).args(args).status().unwrap().success());
        }
        app
    }

    #[test]
    fn a_probe_with_nothing_to_send_ends_the_hold() {
        let (dir, file, machine, shared, app) = refused_project("probe-nothing");
        let hub = AccountHub::default();
        let mut syncer = syncer_of(&["github.com/acme/app"]);
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(hub.pushes.get(), 1, "the hub refuses the first push");
        assert!(syncer.backoff.limited());
        std::fs::remove_file(app.join(".claude/skills/skill.md")).unwrap();
        probe_due(&mut syncer);
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(hub.pushes.get(), 1, "the probe has nothing to send");
        assert!(!syncer.backoff.limited(), "a probe cycle that finishes clean ends the hold, whether or not it sent anything");
        assert_eq!(machine.call(Call::SyncLimit), Ok(Value::Null), "and the notice goes");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_clean_project_does_not_end_the_hold_while_another_is_refused() {
        let (dir, file, machine, shared, _frames) = stopped_sync_fixture("clean-and-refused");
        let site = init_project(&dir, "site", "git@github.com:acme/site.git");
        machine.open_project(site.clone());
        // The projects sync most recently opened first, and a second is the grain of that order.
        std::thread::sleep(Duration::from_millis(1100));
        let app = init_project(&dir, "app", "git@github.com:acme/app.git");
        machine.open_project(app.clone());
        assert_eq!(machine.checkouts().iter().map(|c| c.project.key.as_str()).collect::<Vec<_>>(), ["github.com/acme/app", "github.com/acme/site"]);
        let hub = AccountHub::default();
        let mut syncer = syncer_of(&["github.com/acme/app", "github.com/acme/site"]);
        let on_hub = |project: &str| hub.files.borrow().get(&(project.to_owned(), "skills/skill.md".to_owned())).map(|f| f.entry.sha.clone());

        hub.room.set(true);
        std::fs::write(app.join(".claude/skills/skill.md"), grown(40)).unwrap();
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(on_hub("github.com/acme/app"), Some(sync::sha(grown(40).as_bytes())), "the app's skill reached the hub");

        hub.room.set(false);
        std::fs::write(site.join(".claude/skills/skill.md"), grown(40)).unwrap();
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(on_hub("github.com/acme/site"), None, "the hub refuses the site");
        assert_eq!(syncer.backoff.limit.as_ref().map(|limit| limit.wait), Some(PUSH_HOLD_MIN));

        std::fs::write(app.join(".claude/skills/skill.md"), "short").unwrap();
        probe_due(&mut syncer);
        syncer.cycle(&shared, &hub, &machine, &file);
        assert_eq!(on_hub("github.com/acme/app"), Some(sync::sha(b"short")), "the app has something the hub takes in the same probe cycle");
        assert_eq!(on_hub("github.com/acme/site"), None, "the site is refused again");
        assert_eq!(syncer.backoff.limit.as_ref().map(|limit| limit.wait), Some(PUSH_HOLD_MIN * 2), "a project that synced fine before a refused one does not reset the doubling");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `AccountHub`, except that listing the project `broken` fails, as a hub that cannot read it.
    struct BrokenList<'a> {
        hub: &'a AccountHub,
        broken: &'a [&'a str],
    }

    impl sync::Hub for BrokenList<'_> {
        fn list(&self, project: &str) -> Result<Vec<sync::Entry>, String> {
            if self.broken.contains(&project) {
                return Err("the hub could not list the project".into());
            }
            self.hub.list(project)
        }
        fn push(&self, project: &str, files: Vec<sync::File>) -> Result<(), String> {
            self.hub.push(project, files)
        }
        fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<sync::File>, String> {
            self.hub.pull(project, paths)
        }
        fn mark_executable(&self, project: &str, repairs: Vec<sync::ModeRepair>) -> Result<(), String> {
            self.hub.mark_executable(project, repairs)
        }
    }

    /// The app and the site, the app refused at 0 s with draw 0.5, so the hold is `PUSH_HOLD_MIN`
    /// and the cycle at 20 s is a probe. Returns the probe's errors, with the listing of each project
    /// in `broken` failing.
    fn probe_with_broken(name: &str, room: bool, broken: &[&str]) -> (PushBackoff, Vec<String>) {
        let (dir, app, app_notes) = project_checkout("github.com/acme/app", &format!("{name}-app"));
        let (site_dir, site, site_notes) = project_checkout("github.com/acme/site", &format!("{name}-site"));
        let hub = AccountHub::default();
        let mut backoff = PushBackoff::with_draw(|| 0.5);
        let (mut state, start) = (sync::State::default(), Instant::now());
        std::fs::write(&app_notes, grown(0)).unwrap();
        std::fs::write(&site_notes, grown(0)).unwrap();
        backoff.begin(start);
        assert!(matches!(backoff.sync(&hub, &mut state, &app, start).unwrap().push, sync::Push::Refused { .. }));
        backoff.sync(&hub, &mut state, &site, start).unwrap();
        backoff.finish();
        assert_eq!(backoff.limit.as_ref().map(|limit| limit.wait), Some(PUSH_HOLD_MIN));

        hub.room.set(room);
        std::fs::write(&app_notes, grown(20)).unwrap();
        let broken = BrokenList { hub: &hub, broken };
        let probe = start + SYNC_EVERY;
        backoff.begin(probe);
        let errors = [&site, &app].into_iter().filter_map(|checkout| backoff.sync(&broken, &mut state, checkout, probe).err()).collect();
        backoff.finish();
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(site_dir));
        (backoff, errors)
    }

    fn probe_with_a_broken_site(name: &str, room: bool) -> (PushBackoff, Vec<String>) {
        probe_with_broken(name, room, &["github.com/acme/site"])
    }

    #[test]
    fn a_probe_cycle_whose_every_sync_errors_keeps_the_hold_as_it_is() {
        let (backoff, errors) = probe_with_broken("probe-all-error", true, &["github.com/acme/site", "github.com/acme/app"]);
        assert_eq!(errors.len(), 2, "both syncs fail before they push");
        assert!(backoff.limited(), "no sync reached the hub, so the probe says nothing of its room");
        assert_eq!(backoff.limit.as_ref().map(|limit| limit.wait), Some(PUSH_HOLD_MIN), "the hold is neither reset nor doubled");
        assert!(backoff.refusal().is_some(), "and the notice stays");
        assert!(!backoff.shrinking(Instant::now() + SYNC_EVERY), "the wait is over, so the next cycle probes again");
    }

    #[test]
    fn a_probe_cycle_that_skips_a_sync_keeps_the_hold() {
        let (dir, app, app_notes) = project_checkout("github.com/acme/app", "probe-skipped");
        let hub = AccountHub::default();
        let mut backoff = PushBackoff::with_draw(|| 0.5);
        let (mut state, start) = (sync::State::default(), Instant::now());
        std::fs::write(&app_notes, grown(0)).unwrap();
        backoff.begin(start);
        backoff.sync(&hub, &mut state, &app, start).unwrap();
        backoff.finish();
        assert!(backoff.limited());

        hub.room.set(true);
        let probe = start + SYNC_EVERY;
        backoff.begin(probe);
        backoff.sync(&hub, &mut state, &app, probe).unwrap();
        backoff.skipped();
        backoff.finish();
        assert!(backoff.limited(), "a sync that never ran, as when the link dropped, leaves the hold on");
        assert_eq!(backoff.limit.as_ref().map(|limit| limit.wait), Some(PUSH_HOLD_MIN), "and does not double it");

        backoff.begin(probe + SYNC_EVERY);
        backoff.sync(&hub, &mut state, &app, probe + SYNC_EVERY).unwrap();
        backoff.finish();
        assert!(!backoff.limited(), "the next probe that runs whole ends it");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_probe_cycle_with_an_error_but_no_limit_refusal_ends_the_hold() {
        let (backoff, errors) = probe_with_a_broken_site("probe-error", true);
        assert_eq!(errors, ["the hub could not list the project"], "the site's sync fails on its own");
        assert!(!backoff.limited(), "a probe cycle with no limit refusal ends the hold, whatever other errors it had");
        assert_eq!(backoff.refusal(), None, "and the notice goes");
    }

    #[test]
    fn a_probe_cycle_with_an_error_and_a_limit_refusal_keeps_the_hold_and_doubles_it() {
        let (backoff, errors) = probe_with_a_broken_site("probe-error-refused", false);
        assert_eq!(errors, ["the hub could not list the project"], "the site's sync fails on its own");
        assert_eq!(backoff.limit.as_ref().map(|limit| limit.wait), Some(PUSH_HOLD_MIN * 2), "the app's refusal doubles the hold");
        assert!(backoff.refusal().is_some_and(|refusal| refusal.starts_with(sync::LIMIT_REFUSAL)), "and the notice stays");
    }

    #[test]
    fn a_hold_never_exceeds_ten_minutes_with_the_jitter_at_its_longest() {
        let (dir, file, machine, shared, _app) = refused_project("hold-cap");
        let hub = AccountHub::default();
        let mut syncer = Syncer { backoff: PushBackoff::with_draw(|| 1.0), ..syncer_of(&["github.com/acme/app"]) };
        let mut holds = vec![];
        for cycle in 0..9 {
            if cycle > 0 {
                probe_due(&mut syncer);
            }
            syncer.cycle(&shared, &hub, &machine, &file);
            let limit = syncer.backoff.limit.as_ref().expect("the hub keeps refusing");
            holds.push(limit.until.saturating_duration_since(Instant::now()).as_secs());
        }
        assert_eq!(syncer.backoff.limit.as_ref().map(|limit| limit.wait), Some(PUSH_HOLD_MAX), "the steps reach the cap");
        assert!(holds.iter().all(|seconds| *seconds <= PUSH_HOLD_MAX.as_secs()), "no hold is longer than {PUSH_HOLD_MAX:?}: {holds:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
