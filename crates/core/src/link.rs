//! The binary installs rustls's ring provider before spawning the link.

use crate::account::Credentials;
use crate::agent::Message;
use crate::machine::{Machine, ViewerId};
use crate::sync;
use crate::wire::{Bytes, Call, HubCall, MachineEvent, MachineFrame, Reply, TabId, ToMachine};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
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
const SHARE_WAIT: Duration = Duration::from_secs(15);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
const SYNC_FIRST: Duration = Duration::from_secs(5);
const SYNC_EVERY: Duration = Duration::from_secs(20);
/// The close code the hub sends when it doesn't know the token a socket signed in with.
pub const UNAUTHORIZED: u16 = 4001;

pub struct Link {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
    thread: std::thread::JoinHandle<()>,
}

#[derive(Default)]
struct Shared {
    out: Mutex<Option<channel::UnboundedSender<String>>>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>,
    next_call: AtomicU64,
    subscribed: Mutex<HashSet<(TabId, ViewerId)>>,
    sent: Mutex<HashMap<String, (u64, String)>>,
}

impl Shared {
    fn send(&self, frame: &MachineFrame) -> bool {
        let out = self.out.lock().unwrap().clone();
        out.is_some_and(|out| out.send(serde_json::to_string(frame).unwrap()).is_ok())
    }

    /// Asks the hub, blocking up to 60 seconds for the answer. Fails at once while offline.
    fn call(&self, call: HubCall) -> Result<Value, String> {
        self.call_for(call, CALL_WAIT)
    }

    fn call_for(&self, call: HubCall, wait: Duration) -> Result<Value, String> {
        let id = self.next_call.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, answer);
        if !self.send(&MachineFrame::Call { id, call }) {
            self.pending.lock().unwrap().remove(&id);
            return Err("not connected to the hub".into());
        }
        match answered.recv_timeout(wait) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("the hub did not answer within {} seconds", wait.as_secs()))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("the connection to the hub dropped".into()),
        }
    }

    fn online(&self) -> bool {
        self.out.lock().unwrap().is_some()
    }

    fn disconnected(&self, machine: &Machine) {
        *self.out.lock().unwrap() = None;
        self.pending.lock().unwrap().clear();
        self.sent.lock().unwrap().clear();
        for (tab, viewer) in self.subscribed.lock().unwrap().drain() {
            machine.detach(&tab, &viewer);
        }
    }
}

/// Connects `machine` to the hub `creds` names, and keeps it connected until `Link::stop`.
/// `version` is the binary's, which the hub logs.
pub fn spawn(creds: Credentials, machine: Arc<Machine>, version: &str) -> Link {
    let shared = Arc::new(Shared::default());
    let (stop, stopped) = watch::channel(false);
    let (syncing, synced_machine, sync_stopped) = (shared.clone(), machine.clone(), stopped.clone());
    std::thread::spawn(move || sync_projects(&syncing, &synced_machine, &sync_stopped));
    let (link, version) = (shared.clone(), version.to_owned());
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("could not start the hub link's runtime");
        runtime.block_on(run(link, creds, machine, version, stopped));
    });
    Link { shared, stop, thread }
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
        let _ = self.thread.join();
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
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("the hub sent something pinned can't read: {e}"))
}

/// Keeps the machine's projects in step with the account's while the link runs.
fn sync_projects(shared: &Shared, machine: &Machine, stopped: &watch::Receiver<bool>) {
    let Some(file) = machine.sync_file().map(Path::to_path_buf) else { return };
    let mut state = sync::State::load(&file, &crate::machine::home().join(".claude"));
    let mut shared_projects = HashSet::new();
    let mut wait = SYNC_FIRST;
    loop {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if *stopped.borrow() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        wait = SYNC_EVERY;
        if !shared.online() {
            continue;
        }
        for checkout in machine.checkouts() {
            let key = checkout.project.key.clone();
            if !shared_projects.contains(&key) && shared.call(HubCall::ShareProject { project: checkout.project.clone() }).is_ok() {
                shared_projects.insert(key.clone());
            }
            match sync::sync(shared, &mut state, &checkout, SystemTime::now()) {
                Ok(done) if !done.pushed.is_empty() || !done.pulled.is_empty() => {
                    eprintln!("pinned: synced {key}: {} up, {} down", done.pushed.len(), done.pulled.len())
                }
                Ok(_) => {}
                Err(e) => eprintln!("pinned: could not sync {key}: {e}"),
            }
        }
        state.save(&file);
        push_shares(shared, machine);
    }
}

