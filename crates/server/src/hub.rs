use crate::db::{self, Db};
use pinned_core::agent::Subject;
use pinned_core::connectors::{Connectors, Storage, Work};
use pinned_core::wire::{
    Call, HubCall, MachineEvent, MachineId, MachineView, PairCode, Reply, TabId, TabView, Target, ToClient, ToMachine, EXE_PLACEHOLDER,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, OnceCell};
use uuid::Uuid;

const MACHINE_CALL_WAIT: Duration = Duration::from_secs(300);
/// A run may clone a large repository before its session starts.
const RUN_WAIT: Duration = Duration::from_secs(30 * 60);

/// Why a call to a machine has no answer.
pub enum Unanswered {
    /// The machine answered with an error, or is offline: nothing happened there.
    Refused(String),
    /// The machine went offline or stopped answering after the call reached it, so it may have acted on it.
    Lost(String),
}

impl From<Unanswered> for String {
    fn from(failure: Unanswered) -> String {
        match failure {
            Unanswered::Refused(e) | Unanswered::Lost(e) => e,
        }
    }
}

/// Frames a socket holds for a peer that reads slower than they arrive, such as a phone on a busy tab.
const OUTBOX_FRAMES: usize = 1024;

/// A socket's outgoing frames. A peer that lets `OUTBOX_FRAMES` pile up is dropped rather than
/// letting the hub's memory grow; it reconnects and its terminals replay their screens.
#[derive(Clone)]
pub struct Outbox {
    frames: mpsc::Sender<String>,
    overflowed: Arc<tokio::sync::Notify>,
}

impl Outbox {
    /// The outbox, the socket's end of it, and what fires once the peer fell behind.
    pub fn new() -> (Outbox, mpsc::Receiver<String>, Arc<tokio::sync::Notify>) {
        let (frames, queue) = mpsc::channel(OUTBOX_FRAMES);
        let overflowed = Arc::new(tokio::sync::Notify::new());
        (Outbox { frames, overflowed: overflowed.clone() }, queue, overflowed)
    }

    pub fn send(&self, frame: String) -> Result<(), ()> {
        match self.frames.try_send(frame) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.overflowed.notify_one();
                Err(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(()),
        }
    }
}
pub type Sent = (String, oneshot::Receiver<Result<Value, String>>);

pub struct Hub {
    pub db: Db,
    pub public_url: String,
    accounts: Mutex<HashMap<Uuid, Arc<Account>>>,
    /// Numbers every socket, so a machine's old connection can't unregister its new one.
    sockets: AtomicU64,
}

pub struct Account {
    pub id: Uuid,
    pub email: String,
    live: Mutex<Live>,
    connectors: OnceCell<Arc<Connectors>>,
    watching: AtomicBool,
}

#[derive(Default)]
struct Live {
    machines: HashMap<MachineId, Online>,
    clients: HashMap<u64, Client>,
}

struct Online {
    socket: u64,
    name: String,
    out: Outbox,
    pending: HashMap<u64, oneshot::Sender<Result<Value, String>>>,
    next_call: u64,
    tabs: Vec<TabView>,
}

struct Client {
    out: Outbox,
    login: Vec<u8>,
    attached: HashSet<Target>,
}

fn viewer(client: u64) -> String {
    format!("client-{client}")
}

fn client_of(viewer: &str) -> Option<u64> {
    viewer.strip_prefix("client-")?.parse().ok()
}

fn frame<T: Serialize>(frame: &T) -> String {
    serde_json::to_string(frame).unwrap()
}

fn json<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

impl Hub {
    pub fn new(db: Db, public_url: String) -> Arc<Hub> {
        Arc::new(Hub { db, public_url: public_url.trim_end_matches('/').to_owned(), accounts: Mutex::default(), sockets: AtomicU64::new(1) })
    }

