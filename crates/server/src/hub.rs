use crate::boxes::{self, BoxApi, Boxes, Seen};
use crate::db::{self, Db};
use pinned_core::account::Signup;
use pinned_core::accounts::AccountId;
use pinned_core::agent::{AgentStatus, Stage, Subject, Usage, CONNECTOR_ENV};
use pinned_core::schedule::{self, Actor, SaveAnswer, Task};
use pinned_core::connectors::{Connectors, Delivery, Keys, Placement, Run, RunsOn, Storage, Work};
use pinned_core::convex::client::Http as ConvexHttp;
use pinned_core::ideas::{Idea, IdeaStatus};
use pinned_core::delegate::{RunTarget, TaskRecord};
use pinned_core::marks::{self, Archived, Mark, MarkChange, SessionMarks};
use pinned_core::taste::{TasteField, TastePatch, TasteScope, Tastes};
use pinned_core::wire::BOX_USD_PER_HOUR;
use pinned_core::upstash;
use base64::Engine;
use pinned_core::{github, sync};
use pinned_core::wire::{
    Call, HubCall, Invites, MachineEvent, MachineId, MachineView, PairCode, ProjectView, Reply, RunSpec, RunStarted, TabId, TabView, Target, ToClient, ToMachine, UpdateStatus, EXE_PLACEHOLDER,
};
use pinned_core::surface::{self, Side, Unlocked};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, OnceCell};
use uuid::Uuid;

const MACHINE_CALL_WAIT: Duration = Duration::from_secs(300);
/// Longer than a machine's `claude setup-token` waits for the user's approval in the browser.
const MINT_WAIT: Duration = Duration::from_secs(pinned_core::setup_token::WAIT.as_secs() + 30);
/// How long importing an MCP server waits for the machine to read its account's config.
const MCP_IMPORT_WAIT: Duration = Duration::from_secs(20);
const MAX_RECORDS_PER_CALL: usize = 500;
/// How long the password the app took counts as the owner being there.
const UNLOCK_FOR: Duration = Duration::from_secs(12 * 60 * 60);
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

/// The call that starts `spec` on a machine. A Run now goes as `RunNow`, which the machine holds
/// to the limits of the project it runs in.
pub(crate) fn run_call(spec: RunSpec, subject: &Subject) -> Call {
    if pinned_core::connectors::is_run_now(subject) {
        let title = pinned_core::connectors::run_now_title(&spec);
        Call::RunNow { run: spec, title }
    } else {
        Call::Run { run: spec }
    }
}

/// A machine whose pinned predates Run now can't read it: say so, instead of its parse error.
pub(crate) fn too_old_for_run_now(account: &Account, machine: &str, subject: &Subject, refused: String) -> String {
    if !pinned_core::connectors::is_run_now(subject) || !refused.contains("can't answer that") {
        return refused;
    }
    let name = account.name_of(machine).unwrap_or_else(|| machine.to_owned());
    format!("{name} runs an older pinned that has no Run now: update it in Settings › Machines")
}

pub struct Hub {
    pub db: Db,
    pub public_url: String,
    pub signup: Signup,
    pub boxes: Boxes,
    /// Every Convex read and key check goes through it.
    pub convex_http: Arc<dyn ConvexHttp>,
    accounts: Mutex<HashMap<Uuid, Arc<Account>>>,
    /// Numbers every socket, so a machine's old connection can't unregister its new one.
    sockets: AtomicU64,
    /// Serializes external agents' work on one project's orchestrator.
    agent_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The unlocks apps hold in memory, by the sha256 of the token. A hub restart forgets them, and
    /// the app asks for the password again.
    unlocks: Mutex<HashMap<Vec<u8>, Unlock>>,
    /// Turns true when the hub is about to stop, so every socket sends its close frame.
    going_away: tokio::sync::watch::Sender<bool>,
}

struct Unlock {
    /// The sha256 of the login it was taken on, the only one it works for.
    login: Vec<u8>,
    until: Instant,
}

pub struct Account {
    pub id: Uuid,
    pub email: String,
    live: Mutex<Live>,
    connectors: OnceCell<Arc<Connectors>>,
    watching: AtomicBool,
    updates: Mutex<HashMap<MachineId, UpdateStatus>>,
    /// Each machine's projects as it last listed them, so the global orchestrator's overview still
    /// shows them while it is offline or slow to answer.
    projects: Mutex<HashMap<MachineId, Vec<ProjectView>>>,
    /// Held across a taste's read, change and write, so two changes at once both land.
    tastes: tokio::sync::Mutex<()>,
    /// Held across a marks write and its push, so every client hears the changes in the order they
    /// were kept.
    marks: tokio::sync::Mutex<()>,
    /// Held while task records are billed and saved, so two sends from one box never bill the
    /// same box time twice.
    task_records: tokio::sync::Mutex<()>,
    /// When the global orchestrator's last repo reads came, for their per-minute and per-hour limit.
    pub repo_reads: Mutex<crate::relay::ReadRate>,
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
    tabs: Option<Vec<TabView>>,
    /// The last usage the machine reported for each account, for clients that connect after it.
    usage: BTreeMap<AccountId, Usage>,
    /// It runs in one of the account's boxes, so no other run goes to it.
    dedicated: bool,
    version: String,
    /// What the machine last reported about its load and accounts. It goes with the connection, so
    /// an offline machine shows none.
    state: Option<pinned_core::load::MachineState>,
    /// Whether its desktop app runs in the background, as it last reported. A node reports none.
    background: Option<bool>,
}

impl Online {
    fn tabs(&self) -> &[TabView] {
        self.tabs.as_deref().unwrap_or(&[])
    }
}

struct Client {
    out: Outbox,
    login: Vec<u8>,
    /// Its app asks for the password before a call only the owner makes.
    locked: bool,
    /// Until when the password the app took counts for this connection.
    unlocked: Option<Instant>,
    attached: HashSet<Target>,
    threads: HashMap<MachineId, HashSet<TabId>>,
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
    pub fn new(db: Db, public_url: String, signup: Signup, box_api: Arc<dyn BoxApi>, box_policy: boxes::Policy, convex_http: Arc<dyn ConvexHttp>) -> Arc<Hub> {
        let public_url = public_url.trim_end_matches('/').to_owned();
        let boxes = Boxes::new(box_api, box_policy, &public_url);
        Arc::new(Hub { db, public_url, signup, boxes, convex_http, accounts: Mutex::default(), sockets: AtomicU64::new(1), agent_locks: Mutex::default(), unlocks: Mutex::default(), going_away: tokio::sync::watch::channel(false).0 })
    }

    /// Tells every socket to close with a going away frame. The hub stops serving afterwards.
    pub fn going_away(&self) {
        self.going_away.send_replace(true);
    }

    /// Turns true once `going_away` ran.
    pub fn watch_going_away(&self) -> tokio::sync::watch::Receiver<bool> {
        self.going_away.subscribe()
    }

    /// Waits until no one else reaches the Capitão of the project at `path` on `machine`, the hub
    /// MCP's `message_orchestrator` and the global orchestrator's relay alike. Two messages at once
    /// would both see it stopped and start it twice. Locks nobody holds are dropped as others are
    /// taken, so the map stays small.
    pub async fn hold_captain(&self, account: &Account, machine: &str, path: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.hold(format!("{}:{machine}:{path}", account.id)).await
    }