#[derive(serde::Deserialize)]
struct Peer {
    id: String,
    #[serde(default)]
    generation: u64,
}

fn push_shares(shared: &Shared, machine: &Machine) {
    let Some(body) = machine.share_body() else { return };
    let Ok(bytes) = serde_json::to_vec(&body) else { return };
    if bytes.len() > crate::share::RELAY_LIMIT {
        return;
    }
    let fingerprint = fingerprint(&bytes);
    let peers = match shared.call_for(HubCall::Peers, SHARE_WAIT) {
        Ok(value) => serde_json::from_value::<Vec<Peer>>(value).unwrap_or_default(),
        Err(e) => {
            eprintln!("pinned: could not list machines to share with: {e}");
            return;
        }
    };
    let pending: Vec<Peer> = {
        let mut sent = shared.sent.lock().unwrap();
        sent.retain(|id, _| peers.iter().any(|peer| &peer.id == id));
        peers.into_iter().filter(|peer| !sent.get(&peer.id).is_some_and(|(generation, have)| *generation == peer.generation && have == &fingerprint)).collect()
    };
    for peer in pending {
        match shared.call_for(HubCall::Relay { to: peer.id.clone(), body: body.clone() }, SHARE_WAIT) {
            Ok(_) => {
                shared.sent.lock().unwrap().insert(peer.id, (peer.generation, fingerprint.clone()));
            }
            Err(e) => eprintln!("pinned: could not share with {}: {e}", peer.id),
        }
    }
}

