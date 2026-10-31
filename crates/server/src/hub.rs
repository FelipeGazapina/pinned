use crate::boxes::{self, BoxApi, Boxes, Seen};
use crate::db::{self, Db};
use pinned_core::accounts::AccountId;
use pinned_core::agent::{AgentStatus, Stage, Subject, Usage, CONNECTOR_ENV};
use pinned_core::connectors::{Connectors, Delivery, Keys, Placement, Run, RunsOn, Storage, Work};
use pinned_core::upstash;
use base64::Engine;
use pinned_core::{github, sync};
use pinned_core::wire::{
    Call, HubCall, MachineEvent, MachineId, MachineView, PairCode, Reply, TabId, TabView, Target, ToClient, ToMachine, UpdateStatus, EXE_PLACEHOLDER,
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
pub(crate) const RUN_WAIT: Duration = Duration::from_secs(30 * 60);

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
    pub boxes: Boxes,
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
    updates: Mutex<HashMap<MachineId, UpdateStatus>>,
}

#[derive(Default)]
struct Live {
    machines: HashMap<MachineId, Online>,
    clients: HashMap<u64, Client>,
}

struct Pending {
    answer: oneshot::Sender<Result<Value, String>>,
    update: bool,
}

struct Online {
    socket: u64,
    name: String,
    out: Outbox,
    pending: HashMap<u64, Pending>,
    next_call: u64,
    tabs: Vec<TabView>,
    /// The last usage the machine reported for each account, for clients that connect after it.
    usage: BTreeMap<AccountId, Usage>,
    /// It runs in one of the account's boxes, so no other run goes to it.
    dedicated: bool,
    version: String,
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
    pub fn new(db: Db, public_url: String, box_api: Arc<dyn BoxApi>, box_policy: boxes::Policy) -> Arc<Hub> {
        let public_url = public_url.trim_end_matches('/').to_owned();
        let boxes = Boxes::new(box_api, box_policy, &public_url);
        Arc::new(Hub { db, public_url, boxes, accounts: Mutex::default(), sockets: AtomicU64::new(1) })
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
        let fresh = Arc::new(Account { id, email, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default() });
        Ok(self.accounts.lock().unwrap().entry(id).or_insert(fresh).clone())
    }