    pub fn next_socket(&self) -> u64 {
        self.sockets.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn watch_all(self: &Arc<Self>) -> Result<(), String> {
        for id in self.db.accounts_with_connectors().await? {
            let account = self.account(id).await?;
            self.watch(&account).await?;
        }
        Ok(())
    }

    pub async fn account(&self, id: Uuid) -> Result<Arc<Account>, String> {
        if let Some(account) = self.accounts.lock().unwrap().get(&id) {
            return Ok(account.clone());
        }
        let email = self.db.email(id).await?.ok_or("the account was deleted")?;
        let fresh = Arc::new(Account { id, email, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false) });
        Ok(self.accounts.lock().unwrap().entry(id).or_insert(fresh).clone())
    }

    async fn connectors(&self, account: &Account) -> Result<Arc<Connectors>, String> {
        let connectors = account
            .connectors
            .get_or_try_init(|| async {
                let stored = self.db.connectors(account.id).await?;
                let storage = PgStorage::new(self.db.clone(), account.id, stored);
                Ok::<_, String>(Arc::new(Connectors::new(storage, EXE_PLACEHOLDER.into())))
            })
            .await?;
        Ok(connectors.clone())
    }

    async fn watch(self: &Arc<Self>, account: &Arc<Account>) -> Result<(), String> {
        let connectors = self.connectors(account).await?;
        if account.watching.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let (hub, id) = (self.clone(), account.id);
        let checked = (self.clone(), account.clone());
        tokio::spawn(async move {
            let open = |work| {
                let hub = hub.clone();
                async move { hub.open(id, work).await }
            };
            let on_check = |_: &pinned_core::connectors::Check| {
                let (hub, account) = checked.clone();
                tokio::spawn(async move { hub.push_settings(&account).await });
            };
            connectors.watch(open, on_check).await
        });
        Ok(())
    }

    /// Hands a connector's work to a machine of the account, and answers whether one took it.
    /// A run goes to the connector's machine when it is online, else to the others, the most
    /// recently connected first. New thread messages go to whichever machine runs a session of
    /// that connector and has the thread's.
    pub async fn open(&self, account: Uuid, work: Work) -> bool {
        let Ok(account) = self.account(account).await else { return false };
        match work {
            Work::Run(run) => {
                if account.working_on(&run.connector, &run.subject, |_| true) {
                    return true;
                }
                let preferred = match self.connectors(&account).await {
                    Ok(c) => c.get_settings().connectors.into_iter().find(|c| c.id == run.connector).map(|c| c.machine).unwrap_or_default(),
                    Err(_) => String::new(),
                };
                for machine in account.dispatch_order(&preferred, |_| true) {
                    match self.call_machine(&account, &machine, Call::Run { run: run.spec() }, RUN_WAIT).await {
                        Ok(_) => return true,
                        Err(Unanswered::Refused(e)) => eprintln!("pinned-server: {machine} refused a run of connector {}: {e}", run.connector),
                        // The run may have started there, so it doesn't go to a second machine. The next
                        // check finds its tab, or sends it again, and a machine answers a run it has with that session.
                        Err(Unanswered::Lost(e)) => {
                            eprintln!("pinned-server: lost {machine}'s answer to a run of connector {}: {e}", run.connector);
                            return false;
                        }
                    }
                }
                false
            }
            Work::Delivery(d) => {
                let runs_connector = |tabs: &[TabView]| tabs.iter().any(|t| t.running && t.connector.as_deref() == Some(&d.connector));
                for machine in account.dispatch_order("", runs_connector) {
                    let call = Call::Deliver { env: d.env(), text: d.text() };
                    match self.call_machine(&account, &machine, call, MACHINE_CALL_WAIT).await {
                        Ok(Value::Bool(true)) => return true,
                        // The messages were likely typed; opening another session for the thread would answer it twice.
                        Err(Unanswered::Lost(_)) => return true,
                        _ => {}
                    }
                }
                false
            }
        }
    }

    pub async fn call_machine(&self, account: &Account, machine: &str, call: Call, wait: Duration) -> Result<Value, Unanswered> {
        let sent = account.send_call(machine, call);
        self.answer(account, machine, sent, wait).await
    }