    /// Held while the worker for task `task` of thread `tab` on `machine` starts elsewhere, so a
    /// repeat of that start waits for the first and finds its worker instead of making another.
    pub async fn hold_start(&self, account: &Account, machine: &str, tab: &str, task: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.hold(format!("start:{}:{machine}:{tab}:{task}", account.id)).await
    }

    async fn hold(&self, key: String) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.agent_locks.lock().unwrap();
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
            locks.entry(key).or_default().clone()
        };
        lock.lock_owned().await
    }

    /// Unlocks client `client` of `account` for `UNLOCK_FOR`, once it gave the account's password,
    /// and answers the token its app sends again after a reconnect.
    pub fn unlock(&self, account: &Account, client: u64) -> Option<Unlocked> {
        let token = db::Token::new();
        let now = Instant::now();
        let until = now + UNLOCK_FOR;
        let login = account.unlocked(client, until)?;
        let mut unlocks = self.unlocks.lock().unwrap();
        unlocks.retain(|_, unlock| unlock.until > now);
        unlocks.insert(token.hash, Unlock { login, until });
        let ends = SystemTime::now() + UNLOCK_FOR;
        Some(Unlocked { token: token.text, until: ends.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) })
    }

    /// When the unlock `token` ends, if it was taken on login `login` and has not ended yet.
    pub fn unlocked_until(&self, token: &str, login: &[u8]) -> Option<Instant> {
        let unlocks = self.unlocks.lock().unwrap();
        unlocks.get(&db::hash(token)).filter(|unlock| unlock.login == login && unlock.until > Instant::now()).map(|unlock| unlock.until)
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
        let fresh = Arc::new(Account { id, email, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() });
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

    /// The user's Run now of workflow `id`: it goes where a triggered run of it would go, to its
    /// machine, any machine, or its Upstash Box, and each machine holds it to its project's limits.
    pub async fn run_workflow(&self, account: &Account, id: &str) -> Result<RunStarted, String> {
        let run = self.connectors(account).await?.run_now(id, "the account's owner, from Settings › Workflows")?;
        let placed = self.open_placed(account.id, Work::Run(run)).await?.ok_or("no machine started the run")?;
        Ok(account.started(id, placed))
    }

    pub(crate) async fn run_spec(&self, account: &Account, run: &Run) -> RunSpec {
        let token = self.connectors(account).await.ok().and_then(|c| c.github_token());
        let mut spec = run.spec();
        spec.token = token.filter(|_| spec.repo.as_deref().is_some_and(github::hosts));
        spec
    }

    async fn open_on_pool(&self, account: &Account, run: Run, preferred: &str) -> Result<Placement, String> {
        let spec = self.run_spec(account, &run).await;
        let mut refused: Vec<String> = vec![];
        for machine in account.run_pool(preferred) {
            match self.call_machine(account, &machine, run_call(spec.clone(), &run.subject), RUN_WAIT).await {
                Ok(value) => {
                    let tab = value.as_str().map(str::to_owned).ok_or_else(|| "the machine started the session without a tab id".to_owned())?;
                    return Ok(Placement { machine, tab });
                }
                Err(Unanswered::Refused(e)) => {
                    eprintln!("pinned-server: {machine} refused a run of workflow {}: {e}", run.connector);
                    let e = too_old_for_run_now(account, &machine, &run.subject, e);
                    // Every machine reads the same tastes, so a limit refuses alike on each.
                    if !refused.contains(&e) {
                        refused.push(e);
                    }
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

    /// A machine whose pinned predates Use for boxes can't read the call: say so, instead of its parse error.
    fn too_old_to_mint(&self, account: &Account, machine: &str, refused: Unanswered) -> String {
        let refused = String::from(refused);
        if !pinned_core::link::is_unknown_command(&refused) {
            return refused;
        }
        let name = account.name_of(machine).unwrap_or_else(|| machine.to_owned());
        format!("{name} runs an older pinned that can't mint a Claude token for boxes: update it in Settings › Machines")
    }

    /// A Convex tool's ask from the session in `tab` on `machine`, decided from the tabs the
    /// account's machines reported and the repo key `machine` names for that tab.
    pub async fn convex(&self, account: &Arc<Account>, machine: &str, tab: &str, repo: Option<&str>, ask: pinned_core::convex::Ask) -> Result<Value, String> {
        let find = |on: &str, id: &str| account.tab(on, id);
        self.connectors(account).await?.convex(&*self.convex_http, machine, tab, repo, &find, ask).await
    }

    /// Answers an account-level call, from a client or (only a session's callbacks) a machine.
    pub async fn hub_call(self: &Arc<Self>, account: &Arc<Account>, call: HubCall) -> Result<Value, String> {
        let c = || self.connectors(account);
        let (answer, changed) = match call {
            HubCall::Machines | HubCall::MachineStates => return json(self.machines(account).await?),
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
            HubCall::CreateInvite => {
                if self.signup != Signup::Invite {
                    return Err("this hub takes no invites: anyone may sign up, or only its first account".into());
                }
                return json(self.db.create_invite(account.id).await?);
            }
            HubCall::Invites => return json(Invites { signup: self.signup, pending: self.db.invites(account.id).await? }),
            HubCall::RevokeInvite { id } => {
                let unknown = || format!("this account has no unused invite {id}");
                let invite = Uuid::parse_str(&id).map_err(|_| unknown())?;
                if !self.db.revoke_invite(account.id, invite).await? {
                    return Err(unknown());
                }
                return Ok(Value::Null);
            }
            HubCall::GetSettings => return json(self.settings(account).await?),
            HubCall::ConnectLinear { key } => (json(c().await?.connect_linear(&key).await?), true),
            HubCall::DisconnectLinear => (json(c().await?.disconnect_linear()?), true),
            HubCall::LinearSuggestions => (json(c().await?.linear_suggestions().await?), false),
            HubCall::ConnectSlack { token } => (json(c().await?.connect_slack(&token).await?), true),
            HubCall::DisconnectSlack { bot } => (json(c().await?.disconnect_slack(&bot)?), true),
            HubCall::SlackSuggestions { bot } => (json(c().await?.slack_suggestions(&bot).await?), false),
            HubCall::SaveConnector { connector } => (json(c().await?.save_connector(connector)?), true),
            HubCall::DeleteConnector { id } => (json(c().await?.delete_connector(&id)?), true),
            call @ (HubCall::ListSchedules | HubCall::SaveSchedule { .. } | HubCall::PauseSchedule { .. } | HubCall::DeleteSchedule { .. } | HubCall::ScheduleRuns { .. }) => return self.schedule_call(account, call, Actor::User).await,
            HubCall::SetReleaseProtection { on } => {
                self.db.set_release_protection(account.id, on).await?;
                return json(on);
            }
            HubCall::RunWorkflow { id } => return json(self.run_workflow(account, &id).await?),
            HubCall::GenerateWorkflow { .. } => return Err("Generate with AI was removed: ask an agent to create the workflow".into()),
            HubCall::OpenBox { repo, size, budget_usd } => (json(self.boxes.open_session(self, account, repo, size, budget_usd, None).await?), false),
            HubCall::BoxStatus { machine } => {
                let machine = Uuid::parse_str(&machine).map_err(|_| format!("this account has no machine {machine}"))?;
                (json(self.boxes.box_status(self, account, machine).await?), false)
            }
            HubCall::ExtendBox { machine, extra_usd } => {
                let machine = Uuid::parse_str(&machine).map_err(|_| format!("this account has no machine {machine}"))?;
                (json(self.boxes.extend_box(self, account, machine, extra_usd).await?), false)
            }
            HubCall::Import { store, linear_key, slack_key, slack_keys, github_key, vercel_key } => {
                let keys = Keys { linear: linear_key, slack: slack_key, slack_bots: slack_keys, github: github_key, vercel: vercel_key };
                let imported = c().await?.import(store, keys)?;
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
            HubCall::SyncMarkExecutable { project, repairs } => {
                if project != sync::PLUGINS_KEY {
                    return Err("only a plugin file carries an executable bit".into());
                }
                for repair in &repairs {
                    if !sync::accepts(&project, &repair.path) {
                        return Err(format!("{} is not a file a project shares", repair.path));
                    }
                }
                return json(self.db.sync_mark_executable(account.id, &project, &repairs).await?);
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
            HubCall::SetBoxClaudeToken { name, token } => (json(c().await?.set_box_claude_token(&name, &token)?), true),
            HubCall::RemoveBoxClaudeToken { name } => (json(c().await?.remove_box_claude_token(&name)?), true),
            HubCall::MintBoxClaudeToken { machine, account: name } => {
                let connectors = c().await?;
                if connectors.box_keys().is_none() {
                    return Err("connect Upstash Box first".into());
                }
                let minted = self.call_machine(account, &machine, Call::SetupToken { account: name.clone() }, MINT_WAIT).await.map_err(|e| self.too_old_to_mint(account, &machine, e))?;
                let token = minted.as_str().ok_or("the machine answered no token")?;
                (json(connectors.set_box_claude_token(&name, token)?), true)
            }
            HubCall::CancelBoxClaudeToken { machine, account: name } => {
                return self.call_machine(account, &machine, Call::CancelSetupToken { account: name }, MACHINE_CALL_WAIT).await.map_err(|e| self.too_old_to_mint(account, &machine, e));
            }
            HubCall::Delegate { .. } | HubCall::RemoteTask { .. } | HubCall::RemoteEnded { .. } | HubCall::RemoteAsked { .. } | HubCall::Place { .. } => {
                return Err("only a thread's machine delegates a task, asks after it or reports its end".into());
            }
            HubCall::SetBoxAccess { access } => (json(c().await?.set_box_access(access)?), true),
            HubCall::SetRepoInsight { settings } => (json(c().await?.set_repo_insight(settings)?), true),
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
            HubCall::RunSet { connector, subject, name, value } => (json(c().await?.run_set(&connector, &subject, &name, &value)?), false),
            HubCall::ExternalAgents => return json(crate::oauth::agents(self, account).await?),
            HubCall::AgentRequest { request } => return json(crate::oauth::request(self, &request).await?),
            HubCall::ApproveAgent { request, scopes, projects } => return json(crate::oauth::approve(self, account, &request, scopes, projects).await?),
            HubCall::DenyAgent { request } => return json(crate::oauth::deny(self, &request).await?),
            HubCall::RevokeAgent { id } => {
                crate::oauth::revoke_grant(self, account, &id).await?;
                return Ok(Value::Null);
            }
            call @ (HubCall::Relay { .. } | HubCall::Overview | HubCall::ReadCaptain { .. } | HubCall::ReadSession { .. } | HubCall::RepoLook { .. }) => return crate::relay::serve(self, account, crate::relay::Source::Client, call).await,
            HubCall::GetTastes => return json(self.db.tastes(account.id).await?.unwrap_or_default()),
            HubCall::SetTaste { scope, patch } => return self.change_tastes(account, |tastes| tastes.set(&scope, patch)).await,
            HubCall::ResetTasteField { scope, field } => {
                let reset = |tastes: &mut Tastes| {
                    tastes.reset(&scope, field);
                    Ok(())
                };
                return self.change_tastes(account, reset).await;
            }
            HubCall::GetBoxSize { repo, size } => return json(crate::boxes::box_size(self, account, repo.as_deref(), size).await?),
            HubCall::SetBoxSize { repo, size } => {
                let scope = match repo {
                    Some(repo) => TasteScope::Project { repo: crate::boxes::project_key(&repo).ok_or_else(|| format!("{repo} names no repository: use owner/repo or its clone URL"))? },
                    None => TasteScope::Captains,
                };
                let set = |tastes: &mut Tastes| match size {
                    Some(size) => tastes.set(&scope, TastePatch { box_size: Some(size), ..TastePatch::default() }),
                    None => {
                        tastes.reset(&scope, TasteField::BoxSize);
                        Ok(())
                    }
                };
                return self.change_tastes(account, set).await;
            }
            HubCall::ImportTastes { tastes } => {
                tastes.check()?;
                let _held = account.tastes.lock().await;
                return json(self.db.import_tastes(account.id, &tastes).await?);
            }
            HubCall::RecordTasks { .. } => return Err("only a machine records the tasks it ran".into()),
            HubCall::DelegationStats { days } => {
                let days = pinned_core::delegate::stats_days(Some(u64::from(days)))?;
                let now = pinned_core::connectors::unix_millis();
                let since = now.saturating_sub(u64::from(days) * 24 * 60 * 60 * 1000);
                let (records, unreadable) = self.db.task_records(account.id, since).await?;
                let mut stats = pinned_core::delegate::aggregate(&records, now, days);
                stats.unreadable = unreadable;
                return json(stats);
            }
            HubCall::BoxUsage => return json(boxes::usage(self, account).await?),
            HubCall::SessionMarks => return json(self.db.session_marks(account.id).await?),
            HubCall::MarkSessions { changes } => {
                marks::check(&changes)?;
                let _held = account.marks.lock().await;
                return json(self.write_marks(account, changes).await?);
            }
            HubCall::ArchiveSession { machine, tab } => return json(self.archive_session(account, &machine, &tab).await?),
            HubCall::AutoArchive { .. } => return Err("only a machine archives its own finished threads".into()),
            HubCall::UnarchiveSession { session } => return json(self.unarchive_session(account, &session, true).await?),
            HubCall::DeleteSession { machine, tab } => {
                self.delete_session(account, &machine, &tab).await?;
                return Ok(Value::Null);
            }
            HubCall::Act { .. } => return Err("only an agent session on a machine acts".into()),
            HubCall::Confirmations => return json(self.db.confirmations(account.id).await?),
            HubCall::DecideConfirmation { id, approve, entered } => return crate::surface::decide(self, account, &id, approve, entered).await,
            HubCall::Unlock { .. } => return Err("only the app unlocks, on its own connection".into()),
            HubCall::AgentAudit { before, limit } => {
                let limit = i64::from(limit.unwrap_or(50).clamp(1, 200));
                return json(self.db.audit_rows(account.id, before, limit).await?);
            }
            HubCall::ListIdeas { project } => return json(self.db.ideas(account.id, project.as_deref().map(str::trim).filter(|p| !p.is_empty())).await?),
            HubCall::SaveIdea { id, title, text, project, status } => return json(self.save_idea(account, Actor::User, id.as_deref(), &title, &text, project.as_deref(), status).await?),
            HubCall::DeleteIdea { id } => {
                let unknown = || format!("this account has no idea {id}");
                if !self.db.delete_idea(account.id, Uuid::parse_str(&id).map_err(|_| unknown())?).await? {
                    return Err(unknown());
                }
                self.push_settings(account).await;
                return Ok(Value::Null);
            }
            HubCall::ListMcpServers => return json(c().await?.mcp_servers()),
            HubCall::SaveMcpServer { id, server, secrets } => return json(c().await?.save_mcp_server(id.as_deref(), server, &secrets)?),
            HubCall::EnableMcpServer { id, enabled } => return json(c().await?.enable_mcp_server(&id, enabled)?),
            HubCall::RemoveMcpServer { id } => {
                c().await?.remove_mcp_server(&id)?;
                return Ok(Value::Null);
            }
            HubCall::ImportMcpServer { machine, account: agent_account, name, path, project, secrets } => {
                let (target, machine_name) = crate::remote::resolve(self, account, &machine).await?;
                let read = Call::McpImportable { account: Some(agent_account.clone()), path };
                let found = self.call_machine(account, &target, read, MCP_IMPORT_WAIT).await.map_err(String::from)?;
                let found: Vec<pinned_core::mcp::Found> = serde_json::from_value(found).map_err(|e| format!("{machine_name} answered servers that did not read: {e}"))?;
                let found = found.into_iter().find(|f| f.server.name == name).ok_or_else(|| format!("Claude Code account {agent_account} on {machine_name} has no MCP server {name}"))?;
                let server = pinned_core::mcp::Spec { project, ..found.server };
                return json(c().await?.save_mcp_server(None, server, &secrets)?);
            }
            HubCall::LoadMcpServers => return Err("only a machine loads the account's MCP servers into the sessions it starts".into()),
            HubCall::ConnectConvex { key, repos } => (json(c().await?.connect_convex(&*self.convex_http, &key, repos).await?), true),
            HubCall::SetConvexRepos { id, repos } => (json(c().await?.set_convex_repos(&id, repos)?), true),
            HubCall::DisconnectConvex { id } => (json(c().await?.disconnect_convex(&id)?), true),
            HubCall::ConvexTables { id } => (json(c().await?.convex_tables(&*self.convex_http, &id).await?), false),
            HubCall::Convex { .. } => return Err("only a session on a machine reads Convex".into()),
        };
        if changed {
            self.watch(account).await?;
            self.push_settings(account).await;
        }
        answer
    }

    /// Keeps what `machine` sent about the tasks it ran. A record from a machine that runs in one of
    /// the account's boxes is box work: it is billed the box time since the box's previous task
    /// ended (the first task, since the box was made), at the box rate. A record sent again keeps
    /// the box time it was billed the first time.
    pub async fn record_tasks(&self, account: &Account, machine: &str, mut records: Vec<TaskRecord>) -> Result<(), String> {
        if records.len() > MAX_RECORDS_PER_CALL {
            return Err(format!("send at most {MAX_RECORDS_PER_CALL} task records at once"));
        }
        let id = Uuid::parse_str(machine).map_err(|_| format!("this account has no machine {machine}"))?;
        let name = self.db.machine_name(account.id, id).await?.ok_or_else(|| format!("this account has no machine {machine}"))?;
        let _held = account.task_records.lock().await;
        let boxed = self.db.box_created_ms(account.id, id).await?;
        // Each task is billed the box time since the one that ended before it.
        records.sort_by_key(|record| record.ended);
        for mut record in records {
            record.machine = machine.to_owned();
            record.machine_name = name.clone();
            let billed = self.db.task_record(account.id, id, &record.id).await?.filter(|old| old.usage.box_ms.is_some());
            let box_ms = match (billed, boxed) {
                (Some(old), _) => old.usage.box_ms,
                (None, Some(created)) => {
                    let before = self.db.billed_box_ms(account.id, id, &record.id).await?;
                    Some(record.ended.saturating_sub(created).saturating_sub(before))
                }
                (None, None) => None,
            };
            if let Some(ms) = box_ms {
                record.target = RunTarget::UpstashBox;
                record.usage.box_ms = Some(ms);
                record.usage.box_usd = Some(ms as f64 / 3_600_000.0 * BOX_USD_PER_HOUR);
            }
            self.db.save_task_record(account.id, id, &record).await?;
        }
        Ok(())
    }

    /// Keeps `changes` and tells the account's clients. The caller holds `account.marks`.
    async fn write_marks(&self, account: &Account, changes: Vec<MarkChange>) -> Result<Vec<SessionMarks>, String> {
        let touched = self.db.mark_sessions(account.id, changes).await?;
        if !touched.is_empty() {
            account.broadcast(&ToClient::SessionMarks { marks: touched.clone() });
        }
        Ok(touched)
    }

    fn open_tab(account: &Account, machine: &str, tab: &str) -> Result<(TabView, String), String> {
        let view = account.tab(machine, tab).ok_or_else(|| format!("this account has no tab {tab} on machine {machine}"))?;
        let session = view.session.clone().ok_or_else(|| format!("tab {tab} has no conversation yet"))?;
        Ok((view, session))
    }

    /// Archives as the app does: the thread goes to the archive with its name and pin, then its tab
    /// closes. A tab that does not close gets its marks back.
    async fn archive_session(&self, account: &Account, machine: &str, tab: &str) -> Result<SessionMarks, String> {
        let (view, session) = Self::open_tab(account, machine, tab)?;
        if view.status == Some(AgentStatus::Working) {
            return Err("the thread is working; stop its turn first".into());
        }
        let (was, row) = {
            let _held = account.marks.lock().await;
            let was = self.db.session_marks_of(account.id, &session).await?;
            let archived = Archived {
                machine: machine.into(),
                name: was.title.clone().or(view.title).or(view.prompt_title).unwrap_or_else(|| "New thread".into()),
                project: project_name(&account.last_listed(machine), &view.cwd),
                cwd: view.cwd,
                at: pinned_core::connectors::unix_millis() as f64 / 1000.0,
                created: None,
                title: was.title.clone(),
                pinned: was.pin.is_some(),
            };
            let mark = |mark| MarkChange { session: session.clone(), mark };
            let row = self.write_marks(account, vec![mark(Mark::Clear), mark(Mark::Archived(Some(archived)))]).await?.remove(0);
            (was, row)
        };
        if let Err(e) = self.call_machine(account, machine, Call::CloseTab { tab: tab.into() }, MACHINE_CALL_WAIT).await {
            let _held = account.marks.lock().await;
            self.write_marks(account, was.rebuild()).await?;
            return Err(format!("the thread's tab did not close: {}", String::from(e)));
        }
        Ok(row)
    }

    /// Archives the thread of `tab` for `machine`, which found it finished (`auto_archive::keeps`).
    /// The hub knows what the machine can't: a pinned thread, an orchestrator and a working one stay.
    pub(crate) async fn auto_archive(&self, account: &Account, machine: &str, tab: &str) -> Result<SessionMarks, String> {
        let (view, session) = Self::open_tab(account, machine, tab)?;
        let marks = self.db.session_marks_of(account.id, &session).await?;
        if let Some(keeps) = pinned_core::auto_archive::hub_keeps(&view, &marks) {
            return Err(format!("{tab} stays open: {keeps}"));
        }
        self.archive_session(account, machine, tab).await
    }

    /// Reopens an archived conversation in a new tab with its name and pin back. The user's Unarchive
    /// resumes it at once; with `start` false, as for an agent, the tab stays unstarted: starting it
    /// is `start_claude`'s, which holds it to the agent's limits.
    pub(crate) async fn unarchive_session(&self, account: &Account, session: &str, start: bool) -> Result<Target, String> {
        let mark = |mark| MarkChange { session: session.into(), mark };
        let (was, archived) = {
            let _held = account.marks.lock().await;
            let was = self.db.session_marks_of(account.id, session).await?;
            let archived = was.archived.clone().ok_or_else(|| format!("this account has no archived conversation {session}"))?;
            if !account.is_online(&archived.machine) {
                return Err("the conversation's machine is offline".into());
            }
            // Off the Archive page before the resume starts, so a second Unarchive can't open it twice.
            self.write_marks(account, vec![mark(Mark::Archived(None))]).await?;
            (was, archived)
        };
        let run = RunSpec { folder: archived.cwd, repo: None, args: vec!["--resume".into(), session.into()], env: vec![], token: None, account: None, budget_usd: None };
        let call = if start { Call::Run { run } } else { Call::OpenRun { run } };
        let started = self.call_machine(account, &archived.machine, call, RUN_WAIT).await.map_err(String::from);
        let _held = account.marks.lock().await;
        let tab: TabId = match started.and_then(|tab| serde_json::from_value(tab).map_err(|e| e.to_string())) {
            Ok(tab) => tab,
            Err(e) => {
                self.write_marks(account, was.rebuild()).await?;
                return Err(e);
            }
        };
        let pin = if archived.pinned { Some(self.db.session_marks(account.id).await?.iter().filter_map(|row| row.pin).max().map_or(0, |last| last + 1)) } else { None };
        self.write_marks(account, vec![mark(Mark::Clear), mark(Mark::Title(archived.title)), mark(Mark::Pin(pin))]).await?;
        Ok(Target { machine: archived.machine, tab })
    }

    async fn delete_session(&self, account: &Account, machine: &str, tab: &str) -> Result<(), String> {
        let (_, session) = Self::open_tab(account, machine, tab)?;
        self.call_machine(account, machine, Call::CloseTab { tab: tab.into() }, MACHINE_CALL_WAIT).await?;
        let _held = account.marks.lock().await;
        self.write_marks(account, vec![MarkChange { session, mark: Mark::Clear }]).await?;
        Ok(())
    }

    async fn change_tastes(&self, account: &Account, change: impl FnOnce(&mut Tastes) -> Result<(), String>) -> Result<Value, String> {
        let _held = account.tastes.lock().await;
        let mut tastes = self.db.tastes(account.id).await?.unwrap_or_default();
        change(&mut tastes)?;
        self.db.save_tastes(account.id, &tastes).await?;
        json(tastes)
    }

    pub async fn machines(&self, account: &Account) -> Result<Vec<MachineView>, String> {
        let mut machines = self.db.machines(account.id).await?;
        account.overlay(&mut machines);
        Ok(machines)
    }

    pub async fn push_machines(&self, account: &Account) {
        match self.machines(account).await {
            Ok(machines) => account.publish_machines(machines),
            Err(e) => eprintln!("pinned-server: could not list account {}'s machines: {e}", account.id),
        }
    }

    /// The account's settings with its ideas and scheduled tasks, which live in the hub's own tables. A failed ideas
    /// read leaves them out rather than failing the settings every app loads.
    async fn settings(&self, account: &Account) -> Result<pinned_core::connectors::Settings, String> {
        let mut settings = self.connectors(account).await?.get_settings();
        settings.ideas = self.db.ideas(account.id, None).await.unwrap_or_else(|e| {
            eprintln!("pinned-server: could not read the ideas of account {}: {e}", account.id);
            Vec::new()
        });
        settings.schedules = self.db.tasks(account.id).await?;
        Ok(settings)
    }

    async fn push_settings(&self, account: &Account) {
        if let Ok(settings) = self.settings(account).await {
            account.broadcast(&ToClient::Settings { settings });
        }
    }

    /// The schedule calls, for `actor`, who is recorded as a new task's creator and held to the caps
    /// that apply to agents. Tasks are stored and edited here; nothing fires them yet.
    pub async fn schedule_call(self: &Arc<Self>, account: &Arc<Account>, call: HubCall, actor: Actor) -> Result<Value, String> {
        let unknown = |id: &str| format!("this account has no task {id}");
        let parse = task_key;
        match call {
            HubCall::ListSchedules => json(self.db.tasks(account.id).await?),
            HubCall::ScheduleRuns { id, limit } => {
                let key = parse(&id)?;
                self.db.task(account.id, key).await?.ok_or_else(|| unknown(&id))?;
                json(self.db.task_runs(account.id, key, i64::from(limit.unwrap_or(20).clamp(1, 200))).await?)
            }
            HubCall::DeleteSchedule { id } => {
                if !self.db.delete_task(account.id, parse(&id)?).await? {
                    return Err(unknown(&id));
                }
                self.push_settings(account).await;
                Ok(Value::Null)
            }
            HubCall::PauseSchedule { id, paused } => {
                let mut task = self.db.task(account.id, parse(&id)?).await?.ok_or_else(|| unknown(&id))?;
                task.enabled = !paused;
                task.updated_at = schedule::now();
                task.next_at = if task.enabled { schedule::next_after(&task.schedule, &task.tz, task.updated_at)? } else { None };
                self.db.save_task(account.id, &task).await?;
                self.push_settings(account).await;
                json(task)
            }
            HubCall::SaveSchedule { task, dry_run } => {
                let now = schedule::now();
                let protected = self.db.release_protection(account.id).await?;
                let existing = match task.id.trim() {
                    "" => None,
                    id => self.db.task(account.id, parse(id)?).await?,
                };
                let agent = matches!(actor, Actor::Agent { .. });
                if agent && existing.is_none() && !task.id.trim().is_empty() {
                    return Err(format!("this account has no task {}: leave id empty to create one", task.id));
                }
                let mut task = Task {
                    id: existing.as_ref().map_or_else(|| if task.id.trim().is_empty() { Uuid::new_v4().to_string() } else { task.id.trim().to_owned() }, |e| e.id.clone()),
                    created_by: existing.as_ref().map_or_else(|| actor.clone(), |e| e.created_by.clone()),
                    created_at: existing.as_ref().map_or(now, |e| e.created_at),
                    updated_at: now,
                    next_at: None,
                    last_run: None,
                    fyi: existing.as_ref().and_then(|e| e.fyi),
                    ..task
                };
                // Changing what a task does or where it goes takes its approvals away: the user grants them again.
                if let Some(before) = &existing {
                    if (before.action != task.action || before.target != task.target) && before.approvals == task.approvals {
                        task.approvals.clear();
                    }
                }
                schedule::check_save(&task, protected)?;
                if task.enabled {
                    task.next_at = schedule::next_after(&task.schedule, &task.tz, now)?;
                    if task.next_at.is_none() {
                        return Err("that time has already passed: pick a time that is still ahead".into());
                    }
                }
                let next = schedule::next_run_times(&task.schedule, &task.tz, now)?;
                if agent && existing.is_none() {
                    let held = self.db.tasks(account.id).await?.into_iter().filter(|t| matches!(t.created_by, Actor::Agent { .. })).collect::<Vec<_>>();
                    if held.len() >= schedule::AGENT_MAX_TASKS {
                        let titles: Vec<&str> = held.iter().map(|t| t.title.as_str()).collect();
                        return Err(format!("agents may hold at most {} tasks and {} exist: delete one first ({})", schedule::AGENT_MAX_TASKS, held.len(), titles.join(", ")));
                    }
                    // The notice rule: a task an agent creates is active at once, and the user sees an Inbox row.
                    task.fyi = Some(now);
                } else if !agent {
                    task.fyi = None;
                }
                if dry_run {
                    return json(SaveAnswer { task: None, next });
                }
                if !self.db.save_task(account.id, &task).await? {
                    return Err(unknown(&task.id));
                }
                self.push_settings(account).await;
                json(SaveAnswer { task: Some(task), next })
            }
            _ => Err("not a schedule call".into()),
        }
    }

    /// Adds an idea made by `by`, or with `id` changes that one. The hub sets who made it and the times.
    pub(crate) async fn save_idea(&self, account: &Account, by: Actor, id: Option<&str>, title: &str, text: &str, project: Option<&str>, status: Option<IdeaStatus>) -> Result<Idea, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("an idea needs a title".into());
        }
        let project = project.map(str::trim).filter(|p| !p.is_empty());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let idea = match id {
            Some(id) => {
                let unknown = || format!("this account has no idea {id}");
                self.db.edit_idea(account.id, Uuid::parse_str(id).map_err(|_| unknown())?, title, text.trim(), project, status, now).await?.ok_or_else(unknown)?
            }
            None => self.db.add_idea(account.id, title, text.trim(), project, status.unwrap_or_default(), &by, now).await?,
        };
        self.push_settings(account).await;
        Ok(idea)
    }

    pub fn logged_out(&self, hash: &[u8]) {
        let accounts: Vec<Arc<Account>> = self.accounts.lock().unwrap().values().cloned().collect();
        for account in accounts {
            let signed_in: Vec<u64> = account.live.lock().unwrap().clients.iter().filter(|(_, c)| c.login == hash).map(|(id, _)| *id).collect();
            for client in signed_in {
                account.client_gone(client);
            }
        }
        self.unlocks.lock().unwrap().retain(|_, unlock| unlock.login != hash);
    }
}

/// Whether the tab keeps its box from counting as quiet. Claude reports a finished turn as waiting
/// for input a minute later, so Working holds a box, and so does an open questionnaire: its
/// worker waits on the user, and the box ending would take the question with it. A permission
/// prompt holds nothing. `max_life` and the budget still end the box.
fn holds_box(tab: &TabView) -> bool {
    tab.status == Some(AgentStatus::Working) || tab.questions.is_some()
}

impl Account {
    /// Registers a machine's connection and sends it every client's watch of it. An older one must
    /// have gone offline first.
    pub fn came_online(&self, machine: &str, socket: u64, name: String, dedicated: bool, version: String, out: Outbox) {
        let mut live = self.live.lock().unwrap();
        for (id, client) in &live.clients {
            if let Some(tabs) = client.threads.get(machine) {
                let _ = out.send(frame(&ToMachine::WatchThreads { viewer: viewer(*id), tabs: tabs.iter().cloned().collect() }));
            }
        }
        let online = Online { socket, name, out, pending: HashMap::new(), next_call: 1, tabs: None, usage: BTreeMap::new(), dedicated, version, state: None, background: None };
        live.machines.insert(machine.to_owned(), online);
    }

    /// Forgets the machine's connection, `socket` or whichever it has: its calls fail and its
    /// tabs are detached, so clients attach again once it is back. Answers whether it was online.
    pub fn went_offline(&self, machine: &str, socket: Option<u64>) -> bool {
        let mut live = self.live.lock().unwrap();
        if socket.is_some_and(|s| live.machines.get(machine).is_some_and(|m| m.socket != s)) {
            return false;
        }
        let Some(gone) = live.machines.remove(machine) else { return false };
        // Dropped unanswered, so each caller sees `Unanswered::Lost`: the call was sent and may have acted.
        drop(gone.pending);
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

    /// What the online `machine` last reported about its load and accounts.
    pub fn state_of(&self, machine: &str) -> Option<pinned_core::load::MachineState> {
        self.live.lock().unwrap().machines.get(machine)?.state.clone()
    }

    /// Keeps `state`, which the online `machine` just answered, as its latest report.
    pub fn note_state(&self, machine: &str, state: pinned_core::load::MachineState) {
        let reported_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        if let Some(online) = self.live.lock().unwrap().machines.get_mut(machine) {
            online.state = Some(pinned_core::load::MachineState { reported_at, ..state });
        }
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

    fn reported_tabs(machines: &HashMap<MachineId, Online>) -> BTreeMap<MachineId, Vec<TabView>> {
        machines.iter().filter_map(|(id, machine)| machine.tabs.clone().map(|tabs| (id.clone(), tabs))).collect()
    }

    fn publish_machines(&self, mut machines: Vec<MachineView>) {
        let live = self.live.lock().unwrap();
        for machine in &mut machines {
            machine.online = live.machines.contains_key(&machine.id);
        }
        let text = frame(&ToClient::Machines { machines, tabs: Some(Self::reported_tabs(&live.machines)) });
        for client in live.clients.values() {
            let _ = client.out.send(text.clone());
        }
    }

    /// The online machine and tab already working on the connector's `subject`.
    pub fn placed(&self, connector: &str, subject: &Subject) -> Option<(MachineId, TabId)> {
        let live = self.live.lock().unwrap();
        live.machines.iter().find_map(|(id, machine)| {
            machine
                .tabs()
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
            .flat_map(|(_, m)| m.tabs())
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
            live.machines.iter().filter(|(_, m)| m.tabs().iter().any(|t| t.running && t.connector.as_deref() == Some(connector))).collect();
        online.sort_by_key(|(_, m)| std::cmp::Reverse(m.socket));
        online.into_iter().map(|(id, _)| id.clone()).collect()
    }

    pub fn is_online(&self, machine: &str) -> bool {
        self.live.lock().unwrap().machines.contains_key(machine)
    }

    /// The id and name of every online machine.
    pub fn online(&self) -> Vec<(MachineId, String)> {
        self.live.lock().unwrap().machines.iter().map(|(id, m)| (id.clone(), m.name.clone())).collect()
    }

    /// What a Run now of workflow `workflow` answers once `placed` took it.
    pub fn started(&self, workflow: &str, placed: Placement) -> RunStarted {
        RunStarted { workflow: workflow.to_owned(), machine_name: self.name_of(&placed.machine).unwrap_or_default(), machine: placed.machine, tab: placed.tab }
    }

    pub fn name_of(&self, machine: &str) -> Option<String> {
        self.live.lock().unwrap().machines.get(machine).map(|m| m.name.clone())
    }

    /// The tab as the online `machine` last reported it.
    pub fn tab(&self, machine: &str, tab: &str) -> Option<TabView> {
        self.live.lock().unwrap().machines.get(machine)?.tabs().iter().find(|t| t.id == tab).cloned()
    }

    /// The tabs the online `machine` last reported, or None when it is offline or has not yet.
    pub fn tabs_of(&self, machine: &str) -> Option<Vec<TabView>> {
        self.live.lock().unwrap().machines.get(machine)?.tabs.clone()
    }

    /// The projects `machine` listed, kept for when it can't list them.
    pub fn listed(&self, machine: &str, projects: Vec<ProjectView>) {
        self.projects.lock().unwrap().insert(machine.to_owned(), projects);
    }

    /// The projects `machine` listed last, or none if it never did since the hub started.
    pub fn last_listed(&self, machine: &str) -> Vec<ProjectView> {
        self.projects.lock().unwrap().get(machine).cloned().unwrap_or_default()
    }

    /// What a box's machine shows of the session of `connector` on `subject`.
    pub fn seen(&self, machine: &str, connector: &str, subject: &Subject) -> Seen {
        let live = self.live.lock().unwrap();
        let Some(m) = live.machines.get(machine) else { return Seen::Offline };
        match m.tabs().iter().find(|t| t.connector.as_deref() == Some(connector) && t.subject.as_ref() == Some(subject)) {
            None => Seen::NoTab,
            Some(t) => Seen::Tab { running: t.running, quiet: !holds_box(t) },
        }
    }

    /// What a worker's or a Captain's box shows: any of its tabs, working or not.
    pub fn seen_any(&self, machine: &str) -> Seen {
        let live = self.live.lock().unwrap();
        let Some(m) = live.machines.get(machine) else { return Seen::Offline };
        match m.tabs() {
            [] => Seen::NoTab,
            tabs => Seen::Tab { running: tabs.iter().any(|t| t.running), quiet: !tabs.iter().any(holds_box) },
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
            MachineEvent::Tabs { tabs } => m.tabs = Some(tabs.clone()),
            MachineEvent::Usage { usage, account } => {
                m.usage.insert(account.clone(), *usage);
            }
            MachineEvent::State { state } => {
                let reported_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                m.state = Some(pinned_core::load::MachineState { reported_at, ..state.clone() });
                return;
            }
            MachineEvent::Background { on } => {
                m.background = Some(*on);
                return;
            }
            _ => {}
        }
        let thread_tab = match &event {
            MachineEvent::Thread { tab } => Some(tab.clone()),
            _ => None,
        };
        let text = frame(&ToClient::Event { machine: machine.to_owned(), event });
        for client in live.clients.values() {
            if thread_tab.as_ref().is_some_and(|tab| !client.threads.get(machine).is_some_and(|tabs| tabs.contains(tab))) {
                continue;
            }
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
    pub fn client_connected(&self, id: u64, login: Vec<u8>, locked: bool, unlocked: Option<Instant>, out: Outbox, mut machines: Vec<MachineView>) {
        let mut live = self.live.lock().unwrap();
        {
            let updates = self.updates.lock().unwrap();
            apply_overlay(&live, &updates, &mut machines);
        }
        let tabs = Self::reported_tabs(&live.machines);
        let account = pinned_core::wire::AccountView { id: self.id.to_string(), email: self.email.clone() };
        let _ = out.send(frame(&ToClient::Hello { account, machines, tabs }));
        for (id, m) in &live.machines {
            for (account, usage) in &m.usage {
                let event = MachineEvent::Usage { usage: *usage, account: account.clone() };
                let _ = out.send(frame(&ToClient::Event { machine: id.clone(), event }));
            }
        }
        live.clients.insert(id, Client { out, login, locked, unlocked, attached: HashSet::new(), threads: HashMap::new() });
    }

    /// Whether client `id` must unlock before `call` on `side`: only the account's owner makes it,
    /// and the password the app took for this connection is missing or past its time.
    pub fn needs_unlock(&self, id: u64, side: Side, call: &impl Serialize) -> bool {
        let (locked, unlocked) = match self.live.lock().unwrap().clients.get(&id) {
            Some(client) => (client.locked, client.unlocked.is_some_and(|until| until > Instant::now())),
            None => return true,
        };
        !unlocked && surface::owner_only(side, &surface::cmd_of(call), locked)
    }

    /// The sha256 of the login client `id` signed in with.
    pub fn login_of(&self, id: u64) -> Option<Vec<u8>> {
        self.live.lock().unwrap().clients.get(&id).map(|client| client.login.clone())
    }

    /// Counts client `id` as its owner until `until`, and answers its login.
    fn unlocked(&self, id: u64, until: Instant) -> Option<Vec<u8>> {
        let mut live = self.live.lock().unwrap();
        let client = live.clients.get_mut(&id)?;
        client.unlocked = Some(until);
        Some(client.login.clone())
    }

    pub fn client_gone(&self, id: u64) {
        let mut live = self.live.lock().unwrap();
        let Some(client) = live.clients.remove(&id) else { return };
        for target in client.attached {
            if let Some(m) = live.machines.get(&target.machine) {
                let _ = m.out.send(frame(&ToMachine::Unsubscribe { tab: target.tab, viewer: viewer(id) }));
            }
        }
        for machine in client.threads.keys() {
            if let Some(m) = live.machines.get(machine) {
                let _ = m.out.send(frame(&ToMachine::WatchThreads { viewer: viewer(id), tabs: vec![] }));
            }
        }
    }

    pub fn watch_threads(&self, client: u64, machine: &str, tabs: Vec<TabId>) {
        let mut live = self.live.lock().unwrap();
        {
            let Some(c) = live.clients.get_mut(&client) else { return };
            if tabs.is_empty() {
                c.threads.remove(machine);
            } else {
                c.threads.insert(machine.to_owned(), tabs.iter().cloned().collect());
            }
        }
        if let Some(m) = live.machines.get(machine) {
            let _ = m.out.send(frame(&ToMachine::WatchThreads { viewer: viewer(client), tabs }));
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

/// The name of the listed project that holds `cwd`, the innermost one, else the folder's own name.
fn project_name(projects: &[ProjectView], cwd: &str) -> String {
    let holds = |p: &&ProjectView| cwd == p.path || cwd.strip_prefix(&p.path).is_some_and(|rest| rest.starts_with('/'));
    match projects.iter().filter(holds).max_by_key(|p| p.path.len()) {
        Some(project) => project.name.clone(),
        None => cwd.trim_end_matches('/').rsplit('/').next().unwrap_or(cwd).to_owned(),
    }
}

fn apply_overlay(live: &Live, updates: &HashMap<MachineId, UpdateStatus>, machines: &mut [MachineView]) {
    for machine in machines {
        machine.online = live.machines.contains_key(&machine.id);
        machine.update = updates.get(&machine.id).cloned();
        machine.state = live.machines.get(&machine.id).and_then(|online| online.state.clone());
        machine.background = live.machines.get(&machine.id).and_then(|online| online.background);
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
        Some(UpdateStatus::Updating { to }) | Some(UpdateStatus::Restarting { to }) => {
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

/// A task id as an app or an agent sends it. Every schedule call, and the agent check before it
/// (`surface::schedule_needs_ok`), reads ids through this one parse, so the check and the act
/// always name the same task; an id it cannot read is refused, never passed.
pub(crate) fn task_key(id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id.trim()).map_err(|_| format!("this account has no task {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_back_before_a_client_saw_it_leave_gets_that_clients_watch() {
        let account = Account { id: Uuid::nil(), email: String::new(), live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
        for client in [1, 2] {
            account.client_connected(client, vec![], false, None, Outbox::new().0, vec![]);
        }
        let (old, _, _) = Outbox::new();
        account.came_online("m", 1, "box".into(), false, "0.16.1".into(), old);
        account.watch_threads(1, "m", vec!["tab-1".into()]);
        account.watch_threads(2, "m", vec!["tab-2".into()]);
        assert!(account.went_offline("m", Some(1)), "the old socket's close");
        account.watch_threads(2, "m", vec![]);
        let (new, mut sent, _) = Outbox::new();
        account.came_online("m", 2, "box".into(), false, "0.16.1".into(), new);
        let mut frames = vec![];
        while let Ok(text) = sent.try_recv() {
            frames.push(serde_json::from_str::<Value>(&text).unwrap());
        }
        assert_eq!(frames, vec![serde_json::json!({ "type": "watch_threads", "viewer": "client-1", "tabs": ["tab-1"] })], "the watch of client 1, which never saw the machine leave, and none for client 2, which unwatched it");
    }

    #[test]
    fn a_machine_reports_run_in_the_background_on_its_view_while_it_is_online() {
        let account = Account { id: Uuid::nil(), email: String::new(), live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
        let (client, mut heard, _) = Outbox::new();
        account.client_connected(1, vec![], false, None, client, vec![]);
        let (out, _sent, _) = Outbox::new();
        account.came_online("m", 1, "studio".into(), false, "0.17.24".into(), out);
        let view = || MachineView { id: "m".into(), name: "studio".into(), os: "macos".into(), online: false, last_seen: 0, upstash_box: None, version: String::new(), update: None, state: None, background: None };
        let shown = |account: &Account| {
            let mut views = [view()];
            account.overlay(&mut views);
            views[0].background
        };
        assert_eq!(shown(&account), None, "nothing until the app reports it");
        while heard.try_recv().is_ok() {}
        account.machine_event("m", 1, MachineEvent::Background { on: true });
        assert_eq!(shown(&account), Some(true));
        account.machine_event("m", 1, MachineEvent::Background { on: false });
        assert_eq!(shown(&account), Some(false));
        assert!(heard.try_recv().is_err(), "clients get it with the machines list, not as an event");
        assert!(account.went_offline("m", Some(1)));
        assert_eq!(shown(&account), None, "an offline machine can't be switched");
    }

    #[test]
    fn a_call_cut_off_by_its_machine_going_offline_is_lost_not_refused() {
        let account = Account { id: Uuid::nil(), email: String::new(), live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
        let (out, _sent, _) = Outbox::new();
        account.came_online("m", 1, "box".into(), false, "0.17.14".into(), out);
        let (_, mut answered) = account.send_call("m", Call::Tabs).unwrap();
        assert!(account.went_offline("m", Some(1)));
        // `Hub::answer` reads a closed channel as `Unanswered::Lost`, and an answer sent into it as `Refused`.
        assert_eq!(answered.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Closed));
    }

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

    /// A box's account with one machine showing a single tab with `status`, laid over by an open
    /// questionnaire when `asked`, as the node shows a tab whose worker called `question_user`.
    fn box_showing(status: AgentStatus, asked: bool, running: bool) -> Account {
        use pinned_core::questions::{self, QOption, Question, Questionnaire};
        let account = Account { id: Uuid::nil(), email: String::new(), live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
        let (out, _sent, _) = Outbox::new();
        account.came_online("box", 1, "box".into(), true, "0.18.2".into(), out);
        let mut tab: TabView = serde_json::from_value(serde_json::json!({
            "id": "tab-1", "cwd": "/work/repo", "bypass": true, "running": running, "opened": 1_794_240_000u64,
            "status": status, "connector": "delegate", "subject": { "issue": "FAKE-1" },
        }))
        .unwrap();
        if asked {
            let question = Question { header: None, question: "Base the fix on main?".into(), changes: "which branch the PR targets".into(), options: vec![QOption { label: "main".into(), description: None, recommended: true }], multi_select: false };
            tab = questions::shown(tab, &Questionnaire { id: "q-1".into(), title: None, questions: vec![question], projects: vec![], at: 1_794_240_000 });
        }
        account.machine_event("box", 1, MachineEvent::Tabs { tabs: vec![tab] });
        account
    }

    /// What the reaper decides for the box once `held` has passed since it first saw `seen`, with
    /// the box `age` old, under the default policy (idle 10 minutes, max_life 4 hours).
    fn reaped(seen: impl Fn() -> Seen, held: Duration, age: Duration) -> Option<boxes::End> {
        let (start, mut timers, policy) = (Instant::now(), boxes::Timers::default(), boxes::Policy::default());
        assert_eq!(boxes::verdict(Duration::ZERO, seen(), &mut timers, start, &policy), None, "nothing ends it at its first look");
        boxes::verdict(age, seen(), &mut timers, start + held, &policy)
    }

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn a_box_waiting_on_an_open_question_is_not_idle() {
        let account = box_showing(AgentStatus::NeedsInput, true, true);
        let subject = Subject::LinearIssue { issue: "FAKE-1".into() };
        let looks: [(&str, Box<dyn Fn() -> Seen>); 2] = [("seen", Box::new(|| account.seen("box", "delegate", &subject))), ("seen_any", Box::new(|| account.seen_any("box")))];
        for (which, look) in looks {
            assert_eq!(look(), Seen::Tab { running: true, quiet: false }, "{which}: an open questionnaire holds the box");
            assert_eq!(reaped(&look, 3 * 60 * MIN, 3 * 60 * MIN), None, "{which}: three hours of waiting on the user is not idle");
            // Waiting past max_life ends the box as Expired, whatever it waits on.
            assert_eq!(reaped(&look, 3 * 60 * MIN, 4 * 60 * MIN), Some(boxes::End::Expired), "{which}: max_life still ends it");
        }
    }

    #[test]
    fn a_box_on_a_permission_prompt_or_idle_still_ends_as_idle() {
        let subject = Subject::LinearIssue { issue: "FAKE-1".into() };
        for status in [AgentStatus::NeedsInput, AgentStatus::Done] {
            let account = box_showing(status, false, true);
            let looks: [(&str, Box<dyn Fn() -> Seen>); 2] = [("seen", Box::new(|| account.seen("box", "delegate", &subject))), ("seen_any", Box::new(|| account.seen_any("box")))];
            for (which, look) in looks {
                assert_eq!(look(), Seen::Tab { running: true, quiet: true }, "{which} {status:?}: no questionnaire, so it is quiet");
                assert_eq!(reaped(&look, 9 * MIN, 9 * MIN), None, "{which} {status:?}: quiet 9 of 10 minutes");
                assert_eq!(reaped(&look, 11 * MIN, 11 * MIN), Some(boxes::End::Idle), "{which} {status:?}: idle ends it");
            }
        }
    }

    #[test]
    fn a_box_whose_session_exited_with_a_question_open_still_ends_as_exited() {
        let account = box_showing(AgentStatus::NeedsInput, true, false);
        let subject = Subject::LinearIssue { issue: "FAKE-1".into() };
        let looks: [(&str, Box<dyn Fn() -> Seen>); 2] = [("seen", Box::new(|| account.seen("box", "delegate", &subject))), ("seen_any", Box::new(|| account.seen_any("box")))];
        for (which, look) in looks {
            assert_eq!(look(), Seen::Tab { running: false, quiet: false }, "{which}: the question holds, the process does not");
            assert_eq!(reaped(&look, 4 * MIN, 4 * MIN), None, "{which}: exited 4 of 5 minutes");
            assert_eq!(reaped(&look, 6 * MIN, 6 * MIN), Some(boxes::End::Exited), "{which}: an open question does not keep an exited session's box");
        }
    }

    #[test]
    fn a_working_box_is_not_quiet() {
        let account = box_showing(AgentStatus::Working, false, true);
        assert_eq!(account.seen_any("box"), Seen::Tab { running: true, quiet: false });
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
        assert_eq!(
            reconcile(Some(empty), "1.2.0", None),
            Some(UpdateStatus::Failed { to: String::new(), reason: "came back on 1.2.0".into() }),
        );
        assert_eq!(reconcile(None, "1.2.0", None), None);
    }
}
