use crate::boxes::{self, BoxApi, Boxes, Seen};
use crate::db::{self, Db};
use crate::dev::Dev;
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
use pinned_core::wire::box_usd_per_hour;
use pinned_core::upstash;
use base64::Engine;
use pinned_core::{github, sync};
use pinned_core::wire::{
    Call, HubCall, Invites, MachineEvent, MachineId, MachineView, PairCode, ProjectView, Reply, RunSpec, RunStarted, TabId, TabView, Target, ToClient, ToMachine, UpdateStatus, EXE_PLACEHOLDER,
};
use pinned_core::surface::{self, Side, Unlocked};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use crate::store_writes::{self, Landed, Writes};
use tokio::sync::{mpsc, oneshot, OnceCell};
use uuid::Uuid;

const MACHINE_CALL_WAIT: Duration = Duration::from_secs(300);
/// Longer than a machine's `claude setup-token` waits for the user's approval in the browser.
const MINT_WAIT: Duration = Duration::from_secs(pinned_core::setup_token::WAIT.as_secs() + 30);
/// How long importing an MCP server waits for the machine to read its account's config.
const MCP_IMPORT_WAIT: Duration = Duration::from_secs(20);
const MAX_RECORDS_PER_CALL: usize = 500;
/// Why `open_placed` refuses a run on a team's account.
pub const TEAM_RUNS: &str = "a team's runs start only through its bots";
/// Why the hub refuses a worker profile proposal: only a Captain's or the global orchestrator's
/// session proposes one, through its machine.
const NOT_AN_ORCHESTRATOR: &str = "only a Captain or the global orchestrator proposes a worker profile";
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
    /// The bytes of the frames that wait, for the census.
    bytes: Arc<AtomicUsize>,
}

/// A frame to send, with its type for the census (`traffic`).
#[derive(Clone)]
pub struct Frame {
    label: &'static str,
    text: String,
}

/// The socket's end of an `Outbox`.
pub struct OutboxQueue {
    frames: mpsc::Receiver<String>,
    bytes: Arc<AtomicUsize>,
}

impl OutboxQueue {
    pub async fn recv(&mut self) -> Option<String> {
        let text = self.frames.recv().await?;
        self.bytes.fetch_sub(text.len(), Ordering::Relaxed);
        Some(text)
    }

    pub fn try_recv(&mut self) -> Result<String, mpsc::error::TryRecvError> {
        let text = self.frames.try_recv()?;
        self.bytes.fetch_sub(text.len(), Ordering::Relaxed);
        Ok(text)
    }
}

impl Outbox {
    /// The outbox, the socket's end of it, and what fires once the peer fell behind.
    pub fn new() -> (Outbox, OutboxQueue, Arc<tokio::sync::Notify>) {
        let (frames, queue) = mpsc::channel(OUTBOX_FRAMES);
        let overflowed = Arc::new(tokio::sync::Notify::new());
        let bytes = Arc::new(AtomicUsize::new(0));
        (Outbox { frames, overflowed: overflowed.clone(), bytes: bytes.clone() }, OutboxQueue { frames: queue, bytes }, overflowed)
    }