    pub async fn client_call(&self, account: &Account, machine: &str, sent: Option<Sent>) -> Result<Value, String> {
        Ok(self.answer(account, machine, sent, MACHINE_CALL_WAIT).await?)
    }

    async fn answer(&self, account: &Account, machine: &str, sent: Option<Sent>, wait: Duration) -> Result<Value, Unanswered> {
        let Some((name, answered)) = sent else {
            let known = match Uuid::parse_str(machine) {
                Ok(id) => self.db.machine_name(account.id, id).await.map_err(Unanswered::Refused)?,
                Err(_) => None,
            };
            let error = known.map_or_else(|| format!("this account has no machine {machine}"), |name| format!("{name} is offline"));
            return Err(Unanswered::Refused(error));
        };
        match tokio::time::timeout(wait, answered).await {
            Ok(Ok(result)) => result.map_err(Unanswered::Refused),
            Ok(Err(_)) => Err(Unanswered::Lost(format!("{name} went offline"))),
            Err(_) => Err(Unanswered::Lost(format!("{name} did not answer"))),
        }
    }

    /// Answers an account-level call, from a client or (only a session's callbacks) a machine.
    pub async fn hub_call(self: &Arc<Self>, account: &Arc<Account>, call: HubCall) -> Result<Value, String> {
        let c = || self.connectors(account);
        let (answer, changed) = match call {
            HubCall::Machines => return json(self.machines(account).await?),
            HubCall::RenameMachine { id, name } => {
                let name = name.trim();
                if name.is_empty() {
                    return Err("a machine needs a name".into());
                }
                let machine = Uuid::parse_str(&id).map_err(|_| format!("this account has no machine {id}"))?;
                if !self.db.rename_machine(account.id, machine, name).await? {
                    return Err(format!("this account has no machine {id}"));
                }
                account.renamed(&id, name);
                self.push_machines(account).await;
                return Ok(Value::Null);
            }
            HubCall::RemoveMachine { id } => {
                let machine = Uuid::parse_str(&id).map_err(|_| format!("this account has no machine {id}"))?;
                if !self.db.remove_machine(account.id, machine).await? {
                    return Err(format!("this account has no machine {id}"));
                }
                account.went_offline(&id, None);
                self.push_machines(account).await;
                return Ok(Value::Null);
            }
            HubCall::PairCode => {
                let (code, expires_at) = self.db.create_pair_code(account.id).await?;
                let command = format!("curl -fsSL {}/install.sh | sh -s -- {code}", self.public_url);
                return json(PairCode { code, expires_at, command });
            }
            HubCall::GetSettings => (json(c().await?.get_settings()), false),
            HubCall::ConnectLinear { key } => (json(c().await?.connect_linear(&key).await?), true),
            HubCall::DisconnectLinear => (json(c().await?.disconnect_linear()?), true),
            HubCall::LinearSuggestions => (json(c().await?.linear_suggestions().await?), false),
            HubCall::ConnectSlack { token } => (json(c().await?.connect_slack(&token).await?), true),
            HubCall::DisconnectSlack => (json(c().await?.disconnect_slack()?), true),
            HubCall::SlackSuggestions => (json(c().await?.slack_suggestions().await?), false),
            HubCall::SaveConnector { connector } => (json(c().await?.save_connector(connector)?), true),
            HubCall::DeleteConnector { id } => (json(c().await?.delete_connector(&id)?), true),
            HubCall::Import { store, linear_key, slack_key } => {
                let imported = c().await?.import(store, linear_key, slack_key)?;
                (json(imported), imported)
            }
            HubCall::Stage { connector, subject, stage } => (json(c().await?.stage(&connector, &subject, stage).await?), false),
            HubCall::SlackReply { connector, channel, thread, text } => {
                (json(c().await?.slack_reply(&connector, &channel, &thread, &text).await?), false)
            }
            HubCall::LinearCreate { connector, channel, thread, title, description, assignee } => {
                (json(c().await?.linear_create(&connector, &channel, &thread, &title, &description, &assignee).await?), true)
            }
            HubCall::LinearTake { connector, channel, thread, issue } => {
                (json(c().await?.linear_take(&connector, &channel, &thread, &issue).await?), true)
            }
        };
        if changed {
            self.watch(account).await?;
            self.push_settings(account).await;
        }
        answer
    }