    pub(crate) async fn connectors(&self, account: &Account) -> Result<Arc<Connectors>, String> {
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
                async move { hub.open_placed(id, work).await }
            };
            let on_check = |_: &pinned_core::connectors::Check| {
                let (hub, account) = checked.clone();
                tokio::spawn(async move { hub.push_settings(&account).await });
            };
            connectors.watch(open, on_check).await
        });
        let (hub, account) = (self.clone(), account.clone());
        tokio::spawn(async move {
            loop {
                hub.boxes.reap(&hub, &account).await;
                tokio::time::sleep(hub.boxes.policy.tick).await;
            }
        });
        Ok(())
    }

    /// Hands a connector's work to a machine of the account, and answers whether one took it, or
    /// why none did. A box run goes to its own box. Any other run goes to the connector's machine
    /// when it is online, else to the others, the most recently connected first, never to a box.
    /// New thread messages go to whichever machine runs a session of that connector and has the thread's.
    pub async fn open(&self, account: Uuid, work: Work) -> Result<(), String> {
        self.open_placed(account, work).await.map(drop)
    }

    /// Hands `work` to a machine and returns the tab that took a run, or none for a delivery.
    pub async fn open_placed(&self, account: Uuid, work: Work) -> Result<Option<Placement>, String> {
        let account = self.account(account).await?;
        match work {
            Work::Run(run) => {
                if let Some((machine, tab)) = account.placed(&run.connector, &run.subject) {
                    return Ok(Some(Placement { machine, tab }));
                }
                match run.runs_on.clone() {
                    RunsOn::FreshBox(step) => self.boxes.open(self, &account, run, step).await.map(Some),
                    RunsOn::Any => self.open_on_pool(&account, run, "").await.map(Some),
                    RunsOn::Machine(id) => self.open_on_pool(&account, run, &id).await.map(Some),
                }
            }
            Work::Delivery(d) => self.deliver(&account, d).await.map(|()| None),
            Work::Prompt { env, text } => self.type_prompt(&account, &env, &text).await.map(|()| None),
        }
    }

    async fn open_on_pool(&self, account: &Account, run: Run, preferred: &str) -> Result<Placement, String> {
        let token = self.connectors(account).await.ok().and_then(|c| c.github_token());
        let mut spec = run.spec();
        spec.token = token.filter(|_| spec.repo.as_deref().is_some_and(github::hosts));
        let mut refused = vec![];
        for machine in account.run_pool(preferred) {
            match self.call_machine(account, &machine, Call::Run { run: spec.clone() }, RUN_WAIT).await {
                Ok(value) => {
                    let tab = value.as_str().map(str::to_owned).ok_or_else(|| "the machine started the session without a tab id".to_owned())?;
                    return Ok(Placement { machine, tab });
                }
                Err(Unanswered::Refused(e)) => {
                    eprintln!("pinned-server: {machine} refused a run of workflow {}: {e}", run.connector);
                    refused.push(e);
                }
                // The run may have started there, so it doesn't go to a second machine. The next
                // check finds its tab, or sends it again, and a machine answers a run it has with that session.
                Err(Unanswered::Lost(e)) => {
                    eprintln!("pinned-server: lost {machine}'s answer to a run of workflow {}: {e}", run.connector);
                    return Err(e);
                }
            }
        }
        match refused.is_empty() {
            true => Err("no machine of the account is online".into()),
            false => Err(refused.join("; ")),
        }
    }

    async fn deliver(&self, account: &Account, d: Delivery) -> Result<(), String> {
        for machine in account.running(&d.connector) {
            let call = Call::Deliver { env: d.env(), text: d.text() };
            match self.call_machine(account, &machine, call, MACHINE_CALL_WAIT).await {
                Ok(Value::Bool(true)) => return Ok(()),
                // The messages were likely typed; opening another session for the thread would answer it twice.
                Err(Unanswered::Lost(_)) => return Ok(()),
                _ => {}
            }
        }
        Err("no session answers that thread".into())
    }

    async fn type_prompt(&self, account: &Account, env: &[(String, String)], text: &str) -> Result<(), String> {
        let connector = env.iter().find(|(key, _)| key == CONNECTOR_ENV).map(|(_, value)| value.as_str()).unwrap_or("");
        for machine in account.running(connector) {
            let call = Call::Deliver { env: env.to_vec(), text: text.to_owned() };
            match prompt_outcome(self.call_machine(account, &machine, call, MACHINE_CALL_WAIT).await) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(why) => return Err(why),
            }
        }
        Err("no session answers that thread".into())
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
                if self.boxes.end_for_machine(self, account, machine).await? {
                    return Ok(Value::Null);
                }
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
            HubCall::Import { store, linear_key, slack_key, github_key, vercel_key } => {
                let imported = c().await?.import(store, Keys { linear: linear_key, slack: slack_key, github: github_key, vercel: vercel_key })?;
                (json(imported), imported)
            }
            HubCall::SharedProjects => return json(self.db.shared_projects(account.id).await?),
            HubCall::ShareProject { project } => return json(self.db.share_project(account.id, &project).await?),
            HubCall::SyncList { project } => return json(self.db.sync_list(account.id, &project).await?),
            HubCall::SyncPush { project, files } => {
                let mut kept = vec![];
                for file in files {
                    if !sync::accepts(&project, &file.entry.path) {
                        return Err(format!("{} is not a file a project shares", file.entry.path));
                    }
                    let data = match &file.data {
                        Some(data) => base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?,
                        None => vec![],
                    };
                    if data.len() as u64 > sync::FILE_LIMIT {
                        return Err(format!("{} is bigger than a synced file may be", file.entry.path));
                    }
                    kept.push((file.entry, data));
                }
                return json(self.db.sync_push(account.id, &project, kept).await?);
            }
            HubCall::SyncPull { project, paths } => {
                let files = self.db.sync_pull(account.id, &project, &paths).await?;
                let files: Vec<sync::File> = files
                    .into_iter()
                    .map(|(entry, data)| {
                        let data = (!entry.sha.is_empty()).then(|| base64::engine::general_purpose::STANDARD.encode(data));
                        sync::File { entry, data }
                    })
                    .collect();
                return json(files);
            }
            HubCall::ConnectGithub { token } => (json(c().await?.connect_github(&token).await?), true),
            HubCall::DisconnectGithub => (json(c().await?.disconnect_github()?), true),
            HubCall::GithubRepos => (json(c().await?.github_repos().await?), false),
            HubCall::ConnectVercel { key } => (json(c().await?.connect_vercel(&key).await?), true),
            HubCall::DisconnectVercel => (json(c().await?.disconnect_vercel()?), true),
            HubCall::VercelModels => (json(c().await?.vercel_models().await?), false),
            HubCall::ConnectUpstash { key, claude_token } => {
                let keys = upstash::Keys::parse(&key, &claude_token)?;
                self.boxes.check_key(&keys.api_key).await?;
                (json(c().await?.connect_upstash(&keys)?), true)
            }
            HubCall::DisconnectUpstash => {
                self.boxes.end_all(self, account).await?;
                (json(c().await?.disconnect_upstash()?), true)
            }
            HubCall::SetBoxAccess { access } => (json(c().await?.set_box_access(access)?), true),
            HubCall::AnswerBox { id, allow } => (json(c().await?.answer_box(&id, allow)?), true),
            HubCall::Clone { machine, repo, dest } => {
                let repo = github::clone_url(&repo);
                let token = c().await?.github_token().filter(|_| github::hosts(&repo));
                return Ok(self.call_machine(account, &machine, Call::Clone { repo, token, dest }, RUN_WAIT).await?);
            }
            HubCall::Stage { connector, subject, stage } => {
                if stage == Stage::Review {
                    self.boxes.work_done(self, account, &connector, &subject).await?;
                }
                (json(c().await?.stage(&connector, &subject, stage).await?), false)
            }
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
        account.overlay(&mut machines);
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
    pub fn came_online(&self, machine: &str, socket: u64, name: String, dedicated: bool, version: String, out: Outbox) {
        let online = Online { socket, name, out, pending: HashMap::new(), next_call: 1, tabs: vec![], usage: BTreeMap::new(), dedicated, version };
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
            let _ = waiting.answer.send(Err(format!("{} went offline", gone.name)));
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

    fn overlay(&self, machines: &mut [MachineView]) {
        let live = self.live.lock().unwrap();
        let updates = self.updates.lock().unwrap();
        apply_overlay(&live, &updates, machines);
    }

    pub fn dedicated(&self, machine: &str) -> bool {
        self.live.lock().unwrap().machines.get(machine).is_some_and(|online| online.dedicated)
    }

    pub fn note_update(&self, machine: &str, snapshot: Option<UpdateStatus>) {
        let version = self.live.lock().unwrap().machines.get(machine).map(|online| online.version.clone()).unwrap_or_default();
        let mut updates = self.updates.lock().unwrap();
        match reconcile(updates.get(machine).cloned(), &version, snapshot) {
            Some(status) => {
                updates.insert(machine.to_owned(), status);
            }
            None => {
                updates.remove(machine);
            }
        }
    }

    /// The online machine and tab already working on the connector's `subject`.
    pub fn placed(&self, connector: &str, subject: &Subject) -> Option<(MachineId, TabId)> {
        let live = self.live.lock().unwrap();
        live.machines.iter().find_map(|(id, machine)| {
            machine
                .tabs
                .iter()
                .find(|tab| tab.running && tab.connector.as_deref() == Some(connector) && tab.subject.as_ref() == Some(subject))
                .map(|tab| (id.clone(), tab.id.clone()))
        })
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

    /// Machines a workflow run may go to: online, never a box's, `preferred` first, then the most recently connected.
    fn run_pool(&self, preferred: &str) -> Vec<MachineId> {
        let live = self.live.lock().unwrap();
        let mut online: Vec<(&MachineId, &Online)> = live.machines.iter().filter(|(_, m)| !m.dedicated).collect();
        online.sort_by_key(|(id, m)| (id.as_str() != preferred, std::cmp::Reverse(m.socket)));
        online.into_iter().map(|(id, _)| id.clone()).collect()
    }

    /// Machines with a running session of `connector`, boxes' included, the most recently connected first.
    fn running(&self, connector: &str) -> Vec<MachineId> {
        let live = self.live.lock().unwrap();
        let mut online: Vec<(&MachineId, &Online)> =
            live.machines.iter().filter(|(_, m)| m.tabs.iter().any(|t| t.running && t.connector.as_deref() == Some(connector))).collect();
        online.sort_by_key(|(_, m)| std::cmp::Reverse(m.socket));
        online.into_iter().map(|(id, _)| id.clone()).collect()
    }

    pub fn is_online(&self, machine: &str) -> bool {
        self.live.lock().unwrap().machines.contains_key(machine)
    }

    /// What a box's machine shows of the session of `connector` on `subject`.
    pub fn seen(&self, machine: &str, connector: &str, subject: &Subject) -> Seen {
        let live = self.live.lock().unwrap();
        let Some(m) = live.machines.get(machine) else { return Seen::Offline };
        match m.tabs.iter().find(|t| t.connector.as_deref() == Some(connector) && t.subject.as_ref() == Some(subject)) {
            None => Seen::NoTab,
            // Claude reports a finished turn as waiting for input a minute later, so only Working holds a box.
            Some(t) => Seen::Tab { running: t.running, quiet: t.status != Some(AgentStatus::Working) },
        }
    }

    /// Sends the call at once, so calls reach the machine in the order they came, and answers
    /// the machine's name and where its answer arrives, or None while it is offline.
    pub fn send_call(&self, machine: &str, call: Call) -> Option<Sent> {
        let mut live = self.live.lock().unwrap();
        let m = live.machines.get_mut(machine)?;
        let id = m.next_call;
        m.next_call += 1;
        // Calls whose callers stopped waiting, as after a timeout, would otherwise stay until the machine disconnects.
        m.pending.retain(|_, waiting| !waiting.answer.is_closed());
        let (answer, answered) = oneshot::channel();
        let update = matches!(call, Call::UpdateSelf);
        m.pending.insert(id, Pending { answer, update });
        let _ = m.out.send(frame(&ToMachine::Call { id, call }));
        Some((m.name.clone(), answered))
    }

    pub fn answered(&self, machine: &str, socket: u64, reply: Reply) -> bool {
        let status = {
            let mut live = self.live.lock().unwrap();
            let Some(m) = live.machines.get_mut(machine).filter(|m| m.socket == socket) else { return false };
            let Some(pending) = m.pending.remove(&reply.id) else { return false };
            let result = reply.result();
            let status = pending.update.then(|| status_of_update(&result)).flatten();
            let _ = pending.answer.send(result);
            status
        };
        let Some(status) = status else { return false };
        self.note_update(machine, Some(status));
        true
    }

    /// Passes a machine's event to the clients, keeping its tabs and usage for those that connect later.
    pub fn machine_event(&self, machine: &str, socket: u64, event: MachineEvent) {
        let mut live = self.live.lock().unwrap();
        let Some(m) = live.machines.get_mut(machine).filter(|m| m.socket == socket) else { return };
        match &event {
            MachineEvent::Tabs { tabs } => m.tabs = tabs.clone(),
            MachineEvent::Usage { usage, account } => {
                m.usage.insert(account.clone(), *usage);
            }
            _ => {}
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
        {
            let updates = self.updates.lock().unwrap();
            apply_overlay(&live, &updates, &mut machines);
        }
        let tabs: BTreeMap<MachineId, Vec<TabView>> = live.machines.iter().map(|(id, m)| (id.clone(), m.tabs.clone())).collect();
        let account = pinned_core::wire::AccountView { id: self.id.to_string(), email: self.email.clone() };
        let _ = out.send(frame(&ToClient::Hello { account, machines, tabs }));
        for (id, m) in &live.machines {
            for (account, usage) in &m.usage {
                let event = MachineEvent::Usage { usage: *usage, account: account.clone() };
                let _ = out.send(frame(&ToClient::Event { machine: id.clone(), event }));
            }
        }
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

/// One moment of the account document. Store and runs share a row, so they share this lock.
struct Stored {
    store: Option<Vec<u8>>,
    runs: Vec<u8>,
}

/// Keeps an account's store and keys in memory and writes them to its row behind the
/// connectors' back, so `Storage`'s synchronous calls never wait on Postgres.
struct PgStorage {
    state: Mutex<Stored>,
    keys: Mutex<BTreeMap<String, String>>,
    writes: mpsc::UnboundedSender<Write>,
}

fn apply_overlay(live: &Live, updates: &HashMap<MachineId, UpdateStatus>, machines: &mut [MachineView]) {
    for machine in machines {
        machine.online = live.machines.contains_key(&machine.id);
        machine.update = updates.get(&machine.id).cloned();
        if let Some(version) = live.machines.get(&machine.id).map(|online| online.version.clone()).filter(|version| !version.is_empty()) {
            machine.version = version;
        }
    }
}

fn status_of_update(result: &Result<Value, String>) -> Option<UpdateStatus> {
    match result {
        Ok(value) => serde_json::from_value(value.clone()).ok(),
        Err(text) if pinned_core::link::is_unknown_command(text) => Some(UpdateStatus::Cannot { reason: "update by hand".into() }),
        Err(_) => None,
    }
}

/// An idle connect after Updating or Restarting settles against the version Auth just wrote.
/// A snapshot from the running process replaces whatever the hub remembered.
pub fn reconcile(previous: Option<UpdateStatus>, version: &str, snapshot: Option<UpdateStatus>) -> Option<UpdateStatus> {
    if snapshot.is_some() {
        return snapshot;
    }
    match previous {
        Some(UpdateStatus::Updating { to }) | Some(UpdateStatus::Restarting { to }) if !to.is_empty() => {
            if crate::self_update::same_release(version, &to) {
                Some(UpdateStatus::UpToDate { version: version.to_owned() })
            } else {
                Some(UpdateStatus::Failed { to, reason: format!("came back on {version}") })
            }
        }
        other => other,
    }
}

/// A graph prompt treats a lost machine as not delivered. The cursor stays and the next check retries.
fn prompt_outcome(answer: Result<Value, Unanswered>) -> Result<bool, String> {
    match answer {
        Ok(Value::Bool(true)) => Ok(true),
        Err(Unanswered::Lost(why)) => Err(why),
        _ => Ok(false),
    }
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
                        eprintln!("pinned-server: could not save account {account}'s workflows: {e}");
                    }
                }
                if let Some(secrets) = secrets {
                    if let Err(e) = db.save_secrets(account, &secrets).await {
                        eprintln!("pinned-server: could not save account {account}'s connector keys: {e}");
                    }
                }
            }
        });
        let mut doc = stored.store;
        let runs = doc.as_object_mut().and_then(|map| map.remove("runs")).and_then(|value| serde_json::to_vec(&value).ok()).unwrap_or_default();
        let store = (doc != serde_json::json!({})).then(|| serde_json::to_vec(&doc).unwrap());
        PgStorage {
            state: Mutex::new(Stored { store, runs }),
            keys: Mutex::new(stored.secrets),
            writes,
        }
    }

    /// Queues the store and the runs as one document. Callers hold the `state` lock, so the
    /// writer receives the documents in the order the state changed and the newest one wins.
    fn persist(&self, state: &Stored) -> Result<(), String> {
        let (store, runs) = (&state.store, &state.runs);
        let mut doc: Value = match store {
            Some(bytes) => serde_json::from_slice(bytes).map_err(|e| e.to_string())?,
            None => serde_json::json!({}),
        };
        let object = doc.as_object_mut().ok_or("the workflow store is not an object")?;
        if runs.is_empty() {
            object.remove("runs");
        } else {
            object.insert("runs".to_owned(), serde_json::from_slice(runs).map_err(|e| e.to_string())?);
        }
        self.writes.send(Write::Store(doc)).map_err(|_| "the hub is shutting down".to_string())
    }

    fn write_keys(&self, keys: &BTreeMap<String, String>) {
        let _ = self.writes.send(Write::Secrets(keys.clone()));
    }
}