    pub fn send(&self, frame: Frame) -> Result<(), ()> {
        let (label, len) = (frame.label, frame.text.len());
        // Counted before the send, so the queue's end never subtracts bytes not added yet.
        self.bytes.fetch_add(len, Ordering::Relaxed);
        match self.frames.try_send(frame.text) {
            Ok(()) => {
                crate::traffic::sent(label, len);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.bytes.fetch_sub(len, Ordering::Relaxed);
                self.overflowed.notify_one();
                Err(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.bytes.fetch_sub(len, Ordering::Relaxed);
                Err(())
            }
        }
    }

    /// How many frames wait for the peer to read them, and their bytes.
    fn queued(&self) -> (usize, usize) {
        (self.frames.max_capacity() - self.frames.capacity(), self.bytes.load(Ordering::Relaxed))
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

/// What a workflow's step reads when the hub was busy partway through opening it: unlike the busy
/// refusal, the step is not tried again, since part of it may have happened.
pub const BUSY_PARTWAY: &str = "the hub was busy partway through opening this, so it was not tried again";

/// `e`, the refusal of a step's call, read as `BUSY_PARTWAY` when it is the busy refusal (`db::BUSY`).
fn busy_partway(e: String) -> String {
    match e == db::BUSY {
        true => BUSY_PARTWAY.into(),
        false => e,
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

/// How often the hub logs its census.
pub const CENSUS_EVERY: Duration = Duration::from_secs(300);

/// `census_accounts`' line for one account.
fn account_census(id: Uuid, runs: usize, (store_bytes, runs_bytes, pending): (usize, usize, store_writes::Pending), (check_age_s, check_failing): (i64, bool), watching: bool) -> String {
    format!(
        "pinned-server: census-account id={id} runs={runs} runs_bytes={runs_bytes} store_bytes={store_bytes} store_pending={} store_pending_bytes={} secrets_pending={} secrets_pending_bytes={} check_age_s={check_age_s} check_failing={} watching={}",
        pending.store, pending.store_bytes, pending.secrets, pending.secrets_bytes, u8::from(check_failing), u8::from(watching)
    )
}

/// What the hub's check log keeps of an account's previous check: when it ran, the kind of its
/// error (`error_kind`), when the failure that error belongs to began, and when its line last printed.
#[derive(Clone, Debug, Default, PartialEq)]
struct CheckLog {
    at: Option<i64>,
    error: Option<String>,
    error_since: i64,
    error_logged: i64,
}

/// An error that lasts with the same kind prints its line again after this many seconds.
const CHECK_ERROR_REPEAT_S: i64 = 30 * 60;

/// A check that comes more than this many seconds after the one before it is late.
const CHECK_LATE_AFTER_S: i64 = 120;

/// The check log's lines for one new check of `account`, and what it keeps for the next one. An
/// error's text stays out of the log: its line names the error's class, its part count and its
/// fingerprint. The line prints when the error's kind changes, and while one kind lasts at most once
/// per `CHECK_ERROR_REPEAT_S`, so a text that varies per check (a request id) does not print each time.
fn log_check(prev: &CheckLog, account: Uuid, check: &pinned_core::connectors::Check, now: i64) -> (CheckLog, Vec<String>) {
    let who: String = account.to_string().chars().take(8).collect();
    let mut lines = vec![];
    if let Some(gap) = prev.at.map(|at| now - at).filter(|gap| *gap > CHECK_LATE_AFTER_S) {
        lines.push(format!("pinned-server: check-late account={who} gap_s={gap}"));
    }
    let error = check.error.as_deref();
    let kind = error.map(error_kind);
    let mut next = CheckLog { at: Some(now), error: kind.clone(), ..prev.clone() };
    match (&prev.error, error, &kind) {
        (Some(was), _, Some(new)) if was == new && now - prev.error_logged < CHECK_ERROR_REPEAT_S => {}
        (_, Some(error), Some(_)) => {
            next.error_logged = now;
            // A changed error belongs to the failure that is already on, so it does not restart the count.
            if prev.error.is_none() {
                next.error_since = now;
            }
            lines.push(format!("pinned-server: check-error account={who} class={} parts={} fp={}", error_class(error), error.split("; ").count(), error_fingerprint(error)));
        }
        (Some(_), _, None) => lines.push(format!("pinned-server: check-ok account={who} after_s={}", now - prev.error_since)),
        _ => {}
    }
    (next, lines)
}

/// The first 8 hex digits of an error's SHA-256. The same text has the same fingerprint, and the
/// log keeps it in place of the text.
fn error_fingerprint(error: &str) -> String {
    Sha256::digest(error.as_bytes())[..4].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What decides that an error is another one: its class and the status it names after "answered"
/// (`Linear answered 503`). Neither holds the rest of the text, which may vary per check.
fn error_kind(error: &str) -> String {
    let first = error.split("; ").next().unwrap_or_default();
    let status = first.split("answered ").nth(1).map(|after| after.chars().take_while(char::is_ascii_digit).collect::<String>()).filter(|status| status.len() == 3);
    format!("{}/{}", error_class(error), status.unwrap_or_default())
}

/// The class of a check's error, from the prefix of its first part (the parts are joined by "; "):
/// the connector or the step that failed. A prefix that no class names is `other`.
fn error_class(error: &str) -> &'static str {
    const CLASSES: [(&str, &str); 7] = [
        ("Slack", "slack"),
        ("Linear", "linear"),
        ("Vercel", "vercel"),
        ("Upstash Box", "upstash"),
        ("could not open a session", "session"),
        ("workflow ", "workflow"),
        ("could not read workflow runs", "runs"),
    ];
    let first = error.split("; ").next().unwrap_or_default();
    CLASSES.iter().find(|(prefix, _)| first.starts_with(prefix)).map_or("other", |&(_, class)| class)
}

/// `account_census`' check fields: the last check's age in whole seconds at `now` (-1 when there is
/// none), and whether it failed.
fn check_census(check: Option<&pinned_core::connectors::Check>, now: i64) -> (i64, bool) {
    let age = check.and_then(|check| chrono::DateTime::parse_from_rfc3339(&check.at).ok()).map_or(-1, |at| now - at.timestamp());
    (age, check.is_some_and(|check| check.error.is_some()))
}

/// `census_accounts`' lines, one per persisted account in the order given: each one's census with
/// its connectors' runs and last check, read from `accounts`.
fn census_lines(censuses: Vec<(Uuid, (usize, usize, store_writes::Pending))>, accounts: &HashMap<Uuid, Arc<Account>>, now: i64) -> Vec<String> {
    censuses
        .into_iter()
        .map(|(id, census)| {
            let account = accounts.get(&id);
            let connectors = account.and_then(|account| account.connectors.get());
            let runs = connectors.map_or(0, |connectors| connectors.census().runs);
            let check = check_census(connectors.and_then(|connectors| connectors.last_check()).as_ref(), now);
            let watching = account.is_some_and(|account| account.watching.load(Ordering::SeqCst));
            account_census(id, runs, census, check, watching)
        })
        .collect()
}

/// The current time in whole seconds since the Unix epoch.
fn unix_secs_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The resident set size, in KiB, from the text of `/proc/self/status`.
fn rss_kib(status: &str) -> Option<u64> {
    status.lines().find_map(|line| line.strip_prefix("VmRSS:"))?.trim().strip_suffix("kB")?.trim().parse().ok()
}

/// A byte count from the text of a cgroup v2 `memory.stat`.
fn cgroup_bytes(stat: &str, key: &str) -> Option<u64> {
    stat.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix(' ')?.trim().parse().ok())
}

/// How often the hub expires held ends for every account.
pub const HELD_EXPIRY_EVERY: Duration = Duration::from_secs(3600);

pub struct Hub {
    /// The hub itself, for work a borrowed `&Hub` hands to a task of its own (`teams::queue::next`).
    me: std::sync::Weak<Hub>,
    pub db: Db,
    pub public_url: String,
    pub signup: Signup,
    pub boxes: Boxes,
    /// Every team's call timings and default bot limits (`teams::calls::tick`).
    pub team_policy: pinned_core::teams::TeamPolicy,
    /// When failed team invite joins came, for their limit (`teams::membership::join`).
    pub(crate) team_join_fails: Mutex<crate::teams::membership::JoinFails>,
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
    /// When the hub last expired held ends for every account, so it runs at most once an hour.
    held_expired: Mutex<Option<Instant>>,
    /// While true, every account's runs save fails, as a full disk would make it. Only a test sets
    /// it: a queued Postgres write cannot fail on its own at the moment a box ends.
    #[doc(hidden)]
    pub refuse_run_saves: Arc<AtomicBool>,
    /// When set, the next backstop for reservations (`teams::calls::reap`) that read a reservation
    /// as past its limit tells the first, then waits for the second before it ends it. Only a test
    /// sets it, to run a claim between the backstop's read and its end; it is taken on use.
    #[doc(hidden)]
    pub hold_backstop: Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
    /// Every account's `PgStorage` state, for the census and the flush on stop.
    persisted: Mutex<HashMap<Uuid, Arc<Persisted>>>,
    /// Reporting an issue from the app: pinned's GitHub repository and what the hub keeps about it.
    pub(crate) reports: crate::reports::Reports,
    /// `hub --dev`'s time scale. Timers it shortens call `dev.scale`; `Dev::OFF` otherwise.
    pub dev: Dev,
}

struct Unlock {
    /// The sha256 of the login it was taken on, the only one it works for.
    login: Vec<u8>,
    until: Instant,
}

pub struct Account {
    pub id: Uuid,
    pub email: String,
    /// A team's account (kind `team`), read once when the hub loads it: a kind never changes. Its
    /// runs start only through its bots (`teams::bots::start_reserved`), never through `open_placed`.
    pub team: bool,
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
    /// The tabs a person asked the machine to close (`Call::CloseTab`), until they go.
    closing: HashSet<String>,
    /// How each connector session whose tab went ended: its connector, subject, and `LastTurn`.
    /// Kept for a team's machines only, whose runs end by it (`teams::calls::box_ended`): a
    /// person's machine runs sessions for ever more subjects, and nothing reads theirs.
    gone: Vec<(String, Subject, LastTurn)>,
    /// The projects this socket asked `SyncChanges` for. It hears `SyncChanged` for these only,
    /// and a reconnect starts empty.
    sync_heard: HashSet<String>,
}

/// How a connector session's last turn stood when its tab went or its box ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LastTurn {
    /// Its last hook status was Done: the session ended its own turn (Claude's Stop hook).
    Ended,
    /// A person closed its tab before the session ended its turn.
    ClosedByPerson,
    /// Neither is known: it was mid-turn, or the hub never saw it.
    Unknown,
}

/// What a tab that is still open says about its turn.
fn last_turn(tab: &TabView) -> LastTurn {
    if tab.status == Some(AgentStatus::Done) && tab.questions.is_none() { LastTurn::Ended } else { LastTurn::Unknown }
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
    /// It sent a team call on this connection, so it hears its teams' events (`broadcast_team`).
    hears_teams: bool,
}

fn viewer(client: u64) -> String {
    format!("client-{client}")
}

fn client_of(viewer: &str) -> Option<u64> {
    viewer.strip_prefix("client-")?.parse().ok()
}

pub(crate) fn frame<T: Serialize + crate::traffic::Labeled>(frame: &T) -> Frame {
    Frame { label: frame.label(), text: serde_json::to_string(frame).unwrap() }
}

fn json<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// The answer of `Hub::hub_call` to a team call: team ops come only from the app's own path.
pub const TEAM_CALL_ELSEWHERE: &str = "a team's ops run only from the pinned app";

impl Hub {
    /// `SyncChanged` when the write moved `project`'s head, a push that failed after some rows
    /// landed included. No before-read, or a head that did not move, sends nothing.
    async fn tell_sync(&self, account: &Account, project: &str, before: Option<u64>) {
        let Some(before) = before else { return };
        if self.db.sync_head(account.id, project).await.ok().is_some_and(|after| after != before) {
            account.sync_changed(project);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new(db: Db, public_url: String, signup: Signup, box_api: Arc<dyn BoxApi>, box_policy: boxes::Policy, team_policy: pinned_core::teams::TeamPolicy, convex_http: Arc<dyn ConvexHttp>, reports: Option<crate::reports::Config>, dev: Dev) -> Result<Arc<Hub>, String> {
        let public_url = public_url.trim_end_matches('/').to_owned();
        let boxes = Boxes::new(box_api, box_policy, &public_url);
        let reports = crate::reports::Reports::start(&db, reports).await;
        Ok(Arc::new_cyclic(|me| Hub { me: me.clone(), db, public_url, signup, boxes, team_policy, team_join_fails: Mutex::default(), convex_http, accounts: Mutex::default(), sockets: AtomicU64::new(1), agent_locks: Mutex::default(), unlocks: Mutex::default(), going_away: tokio::sync::watch::channel(false).0, held_expired: Mutex::default(), refuse_run_saves: Arc::default(), hold_backstop: Mutex::default(), persisted: Mutex::default(), reports, dev }))
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

    /// One delivery of the end the hub holds for `origin`'s task `task` at a time.
    pub async fn hold_delivery(&self, account: &Account, origin: Uuid, task: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.hold(format!("held:{}:{origin}:{task}", account.id)).await
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

    /// One log line of counts: what the hub holds in memory, and the process's memory where the
    /// system says. It names no account, machine, address or content. Each lock is held briefly.
    pub fn census(&self) -> String {
        let accounts: Vec<Arc<Account>> = self.accounts.lock().unwrap().values().cloned().collect();
        let (mut clients, mut machines, mut frames_sum, mut frames_max, mut pending) = (0, 0, 0, 0, 0);
        let (mut outbox_bytes_sum, mut outbox_bytes_max) = (0, 0);
        let (mut known_tabs, mut known_tab_facts, mut drive_waiters, mut tab_drive_waiting, mut tab_drives_coalesced) = (0, 0, 0, 0, 0);
        for account in &accounts {
            {
                let live = account.live.lock().unwrap();
                clients += live.clients.len();
                machines += live.machines.len();
                pending += live.machines.values().map(|machine| machine.pending.len()).sum::<usize>();
                for out in live.clients.values().map(|client| &client.out).chain(live.machines.values().map(|machine| &machine.out)) {
                    let (queued, bytes) = out.queued();
                    frames_sum += queued;
                    frames_max = frames_max.max(queued);
                    outbox_bytes_sum += bytes;
                    outbox_bytes_max = outbox_bytes_max.max(bytes);
                }
            }
            if let Some(connectors) = account.connectors.get() {
                let census = connectors.census();
                known_tabs += census.known_tabs;
                known_tab_facts += census.known_tab_facts;
                drive_waiters += census.drive_waiters;
                tab_drive_waiting += census.tab_drive_waiting;
                tab_drives_coalesced += census.tab_drives_coalesced;
            }
        }
        let mut line = format!(
            "pinned-server: census accounts={} clients={clients} machines={machines} outbox_frames_sum={frames_sum} outbox_frames_max={frames_max} outbox_bytes_sum={outbox_bytes_sum} outbox_bytes_max={outbox_bytes_max} pending_calls={pending} known_tabs={known_tabs} known_tab_facts={known_tab_facts} drive_waiters={drive_waiters} tab_drive_waiting={tab_drive_waiting} tab_drives_coalesced={tab_drives_coalesced}",
            accounts.len()
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            line.push_str(&format!(" tasks_alive={}", runtime.metrics().num_alive_tasks()));
        }
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        if let Some(kib) = rss_kib(&status) {
            line.push_str(&format!(" rss_mb={}", kib / 1024));
        }
        let stat = std::fs::read_to_string("/sys/fs/cgroup/memory.stat").unwrap_or_default();
        for (key, field) in [("anon", "cgroup_anon_mb"), ("file", "cgroup_file_mb")] {
            if let Some(bytes) = cgroup_bytes(&stat, key) {
                line.push_str(&format!(" {field}={}", bytes >> 20));
            }
        }
        line.push_str(&crate::heap::census());
        line.push_str(&crate::traffic::take().fields());
        line
    }

    /// One census line per account whose connectors run: its runs and store, and what waits to be
    /// written to its row. Counts and sizes only; the account appears by id.
    pub fn census_accounts(&self) -> Vec<String> {
        let mut persisted: Vec<(Uuid, Arc<Persisted>)> = self.persisted.lock().unwrap().iter().map(|(id, p)| (*id, p.clone())).collect();
        persisted.sort_by_key(|(id, _)| *id);
        // The accounts' lock is let go before any account's own locks are taken.
        let accounts: HashMap<Uuid, Arc<Account>> = self.accounts.lock().unwrap().clone();
        let censuses = persisted.iter().map(|(id, p)| (*id, p.census())).collect();
        census_lines(censuses, &accounts, unix_secs_now())
    }

    /// Writes every account's store and keys that wait, all at once, and again what was saved
    /// meanwhile, until nothing waits or `within` passed. Answers how many accounts still have a
    /// document waiting, which their ids follow in the log.
    pub async fn flush_writes(&self, within: Duration) -> usize {
        let started = Instant::now();
        let accounts = || self.persisted.lock().unwrap().iter().map(|(id, p)| (*id, p.writes.clone())).collect();
        let (waiting, left) = store_writes::flush_all(accounts, within).await;
        for id in &left {
            eprintln!("pinned-server: account {id}'s workflows or connector keys were not saved before the hub stopped");
        }
        if waiting > 0 {
            eprintln!("pinned-server: {waiting} accounts had saves waiting at stop; {} written in {} ms", waiting.saturating_sub(left.len()), started.elapsed().as_millis());
        }
        left.len()
    }

    pub async fn watch_all(self: &Arc<Self>) -> Result<(), String> {
        for id in self.db.accounts_with_connectors().await? {
            let account = self.account(id).await?;
            self.watch(&account).await?;
        }
        Ok(())
    }

    /// Deletes up to `db::HELD_EXPIRY_BATCH` held ends of any account, waiting or unclaimed, held
    /// longer than 30 days, so an account whose machines never come back keeps none past them. Runs
    /// at most once per `HELD_EXPIRY_EVERY`, or on the next tick after a run that deleted a full
    /// batch; a call before that, or one whose delete failed, deletes nothing and returns 0. A
    /// failure is logged without any row's text. Each run first expires the trigger fires
    /// (`expire_trigger_fires`).
    pub async fn expire_held_ends(&self) -> u64 {
        {
            let mut last = self.held_expired.lock().unwrap();
            if last.is_some_and(|at| at.elapsed() < HELD_EXPIRY_EVERY) {
                return 0;
            }
            *last = Some(Instant::now());
        }
        self.expire_trigger_fires().await;
        match self.db.expire_all_held_ends(crate::db::HELD_EXPIRY_BATCH).await {
            Ok(0) => 0,
            Ok(n) => {
                eprintln!("pinned-server: {n} held ends older than 30 days expired");
                if n >= crate::db::HELD_EXPIRY_BATCH as u64 {
                    // A full batch may leave more: the next tick runs again.
                    *self.held_expired.lock().unwrap() = None;
                }
                n
            }
            Err(e) => {
                eprintln!("pinned-server: could not expire the held ends older than 30 days: {e}");
                0
            }
        }
    }

    /// Deletes every trigger's claimed fire older than 7 days, in the hourly run of
    /// `expire_held_ends`. A failure is logged without any row's text.
    async fn expire_trigger_fires(&self) {
        match self.db.expire_trigger_fires().await {
            Ok(0) => {}
            Ok(n) => eprintln!("pinned-server: {n} trigger fires older than 7 days expired"),
            Err(e) => eprintln!("pinned-server: could not expire the trigger fires older than 7 days: {e}"),
        }
    }

    /// The account if the hub holds it already, as it does while any of its apps is connected.
    pub fn loaded(&self, id: Uuid) -> Option<Arc<Account>> {
        self.accounts.lock().unwrap().get(&id).cloned()
    }

    pub async fn account(&self, id: Uuid) -> Result<Arc<Account>, String> {
        if let Some(account) = self.accounts.lock().unwrap().get(&id) {
            return Ok(account.clone());
        }
        let email = self.db.email(id).await?.ok_or("the account was deleted")?;
        let team = crate::teams::db::is_team(&self.db, id).await?;
        let fresh = Arc::new(Account { id, email, team, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() });
        Ok(self.accounts.lock().unwrap().entry(id).or_insert(fresh).clone())
    }

    #[doc(hidden)]
    pub async fn connectors(&self, account: &Account) -> Result<Arc<Connectors>, String> {
        let connectors = account
            .connectors
            .get_or_try_init(|| async {
                let stored = self.db.connectors(account.id).await?;
                let storage = PgStorage::new(self.db.clone(), account.id, stored, self.refuse_run_saves.clone());
                self.persisted.lock().unwrap().insert(account.id, storage.0.clone());
                let connectors = Arc::new(Connectors::new(storage, EXE_PLACEHOLDER.into()));
                // A tab list a machine sent before this load found no workflows to wake: the next
                // pass reads the lists the account's machines last reported.
                let observed = pinned_core::connectors::unix_millis();
                for (machine, _) in account.online() {
                    if let Some(tabs) = account.tabs_of(&machine) {
                        connectors.note_tabs(&machine, observed, &tabs);
                    }
                }
                // A team's events ask its bots' lines first, before anything outside pinned.
                if account.team {
                    connectors.set_admission(Arc::new(crate::teams::triggers::TeamAdmission { hub: self.me.clone(), team: account.id }));
                }
                Ok::<_, String>(connectors)
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
                async move { hub.open_found(id, work).await }
            };
            let log = Mutex::new(CheckLog::default());
            let on_check = move |check: &pinned_core::connectors::Check| {
                let mut state = log.lock().unwrap();
                let (next, lines) = log_check(&state, id, check, unix_secs_now());
                *state = next;
                drop(state);
                for line in lines {
                    eprintln!("{line}");
                }
                let (hub, account) = checked.clone();
                tokio::spawn(async move { hub.push_settings(&account).await });
            };
            connectors.watch(open, on_check).await
        });
        let (hub, account) = (self.clone(), account.clone());
        tokio::spawn(async move {
            loop {
                hub.boxes.reap(&hub, &account).await;
                // Hub-wide, at most once an hour whichever account's tick comes first; it only logs
                // a failure, so the reaper goes on either way.
                hub.expire_held_ends().await;
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

    /// Hands `work` to a machine and returns the tab that took a run, or none for a delivery. A
    /// team's account takes no run here: its runs start only through its bots (`open_team_run`).
    pub async fn open_placed(&self, account: Uuid, work: Work) -> Result<Option<Placement>, String> {
        let account = self.account(account).await?;
        match work {
            Work::Run(_) if account.team => Err(TEAM_RUNS.into()),
            Work::Run(run) => self.place_run(&account, run).await.map(Some),
            Work::Delivery(d) => self.deliver(&account, d).await.map(|()| None),
            Work::Prompt { env, text } => self.type_prompt(&account, &env, &text).await.map(|()| None),
        }
    }

    /// What an account's own pollers and tab wakes found: a person's goes to `open_placed`, a
    /// team's to its bots (`teams::triggers::fired`).
    /// A busy refusal is answered as is only where the hub refuses before it does anything (the
    /// account lookup, and a run sent to a machine pool, whose refusals come before any machine
    /// starts it): the workflow then tries the same step again. A team's start and a box's open can
    /// be refused busy partway, so theirs answer `BUSY_PARTWAY` and the step is not repeated.
    pub async fn open_found(self: &Arc<Self>, account: Uuid, work: Work) -> Result<Option<Placement>, String> {
        let boxed = matches!(&work, Work::Run(run) if matches!(run.runs_on, RunsOn::FreshBox(_)));
        match self.account(account).await?.team {
            true => crate::teams::triggers::fired(self, account, work).await.map_err(busy_partway),
            false if boxed => self.open_placed(account, work).await.map_err(busy_partway),
            false => self.open_placed(account, work).await,
        }
    }

    /// The hub as an `Arc`, while it is still up.
    pub fn arc(&self) -> Option<Arc<Hub>> {
        self.me.upgrade()
    }

    /// Opens a team's run, which only `teams::bots::start_reserved` asks for: the pass is its proof.
    pub async fn open_team_run(&self, _pass: crate::teams::bots::TeamRunPass, team: &Arc<Account>, run: Run) -> Result<Placement, String> {
        self.place_run(team, run).await
    }

    /// A run goes to the tab that already has it, else where its workflow says.
    async fn place_run(&self, account: &Arc<Account>, run: Run) -> Result<Placement, String> {
        if let Some((machine, tab)) = account.placed(&run.connector, &run.subject) {
            return Ok(Placement { machine, tab });
        }
        match run.runs_on.clone() {
            RunsOn::FreshBox(step) => self.boxes.open(self, account, run, step).await,
            RunsOn::Any => self.open_on_pool(account, run, "").await,
            RunsOn::Machine(id) => self.open_on_pool(account, run, &id).await,
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
            // Only an app reaches this, unlocked (`surface::owner_only`): no agent action and no machine call sets it.
            HubCall::SetReleaseProtection { on } => {
                let (args, detail) = (serde_json::json!({ "on": on }).to_string(), format!("the user's app turned release protection {}", if on { "on" } else { "off" }));
                let row = db::Audit { account: account.id, machine: Uuid::nil(), tab: "app", role: surface::ActorRole::Session, action: "set_release_protection", args: &args, outcome: surface::AuditOutcome::Done, detail: &detail, request_id: None, task: None };
                self.db.set_release_protection(account.id, on, &row).await?;
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
            HubCall::SyncChanges { project, since } => return json(self.db.sync_changes(account.id, &project, since).await?),
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
                    // A deletion is compared with what the hub holds through `agreed`, a history through `base`.
                    let agreed = file.agreed.filter(|_| file.entry.sha.is_empty());
                    let base = file.base.filter(|_| file.entry.path == sync::HISTORY && !file.entry.sha.is_empty());
                    kept.push((file.entry, data, agreed, base));
                }
                let before = self.db.sync_head(account.id, &project).await.ok();
                let pushed = self.db.sync_push(account.id, &project, kept).await;
                self.tell_sync(account, &project, before).await;
                return json(pushed?);
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
                let before = self.db.sync_head(account.id, &project).await.ok();
                let marked = self.db.sync_mark_executable(account.id, &project, &repairs).await;
                self.tell_sync(account, &project, before).await;
                return json(marked?);
            }
            HubCall::SyncPull { project, paths } => {
                let files = self.db.sync_pull(account.id, &project, &paths).await?;
                crate::traffic::sync_pulled(files.iter().map(|(_, data)| data.len()).sum());
                let files: Vec<sync::File> = files
                    .into_iter()
                    .map(|(entry, data)| {
                        let data = (!entry.sha.is_empty()).then(|| base64::engine::general_purpose::STANDARD.encode(data));
                        sync::File { entry, data, agreed: None, base: None }
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
            HubCall::SetBoxAccess { access } if account.team && [&access.workflow, &access.session, &access.worker, &access.orchestrator].iter().any(|p| matches!(p, pinned_core::connectors::BoxPermission::Fixed { .. })) => {
                return Err(crate::teams::bots::OWN_BOX_EACH.into());
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
            HubCall::SetReviewer { patch } => return self.change_tastes(account, |tastes| tastes.set_reviewer(patch)).await,
            HubCall::SetWorkerProfiles { project, profiles, drop_proposed } => {
                let set = |tastes: &mut Tastes| {
                    tastes.set_worker_profiles(project.as_deref(), profiles)?;
                    tastes.drop_proposed(&drop_proposed);
                    Ok(())
                };
                return self.change_tastes(account, set).await;
            }
            HubCall::ProposeWorkerProfile { .. } => return Err(NOT_AN_ORCHESTRATOR.into()),
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
            HubCall::StuckBoxes => return json(boxes::stuck_boxes(self, account).await?),
            HubCall::UnclaimedEnds => return json(self.db.unclaimed_ends(account.id).await?),
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
            HubCall::DecideConfirmation { id, approve, entered, reason } => return crate::surface::decide(self, account, &id, approve, entered, reason).await,
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
            HubCall::Reports { ask } => return self.reports_ask(account, ask, false).await,
            HubCall::ReportIssue { draft, shown, draft_id, request_id } => return self.report_issue(account, draft, shown, draft_id, request_id, false).await,
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
            // A team op runs only on the app's own path, `http::client_frame`, which reads the op
            // and checks the unlock on it first; a team box's ask goes through `teams::machine_call`.
            // An agent's act, a team's own call or any other path that lands here runs none.
            HubCall::Team { .. } => return Err(TEAM_CALL_ELSEWHERE.into()),
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
        // A box whose size this hub can't read bills no box time, and the records are still kept.
        let boxed = match self.db.box_made(account.id, id).await? {
            Some((created, Ok(size))) => Some((created, size)),
            Some((_, Err(e))) => {
                eprintln!("pinned-server: machine {machine}'s task records are kept with no box time: {e}");
                None
            }
            None => None,
        };
        // Each task is billed the box time since the one that ended before it.
        records.sort_by_key(|record| record.ended);
        for mut record in records {
            record.machine = machine.to_owned();
            record.machine_name = name.clone();
            let billed = self.db.task_record(account.id, id, &record.id).await?.filter(|old| old.usage.box_ms.is_some());
            let usd = |ms: u64, size| ms as f64 / 3_600_000.0 * box_usd_per_hour(size);
            // A record sent again keeps the figure it was billed, at the rate of its box's size.
            let bill = match (billed, boxed) {
                (Some(old), boxed) => old.usage.box_ms.map(|ms| (ms, old.usage.box_usd.or_else(|| boxed.map(|(_, size)| usd(ms, size))))),
                (None, Some((created, size))) => {
                    let before = self.db.billed_box_ms(account.id, id, &record.id).await?;
                    let ms = record.ended.saturating_sub(created).saturating_sub(before);
                    Some((ms, Some(usd(ms, size))))
                }
                (None, None) => None,
            };
            if let Some((ms, box_usd)) = bill {
                record.target = RunTarget::UpstashBox;
                record.usage.box_ms = Some(ms);
                record.usage.box_usd = box_usd;
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

    /// `HubCall::ProposeWorkerProfile` from `machine`, which it takes only when `tab` is a tab that
    /// machine reports as a Captain or the global orchestrator.
    pub async fn propose_worker_profile(&self, account: &Account, machine: &str, tab: &str, profile: pinned_core::profiles::WorkerProfile) -> Result<Value, String> {
        let orchestrates = |t: &TabView| t.id == tab && (t.orchestrator_global || t.role == Some(pinned_core::agent::Role::Orchestrator));
        if !account.tabs_of(machine).is_some_and(|tabs| tabs.iter().any(orchestrates)) {
            return Err(NOT_AN_ORCHESTRATOR.into());
        }
        self.change_tastes(account, |tastes| tastes.propose_worker_profile(profile)).await
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
    /// or tasks read leaves them out rather than failing the settings every app loads, and the push of them.
    async fn settings(&self, account: &Account) -> Result<pinned_core::connectors::Settings, String> {
        let mut settings = self.connectors(account).await?.get_settings();
        settings.ideas = self.db.ideas(account.id, None).await.unwrap_or_else(|e| {
            eprintln!("pinned-server: could not read the ideas of account {}: {e}", account.id);
            Vec::new()
        });
        settings.schedules = self.db.tasks(account.id).await.unwrap_or_else(|e| {
            eprintln!("pinned-server: could not read the scheduled tasks of account {}: {e}", account.id);
            Vec::new()
        });
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
                if task.enabled && task.next_at.is_none() {
                    return Err("that time has already passed: edit the task to pick a time that is still ahead".into());
                }
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
                // An agent's text is stored with secrets masked; the user's own save is stored as typed.
                if agent {
                    task.title = surface::mask_secrets(&task.title);
                    if let schedule::Action::Prompt { text } = &mut task.action {
                        *text = surface::mask_secrets(text);
                    }
                    for approval in &mut task.approvals {
                        approval.must_contain = surface::mask_secrets(&approval.must_contain);
                    }
                }
                // Changing what a task does or where it goes takes its approvals away: the user grants them again.
                if let Some(before) = &existing {
                    if (before.action != task.action || before.target != task.target) && before.approvals == task.approvals {
                        task.approvals.clear();
                    }
                }
                if agent {
                    schedule::agent_floor(&task)?;
                }
                schedule::check_save(&task, protected)?;
                if task.enabled {
                    task.next_at = schedule::next_after(&task.schedule, &task.tz, now)?;
                    if task.next_at.is_none() {
                        return Err("that time has already passed: pick a time that is still ahead".into());
                    }
                }
                let next = schedule::next_run_times(&task.schedule, &task.tz, now)?;
                let created = agent && existing.is_none();
                if created {
                    // The notice rule: a task an agent creates is active at once, and the user sees an Inbox row.
                    task.fyi = Some(now);
                } else if !agent {
                    task.fyi = None;
                }
                if dry_run {
                    return json(SaveAnswer { task: None, next });
                }
                if created {
                    // Counted and stored in one locked step, so two saves at once never pass the cap together.
                    if let Err(held) = self.db.add_agent_task(account.id, &task, schedule::AGENT_MAX_TASKS).await? {
                        return Err(format!("agents may hold at most {} tasks and {} exist: delete one first ({})", schedule::AGENT_MAX_TASKS, held.len(), held.join(", ")));
                    }
                } else if !self.db.save_task(account.id, &task).await? {
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
        let online = Online { socket, name, out, pending: HashMap::new(), next_call: 1, tabs: None, usage: BTreeMap::new(), dedicated, version, state: None, background: None, closing: HashSet::new(), gone: Vec::new(), sync_heard: HashSet::new() };
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

    /// Whether the session in tab `tab` of the online `machine` itself runs the connector on `subject`.
    pub fn runs_in_tab(&self, connector: &str, subject: &Subject, machine: &str, tab: &str) -> bool {
        let live = self.live.lock().unwrap();
        live.machines
            .get(machine)
            .is_some_and(|m| m.tabs().iter().any(|t| t.id == tab && t.running && t.connector.as_deref() == Some(connector) && t.subject.as_ref() == Some(subject)))
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

    /// Every project any machine of the account listed since the hub started.
    pub(crate) fn listed_projects(&self) -> Vec<ProjectView> {
        self.projects.lock().unwrap().values().flatten().cloned().collect()
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

    /// How the session for `connector` and `subject` on `machine` last stood: its open tab's, else
    /// the one its tab had when it went. `Unknown` while the machine is offline.
    pub fn last_turn(&self, machine: &str, connector: &str, subject: &Subject) -> LastTurn {
        let live = self.live.lock().unwrap();
        let Some(m) = live.machines.get(machine) else { return LastTurn::Unknown };
        if let Some(t) = m.tabs().iter().find(|t| t.connector.as_deref() == Some(connector) && t.subject.as_ref() == Some(subject)) {
            return last_turn(t);
        }
        m.gone.iter().find(|(c, s, _)| c == connector && s == subject).map_or(LastTurn::Unknown, |(_, _, turn)| *turn)
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

    /// What a worker's box shows: `Starting` while a session runs and no tab has reported a
    /// conversation; else as `seen_any`.
    pub fn seen_worker(&self, machine: &str) -> Seen {
        let live = self.live.lock().unwrap();
        let Some(m) = live.machines.get(machine) else { return Seen::Offline };
        match m.tabs() {
            [] => Seen::NoTab,
            tabs if tabs.iter().any(|t| t.running) && tabs.iter().all(|t| t.session.is_none()) => Seen::Starting,
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
        if let Call::CloseTab { tab } = &call {
            m.closing.insert(tab.clone());
        }
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
            MachineEvent::Tabs { tabs } => {
                let gone_tabs = m.tabs.iter().flatten().filter(|was| !tabs.iter().any(|t| t.id == was.id));
                for was in gone_tabs.filter(|_| self.team) {
                    let by_person = m.closing.remove(&was.id);
                    let (Some(connector), Some(subject)) = (&was.connector, &was.subject) else { continue };
                    let turn = match last_turn(was) {
                        LastTurn::Unknown if by_person => LastTurn::ClosedByPerson,
                        turn => turn,
                    };
                    m.gone.retain(|(c, s, _)| !(c == connector && s == subject));
                    m.gone.push((connector.clone(), subject.clone(), turn));
                }
                m.tabs = Some(tabs.clone());
            }
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

    /// The live socket of `machine`, when it is online.
    pub fn socket_of(&self, machine: &str) -> Option<u64> {
        self.live.lock().unwrap().machines.get(machine).map(|online| online.socket)
    }

    /// `machine`'s socket asked for `project`'s changes, so it hears of every later write. A socket
    /// that has since reconnected does not: its set starts empty. Called before the hub reads.
    pub fn hears_sync(&self, machine: &str, socket: u64, project: &str) {
        let mut live = self.live.lock().unwrap();
        if let Some(online) = live.machines.get_mut(machine).filter(|online| online.socket == socket) {
            online.sync_heard.insert(project.to_owned());
        }
    }

    /// Tells every socket that asked `SyncChanges` for `project` that it changed, the writer's own
    /// included. The frame is built once; the outboxes are cloned under the live lock and sent
    /// after it drops. Notices leave this process only: a machine on another hub process hears
    /// nothing until its recheck (`pinned_core::mirror::RECHECK`).
    pub fn sync_changed(&self, project: &str) {
        let notice = frame(&ToMachine::SyncChanged { project: project.to_owned() });
        let outboxes: Vec<Outbox> = {
            let live = self.live.lock().unwrap();
            live.machines.values().filter(|online| online.sync_heard.contains(project)).map(|online| online.out.clone()).collect()
        };
        for out in outboxes {
            let _ = out.send(notice.clone());
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
        live.clients.insert(id, Client { out, login, locked, unlocked, attached: HashSet::new(), threads: HashMap::new(), hears_teams: false });
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

    /// Whether client `id` must unlock before a call `owner_only` names, which it answers from
    /// whether the login is locked. A team's op asks this with `teams::owner_only`.
    pub fn needs_unlock_for(&self, id: u64, owner_only: impl FnOnce(bool) -> bool) -> bool {
        let (locked, unlocked) = match self.live.lock().unwrap().clients.get(&id) {
            Some(client) => (client.locked, client.unlocked.is_some_and(|until| until > Instant::now())),
            None => return true,
        };
        !unlocked && owner_only(locked)
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

    /// Client `id` sent a team call: from now on this connection hears its teams' events.
    pub fn hears_teams(&self, id: u64) {
        if let Some(client) = self.live.lock().unwrap().clients.get_mut(&id) {
            client.hears_teams = true;
        }
    }

    /// Sends a team's event to the app connections that sent a team call (`hears_teams`) and to
    /// no other connection; a machine never gets one.
    pub fn broadcast_team(&self, message: &ToClient) {
        let text = frame(message);
        for client in self.live.lock().unwrap().clients.values().filter(|client| client.hears_teams) {
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
struct PgStorage(Arc<Persisted>);

/// What `PgStorage` keeps of one account, which the hub reaches too for its census and to
/// flush the writes when it stops.
pub(crate) struct Persisted {
    state: Mutex<Stored>,
    keys: Mutex<BTreeMap<String, String>>,
    /// The newest store document and keys not yet written (`store_writes`).
    writes: Arc<Writes>,
    /// `Hub::refuse_run_saves`.
    refuse_runs: Arc<AtomicBool>,
    /// Where `claim_fire` claims the account's fires; none in a test of the writes alone.
    fires: Option<Fires>,
}

/// The account's row in `trigger_fires`, and the runtime its database connection runs on.
struct Fires {
    db: Db,
    account: Uuid,
    runtime: tokio::runtime::Handle,
}

/// How long `PgStorage::claim_fire` waits for the database before it answers an error. A check
/// claims its changes one at a time and stops at the first claim that answers an error, so a
/// stalled database holds a check up for one `CLAIM_WAIT` at most, within its call timeout.
const CLAIM_WAIT: Duration = Duration::from_secs(10);

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

impl PgStorage {
    fn new(db: Db, account: Uuid, stored: db::Connectors, refuse_runs: Arc<AtomicBool>) -> PgStorage {
        let fires_db = db.clone();
        let write: store_writes::Write = Arc::new(move |account, version, doc| {
            let db = db.clone();
            Box::pin(async move {
                let version = i64::try_from(version).map_err(|_| store_writes::WriteFailure::Failed("a version past i64".into()))?;
                match doc {
                    store_writes::Doc::Store(bytes) => {
                        // `save` and `save_runs` read the text through, so it is UTF-8; Postgres
                        // answers text that is not with 22021 too. The text is borrowed from
                        // `bytes`, which this write holds, not copied.
                        fn text(bytes: &[u8]) -> Result<&str, store_writes::WriteFailure> {
                            std::str::from_utf8(bytes).map_err(|_| store_writes::WriteFailure::Refused { sqlstate: "22021".into() })
                        }
                        let store = bytes.store.as_deref().map(text).transpose()?;
                        let runs = (!bytes.runs.is_empty()).then(|| text(&bytes.runs)).transpose()?;
                        if db.save_store(account, version, store, runs).await? {
                            return Ok(Landed::Written);
                        }
                        let Some((stored, _)) = db.stored_versions(account).await? else { return Ok(Landed::Gone) };
                        Ok(Landed::Behind(u64::try_from(stored).unwrap_or(0)))
                    }
                    store_writes::Doc::Secrets(secrets) => {
                        if db.save_secrets(account, version, &secrets).await? {
                            return Ok(Landed::Written);
                        }
                        let Some((_, stored)) = db.stored_versions(account).await? else { return Ok(Landed::Gone) };
                        Ok(Landed::Behind(u64::try_from(stored).unwrap_or(0)))
                    }
                }
            })
        });
        let fires = Fires { db: fires_db, account, runtime: tokio::runtime::Handle::current() };
        PgStorage::with_writer(account, stored, refuse_runs, write, Some(fires))
    }

    fn with_writer(account: Uuid, stored: db::Connectors, refuse_runs: Arc<AtomicBool>, write: store_writes::Write, fires: Option<Fires>) -> PgStorage {
        let mut doc = stored.store;
        let runs = doc.as_object_mut().and_then(|map| map.remove("runs")).and_then(|value| serde_json::to_vec(&value).ok()).unwrap_or_default();
        let store = (doc != serde_json::json!({})).then(|| serde_json::to_vec(&doc).unwrap());
        PgStorage(Arc::new(Persisted {
            state: Mutex::new(Stored { store, runs }),
            keys: Mutex::new(stored.secrets),
            writes: Writes::start(account, u64::try_from(stored.version).unwrap_or(0), write),
            refuse_runs,
            fires,
        }))
    }

    /// Queues the store and the runs as one document. Callers hold the `state` lock, so the
    /// documents take their numbers in the order the state changed and the newest one wins.
    fn persist(&self, state: &Stored) {
        self.0.writes.queue_store(state.store.clone(), state.runs.clone());
    }
}

impl Drop for PgStorage {
    fn drop(&mut self) {
        self.0.writes.close();
    }
}

impl Persisted {
    /// `store_bytes`, `runs_bytes` and what waits to be written.
    fn census(&self) -> (usize, usize, store_writes::Pending) {
        let (store, runs) = {
            let state = self.state.lock().unwrap();
            (state.store.as_ref().map_or(0, Vec::len), state.runs.len())
        };
        (store, runs, self.writes.pending())
    }
}

impl Storage for PgStorage {
    fn load(&self) -> Option<Vec<u8>> {
        self.0.state.lock().unwrap().store.clone()
    }

    fn save(&self, store: &[u8]) -> Result<(), String> {
        // Read through, without building the document; the writer sends the text as it is.
        let text = std::str::from_utf8(store).map_err(|e| e.to_string())?;
        serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text).map_err(|e| e.to_string())?;
        jsonb_takes(store, DOCUMENT_DEPTH)?;
        let mut state = self.0.state.lock().unwrap();
        state.store = Some(store.to_vec());
        self.persist(&state);
        Ok(())
    }

    fn load_runs(&self) -> Option<Vec<u8>> {
        let runs = self.0.state.lock().unwrap().runs.clone();
        (!runs.is_empty()).then_some(runs)
    }

    fn save_runs(&self, bytes: &[u8]) -> Result<(), String> {
        if self.0.refuse_runs.load(Ordering::SeqCst) {
            return Err("could not save the runs: refused".into());
        }
        if !bytes.is_empty() {
            let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
            serde_json::from_str::<serde::de::IgnoredAny>(text).map_err(|e| e.to_string())?;
            // One less: the runs sit under the store's `runs`.
            jsonb_takes(bytes, DOCUMENT_DEPTH - 1)?;
        }
        let mut state = self.0.state.lock().unwrap();
        state.runs = bytes.to_vec();
        self.persist(&state);
        Ok(())
    }

    fn key(&self, name: &str) -> Option<String> {
        self.0.keys.lock().unwrap().get(name).cloned().filter(|k| !k.is_empty())
    }

    fn write_key(&self, name: &str, key: &str) -> Result<(), String> {
        let mut keys = self.0.keys.lock().unwrap();
        keys.insert(name.to_owned(), key.to_owned());
        self.0.writes.queue_secrets(keys.clone());
        Ok(())
    }

    fn remove_key(&self, name: &str) {
        let mut keys = self.0.keys.lock().unwrap();
        if keys.remove(name).is_some() {
            self.0.writes.queue_secrets(keys.clone());
        }
    }

    /// The calls of the store that wait on Postgres: the claim must be the database's, so two
    /// hubs on it see one winner. The hub's runtime has many threads, and this one hands its
    /// other tasks to them while it waits.
    fn claim_fire(&self, workflow: &str, key: &str, claimer: &str, retry: bool) -> Result<bool, String> {
        let Some(fires) = &self.0.fires else { return Err("no database to claim the fire in".into()) };
        let claim = async {
            tokio::time::timeout(CLAIM_WAIT, fires.db.claim_fire(fires.account, workflow, key, claimer, retry))
                .await
                .map_err(|_| "could not claim the fire: the database did not answer".to_owned())?
        };
        wait_on_fires(fires, claim)
    }

    fn release_fire(&self, workflow: &str, key: &str, claimer: &str) -> Result<(), String> {
        let Some(fires) = &self.0.fires else { return Err("no database to release the fire in".into()) };
        let release = async {
            tokio::time::timeout(CLAIM_WAIT, fires.db.release_fire(fires.account, workflow, key, claimer))
                .await
                .map_err(|_| "could not give back the fire: the database did not answer".to_owned())?
        };
        wait_on_fires(fires, release)
    }
}

/// Waits for a `trigger_fires` call from a check, on the runtime of its database connection.
fn wait_on_fires<T>(fires: &Fires, call: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
    match tokio::runtime::Handle::try_current().map(|current| current.runtime_flavor()) {
        Err(_) => fires.runtime.block_on(call),
        Ok(tokio::runtime::RuntimeFlavor::CurrentThread) => Err("could not claim the fire: a single-threaded runtime cannot wait for it".into()),
        Ok(_) => tokio::task::block_in_place(|| fires.runtime.block_on(call)),
    }
}

/// One account's store and keys as one hub process keeps them, built as `Hub::connectors` builds
/// them: for a test of two hub processes that write one row.
#[doc(hidden)]
pub struct AccountStore(PgStorage);

impl AccountStore {
    pub async fn load(db: Db, account: Uuid) -> Result<AccountStore, String> {
        let stored = db.connectors(account).await?;
        Ok(AccountStore(PgStorage::new(db, account, stored, Arc::default())))
    }

    pub fn save(&self, store: &[u8]) -> Result<(), String> {
        self.0.save(store)
    }

    pub fn write_key(&self, name: &str, key: &str) -> Result<(), String> {
        self.0.write_key(name, key)
    }

    /// `Writes::flush`.
    pub async fn flush(&self, within: Duration) -> bool {
        self.0 .0.writes.flush(within).await
    }
}

/// The deepest nesting of arrays and objects the account row's document may have: what
/// serde_json reads into a `Value`, as loading the account does (`Db::connectors`). `save` and
/// `save_runs` refuse what is deeper, so a document they took is not too deep for the writer or
/// the next load. A row written some other way may still not load; the load answers an error.
pub const DOCUMENT_DEPTH: usize = 127;

/// Refuses JSON text, already read through, that jsonb would refuse though serde_json reads it:
/// nested deeper than `most`, or with a `\u0000` escape or a lone UTF-16 surrogate escape in a
/// string or a key. serde_json reads `\u0000` into a string, and skips any `\u` escape it
/// ignores; jsonb holds neither.
fn jsonb_takes(json: &[u8], most: usize) -> Result<(), String> {
    let (mut depth, mut in_string) = (0usize, false);
    // The string's last escape was a high surrogate, which a low one must follow.
    let mut high = false;
    let mut at = 0;
    while let Some(&b) = json.get(at) {
        at += 1;
        if in_string {
            let mut unit = None;
            match b {
                b'\\' if json.get(at) == Some(&b'u') => {
                    let hex = json.get(at + 1..at + 5).and_then(|hex| std::str::from_utf8(hex).ok());
                    unit = Some(hex.and_then(|hex| u16::from_str_radix(hex, 16).ok()).ok_or("a bad \\u escape")?);
                    at += 5;
                }
                b'\\' => at += 1,
                b'"' => in_string = false,
                _ => {}
            }
            if unit == Some(0) {
                return Err("a \\u0000 escape, which Postgres cannot store".into());
            }
            if high != matches!(unit, Some(0xDC00..=0xDFFF)) {
                return Err("a lone UTF-16 surrogate escape, which Postgres cannot store".into());
            }
            high = matches!(unit, Some(0xD800..=0xDBFF));
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > most {
                    return Err(format!("nested deeper than {most} levels"));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
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
    fn the_census_reads_memory_counts_and_skips_what_is_missing() {
        assert_eq!(rss_kib("Name:\tpinned-server\nVmRSS:\t  204800 kB\nVmSwap:\t0 kB\n"), Some(204800));
        assert_eq!(rss_kib("Name:\tpinned-server\n"), None);
        let stat = "anon 1048576\nfile 3145728\nanon_thp 0\n";
        assert_eq!(cgroup_bytes(stat, "anon"), Some(1 << 20));
        assert_eq!(cgroup_bytes(stat, "file"), Some(3 << 20));
        assert_eq!(cgroup_bytes("", "anon"), None);
    }

    /// The fields of a census line after its prefix, each `name=<number>`, or the failure.
    fn numeric_fields<'a>(line: &'a str, prefix: &str) -> Vec<(&'a str, i64)> {
        let rest = line.strip_prefix(prefix).unwrap_or_else(|| panic!("{line:?} starts with {prefix:?}"));
        rest.split_whitespace()
            .map(|field| {
                let (name, value) = field.split_once('=').unwrap_or_else(|| panic!("{field:?} is name=value"));
                assert!(!name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{name:?}");
                (name, value.parse().unwrap_or_else(|_| panic!("{field:?} holds a number")))
            })
            .collect()
    }

    #[test]
    fn the_census_fields_are_names_and_numbers_only() {
        crate::traffic::sent("client_event_output", 1500);
        crate::traffic::sent("client_event_output", 500);
        crate::traffic::received("machine_reply", 7);
        crate::traffic::sync_pulled(4096);
        let line = format!("pinned-server: census{}{}", crate::heap::census(), crate::traffic::take().fields());
        let fields: HashMap<&str, i64> = numeric_fields(&line, "pinned-server: census").into_iter().collect();
        for name in ["heap_live_mb", "heap_big_live_mb", "heap_allocated_gb", "sent_n", "sent_bytes", "sent_max", "recv_n", "recv_bytes", "recv_max", "sync_pulls", "sync_pull_bytes"] {
            assert!(fields.contains_key(name), "{name} in {line}");
        }
        // Other tests may send frames meanwhile, so these are at least what this one counted.
        assert!(fields["sent_client_event_output_n"] >= 2 && fields["sent_client_event_output_bytes"] >= 2000 && fields["sent_client_event_output_max"] >= 1500, "{line}");
        assert!(fields["recv_machine_reply_n"] >= 1 && fields["sync_pull_bytes"] >= 4096, "{line}");
        let pending = store_writes::Pending { store: 1, store_bytes: 1234, secrets: 0, secrets_bytes: 0 };
        let id = Uuid::from_u128(7);
        let account = account_census(id, 3, (100, 200, pending), check_census(None, 42), false);
        let fields = numeric_fields(&account, &format!("pinned-server: census-account id={id}"));
        assert_eq!(
            fields,
            vec![("runs", 3), ("runs_bytes", 200), ("store_bytes", 100), ("store_pending", 1), ("store_pending_bytes", 1234), ("secrets_pending", 0), ("secrets_pending_bytes", 0), ("check_age_s", -1), ("check_failing", 0), ("watching", 0)]
        );
    }

    #[test]
    fn the_account_census_says_how_old_the_last_check_is_and_whether_it_failed() {
        let id = Uuid::from_u128(7);
        let pending = || store_writes::Pending { store: 0, store_bytes: 0, secrets: 0, secrets_bytes: 0 };
        let failed = check_at("1970-01-01T00:00:00+00:00", Some("Linear answered 500"));
        assert_eq!(check_census(Some(&failed), 42), (42, true));
        assert_eq!(check_census(None, 42), (-1, false));
        let line = account_census(id, 3, (100, 200, pending()), check_census(Some(&failed), 42), true);
        assert!(line.ends_with(" check_age_s=42 check_failing=1 watching=1"), "{line}");
        let line = account_census(id, 3, (100, 200, pending()), check_census(None, 42), false);
        assert!(line.ends_with(" check_age_s=-1 check_failing=0 watching=0"), "{line}");
    }

    /// A check that ran at `at`, which the log does not read, with `error`.
    fn check_at(at: &str, error: Option<&str>) -> pinned_core::connectors::Check {
        pinned_core::connectors::Check { at: at.into(), error: error.map(Into::into) }
    }

    #[test]
    fn the_check_log_names_late_checks_and_changes_in_the_error_only() {
        let id = Uuid::from_u128(0x1234_5678_9abc_def0_1111_2222_3333_4444);
        let log = |prev: &CheckLog, error: Option<&str>, now: i64| log_check(prev, id, &check_at("", error), now);
        let (state, lines) = log(&CheckLog::default(), None, 1000);
        assert!(lines.is_empty(), "{lines:?}");
        let (state, lines) = log(&state, None, 1030);
        assert!(lines.is_empty(), "{lines:?}");
        let (state, lines) = log(&state, None, 1180);
        assert_eq!(lines, ["pinned-server: check-late account=12345678 gap_s=150"]);
        let (state, lines) = log(&state, Some("Linear answered 500"), 1240);
        assert_eq!(lines, ["pinned-server: check-error account=12345678 class=linear parts=1 fp=1d8aa235"]);
        let (state, lines) = log(&state, Some("Linear answered 500"), 1300);
        assert!(lines.is_empty(), "{lines:?}");
        let (state, lines) = log(&state, Some("Slack answered 403"), 1360);
        assert_eq!(lines, ["pinned-server: check-error account=12345678 class=slack parts=1 fp=7c9e905a"]);
        // The failure began at 1240, and a changed error does not start the count again.
        let (_, lines) = log(&state, None, 1450);
        assert_eq!(lines, ["pinned-server: check-ok account=12345678 after_s=210"]);

        // A repeated error lasts from its first check, so the recovery says 120 s here.
        let (state, _) = log(&CheckLog::default(), Some("Linear answered 500"), 100);
        let (state, _) = log(&state, Some("Linear answered 500"), 150);
        let (_, lines) = log(&state, None, 220);
        assert_eq!(lines, ["pinned-server: check-ok account=12345678 after_s=120"]);

        // The line carries no error text, whatever the error holds: a query string or a newline.
        let (_, lines) = log(&CheckLog::default(), Some("Bad\nthing?token=x&more"), 0);
        assert_eq!(lines, ["pinned-server: check-error account=12345678 class=other parts=1 fp=168b78fb"]);
        let long = "x".repeat(500);
        let (_, lines) = log(&CheckLog::default(), Some(&long), 0);
        assert_eq!(lines, ["pinned-server: check-error account=12345678 class=other parts=1 fp=c38c2bf3"]);
    }

    #[test]
    fn a_check_error_line_names_its_class_and_never_its_text() {
        let id = Uuid::from_u128(0x1234_5678_9abc_def0_1111_2222_3333_4444);
        let error = "workflow \"Workflow Alpha\" names a step \"Step Beta\" it does not have; could not open a session in /home/u/acme-site: no tab took it";
        let (_, lines) = log_check(&CheckLog::default(), id, &check_at("", Some(error)), 0);
        assert_eq!(lines, ["pinned-server: check-error account=12345678 class=workflow parts=2 fp=58bf650f"]);
        for text in ["Workflow Alpha", "Step Beta", "/home/u/acme-site", "names a step"] {
            assert!(!lines[0].contains(text), "{lines:?} holds {text:?}");
        }
    }

    #[test]
    fn a_changed_error_text_logs_again_and_the_same_text_does_not() {
        let id = Uuid::from_u128(0x1234_5678_9abc_def0_1111_2222_3333_4444);
        let log = |prev: &CheckLog, error: Option<&str>, now: i64| log_check(prev, id, &check_at("", error), now);
        let (state, lines) = log(&CheckLog::default(), Some("Linear answered 500"), 0);
        assert_eq!(lines, ["pinned-server: check-error account=12345678 class=linear parts=1 fp=1d8aa235"]);
        let (state, lines) = log(&state, Some("Linear answered 500"), 60);
        assert!(lines.is_empty(), "{lines:?}");
        let (state, lines) = log(&state, Some("Linear answered 503"), 120);
        assert_eq!(lines, ["pinned-server: check-error account=12345678 class=linear parts=1 fp=b616c532"]);
        let (_, lines) = log(&state, Some("Linear answered 503"), 180);
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn a_check_error_logs_when_its_kind_changes_and_every_30_minutes_while_it_lasts() {
        let id = Uuid::from_u128(0x1234_5678_9abc_def0_1111_2222_3333_4444);
        let log = |prev: &CheckLog, error: Option<&str>, now: i64| log_check(prev, id, &check_at("", error), now);
        let starts = |lines: &[String], head: &str| lines.len() == 1 && lines[0].starts_with(head);
        // Ten checks of one kind whose texts differ (a request id each) make one line.
        let mut state = CheckLog::default();
        let mut printed = vec![];
        for n in 0..10 {
            let (next, lines) = log(&state, Some(&format!("Linear answered 500: request req-{n:04}")), n * 60);
            printed.extend(lines);
            state = next;
        }
        assert!(starts(&printed, "pinned-server: check-error account=12345678 class=linear "), "{printed:?}");
        // Another status of the same connector is another kind, and so is another connector.
        let (state, lines) = log(&state, Some("Linear answered 503: request req-0010"), 600);
        assert!(starts(&lines, "pinned-server: check-error account=12345678 class=linear "), "{lines:?}");
        let (state, lines) = log(&state, Some("Slack answered 503"), 660);
        assert!(starts(&lines, "pinned-server: check-error account=12345678 class=slack "), "{lines:?}");
        // The same kind that lasts prints once more after 30 minutes, and not before.
        let (mut state, mut printed) = (state, vec![]);
        for minute in 12..=45 {
            let (next, lines) = log(&state, Some(&format!("Slack answered 503: retry after {minute}")), minute * 60);
            printed.extend(lines.into_iter().map(|line| (minute, line)));
            state = next;
        }
        assert_eq!(printed.iter().map(|(minute, _)| *minute).collect::<Vec<_>>(), [41], "{printed:?}");
        // The recovery prints once, counted from the first failure.
        let (state, lines) = log(&state, None, 46 * 60);
        assert_eq!(lines, ["pinned-server: check-ok account=12345678 after_s=2760"]);
        let (_, lines) = log(&state, None, 47 * 60);
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn check_ok_counts_from_the_first_failure() {
        let id = Uuid::from_u128(0x1234_5678_9abc_def0_1111_2222_3333_4444);
        let log = |prev: &CheckLog, error: Option<&str>, now: i64| log_check(prev, id, &check_at("", error), now);
        let (state, _) = log(&CheckLog::default(), Some("Linear answered 500"), 0);
        let (state, _) = log(&state, Some("Slack answered 403"), 60);
        let (_, lines) = log(&state, None, 150);
        assert_eq!(lines, ["pinned-server: check-ok account=12345678 after_s=150"]);
    }

    /// An account the hub knows, its connectors as given and watched or not.
    fn account_with(id: Uuid, connectors: Option<Connectors>, watching: bool) -> Account {
        Account { id, email: String::new(), team: false, live: Mutex::default(), connectors: OnceCell::new_with(connectors.map(Arc::new)), watching: AtomicBool::new(watching), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() }
    }

    /// Connectors whose `runs.json` does not parse, so they hold a failed check from the start.
    fn connectors_with_broken_runs() -> (tempfile::TempDir, Connectors) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("runs.json"), b"{not json").unwrap();
        let connectors = Connectors::new(pinned_core::connectors::FileStorage::new(dir.path().to_path_buf()), String::new());
        (dir, connectors)
    }

    #[test]
    fn the_census_says_whether_the_account_is_watched() {
        let pending = || store_writes::Pending { store: 0, store_bytes: 0, secrets: 0, secrets_bytes: 0 };
        let (id, other) = (Uuid::from_u128(7), Uuid::from_u128(8));
        let empty = tempfile::tempdir().unwrap();
        let connectors = || Connectors::new(pinned_core::connectors::FileStorage::new(empty.path().to_path_buf()), String::new());
        let accounts: HashMap<Uuid, Arc<Account>> = HashMap::from([(id, Arc::new(account_with(id, Some(connectors()), false))), (other, Arc::new(account_with(other, Some(connectors()), true)))]);
        let lines = census_lines(vec![(id, (0, 0, pending())), (other, (0, 0, pending()))], &accounts, 42);
        assert!(lines[0].ends_with(" check_age_s=-1 check_failing=0 watching=0"), "{lines:?}");
        assert!(lines[1].ends_with(" check_age_s=-1 check_failing=0 watching=1"), "{lines:?}");
    }

    #[test]
    fn the_census_line_reads_the_last_check_its_connectors_kept() {
        let pending = store_writes::Pending { store: 0, store_bytes: 0, secrets: 0, secrets_bytes: 0 };
        let id = Uuid::from_u128(7);
        let (_dir, connectors) = connectors_with_broken_runs();
        let at = chrono::DateTime::parse_from_rfc3339(&connectors.last_check().unwrap().at).unwrap().timestamp();
        let accounts: HashMap<Uuid, Arc<Account>> = HashMap::from([(id, Arc::new(account_with(id, Some(connectors), true)))]);
        let lines = census_lines(vec![(id, (0, 0, pending))], &accounts, at + 42);
        assert_eq!(lines, ["pinned-server: census-account id=00000000-0000-0000-0000-000000000007 runs=0 runs_bytes=0 store_bytes=0 store_pending=0 store_pending_bytes=0 secrets_pending=0 secrets_pending_bytes=0 check_age_s=42 check_failing=1 watching=1"]);
    }

    #[test]
    fn a_machine_back_before_a_client_saw_it_leave_gets_that_clients_watch() {
        let account = Account { id: Uuid::nil(), email: String::new(), team: false, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
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
        let account = Account { id: Uuid::nil(), email: String::new(), team: false, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
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

    /// Only a team's machine keeps how its gone sessions' last turns stood: a person's machine,
    /// whose sessions come and go for ever more subjects, keeps none.
    #[test]
    fn only_a_teams_machine_keeps_its_gone_sessions() {
        let gone_after = |team: bool, sessions: usize| {
            let account = Account { id: Uuid::nil(), email: String::new(), team, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
            let (out, _sent, _) = Outbox::new();
            account.came_online("m", 1, "studio".into(), false, "0.18.5".into(), out);
            for n in 0..sessions {
                let tab: TabView = serde_json::from_value(serde_json::json!({
                    "id": format!("tab-{n}"), "cwd": "/work/repo", "bypass": true, "running": false, "opened": 1_794_240_000u64,
                    "status": "done", "connector": "c1", "subject": { "issue": format!("FAKE-{n}") },
                }))
                .unwrap();
                account.machine_event("m", 1, MachineEvent::Tabs { tabs: vec![tab] });
                account.machine_event("m", 1, MachineEvent::Tabs { tabs: vec![] });
            }
            let turn = account.last_turn("m", "c1", &serde_json::from_value(serde_json::json!({ "issue": "FAKE-0" })).unwrap());
            let kept = account.live.lock().unwrap().machines["m"].gone.len();
            (kept, turn)
        };
        assert_eq!(gone_after(false, 100), (0, LastTurn::Unknown), "a person's machine keeps none");
        assert_eq!(gone_after(true, 3), (3, LastTurn::Ended), "a team's box keeps each session's last turn");
    }

    #[test]
    fn a_call_cut_off_by_its_machine_going_offline_is_lost_not_refused() {
        let account = Account { id: Uuid::nil(), email: String::new(), team: false, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
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
        let account = Account { id: Uuid::nil(), email: String::new(), team: false, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
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
        assert_eq!(boxes::verdict(Duration::ZERO, seen(), &mut timers, start, &policy, false), None, "nothing ends it at its first look");
        boxes::verdict(age, seen(), &mut timers, start + held, &policy, false)
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

    /// A box's account with one machine showing `tabs`.
    fn box_with(tabs: Vec<serde_json::Value>) -> Account {
        let account = Account { id: Uuid::nil(), email: String::new(), team: false, live: Mutex::default(), connectors: OnceCell::new(), watching: AtomicBool::new(false), updates: Mutex::default(), projects: Mutex::default(), tastes: Default::default(), marks: Default::default(), task_records: Default::default(), repo_reads: Default::default() };
        let (out, _sent, _) = Outbox::new();
        account.came_online("box", 1, "box".into(), true, "0.18.7".into(), out);
        let tabs = tabs.into_iter().map(|tab| serde_json::from_value(tab).unwrap()).collect();
        account.machine_event("box", 1, MachineEvent::Tabs { tabs });
        account
    }

    #[test]
    fn a_worker_box_whose_session_reported_no_conversation_is_starting() {
        let tab = |id: &str, status: Option<AgentStatus>, session: Option<&str>, running: bool| {
            serde_json::json!({ "id": id, "cwd": "/work/repo", "bypass": true, "running": running, "opened": 1, "status": status, "session": session, "connector": "delegate" })
        };
        assert_eq!(box_with(vec![tab("tab-1", None, None, true)]).seen_worker("box"), Seen::Starting, "a session that reported no conversation");
        assert_eq!(box_with(vec![tab("tab-1", Some(AgentStatus::Working), None, true)]).seen_worker("box"), Seen::Starting, "a tab that reads working with no turn reported");
        assert_eq!(box_with(vec![tab("tab-1", Some(AgentStatus::Working), Some("s-1"), true)]).seen_worker("box"), Seen::Tab { running: true, quiet: false }, "a worker at work");
        assert_eq!(box_with(vec![tab("tab-1", Some(AgentStatus::Done), Some("s-1"), true)]).seen_worker("box"), Seen::Tab { running: true, quiet: true }, "a worker done with its turn");
        assert_eq!(box_with(vec![tab("tab-1", None, None, false)]).seen_worker("box"), Seen::Tab { running: false, quiet: true }, "an exited session ends as exited");
        let two = box_with(vec![tab("tab-1", Some(AgentStatus::Done), Some("s-1"), true), tab("tab-2", None, None, true)]);
        assert_eq!(two.seen_worker("box"), Seen::Tab { running: true, quiet: true }, "a started worker with a thread of its own still starting");
        assert_eq!(box_with(vec![]).seen_worker("box"), Seen::NoTab);
        assert_eq!(box_with(vec![]).seen_worker("elsewhere"), Seen::Offline);
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

    #[tokio::test]
    async fn saves_through_storage_while_postgres_stalls_hold_one_document_and_the_newest_lands() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let written = Arc::new(Mutex::new(vec![]));
        let write: store_writes::Write = {
            let (gate, written) = (gate.clone(), written.clone());
            Arc::new(move |_, _, doc| {
                let (gate, written) = (gate.clone(), written.clone());
                Box::pin(async move {
                    gate.acquire().await.unwrap().forget();
                    if let store_writes::Doc::Store(bytes) = doc {
                        let mut doc: Value = serde_json::from_slice(bytes.store.as_deref().unwrap_or(b"{}")).unwrap();
                        doc["runs"] = serde_json::from_slice(&bytes.runs).unwrap();
                        written.lock().unwrap().push(doc);
                    }
                    Ok(Landed::Written)
                })
            })
        };
        let stored = db::Connectors { store: serde_json::json!({ "runs": [] }), secrets: BTreeMap::new(), version: 0 };
        let storage = PgStorage::with_writer(Uuid::nil(), stored, Arc::default(), write, None);
        for n in 0..20 {
            storage.save(format!(r#"{{"n":{n}}}"#).as_bytes()).unwrap();
            storage.save_runs(format!(r#"[{{"run":{n}}}]"#).as_bytes()).unwrap();
            tokio::task::yield_now().await;
        }
        assert!(storage.save(b"[1]").is_err(), "a store that is not an object is refused before it is queued");
        assert!(storage.save_runs(b"{").is_err());
        let (store_bytes, runs_bytes, pending) = storage.0.census();
        assert_eq!((store_bytes, runs_bytes), (r#"{"n":19}"#.len(), r#"[{"run":19}]"#.len()));
        assert_eq!((pending.store, pending.store_bytes), (1, store_bytes + runs_bytes), "one document waits");
        gate.add_permits(100);
        assert!(storage.0.writes.flush(Duration::from_secs(5)).await);
        assert_eq!(written.lock().unwrap().last().unwrap(), &serde_json::json!({ "n": 19, "runs": [{ "run": 19 }] }));
    }

    #[tokio::test]
    async fn a_store_or_runs_too_deep_for_the_row_is_refused_at_save_and_one_at_the_limit_is_written_as_sent() {
        let written = Arc::new(Mutex::new(vec![]));
        let write: store_writes::Write = {
            let written = written.clone();
            Arc::new(move |_, _, doc| {
                let written = written.clone();
                Box::pin(async move {
                    if let store_writes::Doc::Store(bytes) = doc {
                        written.lock().unwrap().push((bytes.store.clone(), bytes.runs.clone()));
                    }
                    Ok(Landed::Written)
                })
            })
        };
        let stored = db::Connectors { store: serde_json::json!({}), secrets: BTreeMap::new(), version: 0 };
        let storage = PgStorage::with_writer(Uuid::nil(), stored, Arc::default(), write, None);
        let nested = |n: usize| format!("{}0{}", "[".repeat(n), "]".repeat(n));
        let store_at = |n: usize| format!(r#"{{"s":"[[\\\"{{","deep":{}}}"#, nested(n - 1));
        assert!(storage.save(store_at(200).as_bytes()).is_err(), "a store 200 deep is refused, not taken and dropped");
        assert!(storage.save_runs(nested(200).as_bytes()).is_err(), "runs 200 deep likewise");
        assert!(storage.save(store_at(DOCUMENT_DEPTH + 1).as_bytes()).is_err());
        assert!(storage.save_runs(nested(DOCUMENT_DEPTH).as_bytes()).is_err(), "runs sit one level under the store");
        assert!(storage.save(b"{\"a\":\"\xff\"}").is_err(), "text that is not UTF-8 is refused");
        assert_eq!(storage.0.writes.pending(), store_writes::Pending::default(), "nothing refused was queued");

        let (store, runs) = (store_at(DOCUMENT_DEPTH), nested(DOCUMENT_DEPTH - 1));
        storage.save(store.as_bytes()).unwrap();
        storage.save_runs(runs.as_bytes()).unwrap();
        assert!(storage.0.writes.flush(Duration::from_secs(5)).await);
        assert_eq!(written.lock().unwrap().last().unwrap(), &(Some(store.clone().into_bytes()), runs.clone().into_bytes()), "the writer sends the text as saved");
        // The row's document at the limit still reads back as a Value, as loading the account reads it.
        let mut doc: Value = serde_json::from_str(&store).unwrap();
        doc["runs"] = serde_json::from_str(&runs).unwrap();
        assert!(serde_json::from_str::<Value>(&doc.to_string()).is_ok());
    }

    /// serde_json reads `\u0000` and, through `IgnoredAny`, a lone UTF-16 surrogate, but jsonb
    /// refuses both; `save` and `save_runs` refuse them too, so the writer never drops a save they took.
    #[tokio::test]
    async fn a_nul_escape_or_a_lone_surrogate_is_refused_at_save_and_an_escaped_backslash_before_u0000_is_not() {
        let written = Arc::new(Mutex::new(vec![]));
        let write: store_writes::Write = {
            let written = written.clone();
            Arc::new(move |_, _, doc| {
                let written = written.clone();
                Box::pin(async move {
                    if let store_writes::Doc::Store(bytes) = doc {
                        written.lock().unwrap().push((bytes.store.clone(), bytes.runs.clone()));
                    }
                    Ok(Landed::Written)
                })
            })
        };
        let stored = db::Connectors { store: serde_json::json!({}), secrets: BTreeMap::new(), version: 0 };
        let storage = PgStorage::with_writer(Uuid::nil(), stored, Arc::default(), write, None);
        for store in [
            r#"{"a":"x\u0000y"}"#,
            r#"{"a\u0000":1}"#,
            r#"{"a":{"b\u0000":1}}"#,
            r#"{"a":["\\\u0000"]}"#,
            r#"{"a":"\ud800"}"#,
            r#"{"a":"\udc00"}"#,
            r#"{"a":"\uD800x"}"#,
            r#"{"a":"\ud800\u0041"}"#,
            r#"{"a":"\ud800\ud800\udc00"}"#,
            r#"{"a":{"\udbff":1}}"#,
        ] {
            assert!(serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(store).is_ok(), "serde reads {store}");
            assert!(storage.save(store.as_bytes()).is_err(), "jsonb refuses {store}, so save does");
        }
        for runs in [r#"[{"x":"\u0000"}]"#, r#"["\udfff"]"#, r#"[{"\u0000":1}]"#] {
            assert!(serde_json::from_str::<serde::de::IgnoredAny>(runs).is_ok(), "serde reads {runs}");
            assert!(storage.save_runs(runs.as_bytes()).is_err(), "jsonb refuses {runs}, so save_runs does");
        }
        assert_eq!(storage.0.writes.pending(), store_writes::Pending::default(), "nothing refused was queued");

        // An escaped backslash followed by the text u0000 is ordinary text; so is a surrogate pair.
        let store = r#"{"a":"x\\u0000y","b\\u0000":"\\\\u0000","c":"\ud83d\ude00","d":"\u00e9"}"#;
        let runs = r#"[{"x":"\\u0000","y":"\uDBFF\uDFFF"}]"#;
        storage.save(store.as_bytes()).unwrap();
        storage.save_runs(runs.as_bytes()).unwrap();
        assert!(storage.0.writes.flush(Duration::from_secs(5)).await);
        assert_eq!(written.lock().unwrap().last().unwrap(), &(Some(store.as_bytes().to_vec()), runs.as_bytes().to_vec()));
    }
}