    pub async fn machines(&self, account: &Account) -> Result<Vec<MachineView>, String> {
        let mut machines = self.db.machines(account.id).await?;
        account.mark_online(&mut machines);
        Ok(machines)
    }

    pub async fn push_machines(&self, account: &Account) {
        match self.machines(account).await {
            Ok(machines) => account.broadcast(&ToClient::Machines { machines }),
            Err(e) => eprintln!("pinned-server: could not list account {}'s machines: {e}", account.id),
        }
    }

    async fn push_settings(&self, account: &Account) {
        if let Ok(connectors) = self.connectors(account).await {
            account.broadcast(&ToClient::Settings { settings: connectors.get_settings() });
        }
    }

    pub fn logged_out(&self, hash: &[u8]) {
        let accounts: Vec<Arc<Account>> = self.accounts.lock().unwrap().values().cloned().collect();
        for account in accounts {
            let signed_in: Vec<u64> = account.live.lock().unwrap().clients.iter().filter(|(_, c)| c.login == hash).map(|(id, _)| *id).collect();
            for client in signed_in {
                account.client_gone(client);
            }
        }
    }
}

impl Account {
    /// Registers a machine's connection. An older one must have gone offline first.
    pub fn came_online(&self, machine: &str, socket: u64, name: String, out: Outbox) {
        let online = Online { socket, name, out, pending: HashMap::new(), next_call: 1, tabs: vec![] };
        self.live.lock().unwrap().machines.insert(machine.to_owned(), online);
    }

    /// Forgets the machine's connection, `socket` or whichever it has: its calls fail and its
    /// tabs are detached, so clients attach again once it is back. Answers whether it was online.
    pub fn went_offline(&self, machine: &str, socket: Option<u64>) -> bool {
        let mut live = self.live.lock().unwrap();
        if socket.is_some_and(|s| live.machines.get(machine).is_some_and(|m| m.socket != s)) {
            return false;
        }
        let Some(gone) = live.machines.remove(machine) else { return false };
        for (_, waiting) in gone.pending {
            let _ = waiting.send(Err(format!("{} went offline", gone.name)));
        }
        for client in live.clients.values_mut() {
            client.attached.retain(|t| t.machine != machine);
        }
        true
    }

    fn renamed(&self, machine: &str, name: &str) {
        if let Some(m) = self.live.lock().unwrap().machines.get_mut(machine) {
            m.name = name.to_owned();
        }
    }

    fn mark_online(&self, machines: &mut [MachineView]) {
        let live = self.live.lock().unwrap();
        for m in machines {
            m.online = live.machines.contains_key(&m.id);
        }
    }

    /// Whether a session on an online machine that `on` picks works on the connector's `subject`.
    pub fn working_on(&self, connector: &str, subject: &Subject, on: impl Fn(&MachineId) -> bool) -> bool {
        let live = self.live.lock().unwrap();
        live.machines
            .iter()
            .filter(|(id, _)| on(id))
            .flat_map(|(_, m)| &m.tabs)
            .any(|t| t.running && t.connector.as_deref() == Some(connector) && t.subject.as_ref() == Some(subject))
    }

    fn dispatch_order(&self, preferred: &str, wanted: impl Fn(&[TabView]) -> bool) -> Vec<MachineId> {
        let live = self.live.lock().unwrap();
        let mut online: Vec<(&MachineId, &Online)> = live.machines.iter().filter(|(_, m)| wanted(&m.tabs)).collect();
        online.sort_by_key(|(id, m)| (id.as_str() != preferred, std::cmp::Reverse(m.socket)));
        online.into_iter().map(|(id, _)| id.clone()).collect()
    }

