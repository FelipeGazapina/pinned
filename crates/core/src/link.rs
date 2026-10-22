//! A machine's connection to its account's hub: one outbound WebSocket that answers the hub's
//! calls, streams the tabs it subscribes to, reports every machine event, and carries a connector
//! session's callbacks to the hub, which holds the integrations' keys. It reconnects by itself.
//!
//! The link runs on its own thread and runtime, so the app and the headless node use it the same
//! way. The binary installs rustls's ring provider before spawning it.

use crate::account::Credentials;
use crate::agent::Message;
use crate::machine::{Machine, ViewerId};
use crate::wire::{Bytes, Call, HubCall, MachineEvent, MachineFrame, Reply, TabId, ToMachine};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc as channel, watch};
use tokio_tungstenite::tungstenite::Message as Frame;

const PING: Duration = Duration::from_secs(20);
/// A hub silent this long, pongs included, is gone even if the socket hasn't said so.
const SILENT: Duration = Duration::from_secs(60);
const HELLO_WAIT: Duration = Duration::from_secs(15);
const CALL_WAIT: Duration = Duration::from_secs(60);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// The close code the hub sends when it doesn't know the token a socket signed in with.
pub const UNAUTHORIZED: u16 = 4001;

pub struct Link {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
    thread: std::thread::JoinHandle<()>,
}

/// What the link's thread and its callers share.
#[derive(Default)]
struct Shared {
    /// The live connection's outgoing frames, from the hub's hello until the connection ends.
    out: Mutex<Option<channel::UnboundedSender<String>>>,
    /// `call_hub`s waiting for their reply, by frame id.
    pending: Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>,
    next_call: AtomicU64,
    /// The tabs and viewers the live connection streams to the hub.
    subscribed: Mutex<HashSet<(TabId, ViewerId)>>,
}

impl Shared {
    fn send(&self, frame: &MachineFrame) -> bool {
        let out = self.out.lock().unwrap().clone();
        out.is_some_and(|out| out.send(serde_json::to_string(frame).unwrap()).is_ok())
    }

    /// Forgets the connection that just ended: its callers fail, and its tabs stop streaming.
    fn disconnected(&self, machine: &Machine) {
        *self.out.lock().unwrap() = None;
        self.pending.lock().unwrap().clear();
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
        let id = self.shared.next_call.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::channel();
        self.shared.pending.lock().unwrap().insert(id, answer);
        if !self.shared.send(&MachineFrame::Call { id, call }) {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err("not connected to the hub".into());
        }
        match answered.recv_timeout(CALL_WAIT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.shared.pending.lock().unwrap().remove(&id);
                Err("the hub did not answer within 60 seconds".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("the connection to the hub dropped".into()),
        }
    }

    /// Passes a connector session's callback to the hub and answers what the session prints.
    /// Every other socket message is the machine's own business.
    pub fn forward(&self, message: Message) -> Result<String, String> {
        let call = match message {
            Message::Stage { connector, subject, stage } => HubCall::Stage { connector, subject, stage },
            Message::SlackReply { connector, channel, thread, text } => HubCall::SlackReply { connector, channel, thread, text },
            Message::LinearCreate { connector, channel, thread, title, description } => {
                HubCall::LinearCreate { connector, channel, thread, title, description }
            }
            _ => return Err("only a connector session's callbacks go to the hub".into()),
        };
        match self.call_hub(call)? {
            Value::String(text) => Ok(text),
            other => Ok(other.to_string()),
        }
    }

    /// Whether the hub accepted this machine on the current connection.
    pub fn online(&self) -> bool {
        self.shared.out.lock().unwrap().is_some()
    }

    /// Disconnects for good and waits for the link's thread to end.
    pub fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.thread.join();
    }
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
    let (socket, _) = tokio_tungstenite::connect_async(url.as_str()).await.map_err(|e| format!("could not connect to {url}: {e}"))?;
    let (mut sink, mut stream) = socket.split();
    let auth = MachineFrame::Auth {
        token: creds.machine_token.clone(),
        name: crate::account::hostname(),
        os: std::env::consts::OS.into(),
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

/// Handles one frame from the hub.
fn answer(shared: &Shared, machine: &Arc<Machine>, out: &channel::UnboundedSender<String>, calls: &mpsc::Sender<(u64, Call)>, frame: &str, version: &str) {
    match serde_json::from_str::<ToMachine>(frame) {
        // A run may clone a repository for minutes, so it doesn't hold up the calls behind it.
        Ok(ToMachine::Call { id, call: call @ Call::Run { .. } }) => {
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
        Err(e) => {
            // A hub newer than this machine may call something it doesn't know: say so by the call's id.
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

/// The machine socket of the hub at `hub`, such as `wss://pinned.example.com/ws/machine`.
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
}