impl Storage for PgStorage {
    fn load(&self) -> Option<Vec<u8>> {
        self.state.lock().unwrap().store.clone()
    }

    fn save(&self, store: &[u8]) -> Result<(), String> {
        serde_json::from_slice::<Value>(store).map_err(|e| e.to_string())?;
        let mut state = self.state.lock().unwrap();
        state.store = Some(store.to_vec());
        self.persist(&state)
    }

    fn load_runs(&self) -> Option<Vec<u8>> {
        let runs = self.state.lock().unwrap().runs.clone();
        (!runs.is_empty()).then_some(runs)
    }

    fn save_runs(&self, bytes: &[u8]) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.runs = bytes.to_vec();
        self.persist(&state)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_prompt_is_not_delivered() {
        assert_eq!(prompt_outcome(Err(Unanswered::Lost("gone".into()))), Err("gone".into()));
        assert_eq!(prompt_outcome(Ok(serde_json::Value::Bool(true))), Ok(true));
        assert_eq!(prompt_outcome(Ok(serde_json::Value::Bool(false))), Ok(false));
        assert_eq!(prompt_outcome(Err(Unanswered::Refused("no".into()))), Ok(false));
    }

    #[test]
    fn a_live_snapshot_replaces_the_stored_phase() {
        let live = UpdateStatus::Updating { to: "2.0.0".into() };
        assert_eq!(reconcile(Some(UpdateStatus::Restarting { to: "1.0.0".into() }), "0.9.0", Some(live.clone())), Some(live));
    }