    /// Sends the call at once, so calls reach the machine in the order they came, and answers
    /// the machine's name and where its answer arrives, or None while it is offline.
    pub fn send_call(&self, machine: &str, call: Call) -> Option<Sent> {
        let mut live = self.live.lock().unwrap();
        let m = live.machines.get_mut(machine)?;
        let id = m.next_call;
        m.next_call += 1;
        // Calls whose callers stopped waiting, as after a timeout, would otherwise stay until the machine disconnects.
        m.pending.retain(|_, waiting| !waiting.is_closed());
        let (answer, answered) = oneshot::channel();
        m.pending.insert(id, answer);
        let _ = m.out.send(frame(&ToMachine::Call { id, call }));
        Some((m.name.clone(), answered))
    }

    pub fn answered(&self, machine: &str, socket: u64, reply: Reply) {
        let mut live = self.live.lock().unwrap();
        let Some(m) = live.machines.get_mut(machine).filter(|m| m.socket == socket) else { return };
        if let Some(waiting) = m.pending.remove(&reply.id) {
            let _ = waiting.send(reply.result());
        }
    }

    /// Passes a machine's event to the clients, keeping its tabs for those that connect later.
    pub fn machine_event(&self, machine: &str, socket: u64, event: MachineEvent) {
        let mut live = self.live.lock().unwrap();
        let Some(m) = live.machines.get_mut(machine).filter(|m| m.socket == socket) else { return };
        if let MachineEvent::Tabs { tabs } = &event {
            m.tabs = tabs.clone();
        }
        let text = frame(&ToClient::Event { machine: machine.to_owned(), event });
        for client in live.clients.values() {
            let _ = client.out.send(text.clone());
        }
    }

    pub fn output(&self, machine: &str, viewer: &str, tab: TabId, data: pinned_core::wire::Bytes) {
        let live = self.live.lock().unwrap();
        let Some(client) = client_of(viewer).and_then(|id| live.clients.get(&id)) else { return };
        let target = Target { machine: machine.to_owned(), tab };
        if client.attached.contains(&target) {
            let event = MachineEvent::Output { tab: target.tab, data };
            let _ = client.out.send(frame(&ToClient::Event { machine: target.machine, event }));
        }
    }

    pub fn to_machine(&self, machine: &str, socket: u64, message: &ToMachine) {
        let live = self.live.lock().unwrap();
        if let Some(m) = live.machines.get(machine).filter(|m| m.socket == socket) {
            let _ = m.out.send(frame(message));
        }
    }

    /// Registers a client, sending it `hello` first: built under the same lock that events take,
    /// so no event falls between the snapshot and the stream.
    pub fn client_connected(&self, id: u64, login: Vec<u8>, out: Outbox, mut machines: Vec<MachineView>) {
        let mut live = self.live.lock().unwrap();
        for m in &mut machines {
            m.online = live.machines.contains_key(&m.id);
        }
        let tabs: BTreeMap<MachineId, Vec<TabView>> = live.machines.iter().map(|(id, m)| (id.clone(), m.tabs.clone())).collect();
        let account = pinned_core::wire::AccountView { id: self.id.to_string(), email: self.email.clone() };
        let _ = out.send(frame(&ToClient::Hello { account, machines, tabs }));
        live.clients.insert(id, Client { out, login, attached: HashSet::new() });
    }

    pub fn client_gone(&self, id: u64) {
        let mut live = self.live.lock().unwrap();
        let Some(client) = live.clients.remove(&id) else { return };
        for target in client.attached {
            if let Some(m) = live.machines.get(&target.machine) {
                let _ = m.out.send(frame(&ToMachine::Unsubscribe { tab: target.tab, viewer: viewer(id) }));
            }
        }
    }

    pub fn to_client(&self, id: u64, message: &ToClient) {
        if let Some(client) = self.live.lock().unwrap().clients.get(&id) {
            let _ = client.out.send(frame(message));
        }
    }

