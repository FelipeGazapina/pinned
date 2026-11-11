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
/// The close code the hub sends when it doesn't know the token a socket signed in with.
pub const UNAUTHORIZED: u16 = 4001;

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
}

impl Shared {
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
        written.recv_timeout(Duration::from_secs(5)).is_ok()
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
                if shared.sync_now.swap(false, Ordering::SeqCst) {
                    break;
                }
                std::thread::park_timeout(left);
            }
            wait = SYNC_EVERY;
            if shared.online() {
                report_state(shared, machine);
                if let Some(file) = file {
                    self.cycle(shared, machine, file);
                }
            }
        }
        if let Some(state) = self.state.as_mut() {
            if let Err(e) = state.flush() {
                eprintln!("pinned: {e}");
            }
        }
        machine.hold_sync_state(false);
    }

    fn cycle(&mut self, shared: &Shared, machine: &Machine, file: &Path) {
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
        for checkout in machine.checkouts() {
            if !shared.online() {
                break;
            }
            let key = checkout.project.key.clone();
            if !self.shared_projects.contains(&key) && shared.call(HubCall::ShareProject { project: checkout.project.clone() }).is_ok() {
                self.shared_projects.insert(key.clone());
            }
            match sync::sync(shared, state, &checkout, SystemTime::now()) {
                Ok(done) => {
                    machine.skills_changed(&skills::pulled_by_project_sync(&checkout, &done.pulled));
                    if !done.pushed.is_empty() || !done.pulled.is_empty() {
                        eprintln!("pinned: synced {key}: {} up, {} down", done.pushed.len(), done.pulled.len())
                    }
                }
                Err(e) => eprintln!("pinned: could not sync {key}: {e}"),
            }
        }
        for share in machine.plugin_shares() {
            match sync::sync_plugins(shared, state, &share, SystemTime::now()) {
                Ok(done) => {
                    self.reported_old_hub = false;
                    machine.skills_changed(&skills::pulled_by_plugin_sync(&share, &done.pulled));
                    if !done.pushed.is_empty() || !done.pulled.is_empty() {
                        eprintln!("pinned: synced plugins: {} up, {} down", done.pushed.len(), done.pulled.len());
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
        let spread = 1.0 + BACKOFF_JITTER * (2.0 * unit.clamp(0.0, 1.0) - 1.0);
        let wait = self.next.mul_f64(spread).min(BACKOFF_MAX);
        self.next = (self.next * 2).min(BACKOFF_MAX);
        wait
    }
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
        let up = accepted.map(|at: Instant| at.elapsed());
        match ended {
            None => break,
            Some(Ok(())) => {
                eprintln!("pinned: disconnected from {}", creds.hub);
                backoff.ended(up, true);
            }
            Some(Err(Fault::Refused(e))) => {
                backoff.ended(up, false);
                eprintln!("pinned: hub link: {e}; retrying");
                crate::boot::refuse();
            }
            Some(Err(Fault::Transport(e))) => {
                backoff.ended(up, false);
                eprintln!("pinned: hub link: {e}; retrying");
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
    Refused(String),
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
            Err(Fault::Refused("the hub does not know this machine's token: pair it again".into()))
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
        syncer.cycle(&shared, &machine, &file);
        assert_eq!(std::fs::read(&file).unwrap(), b"", "a stopped sync leaves the file as it was");
        let why = format!("the sync state {} is corrupt (EOF while parsing a value at line 1 column 0)", file.display());
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::String(why)));
        assert_eq!(machine.call(Call::ResetSync), Ok(Value::Null));
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::Null));
        syncer.cycle(&shared, &machine, &file);
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
        syncer.cycle(&shared, &machine, &file);
        std::fs::write(&file, b"").unwrap();
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::Null));
        syncer.cycle(&shared, &machine, &file);
        assert_eq!(machine.call(Call::SyncStopped), Ok(Value::Null), "an idle cycle keeps the state it holds and doesn't rewrite the file");
        assert_eq!(std::fs::read(&file).unwrap(), b"");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn once_the_sync_loop_ends_the_state_file_decides_again() {
        let (dir, file, machine, shared, _frames) = stopped_sync_fixture("ended");
        let mut syncer = Syncer::default();
        syncer.cycle(&shared, &machine, &file);
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
        Syncer::default().cycle(&shared, &machine, &file);
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
        assert!(matches!(accept_hello(close(UNAUTHORIZED)), Err(Fault::Refused(_))));
        assert!(matches!(accept_hello(Some(Ok(Frame::Text(r#"{"type":"nope"}"#.into())))), Err(Fault::Refused(_))));
        assert!(matches!(accept_hello(close(1001)), Err(Fault::Transport(_))));
        assert!(matches!(accept_hello(None), Err(Fault::Transport(_))));
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
}
