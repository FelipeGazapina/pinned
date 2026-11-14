//! What the hub's sockets carried since the last census, by frame type, and the bytes sync pulls
//! read. A frame type is a static name of a wire variant; no frame's content is kept.
use pinned_core::wire::{ClientFrame, MachineEvent, MachineFrame, ToClient, ToMachine};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// A frame type's count, bytes and largest frame in the interval.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tally {
    pub count: u64,
    pub bytes: u64,
    pub max: u64,
}

impl Tally {
    fn add(&mut self, bytes: u64) {
        self.count += 1;
        self.bytes += bytes;
        self.max = self.max.max(bytes);
    }
}

static SENT: Mutex<Option<HashMap<&'static str, Tally>>> = Mutex::new(None);
static RECEIVED: Mutex<Option<HashMap<&'static str, Tally>>> = Mutex::new(None);
static SYNC_PULLS: AtomicU64 = AtomicU64::new(0);
static SYNC_PULL_BYTES: AtomicU64 = AtomicU64::new(0);

fn count(tallies: &Mutex<Option<HashMap<&'static str, Tally>>>, label: &'static str, bytes: usize) {
    tallies.lock().unwrap().get_or_insert_with(HashMap::new).entry(label).or_default().add(bytes as u64);
}

/// A frame the hub queued for a peer.
pub fn sent(label: &'static str, bytes: usize) {
    count(&SENT, label, bytes);
}

/// A frame a peer sent the hub.
pub fn received(label: &'static str, bytes: usize) {
    count(&RECEIVED, label, bytes);
}

/// A sync pull that read `bytes` of file contents.
pub fn sync_pulled(bytes: usize) {
    SYNC_PULLS.fetch_add(1, Ordering::Relaxed);
    SYNC_PULL_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

/// The interval's counts, which start again from zero.
pub struct Interval {
    pub sent: Vec<(&'static str, Tally)>,
    pub received: Vec<(&'static str, Tally)>,
    pub sync_pulls: u64,
    pub sync_pull_bytes: u64,
}

pub fn take() -> Interval {
    let sorted = |tallies: &Mutex<Option<HashMap<&'static str, Tally>>>| {
        let mut list: Vec<_> = tallies.lock().unwrap().take().unwrap_or_default().into_iter().collect();
        list.sort_by_key(|(label, _)| *label);
        list
    };
    Interval { sent: sorted(&SENT), received: sorted(&RECEIVED), sync_pulls: SYNC_PULLS.swap(0, Ordering::Relaxed), sync_pull_bytes: SYNC_PULL_BYTES.swap(0, Ordering::Relaxed) }
}

impl Interval {
    /// ` sent_bytes=… sent_max=… sent_<type>_n=… sent_<type>_bytes=… sent_<type>_max=…`, the same
    /// for `recv`, and the sync pulls, for the census line.
    pub fn fields(&self) -> String {
        let mut line = String::new();
        for (side, tallies) in [("sent", &self.sent), ("recv", &self.received)] {
            let all = tallies.iter().fold(Tally::default(), |all, (_, t)| Tally { count: all.count + t.count, bytes: all.bytes + t.bytes, max: all.max.max(t.max) });
            line.push_str(&format!(" {side}_n={} {side}_bytes={} {side}_max={}", all.count, all.bytes, all.max));
            for (label, t) in tallies {
                line.push_str(&format!(" {side}_{label}_n={} {side}_{label}_bytes={} {side}_{label}_max={}", t.count, t.bytes, t.max));
            }
        }
        line.push_str(&format!(" sync_pulls={} sync_pull_bytes={}", self.sync_pulls, self.sync_pull_bytes));
        line
    }
}

/// A frame's type for the census.
pub trait Labeled {
    fn label(&self) -> &'static str;
}

/// An event's type, as the hub sends it to a client or a machine sends it to the hub.
fn event_label(event: &MachineEvent, to_client: bool) -> &'static str {
    let (client, machine) = match event {
        MachineEvent::Tabs { .. } => ("client_event_tabs", "machine_event_tabs"),
        MachineEvent::Agent { .. } => ("client_event_agent", "machine_event_agent"),
        MachineEvent::Output { .. } => ("client_event_output", "machine_event_output"),
        MachineEvent::Exit { .. } => ("client_event_exit", "machine_event_exit"),
        MachineEvent::Usage { .. } => ("client_event_usage", "machine_event_usage"),
        MachineEvent::Thread { .. } => ("client_event_thread", "machine_event_thread"),
        MachineEvent::Update { .. } => ("client_event_update", "machine_event_update"),
        MachineEvent::SkillsChanged { .. } => ("client_event_skills_changed", "machine_event_skills_changed"),
        MachineEvent::TastesChanged => ("client_event_tastes_changed", "machine_event_tastes_changed"),
        MachineEvent::State { .. } => ("client_event_state", "machine_event_state"),
        MachineEvent::Background { .. } => ("client_event_background", "machine_event_background"),
    };
    if to_client {
        client
    } else {
        machine
    }
}

impl Labeled for ToClient {
    fn label(&self) -> &'static str {
        match self {
            ToClient::Hello { .. } => "client_hello",
            ToClient::Reply(_) => "client_reply",
            ToClient::Machines { .. } => "client_machines",
            ToClient::Event { event, .. } => event_label(event, true),
            ToClient::Settings { .. } => "client_settings",
            ToClient::SessionMarks { .. } => "client_session_marks",
            ToClient::Confirmations { .. } => "client_confirmations",
        }
    }
}

impl Labeled for ToMachine {
    fn label(&self) -> &'static str {
        match self {
            ToMachine::Hello { .. } => "machine_hello",
            ToMachine::Call { .. } => "machine_call",
            ToMachine::Reply(_) => "machine_reply",
            ToMachine::Subscribe { .. } => "machine_subscribe",
            ToMachine::Unsubscribe { .. } => "machine_unsubscribe",
            ToMachine::WatchThreads { .. } => "machine_watch_threads",
        }
    }
}

impl Labeled for ClientFrame {
    fn label(&self) -> &'static str {
        match self {
            ClientFrame::Auth { .. } => "client_auth",
            ClientFrame::Call { .. } => "client_call",
            ClientFrame::Hub { .. } => "client_hub",
            ClientFrame::Attach(_) => "client_attach",
            ClientFrame::Detach(_) => "client_detach",
        }
    }
}

impl Labeled for MachineFrame {
    fn label(&self) -> &'static str {
        match self {
            MachineFrame::Auth { .. } => "machine_auth",
            MachineFrame::Reply(_) => "machine_reply",
            MachineFrame::Event { event } => event_label(event, false),
            MachineFrame::Output { .. } => "machine_output",
            MachineFrame::Call { .. } => "machine_call",
        }
    }
}

/// A frame read from a peer whose text does not parse.
pub const UNREADABLE: &str = "unreadable";