    pub fn broadcast(&self, message: &ToClient) {
        let text = frame(message);
        for client in self.live.lock().unwrap().clients.values() {
            let _ = client.out.send(text.clone());
        }
    }

    /// Streams the tab to the client, its screen first. Attaching again replays the screen.
    pub fn attach(&self, client: u64, target: Target) {
        let mut live = self.live.lock().unwrap();
        let Live { machines, clients } = &mut *live;
        let (Some(m), Some(c)) = (machines.get(&target.machine), clients.get_mut(&client)) else { return };
        let _ = m.out.send(frame(&ToMachine::Subscribe { tab: target.tab.clone(), viewer: viewer(client) }));
        c.attached.insert(target);
    }

    pub fn detach(&self, client: u64, target: Target) {
        let mut live = self.live.lock().unwrap();
        let Live { machines, clients } = &mut *live;
        let Some(c) = clients.get_mut(&client) else { return };
        if c.attached.remove(&target) {
            if let Some(m) = machines.get(&target.machine) {
                let _ = m.out.send(frame(&ToMachine::Unsubscribe { tab: target.tab, viewer: viewer(client) }));
            }
        }
    }
}

/// Keeps an account's store and keys in memory and writes them to its row behind the
/// connectors' back, so `Storage`'s synchronous calls never wait on Postgres.
struct PgStorage {
    store: Mutex<Option<Vec<u8>>>,
    keys: Mutex<BTreeMap<String, String>>,
    writes: mpsc::UnboundedSender<Write>,
}

enum Write {
    Store(Value),
    Secrets(BTreeMap<String, String>),
}

impl PgStorage {
    fn new(db: Db, account: Uuid, stored: db::Connectors) -> PgStorage {
        let (writes, mut queue) = mpsc::unbounded_channel::<Write>();
        tokio::spawn(async move {
            while let Some(first) = queue.recv().await {
                // Only the newest of each kind matters, so a burst of saves is one write.
                let (mut store, mut secrets) = (None, None);
                let mut next = Some(first);
                while let Some(write) = next {
                    match write {
                        Write::Store(v) => store = Some(v),
                        Write::Secrets(v) => secrets = Some(v),
                    }
                    next = queue.try_recv().ok();
                }
                if let Some(store) = store {
                    if let Err(e) = db.save_store(account, &store).await {
                        eprintln!("pinned-server: could not save account {account}'s connectors: {e}");
                    }
                }
                if let Some(secrets) = secrets {
                    if let Err(e) = db.save_secrets(account, &secrets).await {
                        eprintln!("pinned-server: could not save account {account}'s integration keys: {e}");
                    }
                }
            }
        });
        let store = (stored.store != serde_json::json!({})).then(|| serde_json::to_vec(&stored.store).unwrap());
        PgStorage { store: Mutex::new(store), keys: Mutex::new(stored.secrets), writes }
    }

    fn write_keys(&self, keys: &BTreeMap<String, String>) {
        let _ = self.writes.send(Write::Secrets(keys.clone()));
    }
}

impl Storage for PgStorage {
    fn load(&self) -> Option<Vec<u8>> {
        self.store.lock().unwrap().clone()
    }

    fn save(&self, store: &[u8]) -> Result<(), String> {
        let value: Value = serde_json::from_slice(store).map_err(|e| e.to_string())?;
        *self.store.lock().unwrap() = Some(store.to_vec());
        self.writes.send(Write::Store(value)).map_err(|_| "the hub is shutting down".to_string())
    }

    fn key(&self, name: &str) -> Option<String> {
        self.keys.lock().unwrap().get(name).cloned().filter(|k| !k.is_empty())
    }

    fn write_key(&self, name: &str, key: &str) -> Result<(), String> {
        let mut keys = self.keys.lock().unwrap();
        keys.insert(name.to_owned(), key.to_owned());
        self.write_keys(&keys);
        Ok(())
    }

    fn remove_key(&self, name: &str) {
        let mut keys = self.keys.lock().unwrap();
        if keys.remove(name).is_some() {
            self.write_keys(&keys);
        }
    }
}