    #[test]
    fn an_idle_connect_treats_a_v_prefix_as_the_same_release() {
        assert_eq!(
            reconcile(Some(UpdateStatus::Restarting { to: "v1.2.0".into() }), "1.2.0", None),
            Some(UpdateStatus::UpToDate { version: "1.2.0".into() }),
        );
        assert_eq!(
            reconcile(Some(UpdateStatus::Updating { to: "1.2.0".into() }), "v1.2.0", None),
            Some(UpdateStatus::UpToDate { version: "v1.2.0".into() }),
        );
    }

    #[test]
    fn an_idle_connect_on_the_target_version_is_up_to_date() {
        assert_eq!(
            reconcile(Some(UpdateStatus::Updating { to: "1.2.0".into() }), "1.2.0", None),
            Some(UpdateStatus::UpToDate { version: "1.2.0".into() }),
        );
        assert_eq!(
            reconcile(Some(UpdateStatus::Restarting { to: "1.2.0".into() }), "1.2.0", None),
            Some(UpdateStatus::UpToDate { version: "1.2.0".into() }),
        );
    }

    #[test]
    fn an_idle_connect_on_another_version_failed() {
        assert_eq!(
            reconcile(Some(UpdateStatus::Restarting { to: "1.2.0".into() }), "1.1.0", None),
            Some(UpdateStatus::Failed { to: "1.2.0".into(), reason: "came back on 1.1.0".into() }),
        );
        assert_eq!(
            reconcile(Some(UpdateStatus::Updating { to: "1.2.0".into() }), "1.0.0", None),
            Some(UpdateStatus::Failed { to: "1.2.0".into(), reason: "came back on 1.0.0".into() }),
        );
    }

    #[test]
    fn a_finished_phase_stays_when_the_connect_is_idle() {
        let failed = UpdateStatus::Failed { to: "1.2.0".into(), reason: "checksum mismatch".into() };
        assert_eq!(reconcile(Some(failed.clone()), "1.2.0", None), Some(failed));
        let current = UpdateStatus::UpToDate { version: "1.2.0".into() };
        assert_eq!(reconcile(Some(current.clone()), "0.1.0", None), Some(current));
        let cannot = UpdateStatus::Cannot { reason: "update by hand".into() };
        assert_eq!(reconcile(Some(cannot.clone()), "1.2.0", None), Some(cannot));
        let empty = UpdateStatus::Updating { to: String::new() };
        assert_eq!(reconcile(Some(empty.clone()), "1.2.0", None), Some(empty));
        assert_eq!(reconcile(None, "1.2.0", None), None);
    }
}