fn fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn run(shared: Arc<Shared>, creds: Credentials, machine: Arc<Machine>, version: String, mut stopped: watch::Receiver<bool>) {
    let events = shared.clone();
    let listener = machine.subscribe(Box::new(move |event| {
        events.send(&MachineFrame::Event { event: event.clone() });
    }));
    let mut backoff = BACKOFF_MIN;
    loop {
        let ended = tokio::select! {
            ended = connection(&shared, &creds, &machine, &version) => Some(ended),
            _ = stopped.changed() => None,
        };
        shared.disconnected(&machine);
        match ended {
            None => break,
            Some(Ok(())) => {
                eprintln!("pinned: disconnected from {}", creds.hub);
                backoff = BACKOFF_MIN;
            }
            Some(Err(e)) => eprintln!("pinned: hub link: {e}; retrying in {}s", backoff.as_secs()),
        }
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = stopped.changed() => break,
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
    machine.unsubscribe(listener);
}

/// One connection, from dialing to its end. Ok means the hub accepted the machine before the
/// connection ended, so the next attempt waits the least.
async fn connection(shared: &Arc<Shared>, creds: &Credentials, machine: &Arc<Machine>, version: &str) -> Result<(), String> {
    let url = ws_url(&creds.hub)?;
    let (mut sink, mut stream) = dial(&url).await?.split();
    let auth = MachineFrame::Auth {
        token: creds.machine_token.clone(),
        name: crate::account::hostname(),
        os: crate::account::os(),
        version: version.into(),
    };
    sink.send(text(&auth)).await.map_err(|e| e.to_string())?;
    let hello = tokio::time::timeout(HELLO_WAIT, stream.next()).await.map_err(|_| "the hub did not answer the sign in".to_string())?;
    match hello {
        Some(Ok(Frame::Text(t))) if matches!(serde_json::from_str(&t), Ok(ToMachine::Hello { .. })) => {}
        Some(Ok(Frame::Close(Some(close)))) if u16::from(close.code) == UNAUTHORIZED => {
            return Err("the hub does not know this machine's token: pair it again".into())
        }
        other => return Err(format!("the hub answered the sign in with {other:?}")),
    }

    let (out, mut outgoing) = channel::unbounded_channel();
    *shared.out.lock().unwrap() = Some(out.clone());
    let calls = in_order(machine.clone(), out.clone());
    eprintln!("pinned: connected to {}", creds.hub);
    shared.send(&MachineFrame::Event { event: MachineEvent::Tabs { tabs: machine.tabs() } });
    for (account, usage) in machine.usage() {
        shared.send(&MachineFrame::Event { event: MachineEvent::Usage { usage, account } });
    }

    let mut ping = tokio::time::interval(PING);
    let mut heard = Instant::now();
    loop {
        tokio::select! {
            Some(frame) = outgoing.recv() => sink.send(Frame::Text(frame.into())).await.map_err(|e| e.to_string())?,
            frame = stream.next() => {
                heard = Instant::now();
                match frame {
                    Some(Ok(Frame::Text(t))) => answer(shared, machine, &out, &calls, &t, version),
                    Some(Ok(Frame::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e.to_string()),
                }
            }
            _ = ping.tick() => {
                if heard.elapsed() > SILENT {
                    return Err("the hub stopped answering".into());
                }
                sink.send(Frame::Ping(Default::default())).await.map_err(|e| e.to_string())?;
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
fn in_order(machine: Arc<Machine>, out: channel::UnboundedSender<String>) -> mpsc::Sender<(u64, Call)> {
    let (calls, queue) = mpsc::channel::<(u64, Call)>();
    std::thread::spawn(move || {
        for (id, call) in queue {
            let reply = MachineFrame::Reply(Reply::new(id, machine.call(call)));
            let _ = out.send(serde_json::to_string(&reply).unwrap());
        }
    });
    calls
}

fn answer(shared: &Shared, machine: &Arc<Machine>, out: &channel::UnboundedSender<String>, calls: &mpsc::Sender<(u64, Call)>, frame: &str, version: &str) {
    match serde_json::from_str::<ToMachine>(frame) {
        Ok(ToMachine::Call { id, call }) if call.may_wait() => {
            let (machine, out) = (machine.clone(), out.clone());
            std::thread::spawn(move || {
                let reply = MachineFrame::Reply(Reply::new(id, machine.call(call)));
                let _ = out.send(serde_json::to_string(&reply).unwrap());
            });
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
                let _ = out.send(serde_json::to_string(&frame).unwrap());
            };
            if machine.attach(&tab, viewer.clone(), Box::new(sink)).is_ok() {
                shared.subscribed.lock().unwrap().insert((tab, viewer));
            }
        }
        Ok(ToMachine::Unsubscribe { tab, viewer }) => {
            machine.detach(&tab, &viewer);
            shared.subscribed.lock().unwrap().remove(&(tab, viewer));
        }
        Ok(ToMachine::Hello { .. }) => {}
        Ok(ToMachine::Share { id, body }) => {
            let (machine, out) = (machine.clone(), out.clone());
            std::thread::spawn(move || {
                let reply = MachineFrame::Reply(Reply::new(id, machine.apply_share(body).map(|_| Value::Null)));
                let _ = out.send(serde_json::to_string(&reply).unwrap());
            });
        }
        Err(e) => {
            let value: Value = serde_json::from_str(frame).unwrap_or_default();
            if let (Some("call"), Some(id)) = (value["type"].as_str(), value["id"].as_u64()) {
                let reply = MachineFrame::Reply(Reply::new(id, Err(format!("this machine runs pinned {version}, which can't answer that: {e}"))));
                let _ = out.send(serde_json::to_string(&reply).unwrap());
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

    #[test]
    fn the_machine_socket_follows_the_hubs_scheme() {
        assert_eq!(ws_url("https://pinned.example.com/").as_deref(), Ok("wss://pinned.example.com/ws/machine"));
        assert_eq!(ws_url("http://127.0.0.1:8787").as_deref(), Ok("ws://127.0.0.1:8787/ws/machine"));
        assert!(ws_url("pinned.example.com").is_err());
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
}
