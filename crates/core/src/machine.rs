use crate::accounts::{self, Account, Accounts, Auth, Provider};
use crate::agent::{role_env, role_of, session_of, AgentEvent, AgentStatus, Message, ProjectTool, Role, Usage, CONNECTOR_ENV, SOCK_ENV, TAB_ENV, TASK_ENV};
use crate::stats::{ThreadStats, Tally};
use crate::wire::{Call, HubCall, MachineEvent, Program, ProjectView, RunSpec, TabId, TabView, ThreadMatch, ThreadPage, UpdateStatus, EXE_PLACEHOLDER};
use crate::global_orchestrator::{self as global, BusyPolicy, CaptainReport, Handoff, Progress};
use crate::taste::{self, TasteField, TastePatch, TasteScope, Tastes};
use crate::panel::{DeviceHub, FileServer, Shells};
use crate::{branches, commands, favicon, files, github, history, hooks, memory, model2vec, panel, plugin, process, projects, pty, pulls, reading, reminders, scaffold, scripts, sessions, skills, subagents, sync, thread, titles, uploads, usage, workspace};
use portable_pty::{CommandBuilder, PtySize};
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

const MODEL_MENU_FILE: &str = "claude-model-menu.json";

/// A tab keeps this much of its session's output, replayed to a viewer that attaches late.
pub const OUTPUT_KEEP: usize = 256 * 1024;
/// Slack after a usage limit's reset time before the session is told to continue.
/// Task records sent to the hub in one call.
const RECORDS_PER_SEND: usize = 100;
const RECORDS_RETRY: Duration = Duration::from_secs(60);
const LIMIT_MARGIN_SECS: u64 = 5;
const UNWATCHED: (u16, u16) = (120, 32);
/// Claude logs a turn's duration only after its Stop hook returns, within a second in practice.
const TURN_LOG_WAIT: (u32, Duration) = (25, Duration::from_millis(200));
const THREAD_LOOK_EVERY: Duration = Duration::from_millis(40);
const PAGE_POLL: Duration = Duration::from_millis(1200);
const LOCAL_VIEWER: &str = "local";
const NOT_LINKED: &str = "this machine is not linked to a hub";
/// How long a session's start waits for the account's taste before it takes the one `taste.json`
/// kept. A Capitão can start while a Machine lock is held, so a silent hub must not hold it long.
#[cfg(not(test))]
const TASTE_WAIT: Duration = Duration::from_secs(5);
#[cfg(test)]
const TASTE_WAIT: Duration = Duration::from_millis(500);
/// How often a remote task's status asks the machine running it, and an end that did not get
/// through is sent again.
const REMOTE_LOOK: Duration = Duration::from_secs(3);
const NO_PROJECT_WORK: &str = "the global orchestrator does no project work: hand it to the project's Capitão with send_to_captain";

pub type Event = MachineEvent;
/// Names a viewer of a tab, such as `local` for the app's own webview; attaching again replaces it.
pub type ViewerId = String;
/// Receives a tab's output. It runs on the session's reader thread with the tab's screen locked,
/// so it must not attach to or detach from that tab.
pub type Sink = Box<dyn Fn(&[u8]) + Send + Sync>;
pub type Listener = Box<dyn Fn(&Event) + Send + Sync>;

fn merge_context(recall: &str, notices: &str) -> String {
    match (recall.is_empty(), notices.is_empty()) {
        (true, true) => String::new(),
        (false, true) => recall.to_owned(),
        (true, false) => notices.to_owned(),
        (false, false) => format!("{recall}\n\n{notices}"),
    }
}

pub struct Launch {
    pub exe: PathBuf,
    /// Each provider's CLI. One left out runs as its bare name, found on PATH.
    pub bins: BTreeMap<Provider, PathBuf>,
    pub path_env: Option<String>,
    pub settings: PathBuf,
    pub mcp_config: Option<PathBuf>,
    /// Desktop sessions offer reminders. A node keeps the MCP server for delegation and leaves this false.
    pub reminders: bool,
    /// The plugin directory holding the skills that ship inside pinned, and its tutorial helper.
    pub plugin_dir: PathBuf,
    pub tutorial_helper: PathBuf,
    pub sock: PathBuf,
    /// Whether project memory also searches by meaning, with a model it downloads on first use.
    pub vector_search: bool,
}

impl Launch {
    /// Writes the settings, MCP config and plugin every session loads, for sessions that call back `exe`.
    pub fn prepare(exe: &Path) -> std::io::Result<Launch> {
        // One folder per process: the app and a node on the same Mac each point sessions at their own binary.
        // A fresh 0700 folder, since Linux shares /tmp: another user who planted a folder by the
        // same name could swap in hooks that run as this machine's user.
        let prefix = format!("pinned-{}-", std::process::id());
        let mut files = tempfile::Builder::new();
        files.prefix(&prefix);
        // Windows needs no mode: each user has a temp folder of their own.
        #[cfg(unix)]
        files.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let files = files.tempdir()?.keep();
        let settings = files.join("claude-settings.json");
        hooks::write_settings(&settings, exe)?;
        let mcp_config = files.join("claude-mcp.json");
        reminders::write_mcp_config(&mcp_config, exe, true)?;
        let plugin_dir = files.join("plugin");
        let tutorial_helper = plugin::install(&plugin_dir)?;
        let path_env = login_shell_path().or_else(|| std::env::var("PATH").ok());
        Ok(Launch {
            exe: exe.to_owned(),
            bins: Provider::ALL.into_iter().map(|p| (p, find_bin(p.spec().bin, path_env.as_deref()))).collect(),
            path_env,
            settings,
            mcp_config: Some(mcp_config),
            reminders: true,
            plugin_dir,
            tutorial_helper,
            sock: files.join("pinned.sock"),
            vector_search: true,
        })
    }

    pub fn bin(&self, provider: Provider) -> PathBuf {
        self.bins.get(&provider).cloned().unwrap_or_else(|| provider.spec().bin.into())
    }

    fn command(&self, program: Program, account: &Account, tab: &str, cwd: &Path, bypass: bool, args: Vec<String>, env: &[(String, String)], plugin_dirs: &[PathBuf]) -> CommandBuilder {
        let provider = account.provider;
        let spec = provider.spec();
        let mut cmd = CommandBuilder::new(self.bin(provider));
        for key in INHERITED_SESSION_ENV {
            cmd.env_remove(key);
        }
        if account.auth == Auth::OpenCodeGo {
            for key in crate::opencode_go::STRIP_ENV {
                cmd.env_remove(key);
            }
        }
        match program {
            Program::Agent => {
                if spec.harness {
                    if let Some(mcp_config) = &self.mcp_config {
                        // --mcp-config takes any number of values, so --settings right after it keeps it from swallowing the rest.
                        cmd.arg("--mcp-config");
                        cmd.arg(mcp_config);
                    }
                    cmd.arg("--settings");
                    cmd.arg(&self.settings);
                    cmd.arg("--plugin-dir");
                    cmd.arg(&self.plugin_dir);
                    for dir in plugin_dirs {
                        cmd.arg("--plugin-dir");
                        cmd.arg(dir);
                    }
                } else if provider == Provider::Codex {
                    if self.mcp_config.is_some() {
                        cmd.args(crate::delegate::codex_mcp_flags(&self.exe, &self.sock, Some(tab), None, self.reminders));
                    }
                    let home = env
                        .iter()
                        .find(|(key, _)| key == "CODEX_HOME")
                        .map(|(_, value)| PathBuf::from(value))
                        .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
                        .unwrap_or_else(|| home().join(".codex"));
                    let user = crate::delegate::codex_user_notify(&std::fs::read_to_string(home.join("config.toml")).unwrap_or_default());
                    cmd.args(crate::delegate::codex_notify_flag(&self.exe, &user));
                }
                if bypass {
                    cmd.arg(spec.bypass_flag);
                }
                cmd.args(args);
            }
            Program::SignIn => cmd.args(spec.login),
        }
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env(SOCK_ENV, &self.sock);
        cmd.env(TAB_ENV, tab);
        cmd.env(plugin::HELPER_ENV, &self.tutorial_helper);
        for (k, v) in env {
            cmd.env(k, v);
        }
        if let Some(path) = &self.path_env {
            cmd.env("PATH", path);
        }
        cmd
    }
}

/// What a Claude Code session or a parent shell can leak into a pinned session. pinned strips
/// them before it applies the account's own env, so a nested Claude session is not mistaken for
/// this one and a stale Go marker cannot make a login account report Go usage.
pub(crate) const INHERITED_SESSION_ENV: [&str; 12] = [
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SSE_PORT",
    "PINNED_OPENCODE_GO_KEY",
];

/// The environment a headless `claude -p` gets, and the variables it must drop before applying it.
pub struct Headless<'a> {
    pub env: &'a [(String, String)],
    pub strip: &'a [&'a str],
}

#[derive(Default)]
struct Screen {
    kept: VecDeque<u8>,
    viewers: Vec<(ViewerId, Sink)>,
    /// Counts starts, so a replaced session's last bytes can't land on its successor's screen.
    session: u64,
}

impl Screen {
    fn restart(&mut self) -> u64 {
        self.kept.clear();
        self.session += 1;
        self.session
    }

    fn push(&mut self, session: u64, bytes: &[u8]) {
        if session != self.session {
            return;
        }
        self.kept.extend(bytes);
        let excess = self.kept.len().saturating_sub(OUTPUT_KEEP);
        self.kept.drain(..excess);
        for (_, sink) in &self.viewers {
            sink(bytes);
        }
    }

    /// Replays the screen to `sink`, then keeps it for live output. Holding the lock across both
    /// means no byte is missed or sent twice.
    fn attach(&mut self, viewer: ViewerId, sink: Sink) {
        self.detach(&viewer);
        if !self.kept.is_empty() {
            sink(self.kept.make_contiguous());
        }
        self.viewers.push((viewer, sink));
    }

    fn detach(&mut self, viewer: &str) {
        self.viewers.retain(|(v, _)| v != viewer);
    }
}

/// A child `delegate_task` starts from the tab `from`, as the worker thread that shows it.
/// Who a new child is for: a tab or a task here, or a tab on another machine (`origin`), with
/// what it inherits.
struct ChildParent {
    parent: crate::delegate::Parent,
    caller_key: String,
    client_request_id: Option<String>,
    /// The tab here that asked, which must stay open.
    tab: Option<TabId>,
    origin: Option<crate::delegate::Origin>,
    account: Option<accounts::AccountId>,
    inherited_model: Option<String>,
    provider: Provider,
    auth: Auth,
    cwd: PathBuf,
    ceiling: crate::delegate::Access,
    limits: taste::Limits,
    /// Workers at once in the project, when this machine counts them.
    parallel: Option<u32>,
    delegator: crate::delegate::Delegator,
}

/// `delegate_task`'s answer when the project already runs as many workers as it allows.
fn workers_full(working: usize) -> String {
    let working = if working == 1 { "1 worker already runs".to_owned() } else { format!("{working} workers already run") };
    format!("{working} in this project, the most it allows at once. Wait for one to finish, or raise Workers at once in Settings › Way of working.")
}

struct ThreadChild<'a> {
    from: &'a str,
    /// The tab on another machine the thread is for. `from` then names no tab here.
    origin: Option<crate::delegate::Origin>,
    task: &'a str,
    title: &'a str,
    cwd: &'a Path,
    /// The worktree the thread owns and drops when it closes.
    made: Option<PathBuf>,
    model: Option<String>,
}

#[derive(Clone, Copy)]
struct BackgroundWait {
    since: Instant,
    shells_only: bool,
}

struct Tab {
    cwd: PathBuf,
    account: Account,
    program: Program,
    session: Option<pty::Session>,
    starting: bool,
    /// When the running claude started, so a subagent an earlier claude left mid-step reads as stopped.
    started: Option<SystemTime>,
    /// Extra `claude` arguments for the first start only, so a restart doesn't redo the task.
    launch: Vec<String>,
    /// Environment for every start, such as the issue a connector's session reports on.
    env: Vec<(String, String)>,
    /// Every start runs claude with `--dangerously-skip-permissions`, so nothing asks or is blocked.
    bypass: bool,
    /// Claude Code's id for the conversation in progress, known once a hook reports it.
    claude_session: Option<String>,
    /// `claude_session`'s transcript as read so far, which starts over whenever the session changes.
    tally: Tally,
    last_found_transcript: Option<PathBuf>,
    /// A turn is running, or died on an error: the session has work left if pinned quits now.
    interrupted: bool,
    /// Unix time a usage limit that stopped the turn lifts; the session then continues by itself.
    resume_at: Option<u64>,
    /// The session doesn't continue by itself when its usage limit lifts.
    held_at_limit: bool,
    /// The model every start of this tab passes as `--model`, picked for its conversation alone.
    model: Option<String>,
    /// The worktree made for this tab by `set_workspace`, dropped when the tab leaves it unused.
    made: Option<PathBuf>,
    /// The tab that started this worker with `delegate_task`, or whose delegated task this tab
    /// reopened. A worker's news comes back here first, and goes to the project's orchestrator
    /// once this tab is closed.
    spawned_by: Option<TabId>,
    /// The tab on another machine that delegated this worker here. Its end goes back through the
    /// hub, and its tab id is never looked up among this machine's tabs.
    origin: Option<crate::delegate::Origin>,
    /// The `delegate_task` task this worker thread shows. Its first finished turn ends the task
    /// when the tab runs it; a headless child's thread follows the task instead.
    task: Option<String>,
    /// Working tree and prompt as the running turn began, to diff against when it ends.
    turn: Option<(String, Option<String>)>,
    status: Option<AgentStatus>,
    updated: Option<u64>,
    title: Option<String>,
    message: Option<String>,
    /// What the last finished turn asked the user, until the next turn starts or fails.
    question: Option<String>,
    /// Prompts typed before the session took the terminal, typed in once it does.
    held: Vec<String>,
    /// When the latest submit went in. Cleared by Done, Failed, and a new session.
    turn_pending: Option<Instant>,
    /// Set while a worker's turn has ended with background work it started still pending: Claude
    /// Code resumes the session when that work hands back, so the turn end is not the worker's end.
    background_wait: Option<BackgroundWait>,
    /// The terminal's input line. Done does not clear it.
    draft: Draft,
    owes_reload: bool,
    /// Claude's permission mode from the latest hook that reported one. `plan` is read-only.
    permission_mode: Option<String>,
    /// Shift+Tab changed the mode and no hook has reported the new one. A child stays read-only.
    permission_unknown: bool,
    /// Shared with the session's reader thread, which never takes the tabs lock. Never lock it
    /// while holding the tabs lock: a sink may call back into the machine.
    screen: Arc<Mutex<Screen>>,
}

impl Tab {
    /// What reopens the tab's conversation, once claude has reported one.
    /// A sign-in tab has no conversation to resume.
    fn saved(&self) -> Option<sessions::Saved> {
        if self.program != Program::Agent {
            return None;
        }
        Some(sessions::Saved {
            cwd: self.cwd.clone(),
            session: self.claude_session.clone()?,
            env: self.env.clone(),
            bypass: self.bypass,
            interrupted: self.interrupted,
            resume_at: self.resume_at,
            held: self.held_at_limit,
            account: self.account.id.clone(),
            model: self.model.clone(),
            permission_mode: self.permission_mode.clone().filter(|_| !self.permission_unknown),
            spawned_by: None,
        })
    }

    /// What a child of this tab may do. Shift+Tab leaves the mode unknown until a hook reports it,
    /// and an unknown mode is read-only.
    fn child_access(&self) -> crate::delegate::Access {
        if self.permission_unknown || self.permission_mode.as_deref() == Some("plan") {
            crate::delegate::Access::ReadOnly
        } else {
            crate::delegate::Access::derive(self.bypass, false)
        }
    }

    /// The tab as clients see it. `turns` are the projects whose Capitão runs a turn that a message
    /// from the global orchestrator started.
    fn view(&self, id: &str, turns: &HashSet<PathBuf>, queued: &HashMap<String, u32>) -> TabView {
        let role = role_of(&self.env);
        TabView {
            id: id.to_owned(),
            cwd: self.cwd.display().to_string(),
            account: self.account.id.clone(),
            program: self.program,
            bypass: self.bypass,
            running: self.session.is_some(),
            status: self.status,
            title: self.title.clone(),
            message: self.message.clone(),
            resume_at: self.resume_at,
            held: self.held_at_limit,
            connector: self.env.iter().find(|(k, _)| k == CONNECTOR_ENV).map(|(_, v)| v.clone()),
            subject: session_of(&self.env).map(|(_, subject)| subject),
            opened: tab_number(id).into(),
            updated: self.updated,
            session: self.claude_session.clone(),
            interrupted: self.interrupted,
            stats: Some(self.tally.stats()).filter(|s| *s != ThreadStats::default()),
            branch: self.tally.branch.clone(),
            pr: self.tally.pr.clone(),
            last_message_at: self.tally.last_at,
            prompt_title: self.tally.prompt_title.clone(),
            orchestrator_turn: role.as_ref().is_some_and(|(role, project)| *role == Role::Orchestrator && turns.contains(project)),
            role: role.map(|(role, _)| role),
            spawned_by: self.origin.as_ref().map(|o| o.tab.clone()).or_else(|| self.spawned_by.clone()),
            spawned_on: self.origin.as_ref().map(|o| o.machine.clone()),
            orchestrator_global: global::is_global(&self.env),
            taste_chat: taste::is_chat(&self.env),
            queued: queued.get(id).copied().unwrap_or_default(),
            question: self.question.clone(),
        }
    }

    fn notice_idle(&self) -> bool {
        if self.session.is_none() || self.turn_pending.is_some() || !self.draft.empty() {
            return false;
        }
        if self.account.provider == Provider::Cursor {
            return self.status == Some(AgentStatus::Done);
        }
        match self.status {
            Some(AgentStatus::Done) => true,
            Some(AgentStatus::NeedsInput) => !self.interrupted,
            None => self.claude_session.is_some(),
            Some(AgentStatus::Working | AgentStatus::Failed) => false,
        }
    }

    fn note_submit(&mut self) {
        self.turn_pending = Some(Instant::now());
    }

    /// Types a local slash command. Unlike a prompt it leaves `turn_pending` alone: a local
    /// command fires no Stop hook, so nothing would ever clear it (probed on Claude Code 2.1.319).
    fn type_command(&mut self, command: &str) -> bool {
        self.session.as_mut().is_some_and(|s| s.raw() && s.write(&paste(command)).is_ok())
    }

    fn reload_if_idle(&mut self) -> bool {
        let typed = self.owes_reload && self.notice_idle() && self.type_command("/reload-skills");
        if typed {
            self.owes_reload = false;
        }
        typed
    }
}

pub struct Machine {
    launch: Launch,
    default_cwd: PathBuf,
    /// The folder threads started without a project run in.
    scratch: PathBuf,
    tabs: Mutex<HashMap<TabId, Tab>>,
    /// Chosen when this process starts. Tab ids include it, so the next process cannot hand out an id this one used.
    boot: u64,
    next_tab: AtomicU32,
    sessions_file: Option<PathBuf>,
    sync_file: Option<PathBuf>,
    sync_loop_holds_state: AtomicBool,
    projects: Mutex<Vec<projects::Project>>,
    projects_file: Option<PathBuf>,
    accounts: Accounts,
    /// One file of finished turns per project.
    history_dir: Option<PathBuf>,
    /// Files clients attached, saved for sessions to read.
    uploads: Option<PathBuf>,
    /// LiteLLM's price list, downloaded at most daily for the usage page.
    rates_file: Option<PathBuf>,
    /// The model rows claude's /model menu showed last on this machine, kept in `model_menu_file`.
    model_menu: Mutex<Vec<crate::claude_models::MenuRow>>,
    model_menu_file: Option<PathBuf>,
    /// The review guides claude wrote for pull requests.
    guides: PathBuf,
    shells: Shells,
    device_hub: DeviceHub,
    file_server: FileServer,
    /// Present while new sessions may drive the machine's simulators and emulators.
    device_agents: Option<PathBuf>,
    pr_watches: crate::prwatch::Watches,
    clones: PathBuf,
    quitting: AtomicBool,
    delegate_pass_armed: AtomicBool,
    sending_records: AtomicBool,
    /// After the hub refused records, when to offer them again.
    records_retry: Mutex<Option<Instant>>,
    threads_watched: AtomicBool,
    thread_watchers: Mutex<HashMap<ViewerId, HashSet<TabId>>>,
    usage: Mutex<BTreeMap<accounts::AccountId, Usage>>,
    /// When a Go account's allowance was last asked for, so a burst of statuslines makes one request.
    usage_refreshed: Mutex<BTreeMap<accounts::AccountId, Instant>>,
    agents: Mutex<subagents::Cache>,
    delegates: crate::delegate::Delegates,
    /// Ends of tasks run here for another machine's tabs that the hub could not pass on yet.
    remote_ends: Mutex<Vec<(Instant, HubCall)>>,
    memories: Arc<memory::Memories>,
    orchestration_dir: Option<PathBuf>,
    /// Each working worker thread's spend so far, by task, across every session it ran.
    meters: Mutex<HashMap<String, usage::Meter>>,
    /// The price list the meters price with, and when it was read.
    meter_rates: Mutex<Option<(Instant, Arc<usage::RateTable>)>>,
    /// Whether the hub has seen this machine's own tastes since the link last connected, so the
    /// account can take them.
    tastes_shared: Arc<AtomicBool>,
    global: Mutex<global::Settings>,
    /// `~/.pinned/orquestrador`, the global orchestrator's folder and the key of the global memory.
    global_dir: PathBuf,
    /// Every message handed to a Capitão or to the global orchestrator here. Never locked while
    /// holding `tabs`.
    sent: Mutex<global::Ledger>,
    /// The ledger's projects in flight, which a tab's view reads while a hand-off holds `sent`.
    turns: global::Turns,
    events: Box<dyn Fn(Event) + Send + Sync>,
    /// Arcs, so `emit` can call them without holding the lock a listener might subscribe under.
    listeners: Mutex<Vec<(u64, Arc<Listener>)>>,
    next_listener: AtomicU64,
    me: Weak<Machine>,
    gate: Mutex<Gate>,
    sync_waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[allow(clippy::type_complexity)]
    hub: Mutex<Option<Arc<dyn Fn(HubCall) -> Result<Value, String> + Send + Sync>>>,
    /// This machine's id on the hub, from the hub's hello. Empty until the first sign-in.
    hub_id: Mutex<crate::wire::MachineId>,
    /// This machine's name on the hub, which the user may have changed from its hostname.
    hub_name: Mutex<String>,
}

pub trait SelfUpdate: Send + Sync {
    /// Check the release. When this build is behind, install it and arm a restart.
    /// Publish each phase through `machine.report` before returning it.
    fn install(&self, machine: &Machine) -> UpdateStatus;
}

struct Gate {
    installer: Option<Arc<dyn SelfUpdate>>,
    busy: bool,
    busy_gen: u64,
    /// Set when `install` returned `Updating` but the install is still running in the background.
    continuing: bool,
    phase: Option<UpdateStatus>,
    restart: Option<Box<dyn FnOnce() + Send>>,
    restart_gen: u64,
    flusher: Option<Arc<dyn Fn(&MachineEvent) -> bool + Send + Sync>>,
}

impl Default for Gate {
    fn default() -> Self {
        Self { installer: None, busy: false, busy_gen: 0, continuing: false, phase: None, restart: None, restart_gen: 0, flusher: None }
    }
}

thread_local! {
    static ARMED_RESTART: Cell<bool> = const { Cell::new(false) };
}

struct Busy<'a> {
    machine: &'a Machine,
    generation: u64,
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        let mut gate = self.machine.gate.lock().unwrap();
        if gate.busy_gen != self.generation {
            return;
        }
        if gate.continuing || matches!(gate.phase, Some(UpdateStatus::Restarting { .. })) {
            return;
        }
        gate.busy = false;
    }
}

#[derive(Default)]
struct TranscriptLook {
    found: Option<Transcript>,
    missed: Option<Missed>,
}

#[derive(PartialEq)]
struct Transcript {
    session: String,
    path: PathBuf,
    len: u64,
}

/// Claude writes a session's transcript with its first message, so the hook names a session the
/// folders do not hold yet. A turn that starts is when the file appears.
struct Missed {
    session: String,
    status: Option<AgentStatus>,
    at: Instant,
    gap: Duration,
}

impl Machine {
    /// A machine that keeps its sessions, projects and history in `dir`, or nothing without one,
    /// and opens new tabs on `open`, else the most recent project. Everything it reports goes to `events`.
    pub fn new(launch: Launch, dir: Option<PathBuf>, open: Option<PathBuf>, events: impl Fn(Event) + Send + Sync + 'static) -> Arc<Machine> {
        let projects_file = dir.as_ref().map(|d| d.join("projects.json"));
        let mut projects = dir
            .as_ref()
            .zip(projects_file.as_ref())
            .map(|(d, file)| projects::load_or_migrate(file, &d.join("last-folder")))
            .unwrap_or_default();
        if let Some(open) = &open {
            projects::upsert(&mut projects, open.clone(), unix_now());
            if let Some(file) = &projects_file {
                projects::save(file, &projects);
            }
        }
        let uploads = dir.as_ref().map(|d| d.join("uploads"));
        if let Some(uploads) = &uploads {
            uploads::sweep(uploads);
        }
        let models = dir.as_ref().map(|d| d.join("models")).filter(|_| launch.vector_search);
        let load = models.map(|models| -> memory::Loader { Arc::new(move || Ok(Arc::new(model2vec::Model2Vec::load(&model2vec::fetch(&models)?)?) as Arc<dyn memory::Embedder>)) });
        let memories = memory::Memories::new(dir.as_ref().map(|d| d.join("memory")), load);
        let delegates = crate::delegate::Delegates::load(dir.as_ref().map(|d| d.join("tasks.json")));
        let sent = global::Ledger::load(dir.as_deref());
        // Seeds taste.json from the old cost style, then moves the old worker cap into it.
        taste::migrate_worker_cap(dir.as_deref());
        let home = home();
        let global_dir = if test_binary() { dir.clone().unwrap_or_else(std::env::temp_dir).join("home") } else { home.clone() }.join(global::FOLDER[0]).join(global::FOLDER[1]);
        let default_cwd = open
            .or_else(|| projects::recent(&projects).first().map(|p| p.path.clone()))
            .unwrap_or_else(|| home.clone());
        Arc::new_cyclic(|me| Machine {
            launch,
            default_cwd,
            scratch: dir.as_ref().map(|d| d.join("scratch")).unwrap_or_else(|| std::env::temp_dir().join("pinned-scratch")),
            tabs: Mutex::new(HashMap::new()),
            boot: fresh_boot(),
            next_tab: AtomicU32::new(1),
            sessions_file: dir.as_ref().map(|d| d.join("sessions.json")),
            sync_file: dir.as_ref().map(|d| d.join("sync.json")),
            sync_loop_holds_state: AtomicBool::new(false),
            projects: Mutex::new(projects),
            projects_file,
            accounts: Accounts::load(dir.as_deref(), home.clone()),
            history_dir: dir.as_ref().map(|d| d.join("history")),
            uploads,
            rates_file: dir.as_ref().map(|d| d.join("usage-model-rates.json")),
            model_menu: Mutex::new(dir.as_ref().and_then(|d| std::fs::read(d.join(MODEL_MENU_FILE)).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()),
            model_menu_file: dir.as_ref().map(|d| d.join(MODEL_MENU_FILE)),
            guides: dir.as_ref().map(|d| d.join("guides")).unwrap_or_else(|| std::env::temp_dir().join("pinned-guides")),
            shells: Shells::default(),
            device_hub: DeviceHub::default(),
            file_server: FileServer::default(),
            device_agents: dir.as_ref().map(|d| d.join("device-agents")),
            pr_watches: Default::default(),
            clones: home.join("pinned"),
            quitting: AtomicBool::new(false),
            delegate_pass_armed: AtomicBool::new(false),
            sending_records: AtomicBool::new(false),
            records_retry: Mutex::new(None),
            threads_watched: AtomicBool::new(false),
            thread_watchers: Mutex::default(),
            usage: Mutex::default(),
            usage_refreshed: Mutex::default(),
            agents: Mutex::default(),
            delegates,
            remote_ends: Mutex::default(),
            memories,
            orchestration_dir: dir.clone(),
            meters: Mutex::default(),
            meter_rates: Mutex::default(),
            tastes_shared: Arc::default(),
            global: Mutex::new(global::Settings::load(dir.as_deref())),
            global_dir,
            turns: sent.turns(),
            sent: Mutex::new(sent),
            events: Box::new(events),
            listeners: Mutex::new(vec![]),
            next_listener: AtomicU64::new(1),
            me: me.clone(),
            gate: Mutex::new(Gate::default()),
            sync_waker: Mutex::new(None),
            hub: Mutex::new(None),
            hub_id: Mutex::default(),
            hub_name: Mutex::default(),
        })
    }

    pub fn launch(&self) -> &Launch {
        &self.launch
    }

    /// Sends every later event to `listener` too, such as a hub link that starts after sign in.
    /// Returns the id `unsubscribe` takes.
    pub fn subscribe(&self, listener: Listener) -> u64 {
        let id = self.next_listener.fetch_add(1, Ordering::Relaxed);
        self.listeners.lock().unwrap().push((id, Arc::new(listener)));
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.listeners.lock().unwrap().retain(|(l, _)| *l != id);
    }

    pub fn arc(&self) -> Arc<Machine> {
        self.me.upgrade().unwrap()
    }

    pub fn set_installer(&self, installer: Arc<dyn SelfUpdate>) {
        self.gate.lock().unwrap().installer = Some(installer);
    }

    pub fn set_flusher(&self, flush: Arc<dyn Fn(&MachineEvent) -> bool + Send + Sync>) {
        self.gate.lock().unwrap().flusher = Some(flush);
    }

    fn update_self(&self) -> UpdateStatus {
        let (installer, generation, busy_gen) = {
            let mut gate = self.gate.lock().unwrap();
            if gate.busy {
                return gate.phase.clone().unwrap_or(UpdateStatus::Updating { to: String::new() });
            }
            let Some(installer) = gate.installer.clone() else {
                return UpdateStatus::Cannot { reason: "this build cannot install a release".into() };
            };
            gate.busy = true;
            gate.busy_gen = gate.busy_gen.wrapping_add(1);
            (installer, gate.restart_gen, gate.busy_gen)
        };
        let _busy = Busy { machine: self, generation: busy_gen };
        let status = installer.install(self);
        let armed = self.gate.lock().unwrap().restart_gen != generation;
        ARMED_RESTART.with(|flag| flag.set(armed));
        status
    }

    pub fn finish_for_restart(&self, call: Call) -> (Result<Value, String>, bool) {
        ARMED_RESTART.with(|flag| flag.set(false));
        let result = self.call(call);
        let armed = ARMED_RESTART.with(|flag| flag.replace(false));
        (result, armed)
    }

    pub fn report(&self, status: UpdateStatus) -> bool {
        let event = MachineEvent::Update { status: Some(status.clone()) };
        let terminal = matches!(status, UpdateStatus::Failed { .. } | UpdateStatus::Cannot { .. } | UpdateStatus::UpToDate { .. });
        let flusher = {
            let mut gate = self.gate.lock().unwrap();
            gate.phase = Some(status);
            if terminal {
                gate.continuing = false;
                gate.busy = false;
                gate.busy_gen = gate.busy_gen.wrapping_add(1);
            }
            gate.flusher.clone()
        };
        match flusher {
            Some(flush) => flush(&event),
            None => {
                self.emit(event);
                true
            }
        }
    }

    /// Keeps the update gate busy after `install` returns `Updating`, until a final phase is reported.
    pub fn keep_updating(&self) {
        self.gate.lock().unwrap().continuing = true;
    }

    pub fn schedule_restart(&self, restart: Box<dyn FnOnce() + Send>) {
        let mut gate = self.gate.lock().unwrap();
        gate.restart = Some(restart);
        gate.restart_gen = gate.restart_gen.wrapping_add(1);
    }

    pub fn restart_now(&self) {
        if let Some(restart) = self.gate.lock().unwrap().restart.take() {
            restart();
        }
    }

    /// Runs the restart `install` scheduled once the release is in place.
    pub fn restart_if_replacing(&self, status: &UpdateStatus) {
        if matches!(status, UpdateStatus::Restarting { .. }) {
            self.restart_now();
        }
    }

    pub fn update_snapshot(&self) -> Option<UpdateStatus> {
        self.gate.lock().unwrap().phase.clone()
    }

    /// Answers every call a client may make, local or through the hub. `Run` may clone a
    /// repository, so call it off any async runtime's worker threads.
    pub fn call(&self, call: Call) -> Result<Value, String> {
        match call {
            Call::Tabs => json(self.tabs()),
            Call::OpenTab { path, bypass } => json(self.open_tab(path, bypass)?),
            Call::OpenScratch { bypass } => json(self.open_scratch(bypass)?),
            Call::AutoResume { tab, on } => json(self.auto_resume(&tab, on)?),
            Call::StartClaude { tab, cols, rows } => json(self.start_claude(&tab, cols, rows)?),
            Call::SetBypass { tab, on, cols, rows } => json(self.set_bypass(&tab, on, cols, rows)?),
            Call::SetModel { tab, model, cols, rows, account } => json(self.set_model(&tab, &model, cols, rows, account.as_deref())?),
            Call::RestartClaude { tab, cols, rows } => json(self.restart_claude(&tab, cols, rows)?),
            Call::WriteInput { tab, data } => json(self.write_input(&tab, data.as_bytes())?),
            Call::ResizePty { tab, cols, rows } => json(self.resize_pty(&tab, cols, rows)?),
            Call::CloseTab { tab } => {
                self.close_tab(&tab);
                Ok(Value::Null)
            }
            Call::StopTurn { tab } => json(self.stop_turn(&tab)?),
            Call::SetFolder { tab, path } => json(self.set_folder(&tab, &path)?),
            Call::ListProjects => json(self.list_projects()),
            Call::SyncStopped => json(self.sync_stopped()),
            Call::ResetSync => {
                if let Some(file) = &self.sync_file {
                    sync::set_aside_unreadable(file)?;
                }
                Ok(Value::Null)
            }
            Call::ProjectHistory { path, session } => {
                let mut turns = self.project_history(&path);
                if let Some(session) = session {
                    turns.retain(|t| t.session == session);
                }
                json(turns)
            }
            Call::GitRemote { path } => json(git_remote(Path::new(&path))),
            Call::Browse { path } => json(projects::browse(&path, &home())?),
            Call::CreateProject { name } => {
                let mut created = scaffold::create(&self.clones, &name, self.launch.path_env.as_deref())?;
                created.path = self.open_project(created.path.into()).display().to_string();
                json(created)
            }
            Call::PublishProject { path } => json(scaffold::publish(Path::new(&path), self.launch.path_env.as_deref())?),
            Call::Clone { repo, token, dest: None } => json(self.find_folder("", Some(&repo), token.as_deref())?),
            Call::Clone { repo, token, dest: Some(dest) } => json(self.clone_into(&repo, &dest, token.as_deref())?),
            Call::MakeFolder { path } => {
                let dir = projects::expand(&path, &home());
                std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
                json(dunce::canonicalize(&dir).map_err(|e| e.to_string())?)
            }
            Call::SearchThreads { query } => json(self.search_threads(&query)),
            Call::Run { run } => json(self.run(&run)?),
            Call::Deliver { env, text } => json(self.deliver(&env, &text)),
            Call::Upload { name, data } => {
                let dir = self.uploads.as_ref().ok_or("this machine keeps no files")?;
                json(uploads::save(dir, &name, &data.0)?.display().to_string())
            }
            Call::ReadUpload { path } => {
                let dir = self.uploads.as_ref().ok_or("this machine keeps no files")?;
                json(uploads::image(dir, &path)?)
            }
            Call::ReadImage { path } => json(uploads::named(&path, &home())?),
            Call::ListAccounts => json(self.list_accounts()),
            Call::LearnModelMenu { lines } => json(self.learn_model_menu(&lines)?),
            Call::AddAccount { provider, name, auth, key } => {
                let auth = auth.unwrap_or(Auth::Login);
                if auth == Auth::OpenCodeGo {
                    if provider != Provider::Claude {
                        return Err("OpenCode Go runs through Claude Code".into());
                    }
                    let Some(key) = key.as_deref().map(str::trim).filter(|key| !key.is_empty()) else {
                        return Err("OpenCode Go needs an API key".into());
                    };
                    crate::opencode_go::validate(key)?;
                    let account = self.accounts.add(provider, &name, auth)?;
                    if let Err(error) = self.accounts.set_key(&account, key) {
                        let _ = self.accounts.remove(&account.id);
                        return Err(error);
                    }
                    if let Err(error) = self.prepare_go_home(&account) {
                        let _ = self.accounts.remove(&account.id);
                        return Err(error);
                    }
                    json(self.view_account(account))
                } else if key.as_deref().is_some_and(|key| !key.trim().is_empty()) {
                    Err("only an OpenCode Go account takes an API key".into())
                } else {
                    json(self.view_account(self.accounts.add(provider, &name, auth)?))
                }
            }
            Call::SetAccountKey { account, key } => {
                let account = self.accounts.get(&account).ok_or_else(|| format!("no account {account} on this machine"))?;
                if account.auth != Auth::OpenCodeGo {
                    return Err(format!("{id} signs in through its CLI. Replace the API key under Settings > Accounts.", id = account.id));
                }
                let key = key.trim();
                if key.is_empty() {
                    return Err("OpenCode Go needs an API key".into());
                }
                crate::opencode_go::validate(key)?;
                self.accounts.set_key(&account, key)?;
                self.prepare_go_home(&account)?;
                json(self.view_account(account))
            }
            Call::RemoveAccount { account } => {
                // Its tabs would save sessions under an id the next start can't resume.
                if self.tabs.lock().unwrap().values().any(|t| t.account.id == account) {
                    return Err(format!("close the tabs that run under {account} before removing it"));
                }
                json(self.accounts.remove(&account)?)
            }
            Call::SetProjectAccount { path, account } => json(self.set_project_account(&path, account)?),
            Call::OpenOrchestrator { path } => json(self.open_orchestrator(&path)?),
            Call::Enqueue { path, text, request_id, sender, busy } => json(self.enqueue_to_captain(&path, &text, &request_id, &sender, busy)?),
            Call::OrchestratorReport { request_id, report } => json(self.deliver_report(&request_id, &report)?),
            Call::OpenGlobalOrchestrator => json(self.open_global_orchestrator()?),
            Call::OpenTasteChat { scope, path } => json(self.open_taste_chat(&scope, path.as_deref())?),
            Call::Delegate { origin, request, repo, token } => self.start_for_origin(origin, *request, repo, token).and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string())),
            Call::RemoteTask { origin_machine, tab, task, cancel } => self.remote_task(&origin_machine, &tab, &task, cancel.as_deref()),
            Call::RemoteEnded { from, tab, task, end } => json(self.remote_ended(&from, &tab, &task, &end)),
            Call::GlobalOrchestratorSettings => json(self.global_settings()),
            Call::SetGlobalOrchestratorSettings { settings } => json(self.set_global_settings(settings)?),
            Call::ReadSession { tab, items } => self.read_session_here(&tab, read_items(items)).ok_or_else(|| format!("no session {tab} on this machine")),
            Call::MemorySearch { path, query, limit } => json(self.memories.search(&self.known_project(&path)?, &query, limit.unwrap_or(SEARCH_LIMIT))?),
            Call::MemorySave { path, title, text, kind, id } => {
                let draft = memory::Draft::new(&title, &text, kind.as_deref())?;
                match self.memories.save(&self.known_project(&path)?, &draft, id)? {
                    memory::Saved::Added(m) | memory::Saved::Updated(m) => json(m),
                    memory::Saved::Duplicate(m) => Err(format!("Memory #{} \"{}\" already says nearly this. Change that one instead.", m.id, m.title)),
                }
            }
            Call::MemoryForget { path, id } => json(self.memories.forget(&self.known_project(&path)?, id)?),
            Call::MemoryStatus { path } => json(self.memories.status(&self.known_project(&path)?)?),
            Call::SignIn { account } => json(self.sign_in(&account)?),
            Call::SwitchAccount { tab, account, cols, rows } => json(self.switch_account(&tab, &account, cols, rows)?),
            Call::Thread { tab, from } => json(self.thread(&tab, from)?),
            Call::Prompt { tab, text } => json(self.type_prompt(&tab, &text)?),
            Call::Workspace { tab } => {
                let cwd = self.tab(&tab, |t| t.cwd.clone())?;
                json(workspace::Workspace { scratch: cwd == self.scratch, ..workspace::of(&cwd) })
            }
            Call::RegenerateTitle { tab, previous, thread } => {
                let thread = if thread.is_empty() { titles::thread_text(&self.thread(&tab, 0)?.items) } else { thread };
                let cwd = self.tab(&tab, |t| t.cwd.clone())?;
                let env = self.headless_env(&tab)?;
                let headless = Headless { env: &env, strip: self.headless_strip(&tab)? };
                json(titles::regenerate(&self.launch.bin(Provider::Claude), &headless, &cwd, self.launch.path_env.as_deref(), &previous, &thread)?)
            }
            Call::Favicon { tab } => json(favicon::of(&history::project_of(&self.tab(&tab, |t| t.cwd.clone())?))),
            Call::PrStatus { tab, pr } => json(workspace::pr_status(&self.tab(&tab, |t| t.cwd.clone())?, &pr, self.launch.path_env.as_deref())?),
            Call::PullRequest { tab, pr } => json(workspace::pull_request(&self.tab(&tab, |t| t.cwd.clone())?, &pr, self.launch.path_env.as_deref())?),
            Call::PrList { tab } => json(workspace::pull_requests(&self.tab(&tab, |t| t.cwd.clone())?, self.launch.path_env.as_deref())?),
            Call::PrAction { tab, pr, action } => json(workspace::pr_action(&self.tab(&tab, |t| t.cwd.clone())?, &pr, action, self.launch.path_env.as_deref())?),
            Call::Diff { tab, scope, ignore_whitespace, base } => json(workspace::diff(&self.tab(&tab, |t| t.cwd.clone())?, scope, ignore_whitespace, base.as_deref())?),
            Call::Ports { tab } => {
                let pid = self.tab(&tab, |t| t.session.as_ref().and_then(|s| s.pid()))?;
                json(workspace::listeners(pid, self.launch.path_env.as_deref()))
            }
            Call::Editors => json(workspace::editors(&home(), self.launch.path_env.as_deref())),
            Call::OpenIn { tab, editor, path, folder, reveal } => {
                let cwd = self.tab(&tab, |t| t.cwd.clone())?;
                let how = workspace::OpenHow { folder, reveal };
                json(workspace::open_in(&cwd, &editor, path.as_deref(), how, &home(), self.launch.path_env.as_deref())?)
            }
            Call::Commands { tab } => {
                let (cwd, account) = self.tab(&tab, |t| (t.cwd.clone(), t.account.clone()))?;
                let shared = self.shared_plugin_dirs(&account);
                json(commands::list(&self.accounts.home(&account), &cwd, &shared))
            }
            Call::Files { tab } => json(files::list(&self.tab(&tab, |t| t.cwd.clone())?, self.launch.path_env.as_deref())),
            Call::Branches { tab } => json(branches::of(&self.tab(&tab, |t| t.cwd.clone())?, self.launch.path_env.as_deref())?),
            Call::Checkout { tab, branch, create, from } => {
                let cwd = self.idle_folder(&tab, "Switch branches")?;
                json(branches::checkout(&cwd, &branch, create, from.as_deref(), self.launch.path_env.as_deref())?)
            }
            Call::CheckoutPr { tab, pr } => {
                let cwd = self.idle_folder(&tab, "Check out the pull request")?;
                json(workspace::checkout_pr(&cwd, &pr, self.launch.path_env.as_deref())?)
            }
            Call::ProjectScripts { tab } => json(scripts::of(self.scripts_file().as_deref(), &history::project_of(&self.tab(&tab, |t| t.cwd.clone())?))),
            Call::SaveProjectScripts { tab, scripts } => {
                let project = history::project_of(&self.tab(&tab, |t| t.cwd.clone())?);
                let file = self.scripts_file().ok_or("this machine keeps no scripts")?;
                json(scripts::save(&file, &project, scripts)?)
            }
            Call::Git { tab, step } => {
                let cwd = self.idle_folder(&tab, "Run git")?;
                let env = self.headless_env(&tab)?;
                let headless = Headless { env: &env, strip: self.headless_strip(&tab)? };
                json(branches::run_step(&cwd, &step, &self.launch.bin(Provider::Claude), &headless, self.launch.path_env.as_deref())?)
            }
            Call::SetWorkspace { tab, worktree, cols, rows } => json(self.set_workspace(&tab, worktree.as_deref(), cols, rows)?),
            Call::PullRequests { paths, state, involvement, query, filters, limit } => {
                let path = self.launch.path_env.as_deref();
                json(pulls::list(&home(), path, self.gh_repos(&paths), state, involvement, &query, &filters, limit)?)
            }
            Call::PullRequestStack { repo, number } => json(pulls::stack(&home(), self.launch.path_env.as_deref(), &repo, number)?),
            Call::TokenUsage { since } => {
                let homes: Vec<PathBuf> = self.accounts.list().iter().filter(|a| a.provider == Provider::Claude).map(|a| self.accounts.home(a).join("projects")).collect();
                json(usage::read(&homes, since, &usage::rates(self.rates_file.as_deref()), &usage::PriceBook::load(self.prices_file().as_deref())))
            }
            Call::UsagePrices => json(usage::PriceBook::load(self.prices_file().as_deref())),
            Call::SetUsagePrices { prices, aliases } => {
                let file = self.prices_file().ok_or("this machine keeps no settings")?;
                let mut book = usage::PriceBook::load(Some(&file));
                book.apply(prices, aliases)?;
                book.save(&file)?;
                json(book)
            }
            Call::UsageLimits => json(usage::limits(&self.launch.bin(Provider::Claude), &home(), self.launch.path_env.as_deref())?),
            Call::PullRequestFilterOptions { paths, kind, query } => json(pulls::filter_options(&home(), self.launch.path_env.as_deref(), &self.gh_repos(&paths), kind, &query)?),
            Call::PullRequestFiles { host, repo, number } => json(pulls::files(&home(), self.launch.path_env.as_deref(), &host, &repo, number)?),
            Call::PageMeta { url, frame_by } => json(panel::page_meta(&url, frame_by.as_deref())?),
            Call::Refs { tab } => json(panel::refs(&self.tab(&tab, |t| t.cwd.clone())?, self.launch.path_env.as_deref())?),
            Call::ReadFile { tab, path, root, rev } => json(panel::read_file(&self.tab(&tab, |t| t.cwd.clone())?, &path, root, rev.as_deref())?),
            Call::WatchPr { tab, pr, watching } => json(self.watch_pr(&tab, pr.as_deref(), watching)?),
            Call::FileUrl { tab, path, root } => json(self.file_server.url(&self.tab(&tab, |t| t.cwd.clone())?, &path, root)?),
            Call::WriteFile { tab, path, root, text, since } => json(panel::write_file(&self.tab(&tab, |t| t.cwd.clone())?, &path, root, &text, since.as_deref())?),
            Call::Search { tab, query, case, regex } => json(panel::search(&self.tab(&tab, |t| t.cwd.clone())?, &query, case, regex, self.launch.path_env.as_deref())?),
            Call::PrEdit { tab, pr, title, body } => json(panel::pr_edit(&self.tab(&tab, |t| t.cwd.clone())?, &pr, title.as_deref(), body.as_deref(), self.launch.path_env.as_deref())?),
            Call::PrComment { tab, pr, body, reply_to } => json(panel::pr_comment(&self.tab(&tab, |t| t.cwd.clone())?, &pr, &body, reply_to.as_deref(), self.launch.path_env.as_deref())?),
            Call::PrReview { tab, pr, event, body, comments } => json(panel::pr_review(&self.tab(&tab, |t| t.cwd.clone())?, &pr, event, &body, &comments, self.launch.path_env.as_deref())?),
            Call::PrRerun { tab, check } => json(panel::pr_rerun(&self.tab(&tab, |t| t.cwd.clone())?, &check, self.launch.path_env.as_deref())?),
            Call::PrGuide { tab, pr, write } => {
                let env = self.headless_env(&tab)?;
                let headless = Headless { env: &env, strip: self.headless_strip(&tab)? };
                json(panel::guide(&self.tab(&tab, |t| t.cwd.clone())?, &pr, write, &self.guides, &self.launch.bin(Provider::Claude), &headless, self.launch.path_env.as_deref())?)
            }
            Call::PrChange { tab, pr, change } => json(panel::pr_change(&self.tab(&tab, |t| t.cwd.clone())?, &pr, &change, self.launch.path_env.as_deref())?),
            Call::PrCandidates { tab, pr, of } => json(panel::pr_candidates(&self.tab(&tab, |t| t.cwd.clone())?, &pr, of, self.launch.path_env.as_deref())?),
            Call::PrStack { tab, pr, action, method, heads } => json(panel::pr_stack(&self.tab(&tab, |t| t.cwd.clone())?, &pr, action, method.unwrap_or(panel::MergeMethod::Merge), &heads, self.launch.path_env.as_deref())?),
            Call::OpenShell { tab, cols, rows } => json(self.shells.open(&self.tab(&tab, |t| t.cwd.clone())?, cols, rows, self.launch.path_env.as_deref())?),
            Call::CloseShell { shell } => {
                self.shells.close(&shell);
                Ok(Value::Null)
            }
            Call::BusyShells { shells } => json(self.shells.busy(&shells, self.launch.path_env.as_deref())),
            Call::DeviceHub { embed } => json(self.device_hub.url(embed.as_deref(), self.launch.path_env.as_deref())?),
            Call::DeviceAgents { allow } => {
                let file = self.device_agents.as_ref().ok_or("this machine keeps no settings")?;
                match allow {
                    Some(true) => std::fs::write(file, "").map_err(|e| e.to_string())?,
                    Some(false) => drop(std::fs::remove_file(file)),
                    None => {}
                }
                json(file.exists())
            }
            Call::Device { op } => {
                let path = self.launch.path_env.as_deref();
                json(crate::device::Host { port: self.device_hub.port(None, path)?, path }.run(op)?)
            }
            Call::Subagent { tab, tool } => {
                if let Some(steps) = self.delegates.steps_for(&tab, &tool) {
                    return json(steps);
                }
                let file = thread::subagent(&self.transcript(&tab)?, &tool).ok_or("Claude Code kept no transcript of this subagent")?;
                json(thread::read_subagent(&file).map_err(|e| format!("could not read the subagent's transcript: {e}"))?)
            }
            Call::OpenTask { tab, task } => json(self.open_delegated(&tab, &task)?),
            Call::DeliverTask { tab, task } => json(self.deliver_task(&tab, &task)?),
            Call::CancelTask { tab, task } => {
                let path = self.launch.path_env.clone();
                json(self.delegates.cancel(
                    &crate::delegate::Caller { tab: Some(tab), task: None },
                    &task,
                    "cancelled from the card",
                    path.as_deref(),
                )?)
            }
            Call::Rewind { tab, before, files, cols, rows } => json(self.rewind(&tab, &before, files, cols, rows)?),
            Call::Fork { tab, through } => json(self.fork(&tab, &through)?),
            Call::SaveFile { tab, path, text } => json(save_file(&self.tab(&tab, |t| t.cwd.clone())?, &path, &text)?),
            Call::WatchThreads { tabs } => {
                self.set_watched_threads(LOCAL_VIEWER, &tabs);
                Ok(Value::Null)
            }
            Call::UpdateSelf => json(self.update_self()),
            Call::Skills => json(skills::list(&self.places())),
            Call::ReadSkill { dir, file } => json(skills::read(&self.places(), &dir, &file)?),
            Call::SaveSkill { file, text, copies } => {
                let written = skills::save(&self.places(), &file, &text, &copies)?;
                let sessions = self.skills_changed(&written.touched);
                if !written.touched.is_empty() {
                    self.wake_sync();
                }
                json(skills::SaveReport { copies: written.copies, sessions })
            }
        }
    }

    pub fn set_watched_threads(&self, viewer: &str, tabs: &[String]) {
        let mut watchers = self.thread_watchers.lock().unwrap();
        if tabs.is_empty() {
            watchers.remove(viewer);
        } else {
            watchers.insert(viewer.to_owned(), tabs.iter().cloned().collect());
            self.watch_threads();
        }
    }

    pub fn drop_remote_thread_watchers(&self) {
        self.thread_watchers.lock().unwrap().retain(|viewer, _| viewer == LOCAL_VIEWER);
    }

    /// The tab's transcript file, in the home of the account it runs under.
    fn transcript(&self, tab: &str) -> Result<PathBuf, String> {
        let (session, account) = self.tab(tab, |t| (t.claude_session.clone(), t.account.clone()))?;
        let session = session.ok_or("this thread has no conversation yet")?;
        self.find_transcript(&account, &session).ok_or_else(|| "Claude Code's transcript of this conversation is gone".into())
    }

    /// Copies the tab's conversation up to `cut` under a new session id, with the turns pinned saved for
    /// it, so the copy keeps its changed files. Returns the new id, or none when the copy holds no message.
    fn fork_session(&self, tab: &str, cut: thread::Cut, marked: bool) -> Result<Option<String>, String> {
        let (cwd, session) = self.tab(tab, |t| (t.cwd.clone(), t.claude_session.clone()))?;
        let session = session.ok_or("this thread has no conversation yet")?;
        let new = new_session_id(&session);
        let transcript = self.transcript(tab)?;
        let forked = thread::fork(&transcript, cut, &new, marked.then_some(session.as_str()))?;
        if !forked.said {
            let _ = std::fs::remove_file(transcript.with_file_name(format!("{new}.jsonl")));
            return Ok(None);
        }
        let at = chrono::DateTime::parse_from_rfc3339(&forked.until).map_or(u64::MAX, |t| t.timestamp().max(0) as u64 + 1);
        if let Some(dir) = &self.history_dir {
            let file = history::file_for(dir, &history::project_of(&cwd));
            let mut kept: Vec<history::Turn> = history::load(&file).into_iter().filter(|t| t.session == session && t.ended < at).collect();
            kept.reverse();
            for turn in kept {
                history::append(&file, &history::Turn { session: new.clone(), ..turn });
            }
        }
        Ok(Some(new))
    }

    /// Restarts the tab's claude on its conversation as it was before the prompt `before`, as T3 Code's
    /// "Edit from here" reverts to that prompt's checkpoint, and returns the tab's folder.
    pub fn rewind(&self, tab: &str, before: &str, files: bool, cols: u16, rows: u16) -> Result<String, String> {
        let (cwd, working) = self.tab(tab, |t| (t.cwd.clone(), t.status == Some(AgentStatus::Working)))?;
        if working {
            return Err("claude is working. Rewind when the turn ends.".into());
        }
        let checkpoint = if files { self.checkpoint(tab, &cwd, before)? } else { None };
        let new = self.fork_session(tab, thread::Cut::Before(before), false)?;
        if let Some(tree) = checkpoint {
            history::restore(&cwd, &tree)?;
        }
        // Before the first prompt there is nothing to resume, so claude starts a new conversation.
        self.tab(tab, |t| {
            t.launch = new.iter().flat_map(|id| ["--resume".to_owned(), id.clone()]).collect();
            t.claude_session = new.clone();
        })?;
        self.relaunch(tab, cols, rows)
    }

    /// The working tree as the prompt `before` began, from the first turn pinned saved from then on;
    /// none when claude has changed nothing since.
    fn checkpoint(&self, tab: &str, cwd: &Path, before: &str) -> Result<Option<String>, String> {
        let session = self.tab(tab, |t| t.claude_session.clone())?.unwrap_or_default();
        let text = std::fs::read_to_string(self.transcript(tab)?).unwrap_or_default();
        let at = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["uuid"] == before)
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v["timestamp"].as_str()?).ok())
            .ok_or("that prompt is not in this conversation")?
            .timestamp()
            .max(0) as u64;
        let Some(dir) = &self.history_dir else { return Ok(None) };
        let turns = history::load(&history::file_for(dir, &history::project_of(cwd)));
        match turns.into_iter().rfind(|t| t.session == session && t.ended >= at) {
            None => Ok(None),
            Some(turn) => turn.before.map(Some).ok_or_else(|| "pinned saved no checkpoint for that turn, so its files can't be put back".into()),
        }
    }

    /// The session's transcript, in the home of the account it ran under.
    fn find_transcript(&self, account: &Account, session: &str) -> Option<PathBuf> {
        thread::find(&self.accounts.home(account).join("projects"), session)
    }

    fn locate_transcript(&self, account: &Account, session: &str, status: Option<AgentStatus>, look: &mut TranscriptLook, now: Instant) -> Option<PathBuf> {
        let home = self.accounts.home(account);
        if let Some(found) = look.found.as_ref().filter(|found| found.session == session && found.path.starts_with(&home) && found.path.is_file()) {
            return Some(found.path.clone());
        }
        let gap = match &look.missed {
            Some(missed) if missed.session == session && missed.status == status => {
                if now < missed.at + missed.gap {
                    return None;
                }
                (missed.gap * 2).min(PAGE_POLL)
            }
            _ => THREAD_LOOK_EVERY,
        };
        let path = self.find_transcript(account, session);
        look.missed = path.is_none().then(|| Missed { session: session.to_owned(), status, at: now, gap });
        path
    }

    /// Opens a tab beside `tab` whose claude resumes its conversation through the message `through`, as
    /// T3 Code's "Fork from this response" does, and returns the new tab's id.
    pub fn fork(&self, tab: &str, through: &str) -> Result<TabId, String> {
        let (cwd, account, env, bypass) = self.tab(tab, |t| (t.cwd.clone(), t.account.clone(), t.env.clone(), t.bypass))?;
        let new = self.fork_session(tab, thread::Cut::Through(through), true)?.ok_or("there is nothing to fork before this message")?;
        let id = self.add_tab(cwd, account, vec!["--resume".into(), new.clone()], env, bypass, Some(new));
        self.emit_tabs();
        Ok(id)
    }

    /// The tab's conversation from byte `from` of its transcript, empty until claude reports a session.
    pub fn thread(&self, tab: &str, from: u64) -> Result<ThreadPage, String> {
        let (session, process) = self.tab(tab, |t| (t.claude_session.clone(), t.session.as_ref().and(t.started)))?;
        let Some(file) = self.transcript_of(tab)? else {
            return Ok(ThreadPage { session, items: vec![], next: 0, reset: false, context: None, branch: None, todos: None, agents: self.delegates.cards(tab) });
        };
        let page = thread::read(&file, from).map_err(|e| format!("could not read the transcript: {e}"))?;
        let mut agents = subagents::scan(&file.with_extension("").join("subagents"), &mut self.agents.lock().unwrap(), process);
        agents.extend(self.delegates.cards(tab));
        Ok(ThreadPage { session, items: page.items, next: page.next, reset: page.reset, context: page.context, branch: page.branch, todos: page.todos, agents })
    }

    /// The tab's transcript, once claude reported a session that has one.
    fn transcript_of(&self, tab: &str) -> Result<Option<PathBuf>, String> {
        let (session, account, known) = self.tab(tab, |t| (t.claude_session.clone(), t.account.clone(), t.last_found_transcript.clone()))?;
        let file = session.as_deref().and_then(|s| {
            let home = self.accounts.home(&account);
            let still = known.as_ref().filter(|path| path.file_stem().is_some_and(|stem| stem == s) && path.starts_with(&home) && path.is_file());
            still.cloned().or_else(|| self.find_transcript(&account, s))
        });
        if let Some(file) = file.as_ref().filter(|f| known.as_ref() != Some(f)) {
            self.tab(tab, |t| t.last_found_transcript = Some(file.clone()))?;
        }
        Ok(file)
    }

    /// `reading::read` of the tab's session with the last `items` items of its conversation, read
    /// from the end of its transcript. `None` when this machine has no such tab.
    fn read_session_here(&self, tab: &str, items: usize) -> Option<Value> {
        let view = self.tabs().into_iter().find(|t| t.id == tab)?;
        let page = match self.transcript_of(tab) {
            Ok(Some(file)) => reading::end_of(&file, items),
            Ok(None) => Ok(json!({ "items": [], "next": 0, "reset": false })),
            Err(e) => Err(e),
        };
        Some(reading::read(&view, page, items))
    }

    fn watch_threads(&self) {
        if self.threads_watched.swap(true, Ordering::Relaxed) {
            return;
        }
        let mut seen = HashMap::new();
        self.every(THREAD_LOOK_EVERY, move |machine| machine.nudge_thread_watchers(&mut seen));
    }

    fn every(&self, period: Duration, mut tick: impl FnMut(&Machine) + Send + 'static) {
        let me = self.me.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(period);
            let Some(machine) = me.upgrade() else { return };
            if !machine.quitting.load(Ordering::Relaxed) {
                tick(&machine);
            }
        });
    }

    fn nudge_thread_watchers(&self, seen: &mut HashMap<String, TranscriptLook>) {
        let watched: HashSet<String> = self.thread_watchers.lock().unwrap().values().flat_map(|tabs| tabs.iter().cloned()).collect();
        if watched.is_empty() {
            return;
        }
        let open: Vec<(String, String, Option<AgentStatus>, Account)> = {
            let tabs = self.tabs.lock().unwrap();
            tabs.iter().filter_map(|(id, tab)| tab.claude_session.clone().map(|session| (id.clone(), session, tab.status, tab.account.clone()))).collect()
        };
        seen.retain(|id, _| open.iter().any(|(open_id, ..)| open_id == id));
        let now = Instant::now();
        for (id, session, status, account) in open {
            if !watched.contains(&id) {
                continue;
            }
            let look = seen.entry(id.clone()).or_default();
            let Some(path) = self.locate_transcript(&account, &session, status, look, now) else { continue };
            let Ok(len) = std::fs::metadata(&path).map(|meta| meta.len()) else { continue };
            let found = Transcript { session, path, len };
            if look.found.as_ref() == Some(&found) {
                continue;
            }
            look.found = Some(found);
            self.emit(Event::Thread { tab: id });
        }
    }

    /// Every tab, in the order they opened.
    pub fn tabs(&self) -> Vec<TabView> {
        let turns = self.turns.lock().unwrap().clone();
        let queued = self.delegates.queued();
        let mut views: Vec<TabView> = self.tabs.lock().unwrap().iter().map(|(id, t)| t.view(id, &turns, &queued)).collect();
        views.sort_by_key(|v| v.opened);
        views
    }

    pub fn view(&self, tab: &str) -> Result<TabView, String> {
        let turns = self.turns.lock().unwrap().clone();
        let queued = self.delegates.queued();
        self.tab(tab, |t| t.view(tab, &turns, &queued))
    }

    /// Adds a tab on the project at `path`, or the default folder, and returns its id.
    /// With `bypass` the tab's agent never asks for permission.
    pub fn open_tab(&self, path: Option<String>, bypass: bool) -> Result<TabId, String> {
        let cwd = match path {
            Some(path) if Path::new(&path) == self.scratch => self.scratch.clone(),
            Some(path) if !Path::new(&path).is_dir() => return Err(format!("{path} is not a folder on this machine")),
            Some(path) => self.open_project(path.into()),
            None => self.default_cwd.clone(),
        };
        let account = self.account_for(None, &cwd)?;
        let id = self.add_tab(cwd, account, vec![], vec![], bypass, None);
        self.emit_tabs();
        Ok(id)
    }

    /// The account a session in `cwd` runs under: the one `explicit` names, else its project's,
    /// else the default one. A project whose account was removed falls back to the default.
    pub fn account_for(&self, explicit: Option<&str>, cwd: &Path) -> Result<Account, String> {
        if let Some(id) = explicit.filter(|id| !id.is_empty()) {
            return self.accounts.get(id).ok_or_else(|| format!("no account {id} on this machine"));
        }
        let project = history::project_of(cwd);
        let chosen = self.projects.lock().unwrap().iter().find(|p| p.path == project).and_then(|p| p.account.clone());
        Ok(chosen.and_then(|id| self.accounts.get(&id)).unwrap_or_else(|| Account::builtin(Provider::Claude)))
    }

    /// The env of a headless `claude -p` beside the tab: its account's, or the built-in one's
    /// when the tab runs Codex.
    fn headless_env(&self, tab: &str) -> Result<Vec<(String, String)>, String> {
        let account = self.tab(tab, |t| t.account.clone())?;
        Ok(self.accounts.env(&if account.provider == Provider::Claude { account } else { Account::builtin(Provider::Claude) }))
    }

    /// What a headless call must strip before it applies `headless_env`. A Go tab must not keep a
    /// parent shell's Anthropic key or provider flag.
    fn headless_strip(&self, tab: &str) -> Result<&'static [&'static str], String> {
        let account = self.tab(tab, |t| t.account.clone())?;
        Ok(if account.auth == Auth::OpenCodeGo { &crate::opencode_go::STRIP_ENV } else { &[] })
    }

    /// Who each account's CLI says is signed in, asked of every account at once.
    pub fn list_accounts(&self) -> Vec<accounts::View> {
        std::thread::scope(|scope| {
            let asked: Vec<_> = self.accounts.list().into_iter().map(|a| scope.spawn(move || self.view_account(a))).collect();
            asked.into_iter().map(|h| h.join().unwrap()).collect()
        })
    }

    fn view_account(&self, account: Account) -> accounts::View {
        let status = self.status(&account);
        let mut view = accounts::View::new(account, status);
        if view.account.provider == Provider::Claude && view.account.auth != Auth::OpenCodeGo {
            view.models = self.claude_models();
        }
        view
    }

    /// Claude's models on this machine: pinned's table as this machine's claude last showed its
    /// /model menu.
    pub fn claude_models(&self) -> Vec<crate::claude_models::ClaudeModel> {
        crate::claude_models::merged(crate::claude_models::known(), &self.model_menu.lock().unwrap())
    }

    /// Keeps the model rows of claude's /model menu when `lines`, a screen, shows it, and answers
    /// Claude's models as they are now.
    fn learn_model_menu(&self, lines: &[String]) -> Result<Vec<crate::claude_models::ClaudeModel>, String> {
        let rows = crate::claude_models::menu_rows(lines).ok_or("claude's /model menu is not on that screen")?;
        if !rows.is_empty() && *self.model_menu.lock().unwrap() != rows {
            if let Some(file) = &self.model_menu_file {
                let body = serde_json::to_vec_pretty(&rows).map_err(|e| e.to_string())?;
                crate::fsx::replace(file, &body, None, None)?;
            }
            *self.model_menu.lock().unwrap() = rows;
        }
        Ok(self.claude_models())
    }

    fn status(&self, account: &Account) -> accounts::Status {
        self.accounts.status(account, &self.launch.bin(account.provider), self.launch.path_env.as_deref())
    }

    /// Adds a tab in the folder kept for threads without a project, which no project lists, and
    /// returns its id.
    pub fn open_scratch(&self, bypass: bool) -> Result<TabId, String> {
        std::fs::create_dir_all(&self.scratch).map_err(|e| format!("could not make {}: {e}", self.scratch.display()))?;
        let account = self.account_for(None, &self.scratch)?;
        let id = self.add_tab(self.scratch.clone(), account, vec![], vec![], bypass, None);
        self.emit_tabs();
        Ok(id)
    }

    /// Lets the tab's session continue by itself once its usage limit lifts, or holds it.
    pub fn auto_resume(&self, tab: &str, on: bool) -> Result<(), String> {
        self.tab(tab, |t| t.held_at_limit = !on)?;
        self.save_sessions();
        if on {
            self.continue_when_limit_lifts(tab);
        }
        self.emit_tabs();
        Ok(())
    }

    /// Adds a tab for a connector's run without starting it, and returns its id. Refused when
    /// the run's folder can't be found or cloned here, or its account can't run it here.
    pub fn open_run(&self, run: &RunSpec) -> Result<TabId, String> {
        let cwd = self.find_folder(&run.folder, run.repo.as_deref(), run.token.as_deref())?;
        let account = self.account_for(run.account.as_deref(), &cwd)?;
        // The prompt reports through pinned's hooks and commands, which only reach Claude Code.
        if !account.provider.spec().harness {
            return Err(format!("account {} runs {}, which can't run a connector's session", account.id, account.provider.spec().name));
        }
        // The built-in login ran every run before accounts existed, so only an added one is asked.
        if !account.is_builtin() && !self.status(&account).signed_in {
            return Err(format!("account {} is not signed in on this machine", account.id));
        }
        let exe = self.launch.exe.display().to_string();
        let args = run.args.iter().map(|a| a.replace(EXE_PLACEHOLDER, &exe)).collect();
        // A run resuming a conversation shows its transcript before claude's first hook names it.
        let resumed = run.args.windows(2).find(|w| w[0] == "--resume").map(|w| w[1].clone());
        let id = self.add_tab(cwd, account, args, run.env.clone(), false, resumed);
        self.emit_tabs();
        Ok(id)
    }

    /// Opens a connector's run and starts it at once, so it works with no one watching.
    pub fn run(&self, run: &RunSpec) -> Result<TabId, String> {
        // The hub sends a run again when it lost the answer, as on a dropped link: the session it started answers.
        if let Some(work) = session_of(&run.env) {
            let tabs = self.tabs.lock().unwrap();
            if let Some((id, _)) = tabs.iter().find(|(_, t)| t.session.is_some() && session_of(&t.env).as_ref() == Some(&work)) {
                return Ok(id.clone());
            }
        }
        let id = self.open_run(run)?;
        // A run that can't start is refused whole, so the hub's next machine takes it and no idle tab stays behind here.
        if let Err(e) = self.start_claude(&id, UNWATCHED.0, UNWATCHED.1) {
            self.close_tab(&id);
            return Err(e);
        }
        // `claude --resume` exits 1 and prints this when the conversation is gone. The hub then
        // opens a new session seeded with the run's history. A live resume stays up, so the wait ends.
        if run.args.windows(2).any(|w| w[0] == "--resume") {
            if let Err(e) = self.resume_missing(&id) {
                self.close_tab(&id);
                return Err(e);
            }
        }
        Ok(id)
    }

    /// Reads the tab's screen until Claude has rejected the resume or kept a live one up.
    /// The session exists as soon as the process is spawned, before it has written anything, so an
    /// empty screen is not a live resume: a missing conversation exits with this line, and a busy
    /// machine can take a while to schedule that process.
    fn resume_missing(&self, id: &TabId) -> Result<(), String> {
        let start = std::time::Instant::now();
        let mut drew_at = None;
        loop {
            if Self::resume_refused(&self.screen_text(id)) {
                return Err("No conversation found".into());
            }
            if !self.still_running(id) {
                return self.refusal_after_exit(id);
            }
            let screen = self.screen_text(id);
            if !screen.is_empty() {
                let drew = drew_at.get_or_insert_with(std::time::Instant::now);
                // The TUI paints, then a missing id exits. Claude Code 2.1 took about 1.3s to do
                // that, so 400ms of paint is still the startup screen, not a live resume.
                if drew.elapsed() >= std::time::Duration::from_secs(2) {
                    return Ok(());
                }
            }
            if start.elapsed() >= std::time::Duration::from_secs(15) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// The last line can land after the exit is noticed. Read the screen again before
    /// treating a dead process as a resume that worked.
    fn refusal_after_exit(&self, id: &TabId) -> Result<(), String> {
        for _ in 0..10 {
            if Self::resume_refused(&self.screen_text(id)) {
                return Err("No conversation found".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Ok(())
    }

    /// Claude's TUI places each word with a cursor command, so the refusal is not one contiguous
    /// sentence in the pty bytes.
    fn resume_refused(screen: &str) -> bool {
        let mut flat = String::new();
        let mut chars = screen.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                match chars.peek() {
                    Some('[') => {
                        chars.next();
                        for next in chars.by_ref() {
                            if next.is_ascii_alphabetic() {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        chars.next();
                        for next in chars.by_ref() {
                            if next == '\u{7}' || next == '\u{1b}' {
                                break;
                            }
                        }
                    }
                    Some(_) => {
                        chars.next();
                    }
                    None => {}
                }
                continue;
            }
            if c.is_ascii_alphanumeric() {
                flat.push(c.to_ascii_lowercase());
            }
        }
        flat.contains("noconversationfound")
    }

    fn screen_text(&self, id: &str) -> String {
        let screen = self.tabs.lock().unwrap().get(id).map(|tab| tab.screen.clone());
        let Some(screen) = screen else { return String::new() };
        let mut screen = screen.lock().unwrap();
        String::from_utf8_lossy(screen.kept.make_contiguous()).into_owned()
    }

    fn still_running(&self, id: &str) -> bool {
        self.tabs.lock().unwrap().get(id).is_some_and(|tab| tab.session.is_some())
    }

    /// Drops a login a reused home may still hold, then skips Claude Code's theme wizard. A Go
    /// account must not send the user's Claude login to the gateway, and its first launch is a
    /// session, not a setup screen.
    fn prepare_go_home(&self, account: &Account) -> Result<(), String> {
        let home = self.accounts.home(account);
        crate::opencode_go::clear_login(&home, &self.launch.bin(Provider::Claude), self.launch.path_env.as_deref());
        crate::opencode_go::prepare_home(&home)
    }

    /// Opens a tab running the CLI's login for the account, in the account's home, and returns its id.
    pub fn sign_in(&self, account: &str) -> Result<TabId, String> {
        let account = self.accounts.get(account).ok_or_else(|| format!("no account {account} on this machine"))?;
        if account.auth == Auth::OpenCodeGo {
            return Err("OpenCode Go signs in with an API key. Replace the API key under Settings > Accounts.".into());
        }
        let home = self.accounts.home(&account);
        std::fs::create_dir_all(&home).map_err(|e| format!("could not make {}: {e}", home.display()))?;
        let id = self.add_tab(home, account, vec![], vec![], false, None);
        self.tab(&id, |t| t.program = Program::SignIn)?;
        if let Err(e) = self.start_claude(&id, UNWATCHED.0, UNWATCHED.1) {
            self.close_tab(&id);
            return Err(e);
        }
        Ok(id)
    }

    /// Starts the tab's program and returns its folder. A second call while that start is in
    /// flight, or while the session is already running and nothing new was queued, returns the
    /// folder and leaves the conversation alone. Viewers stay attached.
    pub fn start_claude(&self, tab: &str, cols: u16, rows: u16) -> Result<String, String> {
        self.launch_tab(tab, cols, rows, false)
    }

    fn relaunch(&self, tab: &str, cols: u16, rows: u16) -> Result<String, String> {
        self.launch_tab(tab, cols, rows, true)
    }

    fn launch_tab(&self, tab: &str, cols: u16, rows: u16, replace: bool) -> Result<String, String> {
        // A headless child's thread has no session of its own while the child works.
        if self.thread_task(tab, false).is_some_and(|task| self.delegates.get(&task).is_some_and(|t| t.thread.is_some_and(|thread| !thread.runs))) {
            return self.tab(tab, |t| t.cwd.display().to_string());
        }
        let prepared = self.tab(tab, |t| {
            if !replace && (t.session.is_some() || t.starting) && t.launch.is_empty() {
                return None;
            }
            t.starting = true;
            // Without launch arguments this is a fresh conversation, not the one recorded before.
            if t.launch.is_empty() {
                t.claude_session = None;
                t.tally = Tally::default();
            }
            let mut args = std::mem::take(&mut t.launch);
            if let Some(model) = t.model.clone().filter(|_| !args.iter().any(|a| a == "--model")) {
                args.extend(["--model".to_owned(), model]);
            }
            Some((t.cwd.clone(), t.account.clone(), t.program, args, t.env.clone(), t.bypass))
        })?;
        let Some((cwd, account, program, args, env, bypass)) = prepared else {
            return self.tab(tab, |t| t.cwd.display().to_string());
        };
        let mut args = args;
        let mut prompt = vec![];
        if self.device_agents.as_ref().is_some_and(|f| f.exists()) {
            prompt.push(crate::device::AGENT_PROMPT.to_owned());
        }
        let settings = self.global_settings();
        let role = role_of(&env).map(|(role, _)| role);
        let tastes = (global::is_global(&env) || role == Some(Role::Orchestrator)).then(|| self.tastes());
        if global::is_global(&env) {
            prompt.push(global::prompt(&settings.name, &crate::account::hostname()));
            prompt.extend(tastes.as_ref().map(|t| t.resolve(&TasteScope::Orchestrator).brief()));
        }
        if let Some(role) = role {
            if !role.disallowed_tools().is_empty() {
                // --disallowedTools takes any number of values, so the prompt's flag after it keeps it from swallowing the rest.
                args.push("--disallowedTools".to_owned());
                args.extend(role.disallowed_tools().iter().map(|t| t.to_string()));
            }
            prompt.push(role.prompt().to_owned());
            if role == Role::Orchestrator {
                let scoped = tastes.as_ref().map(|t| t.resolve(&captain_scope(&cwd)));
                prompt.extend(scoped.as_ref().map(taste::Taste::brief));
                prompt.extend(scoped.map(|t| t.limits().hint()));
                prompt.push(global::captain_prompt(&settings.name, settings.reports));
            }
        }
        if !prompt.is_empty() {
            args.extend(["--append-system-prompt".to_owned(), prompt.join("\n\n")]);
        }
        self.save_sessions();
        let env = [self.accounts.env(&account), env].concat();
        let plugin_dirs = self.shared_plugin_dirs(&account);
        let cmd = self.launch.command(program, &account, tab, &cwd, bypass, args, &env, &plugin_dirs);
        if let Err(error) = self.spawn(tab, cmd, pty::size(cols, rows)) {
            let _ = self.tab(tab, |t| t.starting = false);
            return Err(error);
        }
        self.continue_when_limit_lifts(tab);
        self.emit_tabs();
        Ok(cwd.display().to_string())
    }

    /// Moves the tab's conversation to `account` and restarts it there, returning its folder. The
    /// transcript is copied into the account's home, where `--resume` finds it, and a turn that
    /// stopped, as on a usage limit, continues at once instead of waiting for the old account.
    pub fn switch_account(&self, tab: &str, account: &str, cols: u16, rows: u16) -> Result<String, String> {
        let to = self.accounts.get(account).ok_or_else(|| format!("no account {account} on this machine"))?;
        self.move_to_account(tab, to, None, cols, rows)
    }

    /// Moves the tab to `to`, keeping its conversation. `model` replaces the picked model, or when
    /// absent, a model from another auth is dropped instead of carried into the new account.
    fn move_to_account(&self, tab: &str, to: Account, model: Option<&str>, cols: u16, rows: u16) -> Result<String, String> {
        let (from, program, session, working) = self.tab(tab, |t| {
            let working = t.status == Some(AgentStatus::Working) || (t.status == Some(AgentStatus::NeedsInput) && t.interrupted);
            (t.account.clone(), t.program, t.claude_session.clone(), working)
        })?;
        if program != Program::Agent {
            return Err("a sign-in tab has no conversation to move".into());
        }
        if to.provider != from.provider {
            return Err(format!("account {} runs {}, which can't resume a {} conversation", to.id, to.provider.spec().name, from.provider.spec().name));
        }
        if to.id == from.id {
            return self.tab(tab, |t| t.cwd.display().to_string());
        }
        if working {
            return Err("claude is working in this tab. Switch accounts when the turn ends.".into());
        }
        if !self.status(&to).signed_in {
            return Err(format!("account {} is not signed in on this machine", to.id));
        }
        if let Some(session) = &session {
            let file = self.find_transcript(&from, session).ok_or_else(|| format!("could not find the transcript of {session} under account {}", from.id))?;
            let project = file.parent().and_then(Path::file_name).ok_or("the transcript has no project folder")?;
            let into = self.accounts.home(&to).join("projects").join(project);
            std::fs::create_dir_all(&into).and_then(|_| copy_tree(file.parent().unwrap(), &into, session)).map_err(|e| format!("could not copy the conversation to {}: {e}", to.id))?;
        }
        self.tab(tab, |t| {
            match model {
                Some(model) => t.model = Some(model.to_owned()),
                // A model id belongs to the auth that served it. Moving between a Claude login and
                // a Go key must not carry one gateway's model into the other.
                None if t.account.auth != to.auth => t.model = None,
                None => {}
            }
            t.account = to;
            t.resume_at = None;
            if let Some(saved) = t.saved() {
                (t.launch, t.resume_at) = saved.reopen(unix_now(), t.account.provider);
            }
        })?;
        self.relaunch(tab, cols, rows)
    }

    /// Turns the tab's permission skipping on or off and returns its folder. Claude reads the flag
    /// only at launch, so the tab restarts on its own conversation.
    pub fn set_bypass(&self, tab: &str, on: bool, cols: u16, rows: u16) -> Result<String, String> {
        let unchanged = self.tab(tab, |t| {
            let unchanged = t.bypass == on && t.session.is_some();
            t.bypass = on;
            unchanged.then(|| t.cwd.display().to_string())
        })?;
        match unchanged {
            Some(cwd) => Ok(cwd),
            None => self.restart_claude(tab, cols, rows),
        }
    }

    /// The GitHub repositories of the folders `paths`, github.com's first, once each, on the hosts gh can reach.
    fn gh_repos(&self, paths: &[String]) -> Vec<pulls::Repo> {
        let path = self.launch.path_env.as_deref();
        let mut repos: Vec<pulls::Repo> = paths.iter().filter_map(|p| git_remote(Path::new(p))).filter_map(|r| pulls::remote_repo(&r)).collect();
        repos.sort_by_key(|r| (r.host != pulls::GITHUB, r.host.clone(), r.name.to_lowercase()));
        repos.dedup_by(|a, b| a.host == b.host && a.name.eq_ignore_ascii_case(&b.name));
        let mut known = HashMap::new();
        repos.retain(|r| *known.entry(r.host.clone()).or_insert_with(|| pulls::gh_knows(&home(), path, &r.host)));
        repos
    }

    /// Where the projects' scripts are kept, beside the saved sessions.
    fn scripts_file(&self) -> Option<PathBuf> {
        self.sessions_file.as_ref().map(|f| f.with_file_name("scripts.json"))
    }

    /// The tab's folder, refused while claude works there: `what` names what to do once the turn ends.
    fn idle_folder(&self, tab: &str, what: &str) -> Result<PathBuf, String> {
        // A permission prompt mid-turn reports NeedsInput with the turn still open.
        let (cwd, working) = self.tab(tab, |t| (t.cwd.clone(), t.status == Some(AgentStatus::Working) || (t.status == Some(AgentStatus::NeedsInput) && t.interrupted)))?;
        if working {
            return Err(format!("claude is working in this folder. {what} when the turn ends."));
        }
        Ok(cwd)
    }

    /// Moves the tab to a new worktree of its project (`"new"`), an existing one, or the project's
    /// own checkout (`None`), and restarts claude there. A worktree made for the tab and left
    /// unused goes when the tab moves on.
    pub fn set_workspace(&self, tab: &str, worktree: Option<&str>, cols: u16, rows: u16) -> Result<String, String> {
        let cwd = self.idle_folder(tab, "Change the workspace")?;
        let path = self.launch.path_env.as_deref();
        let project = history::project_of(&cwd);
        let target = match worktree {
            None => project.clone(),
            Some("new") => branches::new_worktree(&cwd, path)?,
            Some(dir) => {
                let dir = PathBuf::from(dir);
                if !dir.is_dir() || history::project_of(&dir) != project {
                    return Err(format!("{} is not a worktree of {}", dir.display(), project.display()));
                }
                dir
            }
        };
        let left = self.tab(tab, |t| {
            let made = t.made.take();
            t.made = if worktree == Some("new") || made.as_ref() == Some(&target) { Some(target.clone()) } else { None };
            t.cwd = target.clone();
            made.filter(|m| *m != target)
        })?;
        if let Some(made) = left {
            branches::drop_worktree(&made, path);
        }
        self.relaunch(tab, cols, rows)
    }

    /// Restarts the tab's claude on its conversation with `model`, for this conversation alone, and
    /// returns its folder. Claude's own /model saves its pick as the default for every session.
    /// With `account`, a model from another account moves the conversation there first.
    pub fn set_model(&self, tab: &str, model: &str, cols: u16, rows: u16, account: Option<&str>) -> Result<String, String> {
        if model.starts_with('-') || !model.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.[]_".contains(&b)) {
            return Err(format!("{model:?} is not a model name"));
        }
        if let Some(id) = account.filter(|id| !id.is_empty()) {
            let to = self.accounts.get(id).ok_or_else(|| format!("no account {id} on this machine"))?;
            if self.tab(tab, |t| t.account.id != to.id)? {
                return self.move_to_account(tab, to, Some(model), cols, rows);
            }
        }
        self.tab(tab, |t| {
            t.model = Some(model.to_owned());
            if let Some(saved) = t.saved() {
                (t.launch, t.resume_at) = saved.reopen(unix_now(), t.account.provider);
            }
        })?;
        self.relaunch(tab, cols, rows)
    }

    /// Starts claude again on the tab's conversation and returns its folder; a tab with no
    /// conversation yet starts a fresh one.
    pub fn restart_claude(&self, tab: &str, cols: u16, rows: u16) -> Result<String, String> {
        self.tab(tab, |t| {
            if let Some(saved) = t.saved() {
                (t.launch, t.resume_at) = saved.reopen(unix_now(), t.account.provider);
            }
        })?;
        self.relaunch(tab, cols, rows)
    }

    fn spawn(&self, tab: &str, cmd: CommandBuilder, size: PtySize) -> Result<(), String> {
        let (old, screen) = self.tab(tab, |t| {
            t.held.clear();
            (t.session.take(), t.screen.clone())
        })?;
        let session = screen.lock().unwrap().restart();
        // Only once the screen has moved on: dropping a session echoes the EOF it sends as "^D", which
        // would otherwise reach the viewers. It kills it silently, so its exit can't be mistaken for the new one's.
        drop(old);
        let started = SystemTime::now();
        let (me, exited) = (self.me.clone(), tab.to_owned());
        let pty = pty::Session::spawn(
            cmd,
            size,
            move |bytes| screen.lock().unwrap().push(session, &bytes),
            move || {
                if let Some(machine) = me.upgrade() {
                    machine.exited(&exited, session);
                }
            },
        )?;
        self.tab(tab, |t| {
            t.session = Some(pty);
            t.starting = false;
            t.started = Some(started);
            (t.status, t.title, t.message) = (None, None, None);
            t.turn_pending = None;
            t.draft = Draft::default();
        })?;
        self.type_held_once_raw(tab, session);
        Ok(())
    }

    /// Watches the session until it takes the terminal, then types the prompts held for it. The
    /// program says nothing when it does, so the poll asks the pty.
    fn type_held_once_raw(&self, tab: &str, session: u64) {
        let (me, tab) = (self.me.clone(), tab.to_owned());
        std::thread::spawn(move || loop {
            let Some(machine) = me.upgrade() else { return };
            let Ok(screen) = machine.tab(&tab, |t| t.screen.clone()) else { return };
            if screen.lock().unwrap().session != session {
                return;
            }
            let done = machine.tab(&tab, |t| match t.session.as_mut() {
                Some(s) if s.raw() => {
                    let mut wrote = false;
                    for text in std::mem::take(&mut t.held) {
                        if s.write(&paste(&text)).is_ok() {
                            t.turn_pending = Some(Instant::now());
                            wrote = true;
                        }
                    }
                    (true, wrote)
                }
                Some(_) => (false, false),
                None => (true, false),
            });
            let (done, wrote) = done.unwrap_or((true, false));
            if wrote {
                machine.arm_delegate_pass();
            }
            if done {
                return;
            }
            drop(machine);
            std::thread::sleep(std::time::Duration::from_millis(10));
        });
    }

    /// Replays the tab's screen to `sink`, then streams its output there until `detach`.
    pub fn attach(&self, tab: &str, viewer: ViewerId, sink: Sink) -> Result<(), String> {
        if Shells::owns(tab) {
            return self.shells.attach(tab, viewer, sink);
        }
        let screen = self.tab(tab, |t| t.screen.clone())?;
        screen.lock().unwrap().attach(viewer, sink);
        Ok(())
    }

    pub fn detach(&self, tab: &str, viewer: &str) {
        if Shells::owns(tab) {
            return self.shells.detach(tab, viewer);
        }
        if let Ok(screen) = self.tab(tab, |t| t.screen.clone()) {
            screen.lock().unwrap().detach(viewer);
        }
    }

    /// What the app's Stop types. Esc alone ends the turn but leaves background agents and workflows
    /// running, so claude's ctrl+x ctrl+k stops them first; their "stopped by the user" notices then
    /// wake claude for a turn of its own, which the last Esc ends.
    pub fn stop_turn(&self, tab: &str) -> Result<(), String> {
        const KILL_AGENTS: &[u8] = b"\x18\x0b";
        for keys in [KILL_AGENTS, KILL_AGENTS, b"\x1b"] {
            self.write_input(tab, keys)?;
            std::thread::sleep(Duration::from_millis(300));
        }
        std::thread::sleep(Duration::from_millis(1200));
        self.write_input(tab, b"\x1b")
    }

    pub fn write_input(&self, tab: &str, data: &[u8]) -> Result<(), String> {
        if Shells::owns(tab) {
            return self.shells.write(tab, data);
        }
        let (submitted, mode_unknown) = self.tab(tab, |t| {
            let submitted = note_draft(&mut t.draft, data);
            if submitted {
                t.note_submit();
            }
            let mode_unknown = t.account.provider.spec().harness && !t.permission_unknown && data.windows(3).any(|window| window == b"\x1b[Z");
            if mode_unknown {
                t.permission_unknown = true;
            }
            if let Some(s) = t.session.as_mut() {
                s.write(data).map_err(|e| e.to_string())?;
            }
            Ok::<_, String>((submitted, mode_unknown))
        })??;
        if mode_unknown {
            self.save_sessions();
        }
        if submitted {
            self.arm_delegate_pass();
        }
        self.maybe_deliver(tab);
        Ok(())
    }

    pub fn resize_pty(&self, tab: &str, cols: u16, rows: u16) -> Result<(), String> {
        if Shells::owns(tab) {
            return self.shells.resize(tab, cols, rows);
        }
        self.tab(tab, |t| match t.session.as_ref() {
            Some(s) => s.resize(pty::size(cols, rows)),
            None => Ok(()),
        })?
    }

    /// Ends the tab's session and forgets it. Answers whether no tab is left.
    pub fn close_tab(&self, tab: &str) -> bool {
        let path = self.launch.path_env.clone();
        for id in self.delegates.running_under(tab) {
            let _ = self.delegates.cancel_tree(&id, "the parent tab closed", path.as_deref());
        }
        if let Some(task) = self.thread_task(tab, false) {
            let _ = self.delegates.cancel_tree(&task, "its thread was closed", path.as_deref());
        }
        let (closed, owned, last) = {
            let mut tabs = self.tabs.lock().unwrap();
            let closed = tabs.remove(tab);
            let owned: Vec<PathBuf> = tabs.values().filter_map(|t| t.made.clone()).collect();
            (closed, owned, tabs.is_empty())
        };
        let made = closed.as_ref().and_then(|t| t.made.clone());
        drop(closed);
        self.delegates.drop_kept_worktrees(tab, &owned, path.as_deref());
        if let Some(made) = made {
            if !owned.iter().any(|dir| dir == &made) {
                branches::drop_worktree(&made, path.as_deref());
            }
        }
        self.save_sessions();
        self.emit_tabs();
        last
    }

    /// Moves the tab to the folder at `path`, recorded as a project, and returns it canonical.
    /// The next `start_claude` runs there.
    pub fn set_folder(&self, tab: &str, path: &str) -> Result<String, String> {
        if !Path::new(path).is_dir() {
            return Err(format!("{path} is not a folder on this machine"));
        }
        let dir = self.open_project(path.into());
        let account = self.account_for(None, &dir)?;
        self.tab(tab, |t| (t.cwd, t.account) = (dir.clone(), account))?;
        self.emit_tabs();
        Ok(dir.display().to_string())
    }

    /// Records `path` as a project opened now and returns its canonical form.
    pub fn open_project(&self, path: PathBuf) -> PathBuf {
        let path = dunce::canonicalize(&path).unwrap_or(path);
        let mut projects = self.projects.lock().unwrap();
        projects::upsert(&mut projects, path.clone(), unix_now());
        if let Some(file) = &self.projects_file {
            projects::save(file, &projects);
        }
        path
    }

    pub fn list_projects(&self) -> Vec<ProjectView> {
        let paths: Vec<PathBuf> = projects::recent(&self.projects.lock().unwrap()).into_iter().map(|p| p.path.clone()).collect();
        paths.iter().map(|path| self.project_view(path)).collect()
    }

    fn project_view(&self, path: &Path) -> ProjectView {
        ProjectView {
            path: path.display().to_string(),
            name: path.file_name().map_or_else(|| "/".into(), |n| n.to_string_lossy().into_owned()),
            repo: git_remote(path).and_then(|remote| repo_key(&remote)),
            account: self.account_for(None, path).map_or_else(|_| accounts::default_account(), |a| a.id),
        }
    }

    /// Runs the project's later sessions under `account`, or the default account for none.
    pub fn set_project_account(&self, path: &str, account: Option<String>) -> Result<ProjectView, String> {
        if let Some(id) = account.as_deref().filter(|id| self.accounts.get(id).is_none()) {
            return Err(format!("no account {id} on this machine"));
        }
        let path = canonical(path);
        {
            let mut projects = self.projects.lock().unwrap();
            let project = projects.iter_mut().find(|p| p.path == path).ok_or_else(|| format!("{} is not a project on this machine", path.display()))?;
            project.account = account;
            if let Some(file) = &self.projects_file {
                projects::save(file, &projects);
            }
        }
        Ok(self.project_view(&path))
    }

    /// The project's orchestrator tab: the one already open, else a new one, not yet started.
    pub fn open_orchestrator(&self, path: &str) -> Result<TabId, String> {
        if !Path::new(path).is_dir() {
            return Err(format!("{path} is not a folder on this machine"));
        }
        let project = self.open_project(path.into());
        let open = self.tabs.lock().unwrap().iter().filter(|(_, t)| role_of(&t.env) == Some((Role::Orchestrator, project.clone()))).map(|(id, _)| id.clone()).min_by_key(|id| tab_number(id));
        if let Some(id) = open {
            return Ok(id);
        }
        let account = self.account_for(None, &project)?;
        // Its tools and its workers' reports reach it through pinned's MCP server and hooks, which only Claude Code loads.
        if !account.provider.spec().harness {
            return Err(format!("account {} runs {}, which can't be a project's orchestrator", account.id, account.provider.spec().name));
        }
        let id = self.add_tab(project.clone(), account, vec![], role_env(Role::Orchestrator, &project), false, None);
        self.emit_tabs();
        Ok(id)
    }

    /// This machine's global orchestrator tab: the one already open, else a new one, not yet
    /// started, in its folder, which is made when missing and never becomes a project.
    pub fn open_global_orchestrator(&self) -> Result<TabId, String> {
        std::fs::create_dir_all(&self.global_dir).map_err(|e| format!("could not make {}: {e}", self.global_dir.display()))?;
        let folder = self.global_folder();
        let open = self.tabs.lock().unwrap().iter().filter(|(_, t)| global::is_global(&t.env)).map(|(id, _)| id.clone()).min_by_key(|id| tab_number(id));
        if let Some(id) = open {
            return Ok(id);
        }
        let account = self.account_for(None, &folder)?;
        if !account.provider.spec().harness {
            return Err(format!("account {} runs {}, which can't be the global orchestrator", account.id, account.provider.spec().name));
        }
        let id = self.add_tab(folder, account, vec![], global::env(), false, None);
        self.emit_tabs();
        Ok(id)
    }

    /// A new tab, not yet started, that adjusts `scope`'s way of working: in the project folder
    /// `path`, or else in the folder for threads without a project. Its first prompt names the scope.
    pub fn open_taste_chat(&self, scope: &TasteScope, path: Option<&str>) -> Result<TabId, String> {
        let cwd = match path {
            Some(path) if Path::new(path).is_dir() => canonical(path),
            Some(path) => return Err(format!("{path} is not a folder on this machine")),
            None => {
                std::fs::create_dir_all(&self.scratch).map_err(|e| format!("could not make {}: {e}", self.scratch.display()))?;
                self.scratch.clone()
            }
        };
        let account = self.account_for(None, &cwd)?;
        // The taste tools reach it through pinned's MCP server, which only Claude Code loads.
        if !account.provider.spec().harness {
            return Err(format!("account {} runs {}, which can't adjust the way of working", account.id, account.provider.spec().name));
        }
        let id = self.add_tab(cwd, account, vec![taste::chat_prompt(scope)], taste::chat_env(), false, None);
        self.emit_tabs();
        Ok(id)
    }

    /// Applies `change` to the account's tastes: on the hub while this machine is linked, keeping
    /// the answer in `taste.json`, or else to `taste.json` itself. An open Way of working page hears of it.
    fn change_tastes(&self, call: HubCall, change: impl FnOnce(&mut Tastes) -> Result<(), String>) -> Result<Tastes, String> {
        let dir = self.orchestration_dir.as_deref();
        let tastes = match self.hub.lock().unwrap().clone() {
            Some(hub) => {
                let tastes: Tastes = serde_json::from_value(hub(call)?).map_err(|e| format!("the hub's answer did not read: {e}"))?;
                if let Err(e) = taste::save(&tastes, dir) {
                    eprintln!("pinned: could not keep the account's taste: {e}");
                }
                tastes
            }
            None => taste::change(dir, change)?,
        };
        self.emit(Event::TastesChanged);
        Ok(tastes)
    }

    fn global_folder(&self) -> PathBuf {
        dunce::canonicalize(&self.global_dir).unwrap_or_else(|_| self.global_dir.clone())
    }

    pub fn global_settings(&self) -> global::Settings {
        self.global.lock().unwrap().clone()
    }

    pub fn set_global_settings(&self, settings: global::Settings) -> Result<global::Settings, String> {
        let settings = settings.cleaned();
        settings.save(self.orchestration_dir.as_deref())?;
        *self.global.lock().unwrap() = settings.clone();
        Ok(settings)
    }

    /// Hands `text` from the global orchestrator `sender` to the Capitão of the project at `path`,
    /// opening the Capitão when the project has none. It is typed now when the Capitão is idle,
    /// becomes its first prompt when it is stopped or has no conversation yet, and otherwise waits
    /// in its queue, or is refused as busy under `BusyPolicy::Refuse`. A repeat of `request_id`
    /// answers the first outcome without typing. The ledger keeps which turn the message started,
    /// so the Capitão's reply can go back up when that turn ends.
    pub fn enqueue_to_captain(&self, path: &str, text: &str, request_id: &str, sender: &str, busy: Option<BusyPolicy>) -> Result<Handoff, String> {
        let (request_id, text) = (request_id.trim(), text.trim());
        if request_id.is_empty() {
            return Err("request_id is required".into());
        }
        if text.is_empty() {
            return Err("the message is empty".into());
        }
        if canonical(path) == self.global_folder() {
            return Err(format!("{path} is the global orchestrator's folder, which has no Capitão"));
        }
        let settings = self.global_settings();
        let mut sent = self.sent.lock().unwrap();
        if let Some(earlier) = sent.find(request_id) {
            return if earlier.project == canonical(path) { Ok(earlier.handoff.clone()) } else { Err(format!("request_id {request_id} was already used for another message")) };
        }
        let tab = self.open_orchestrator(path)?;
        let project = canonical(path);
        let sender = if sender.trim().is_empty() { settings.name } else { global::Settings { name: sender.to_owned(), ..global::Settings::default() }.cleaned().name };
        let handoff = self.hand_to(&tab, &format!("[{sender}] {text}"), request_id, busy.unwrap_or(settings.busy), &sent)?;
        let progress = match handoff {
            Handoff::Busy { .. } => return Ok(handoff),
            Handoff::Queued { .. } => Progress::Queued,
            Handoff::Delivered { .. } | Handoff::Started { .. } => Progress::InFlight,
        };
        sent.record(global::Sent { request_id: request_id.to_owned(), sender, project, handoff: handoff.clone(), progress, at: unix_now() });
        drop(sent);
        if progress == Progress::InFlight {
            self.emit_tabs();
        }
        Ok(handoff)
    }

    /// Hands a Capitão's report to this machine's global orchestrator as `CaptainReport::line`,
    /// opening it when missing. It never refuses as busy. A repeat of `request_id` answers the first
    /// outcome without typing.
    pub fn deliver_report(&self, request_id: &str, report: &CaptainReport) -> Result<Handoff, String> {
        let request_id = request_id.trim();
        if request_id.is_empty() {
            return Err("request_id is required".into());
        }
        let mut sent = self.sent.lock().unwrap();
        if let Some(earlier) = sent.find(request_id) {
            return if earlier.project == self.global_folder() { Ok(earlier.handoff.clone()) } else { Err(format!("request_id {request_id} was already used for another message")) };
        }
        let tab = self.open_global_orchestrator()?;
        let handoff = self.hand_to(&tab, &report.line(), request_id, BusyPolicy::Queue, &sent)?;
        let progress = if matches!(handoff, Handoff::Queued { .. }) { Progress::Queued } else { Progress::InFlight };
        sent.record(global::Sent { request_id: request_id.to_owned(), sender: report.project.clone(), project: self.global_folder(), handoff: handoff.clone(), progress, at: unix_now() });
        Ok(handoff)
    }

    /// Types `line` into `tab` now when its session is idle, or starts the session on it as its
    /// first prompt when it is stopped or has no conversation yet. Otherwise it queues `line` with
    /// the tab's other news, or answers busy under `BusyPolicy::Refuse`. Starting with the prompt
    /// instead of typing it keeps a startup dialog from taking its Enter.
    fn hand_to(&self, tab: &str, line: &str, request_id: &str, busy: BusyPolicy, sent: &global::Ledger) -> Result<Handoff, String> {
        #[derive(PartialEq)]
        enum Ready {
            Stopped,
            /// Running without a conversation, with nothing typed or started there yet.
            Fresh,
            Idle,
            Busy,
        }
        let ours = sent.started(tab);
        let ready = self.tab(tab, |t| match &t.session {
            None if t.starting => Ready::Busy,
            None => Ready::Stopped,
            Some(session) if session.raw() && t.notice_idle() => Ready::Idle,
            Some(_) if t.claude_session.is_none() && t.status.is_none() && t.turn_pending.is_none() && t.held.is_empty() && t.draft.empty() && !ours => Ready::Fresh,
            Some(_) => Ready::Busy,
        })?;
        let id = tab.to_owned();
        match ready {
            Ready::Stopped | Ready::Fresh => {
                self.tab(tab, |t| {
                    let mut args = t.claude_session.as_deref().map(|session| crate::delegate::resume_args(t.account.provider, session)).unwrap_or_default();
                    args.push(if line.starts_with('-') { format!("\n{line}") } else { line.to_owned() });
                    t.launch = args;
                })?;
                if ready == Ready::Fresh {
                    self.relaunch(tab, UNWATCHED.0, UNWATCHED.1)?;
                } else {
                    self.start_claude(tab, UNWATCHED.0, UNWATCHED.1)?;
                }
                Ok(Handoff::Started { tab: id })
            }
            Ready::Idle if self.write_notice(tab, line) == Ok(true) => Ok(Handoff::Delivered { tab: id }),
            Ready::Idle | Ready::Busy if busy == BusyPolicy::Refuse => Ok(Handoff::Busy { tab: id }),
            Ready::Idle | Ready::Busy => {
                let session = self.tab(tab, |t| t.claude_session.clone())?;
                self.delegates.queue_request(tab, session, line.to_owned(), request_id.to_owned());
                self.arm_delegate_pass();
                self.emit_tabs();
                Ok(Handoff::Queued { position: sent.queued_on(tab) + 1, tab: id })
            }
        }
    }

    /// Hands `message` to `project`'s Capitão: on this machine when the project is here and
    /// `machine` names no other, else on the machine the hub finds.
    fn send_to_captain(&self, project: &str, machine: Option<&str>, message: &str, request_id: &str) -> Result<Handoff, String> {
        let sender = self.global_settings().name;
        match self.local_project(project, machine) {
            Some(path) => self.enqueue_to_captain(&path.display().to_string(), message, request_id, &sender, None),
            None => self.route_remote(project, machine, message, request_id, &sender),
        }
    }

    /// `send_to_captain` with `box`: the hub opens the Capitão of `project`'s repository in an
    /// Upstash Box, with the account's connection, or hands the message to the box that runs it.
    fn send_to_captain_box(&self, project: &str, machine: Option<&str>, boxed: crate::delegate::BoxTarget, message: &str, request_id: &str) -> Result<Value, String> {
        let repo = match boxed.repo {
            Some(repo) => repo,
            None => self.project_origin(project, machine)?,
        };
        let settings = self.global_settings();
        let to = global::RelayTo::CaptainBox { repo, size: boxed.size, text: message.to_owned(), sender: settings.name, busy: Some(settings.busy) };
        self.call_hub(HubCall::Relay { request_id: request_id.to_owned(), to }).map_err(needs_update)
    }

    /// The git origin of `project`: its checkout here, else the repo the hub's overview lists for it.
    fn project_origin(&self, project: &str, machine: Option<&str>) -> Result<String, String> {
        if let Some(remote) = self.local_project(project, machine).as_deref().and_then(git_remote) {
            return Ok(remote);
        }
        let overview = self.projects_overview()?;
        let listed = overview.projects.iter().find(|p| p.key == project || p.repo.as_deref() == Some(project) || p.checkouts.iter().any(|c| c.path == project));
        match listed {
            Some(p) => p.repo.clone().ok_or_else(|| format!("{project} has no git origin, so no box can clone it")),
            None if project.contains('/') && !is_absolute_anywhere(project) => Ok(project.to_owned()),
            None => Err(format!("{project} is not a project of this account")),
        }
    }

    /// The folder on this machine that `project` names, as a key `projects` lists (`repo`, or
    /// `<machine>:<path>`) or as a folder, unless `machine` names another machine.
    fn local_project(&self, project: &str, machine: Option<&str>) -> Option<PathBuf> {
        if machine.is_some_and(|m| !self.is_this_machine(m)) {
            return None;
        }
        let project = project.trim();
        let path = if Path::new(project).is_absolute() {
            Some(project)
        } else {
            project.split_once(':').filter(|(m, path)| Path::new(path).is_absolute() && self.is_this_machine(m)).map(|(_, path)| path)
        };
        let projects = self.list_projects();
        match path {
            Some(path) => {
                let path = canonical(path);
                projects.iter().any(|p| Path::new(&p.path) == path).then_some(path)
            }
            None => projects.into_iter().find(|p| p.repo.as_deref() == Some(project)).map(|p| PathBuf::from(p.path)),
        }
    }

    /// Whether `machine`, an id or a name from `projects`, is this machine. Without the hub's id
    /// for it, only its name matches.
    fn is_this_machine(&self, machine: &str) -> bool {
        let machine = machine_named(machine);
        let id = self.hub_id.lock().unwrap();
        (!id.is_empty() && machine == *id) || machine.eq_ignore_ascii_case(&crate::account::hostname())
    }

    /// Every project of the account for the global orchestrator's `projects` tool: the hub's view
    /// of every machine, or this machine's alone while it is not linked or the hub can't answer.
    fn projects_overview(&self) -> Result<global::Overview, String> {
        match self.call_hub(HubCall::Overview) {
            Ok(overview) => serde_json::from_value(overview).map_err(|e| format!("the hub's overview did not read: {e}")),
            Err(e) if e == NOT_LINKED || crate::link::is_unknown_command(&e) => Ok(self.local_overview()),
            Err(e) => Err(e),
        }
    }

    /// This machine alone, under its hub id once it has one, so its keys and links match the hub's.
    fn local_overview(&self) -> global::Overview {
        let (projects, tabs) = (self.list_projects(), self.tabs());
        let (id, name) = self.machine_ids();
        let here = [global::MachineSnapshot { id: &id, name: &name, online: true, version: env!("CARGO_PKG_VERSION"), projects: &projects, tabs: &tabs, problem: None }];
        global::overview(&here)
    }

    /// This machine's id and name on the hub, each its hostname until the hub's hello.
    fn machine_ids(&self) -> (String, String) {
        let host = crate::account::hostname();
        let id = Some(self.hub_id.lock().unwrap().clone()).filter(|id| !id.is_empty()).unwrap_or_else(|| host.clone());
        let name = Some(self.hub_name.lock().unwrap().clone()).filter(|name| !name.is_empty()).unwrap_or(host);
        (id, name)
    }

    /// Every session of the account in `project` and on `machine` when given, for the `sessions`
    /// tool, with every machine of the account so an offline one or one that needs an update shows.
    fn sessions(&self, project: Option<&str>, machine: Option<&str>) -> Result<String, String> {
        let overview = self.projects_overview()?;
        let (sessions, machines) = match overview.sessions {
            Some(sessions) => (sessions, overview.machines),
            None => {
                let here = self.local_overview();
                (here.sessions.unwrap_or_default(), here.machines)
            }
        };
        let sessions: Vec<global::Session> = sessions.into_iter().filter(|s| s.matches(project, machine)).collect();
        text_json(json!({ "sessions": sessions, "machines": machines }))
    }

    /// The last `items` items of the session in tab `id`, here when this machine has it and
    /// `machine` names no other, else through the hub on the machine that has it.
    fn read_session(&self, id: &str, machine: Option<&str>, items: Option<usize>) -> Result<String, String> {
        let items = read_items(items);
        if machine.is_none_or(|m| self.is_this_machine(m)) {
            if let Some(mut out) = self.read_session_here(id, items) {
                let (machine, machine_name) = self.machine_ids();
                (out["id"], out["machine"], out["machine_name"]) = (json!(id), json!(machine), json!(machine_name));
                return text_json(out);
            }
        }
        let call = HubCall::ReadSession { tab: id.to_owned(), machine: machine.map(|m| machine_named(m).to_owned()), items: Some(items) };
        match self.call_hub(call) {
            Ok(out) => text_json(out),
            Err(e) if e == NOT_LINKED => Err(format!("no session {id} on this machine, and it is not linked to a hub to look on the others")),
            Err(e) => Err(needs_update(e)),
        }
    }

    /// What waits on the user across the account's machines, for the `waiting` tool.
    fn waiting(&self) -> Result<Vec<global::Waiting>, String> {
        Ok(self.projects_overview()?.waiting)
    }

    /// The latest replies of `project`'s Capitão, for the `read_captain` tool, as the hub's MCP
    /// `read_orchestrator` answers them. A Capitão on another machine is read through the hub.
    fn read_captain(&self, project: &str, machine: Option<&str>) -> Result<String, String> {
        let Some(path) = self.local_project(project, machine) else {
            let (machine, path) = self.remote_checkout(project, machine)?;
            return text_json(self.call_hub(HubCall::ReadCaptain { machine, path }).map_err(needs_update)?);
        };
        let path = path.display().to_string();
        let tabs = self.tabs();
        let mut out = match global::captain_tab(&tabs, &path) {
            Some(tab) if tab.running || tab.session.is_some() => {
                let from = self.transcript(&tab.id).map_or(0, |file| thread::last_page(&file));
                reading::read(tab, self.thread(&tab.id, from).and_then(json), reading::DEFAULT_ITEMS)
            }
            _ => json!({ "state": "not_started", "next": reading::next_for("not_started") }),
        };
        out["project"] = json!(path);
        text_json(out)
    }

    /// Hands a message to a Capitão on another machine through `HubCall::Relay`, under this
    /// machine's busy policy, since this machine hosts the orchestrator that sends it.
    fn route_remote(&self, project: &str, machine: Option<&str>, text: &str, request_id: &str, sender: &str) -> Result<Handoff, String> {
        let (target, path) = self.remote_checkout(project, machine)?;
        let to = global::RelayTo::Captain { machine: target, path, text: text.to_owned(), sender: sender.to_owned(), busy: Some(self.global_settings().busy) };
        let answer = self.call_hub(HubCall::Relay { request_id: request_id.to_owned(), to }).map_err(needs_update)?;
        serde_json::from_value(answer).map_err(|e| format!("the hub answered the relay with something unreadable: {e}"))
    }

    /// The machine id and folder of `project` on another machine. `project` is a key, a repo or a
    /// folder from the hub's overview, or `<machine>:<path>`. A machine, there or in `machine`, is
    /// an id or a name, with or without `@`. Without the hub's overview only an id names it.
    fn remote_checkout(&self, project: &str, machine: Option<&str>) -> Result<(crate::wire::MachineId, String), String> {
        let asked = project.trim();
        let (project, machine) = match asked.split_once(':') {
            Some((m, path)) if !is_absolute_anywhere(asked) && is_absolute_anywhere(path) => (path, Some(machine.unwrap_or(m))),
            _ => (asked, machine),
        };
        let machine = machine.map(machine_named);
        let named = machine.filter(|_| is_absolute_anywhere(project));
        let overview = self.call_hub(HubCall::Overview).and_then(|v| serde_json::from_value::<global::Overview>(v).map_err(|e| e.to_string()));
        let overview = match overview {
            Ok(overview) => overview,
            Err(e) => {
                return named.map(|m| (m.to_owned(), project.to_owned())).ok_or_else(|| {
                    format!("{asked} is not a project on this machine, and the hub can't list the other machines' projects ({e}): name it as <machine id>:<path>")
                });
            }
        };
        let is = |m: &str, id: &str, name: &str| m == id || m.eq_ignore_ascii_case(name);
        let checkouts: Vec<&global::OverviewCheckout> = overview
            .projects
            .iter()
            .flat_map(|p| p.checkouts.iter().map(move |c| (p, c)))
            .filter(|(p, c)| p.key == project || p.repo.as_deref() == Some(project) || c.path == project)
            .map(|(_, c)| c)
            .filter(|c| machine.is_none_or(|m| is(m, &c.machine, &c.machine_name)))
            .collect();
        let online: Vec<&global::OverviewCheckout> = checkouts.iter().copied().filter(|c| c.online).collect();
        let chosen = match (checkouts.as_slice(), online.as_slice()) {
            ([], _) => {
                // A folder the overview doesn't list, on a machine named by id or name.
                let Some(m) = named else { return Err(format!("{asked} is not a project on {}", machine.unwrap_or("any machine of the account"))) };
                let id = overview.machines.iter().find(|known| is(m, &known.id, &known.name)).map_or(m, |known| known.id.as_str());
                return Ok((id.to_owned(), project.to_owned()));
            }
            ([only], _) | (_, [only]) => *only,
            _ => {
                let names: Vec<&str> = checkouts.iter().map(|c| c.machine_name.as_str()).collect();
                return Err(format!("{asked} is on {}: pass machine to pick one", names.join(", ")));
            }
        };
        Ok((chosen.machine.clone(), chosen.path.clone()))
    }

    /// Sends a Capitão's report up to the account's global orchestrator: `deliver_report` when this
    /// machine hosts it, else `HubCall::Relay`, whose hub fills in this machine's id and name. The
    /// id is the hub's for this machine, so the chat links the report to the Capitão's thread.
    /// `answers` are the request ids the report ends.
    fn report_up(&self, captain: &str, project: &Path, text: String, answers: Vec<String>) -> Result<Handoff, String> {
        let report = CaptainReport {
            machine: self.hub_id.lock().unwrap().clone(),
            machine_name: crate::account::hostname(),
            project: project.file_name().map_or_else(|| project.display().to_string(), |n| n.to_string_lossy().into_owned()),
            path: project.display().to_string(),
            tab: captain.to_owned(),
            text,
            answers,
        };
        // Its own namespace, so a report's id never meets the id of a message the orchestrator sent.
        let request_id = format!("report:{:016x}", fresh_boot());
        if self.global_settings().host {
            return self.deliver_report(&request_id, &report);
        }
        let handoff = self.call_hub(HubCall::Relay { request_id, to: global::RelayTo::Orchestrator { report } })?;
        serde_json::from_value(handoff).map_err(|e| format!("the hub's answer did not read: {e}"))
    }

    /// Runs when the turn of `project`'s Capitão `captain` ends, before the tab's queue drains. The
    /// orchestrator's messages that started the turn are answered by it, and under
    /// `ReportPolicy::Auto` the Capitão's last reply goes up to the orchestrator with their ids.
    fn captain_turn_ended(&self, captain: &str, project: &Path, event: &AgentEvent) {
        let answers: Vec<String> = self.sent.lock().unwrap().answer(project).into_iter().map(|sent| sent.request_id).collect();
        if answers.is_empty() || self.global_settings().reports != global::ReportPolicy::Auto {
            return;
        }
        let text = match event.status {
            AgentStatus::Failed => format!("Parou com um erro: {}", event.message.as_deref().unwrap_or("desconhecido")),
            _ => truncate(&self.last_reply(captain).unwrap_or_else(|| "(no reply)".into()), REPORT_CHARS, "read_captain has the whole message"),
        };
        // Up to the hub it may wait a minute, which the hook that ended the turn must not.
        let (me, captain, project) = (self.me.clone(), captain.to_owned(), project.to_owned());
        std::thread::spawn(move || {
            let Some(machine) = me.upgrade() else { return };
            if let Err(e) = machine.report_up(&captain, &project, text, answers) {
                eprintln!("pinned: the Capitão {captain} of {} could not report to the global orchestrator: {e}", project.display());
            }
        });
    }

    fn project_tool(&self, tab: &str, tool: ProjectTool) -> Result<String, String> {
        let global = self.tab(tab, |t| global::is_global(&t.env))?;
        let only_global = || if global { Ok(()) } else { Err("only the global orchestrator has this tool".to_owned()) };
        match tool {
            ProjectTool::Agents if global => Err(NO_PROJECT_WORK.into()),
            ProjectTool::Agents => Ok(self.workers(tab, &self.orchestrated(tab)?)),
            ProjectTool::Projects => {
                only_global()?;
                // The sessions tool lists them; here they would bury the projects.
                text_json(global::Overview { sessions: None, ..self.projects_overview()? })
            }
            ProjectTool::Sessions { project, machine } => {
                only_global()?;
                self.sessions(project.as_deref(), machine.as_deref())
            }
            ProjectTool::ReadSession { id, machine, items } => {
                only_global()?;
                self.read_session(&id, machine.as_deref(), items)
            }
            ProjectTool::SendToCaptain { project, message, request_id, machine, boxed: Some(boxed) } => {
                only_global()?;
                text_json(self.send_to_captain_box(&project, machine.as_deref(), boxed, &message, &request_id)?)
            }
            ProjectTool::SendToCaptain { project, message, request_id, machine, boxed: None } => {
                only_global()?;
                text_json(self.send_to_captain(&project, machine.as_deref(), &message, &request_id)?)
            }
            ProjectTool::ReadCaptain { project, machine } => {
                only_global()?;
                self.read_captain(&project, machine.as_deref())
            }
            ProjectTool::Waiting => {
                only_global()?;
                text_json(self.waiting()?)
            }
            ProjectTool::TasteGet { .. } | ProjectTool::TasteSet { .. } | ProjectTool::TasteReset { .. } if !self.tab(tab, |t| taste::is_chat(&t.env))? => {
                Err("only a thread opened with Chat to adjust on the Way of working page has this tool".into())
            }
            ProjectTool::TasteGet { scope } => {
                let scope = TasteScope::parse(&scope)?;
                text_json(self.tastes_now()?.view(&scope))
            }
            ProjectTool::TasteSet { scope, patch } => {
                let scope = TasteScope::parse(&scope)?;
                let call = HubCall::SetTaste { scope: scope.clone(), patch: patch.clone() };
                text_json(self.change_tastes(call, |tastes| tastes.set(&scope, patch))?.view(&scope))
            }
            ProjectTool::TasteReset { scope, field } => {
                let scope = TasteScope::parse(&scope)?;
                let call = HubCall::ResetTasteField { scope: scope.clone(), field };
                let reset = |tastes: &mut Tastes| {
                    tastes.reset(&scope, field);
                    Ok(())
                };
                text_json(self.change_tastes(call, reset)?.view(&scope))
            }
            ProjectTool::TellOrchestrator { text } => match self.tab(tab, |t| role_of(&t.env))? {
                Some((Role::Orchestrator, project)) => text_json(self.report_up(tab, &project, text, vec![])?),
                _ => Err("only a project's Capitão reports to the global orchestrator".into()),
            },
            ProjectTool::MemorySearch { query, limit } => {
                let found = self.memories.search(&self.project_of_tab(tab)?, &query, limit.unwrap_or(SEARCH_LIMIT))?;
                if found.is_empty() {
                    return Ok("No memory matches that.".into());
                }
                Ok(found.iter().map(|m| format!("[{}] {}{}: {}", m.id, m.title, m.kind.as_ref().map_or(String::new(), |k| format!(" ({k})")), m.text)).collect::<Vec<_>>().join("\n"))
            }
            ProjectTool::MemorySave { title, text, kind, id } => {
                let draft = memory::Draft::new(&title, &text, kind.as_deref())?;
                Ok(match self.memories.save(&self.project_of_tab(tab)?, &draft, id)? {
                    memory::Saved::Added(m) => format!("Saved #{}.", m.id),
                    memory::Saved::Updated(m) => format!("Updated #{}.", m.id),
                    memory::Saved::Duplicate(m) => format!("Not saved: #{} \"{}\" already says nearly this: {}\nTo change it, call memory_save with id {}.", m.id, m.title, m.text, m.id),
                })
            }
            ProjectTool::MemoryForget { id } => self.memories.forget(&self.project_of_tab(tab)?, id).map(|m| format!("Forgot #{} \"{}\".", m.id, m.title)),
        }
    }

    /// The memories of the tab's project that bear on the prompt starting a turn, for the hook to
    /// add to it. Nothing for any other event or a thread outside a project.
    fn recall(&self, event: &AgentEvent) -> String {
        let (AgentStatus::Working, Some(tab), Some(prompt)) = (event.status, &event.tab, &event.prompt) else { return String::new() };
        self.project_of_tab(tab).map(|project| self.memories.recall(&project, prompt)).unwrap_or_default()
    }

    /// The project `tab` may fan out to workers in: an orchestrator's or a worker's project, or
    /// a plain thread's own. A worker's workers nest under it; the worker cap is the only limit.
    fn orchestrated(&self, tab: &str) -> Result<PathBuf, String> {
        match self.tab(tab, |t| role_of(&t.env))? {
            Some((_, project)) => Ok(project),
            None => self.project_of_tab(tab),
        }
    }

    /// The project `path` names, if it is one of this machine's, or the global orchestrator's
    /// folder, whose memory is the global memory.
    fn known_project(&self, path: &str) -> Result<PathBuf, String> {
        let path = canonical(path);
        if self.projects.lock().unwrap().iter().any(|p| p.path == path) || path == self.global_folder() {
            Ok(path)
        } else {
            Err(format!("{} is not a project on this machine", path.display()))
        }
    }

    /// The project a tab works in; a worktree's is the repository's. A scratch thread has none.
    /// The global orchestrator's is its folder, which keeps the global memory.
    fn project_of_tab(&self, tab: &str) -> Result<PathBuf, String> {
        let (cwd, global) = self.tab(tab, |t| (t.cwd.clone(), global::is_global(&t.env)))?;
        if global {
            return Ok(self.global_folder());
        }
        let project = history::project_of(&cwd);
        if self.projects.lock().unwrap().iter().any(|p| p.path == project) {
            Ok(project)
        } else {
            Err("This thread is not in a project, so it has no memory. Memory belongs to a project folder opened in pinned.".into())
        }
    }

    /// The delegation limits a call from `tab` is held to: the project's limits in the account's
    /// tastes over the Captains', narrowed by the limits of the task the tab itself runs, when it
    /// is a worker.
    fn caller_limits(&self, tab: &str) -> Result<taste::Limits, String> {
        let (cwd, own_task) = self.tab(tab, |t| (t.cwd.clone(), t.task.clone()))?;
        let limits = self.tastes().resolve(&captain_scope(&history::project_of(&cwd))).limits();
        let outer = own_task.and_then(|id| self.delegates.lineage(&id)).map(|task| task.limits);
        Ok(outer.map_or(limits, |outer| limits.narrow(outer)))
    }

    /// Prices what each working worker thread's transcripts gained, its subagents' included,
    /// records it on its task, and stops the ones past their budget. The `meters` lock is held
    /// through the record, so the 1 s pass and a turn's end never write an older total over a newer
    /// one. Cursor and Codex children run headless and are not metered.
    fn meter_workers(&self) {
        let threads: Vec<(String, String)> = self
            .tabs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, t)| Some((t.task.clone()?, id.clone())))
            .filter(|(task, _)| self.delegates.get(task).is_some_and(|task| task.state == crate::delegate::WorkState::Working && task.thread.is_some_and(|thread| thread.runs)))
            .collect();
        let mut meters = self.meters.lock().unwrap();
        meters.retain(|task, _| threads.iter().any(|(working, _)| working == task));
        let mut moved = vec![];
        for (task, tab) in threads {
            let Some(file) = self.transcript_of(&tab).ok().flatten() else { continue };
            let meter = meters.entry(task.clone()).or_default();
            if meter.advance(&file).unwrap_or(false) && !meter.is_empty() {
                moved.push(task);
            }
        }
        if moved.is_empty() {
            return;
        }
        let (rates, book) = (self.meter_rates(), usage::PriceBook::load(self.prices_file().as_deref()));
        let used: Vec<_> = moved.into_iter().map(|task| (task.clone(), meters[&task].usage(&rates, &book))).collect();
        self.delegates.meter(&used, self.launch.path_env.as_deref());
    }

    /// The price list, read again each hour.
    fn meter_rates(&self) -> Arc<usage::RateTable> {
        let mut held = self.meter_rates.lock().unwrap();
        match &*held {
            Some((read, rates)) if read.elapsed() < Duration::from_secs(3600) => rates.clone(),
            _ => {
                let rates = Arc::new(usage::rates(self.rates_file.as_deref()));
                *held = Some((Instant::now(), rates.clone()));
                rates
            }
        }
    }

    /// The workers `from` sees, one per line, each indented under the worker that started it:
    /// the orchestrator sees every worker of its project, any other thread the ones it started.
    fn workers(&self, from: &str, project: &Path) -> String {
        let mut workers: Vec<TabView> = self.tabs.lock().unwrap().iter().filter(|(_, t)| role_of(&t.env).is_some_and(|(r, p)| r == Role::Worker && p == project)).map(|(id, t)| t.view(id, &HashSet::new(), &HashMap::new())).collect();
        workers.sort_by_key(|w| w.opened);
        let everyone = self.tab(from, |t| role_of(&t.env)).ok().flatten().is_some_and(|(role, _)| role == Role::Orchestrator);
        let is_worker = |id: &str| workers.iter().any(|w| w.id == id);
        let top = |w: &TabView| if everyone { !w.spawned_by.as_deref().is_some_and(is_worker) } else { w.spawned_by.as_deref() == Some(from) };
        let state = |w: &TabView| match (w.running, w.status) {
            (false, _) => "stopped",
            (true, None) => "starting",
            (true, Some(status)) => AgentStatus::ALL.iter().find(|(s, ..)| *s == status).map_or("", |(_, name, _)| name),
        };
        let mut lines = vec![];
        let mut seen = HashSet::new();
        let mut stack: Vec<(&TabView, usize)> = workers.iter().filter(|w| top(w)).rev().map(|w| (w, 0)).collect();
        while let Some((w, depth)) = stack.pop() {
            if !seen.insert(w.id.as_str()) {
                continue;
            }
            lines.push(format!("{}{}: {} ({}) in {}", "  ".repeat(depth), w.id, w.title.as_deref().unwrap_or("untitled"), state(w), w.cwd));
            stack.extend(workers.iter().filter(|c| c.spawned_by.as_deref() == Some(w.id.as_str())).rev().map(|c| (c, depth + 1)));
        }
        if lines.is_empty() {
            return "No workers yet.".into();
        }
        lines.join("\n")
    }

    /// The last text the tab's session wrote in its latest turn, from its transcript.
    fn last_reply(&self, tab: &str) -> Option<String> {
        thread::last_reply(&self.transcript(tab).ok()?).ok()?
    }

    /// Keeps what the tab's finished turn asked the user, which makes it wait on the user.
    fn note_question(&self, tab: &str) {
        let question = self.last_reply(tab).as_deref().and_then(global::question_in);
        let _ = self.tab(tab, |t| t.question = question);
    }

    /// Types a worker's news into the tab that spawned it, or into its project's orchestrator once
    /// that tab is closed: that it finished, with its last message, that it needs input, or that
    /// its turn failed. Dropped when neither is open.
    fn report_worker(&self, worker: &str, project: &Path, event: &AgentEvent, before: Option<AgentStatus>) {
        // A worker for another machine's tab reports only its task's end, through the hub.
        if self.tab(worker, |t| t.origin.is_some()).unwrap_or(false) {
            return;
        }
        let title = self.tab(worker, |t| t.title.clone()).ok().flatten().unwrap_or_else(|| "untitled".into());
        let what = match event.status {
            AgentStatus::Done => format!("finished. Its last message:\n\n{}", truncate(&self.last_reply(worker).unwrap_or_else(|| "(none)".into()), REPORT_CHARS, "the whole message is in the worker's thread")),
            // Claude says it waits for input a while after every finished turn too.
            AgentStatus::NeedsInput if before == Some(AgentStatus::Done) => return,
            AgentStatus::NeedsInput => format!("needs input: {}", event.message.as_deref().unwrap_or("it is waiting")),
            AgentStatus::Failed => format!("stopped on an error: {}", event.message.as_deref().unwrap_or("unknown")),
            AgentStatus::Working => return,
        };
        // The news goes to the tab that spawned the worker, so a worker's own workers report to
        // it, else the project's running orchestrator. Either may have no running session right
        // now (a tab restored after a restart, or one the web UI has not started): then the report
        // waits in the queue instead of dropping, and the delegate pass types it once the tab is
        // idle. A worker always reports home unless its spawner and its orchestrator are both gone.
        let spawner = self.tab(worker, |t| t.spawned_by.clone()).ok().flatten().filter(|id| self.tabs.lock().unwrap().contains_key(id));
        let Some(target) = spawner.or_else(|| self.live_tab(&role_env(Role::Orchestrator, project))).or_else(|| self.find_tab(&role_env(Role::Orchestrator, project))) else {
            eprintln!("pinned: worker {worker} \"{title}\" {what} (no orchestrator or spawner tab takes it)");
            return;
        };
        let line = format!("[pinned] Worker {worker} \"{title}\" {what}");
        let live = self.tab(&target, |t| t.draft.empty() && t.session.as_ref().is_some_and(|session| session.raw())).unwrap_or(false);
        if !live {
            let session = self.tab(&target, |t| t.claude_session.clone()).ok().flatten();
            self.delegates.queue_report(&target, session, line);
            self.arm_delegate_pass();
            self.emit_tabs();
            return;
        }
        if self.type_prompt(&target, &line).is_err() {
            eprintln!("pinned: worker {worker} could not type into {target}; its news waits in the queue");
            let session = self.tab(&target, |t| t.claude_session.clone()).ok().flatten();
            self.delegates.queue_report(&target, session, line);
            self.arm_delegate_pass();
            self.emit_tabs();
        }
    }

    /// The projects this machine syncs: the most recently opened checkout of each git origin.
    /// Worktrees share their main checkout's origin and are left out.
    pub fn checkouts(&self) -> Vec<sync::Checkout> {
        let paths: Vec<PathBuf> = projects::recent(&self.projects.lock().unwrap()).into_iter().map(|p| p.path.clone()).collect();
        let mut seen = std::collections::HashSet::new();
        paths
            .into_iter()
            .filter(|path| path.join(".git").is_dir())
            .filter_map(|path| {
                let repo = git_remote(&path)?;
                let key = repo_key(&repo)?;
                let name = repo_name(&repo).unwrap_or_default().to_owned();
                let account = self.account_for(None, &path).ok()?;
                seen.insert(key.clone()).then(|| sync::Checkout {
                    project: sync::Project { key, name, repo },
                    history: self.history_dir.as_ref().map(|dir| history::file_for(dir, &path)),
                    claude_home: (account.provider == Provider::Claude).then(|| self.accounts.home(&account)),
                    path,
                })
            })
            .collect()
    }

    /// Where this machine remembers what it synced, or None when it keeps nothing.
    pub fn sync_file(&self) -> Option<&Path> {
        self.sync_file.as_deref()
    }

    pub(crate) fn hold_sync_state(&self, held: bool) {
        self.sync_loop_holds_state.store(held, Ordering::SeqCst);
    }

    fn sync_stopped(&self) -> Option<String> {
        if self.sync_loop_holds_state.load(Ordering::SeqCst) {
            return None;
        }
        self.sync_file.as_deref().and_then(sync::unreadable)
    }

    /// One share per Claude home. A test binary under `deps` shares only homes inside its config dir.
    pub fn plugin_shares(&self) -> Vec<sync::PluginShare> {
        let Some(config) = self.sync_file.as_ref().and_then(|file| file.parent()).map(Path::to_path_buf) else { return vec![] };
        let dir = config.join("synced-plugins");
        let mut homes = vec![home().join(".claude")];
        for account in self.accounts.list() {
            if account.provider == Provider::Claude {
                homes.push(self.accounts.home(&account));
            }
        }
        homes.sort();
        homes.dedup();
        if test_binary() {
            homes.retain(|path| path.starts_with(&config));
        }
        homes.into_iter().map(|claude_home| sync::PluginShare { claude_home, dir: dir.clone() }).collect()
    }

    fn places(&self) -> skills::Places {
        let config = self.sync_file.as_ref().and_then(|file| file.parent()).map(Path::to_path_buf);
        let accounts = self.accounts.list();
        let inside = |path: &Path| !test_binary() || config.as_ref().is_some_and(|c| path.starts_with(c));
        let homes = |provider| accounts.iter().filter(|a| a.provider == provider).map(|a| (a.id.clone(), self.accounts.home(a))).filter(|(_, home)| inside(home)).collect();
        let mut projects: Vec<PathBuf> = projects::recent(&self.projects.lock().unwrap()).into_iter().map(|p| p.path.clone()).collect();
        projects.extend(self.tabs.lock().unwrap().values().map(|t| t.cwd.clone()));
        let mut projects: Vec<skills::CanonicalPath> = projects.iter().filter_map(|p| skills::CanonicalPath::new(p).ok()).collect();
        projects.sort();
        projects.dedup();
        let home = if test_binary() { config.clone().unwrap_or_default().join("home") } else { home() };
        skills::Places {
            home,
            claude_homes: homes(Provider::Claude),
            codex_homes: homes(Provider::Codex),
            cursor: accounts.iter().find(|a| a.provider == Provider::Cursor).map(|a| a.id.clone()),
            projects,
            synced_plugins: config.as_ref().map(|c| c.join("synced-plugins")),
            pinned_plugin: self.launch.plugin_dir.clone(),
            installed_plugin_files: config.unwrap_or_else(std::env::temp_dir).join("skill-originals"),
        }
    }

    pub(crate) fn skills_changed(&self, touched: &[skills::Place]) -> Vec<skills::Nudged> {
        if touched.is_empty() {
            return vec![];
        }
        let mut scopes: Vec<skills::Scope> = touched.iter().map(skills::Place::scope).collect();
        scopes.sort();
        scopes.dedup();
        self.emit(Event::SkillsChanged { scopes });
        let running: Vec<(TabId, Account, PathBuf, Option<String>)> = {
            let tabs = self.tabs.lock().unwrap();
            tabs.iter().filter(|(_, t)| t.session.is_some() && t.program == Program::Agent).map(|(id, t)| (id.clone(), t.account.clone(), t.cwd.clone(), t.title.clone())).collect()
        };
        let mut nudged = vec![];
        for (tab, account, cwd, title) in running {
            let (home, cwd, plugin_dirs) = (self.accounts.home(&account), skills::CanonicalPath::new(&cwd).ok(), self.shared_plugin_dirs(&account));
            let session = skills::Session { provider: account.provider, home: &home, cwd: cwd.as_ref(), plugin_dirs: &plugin_dirs };
            let Some(nudge) = touched.iter().find_map(|place| skills::nudge_if_loaded(place, &session)) else { continue };
            let delivery = match nudge {
                skills::Nudge::NextStart => skills::Delivery::NextStart,
                skills::Nudge::ReloadSkills => {
                    let typed = self.tab(&tab, |t| {
                        t.owes_reload = true;
                        t.reload_if_idle()
                    });
                    if typed.unwrap_or(false) {
                        skills::Delivery::Reloaded
                    } else {
                        self.arm_delegate_pass();
                        skills::Delivery::AfterTurn
                    }
                }
            };
            nudged.push(skills::Nudged { tab, title, nudge: delivery });
        }
        nudged.sort_by_key(|n| tab_number(&n.tab));
        nudged
    }

    fn wake_sync(&self) {
        let waker = self.sync_waker.lock().unwrap().clone();
        if let Some(wake) = waker {
            wake();
        }
    }

    pub(crate) fn set_sync_waker(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.sync_waker.lock().unwrap() = Some(wake);
    }

    pub(crate) fn set_hub(&self, hub: Arc<dyn Fn(HubCall) -> Result<Value, String> + Send + Sync>) {
        *self.hub.lock().unwrap() = Some(hub);
    }

    /// Remembers this machine's id and name from the hub's hello, so a project named by it is
    /// local and what this machine lists of itself reads as the hub lists it.
    pub(crate) fn set_hub_id(&self, id: crate::wire::MachineId, name: String) {
        *self.hub_id.lock().unwrap() = id;
        *self.hub_name.lock().unwrap() = name;
    }

    /// Asks the hub, blocking up to 60 seconds for the answer. Never call it while holding a Machine
    /// lock.
    pub(crate) fn call_hub(&self, call: HubCall) -> Result<Value, String> {
        let hub = self.hub.lock().unwrap().clone();
        match hub {
            Some(hub) => hub(call),
            None => Err(NOT_LINKED.into()),
        }
    }

    fn shared_plugin_dirs(&self, account: &Account) -> Vec<PathBuf> {
        let Some(config) = self.sync_file.as_ref().and_then(|file| file.parent()) else { return vec![] };
        sync::plugin_dirs(&self.accounts.home(account), &config.join("synced-plugins"))
    }

    /// The turns saved for the project at `path`, newest first.
    pub fn project_history(&self, path: &str) -> Vec<history::Turn> {
        self.history_dir.as_ref().map(|dir| history::load(&history::file_for(dir, Path::new(path)))).unwrap_or_default()
    }

    /// Types `text` as one prompt into the newest live tab whose environment holds every pair of
    /// `env`, such as the session answering a Slack thread. Answers whether one took it.
    pub fn deliver(&self, env: &[(String, String)], text: &str) -> bool {
        self.live_tab(env).is_some_and(|tab| self.type_prompt(&tab, text).is_ok())
    }

    fn live_tab(&self, env: &[(String, String)]) -> Option<TabId> {
        if env.is_empty() {
            return None;
        }
        let tabs = self.tabs.lock().unwrap();
        let live = tabs.iter().filter(|(_, t)| t.session.is_some() && env.iter().all(|e| t.env.contains(e)));
        live.max_by_key(|(id, _)| tab_number(id)).map(|(id, _)| id.clone())
    }

    /// The newest tab with `env`, whether its session runs or not. A report queues for a tab
    /// without a running session and is typed once the session is idle.
    fn find_tab(&self, env: &[(String, String)]) -> Option<TabId> {
        if env.is_empty() {
            return None;
        }
        let tabs = self.tabs.lock().unwrap();
        tabs.iter().filter(|(_, t)| env.iter().all(|e| t.env.contains(e))).max_by_key(|(id, _)| tab_number(id)).map(|(id, _)| id.clone())
    }

    /// The tab's watched pull requests, after starting or stopping the watch on `pr`.
    fn watch_pr(&self, tab: &str, pr: Option<&str>, watching: bool) -> Result<Vec<String>, String> {
        let cwd = self.tab(tab, |t| t.cwd.clone())?;
        let Some(pr) = pr else { return Ok(self.pr_watches.of(tab)) };
        if !pr.starts_with("https://") {
            return Err(format!("{pr:?} is not a pull request URL"));
        }
        if watching {
            // The first read is what later reads are measured against.
            let view = workspace::run(&cwd, self.launch.path_env.as_deref(), "gh", &["pr", "view", "--json", crate::prwatch::FIELDS, "--", pr]).map_err(|_| "gh couldn't read the pull request".to_owned())?;
            let now = crate::prwatch::snapshot(&serde_json::from_str(&view).map_err(|e| format!("gh sent a bad answer: {e}"))?);
            let list = self.pr_watches.set(tab, pr, true);
            self.pr_watches.update(tab, pr, now);
            self.start_watching();
            return Ok(list);
        }
        Ok(self.pr_watches.set(tab, pr, false))
    }

    /// Reads the watched pull requests now and then, and wakes each tab's claude when it's free.
    fn start_watching(&self) {
        if self.pr_watches.started.swap(true, Ordering::Relaxed) {
            return;
        }
        let every = std::env::var("PINNED_PR_WATCH_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
        self.every(Duration::from_secs(every), Machine::read_watches);
    }

    fn read_watches(&self) {
        let tabs: Vec<String> = self.tabs.lock().unwrap().keys().cloned().collect();
        self.pr_watches.forget(&tabs);
        for tab in tabs {
            let Ok(cwd) = self.tab(&tab, |t| t.cwd.clone()) else { continue };
            for url in self.pr_watches.of(&tab) {
                let view = workspace::run(&cwd, self.launch.path_env.as_deref(), "gh", &["pr", "view", "--json", crate::prwatch::FIELDS, "--", &url]);
                if let Some(view) = view.ok().and_then(|v| serde_json::from_str::<Value>(&v).ok()) {
                    if view["state"] == "MERGED" {
                        self.accept_merged(&tab, view["number"].as_u64());
                    }
                    self.pr_watches.update(&tab, &url, crate::prwatch::snapshot(&view));
                }
            }
            let free = self.tab(&tab, |t| t.session.is_some() && matches!(t.status, None | Some(AgentStatus::Done))).unwrap_or(false);
            if free {
                if let Some(text) = self.pr_watches.take_wake(&tab) {
                    let _ = self.type_prompt(&tab, &text);
                }
            }
        }
    }

    /// Types `text` into the tab's session as one prompt and submits it. A session that has not
    /// taken the terminal yet, as claude starting up, gets it once it does: typed earlier, the
    /// pty's cooked mode would echo it and turn the submitting \r into \n, and claude would never
    /// see a prompt.
    fn type_prompt(&self, tab: &str, text: &str) -> Result<(), String> {
        let wrote = self.tab(tab, |t| match t.session.as_mut() {
            Some(s) if s.raw() => {
                for held in std::mem::take(&mut t.held) {
                    s.write(&paste(&held)).map_err(|e| e.to_string())?;
                    t.turn_pending = Some(Instant::now());
                }
                s.write(&paste(text)).map_err(|e| e.to_string())?;
                t.turn_pending = Some(Instant::now());
                Ok(true)
            }
            Some(_) => {
                t.held.push(text.to_owned());
                Ok(false)
            }
            None => Err(format!("tab {tab} has no running session")),
        })??;
        if wrote {
            self.arm_delegate_pass();
        }
        Ok(())
    }

    /// Reopens the sessions still going at the last quit, resumed, and returns their tab ids.
    pub fn restore_tabs(&self) -> Vec<TabId> {
        // A reloaded webview must not duplicate the tabs that are already open.
        let Some(file) = self.sessions_file.as_ref().filter(|_| self.tabs.lock().unwrap().is_empty()) else {
            return vec![];
        };
        let now = unix_now();
        let restored: Vec<(TabId, Option<String>)> = sessions::load(file)
            .into_iter()
            .filter(|s| s.cwd.is_dir())
            .filter_map(|s| {
                // The transcript is in its account's home: with that account gone, nothing can resume it.
                let account = self.account_for(Some(&s.account), &s.cwd).ok()?;
                let (launch, resume_at) = s.reopen(now, account.provider);
                let id = self.add_tab(s.cwd, account, launch, s.env, s.bypass, Some(s.session));
                let _ = self.tab(&id, |t| (t.interrupted, t.resume_at, t.held_at_limit, t.model, t.permission_mode) = (s.interrupted, resume_at, s.held, s.model, s.permission_mode));
                Some((id, s.spawned_by))
            })
            .collect();
        {
            let mut tabs = self.tabs.lock().unwrap();
            for (id, parent) in &restored {
                let parent = parent.as_ref().and_then(|session| tabs.iter().find(|(other, t)| *other != id && t.claude_session.as_ref() == Some(session)).map(|(other, _)| other.clone()));
                if let Some(tab) = tabs.get_mut(id) {
                    tab.spawned_by = parent;
                }
            }
        }
        let ids: Vec<TabId> = restored.into_iter().map(|(id, _)| id).collect();
        // A turn that asked before the restart still waits on the user.
        for id in ids.iter().filter(|id| self.tab(id, |t| !t.interrupted).unwrap_or(false)) {
            self.note_question(id);
        }
        self.emit_tabs();
        // A resumed session reports no new id, so its transcript is read here.
        for id in &ids {
            self.refresh_stats(id, false);
        }
        let sessions: Vec<(String, String)> = ids
            .iter()
            .filter_map(|id| self.tab(id, |t| t.claude_session.clone()).ok().flatten().map(|session| (session, id.clone())))
            .collect();
        self.delegates.rebind(&sessions);
        if self.delegates.has_pending() {
            self.arm_delegate_pass();
        }
        ids
    }

    /// Each account's usage, as its sessions last reported it.
    pub fn usage(&self) -> BTreeMap<accounts::AccountId, Usage> {
        self.usage.lock().unwrap().clone()
    }


    /// The account's tastes: the hub's while this machine is linked, which `taste.json` keeps for
    /// when the hub can't answer, or else this machine's own. Waits at most `TASTE_WAIT` for the
    /// hub; an answer that comes later still lands in `taste.json` for the next start.
    fn tastes(&self) -> Tastes {
        let dir = self.orchestration_dir.clone();
        let hub = self.hub.lock().unwrap().clone();
        if let Some(hub) = hub {
            let (shared, kept) = (self.tastes_shared.clone(), dir.clone());
            let (answer, answered) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                share_tastes(&*hub, &shared, kept.as_deref());
                let read = hub(HubCall::GetTastes).and_then(|value| serde_json::from_value::<Tastes>(value).map_err(|e| e.to_string()));
                if let Ok(tastes) = &read {
                    if let Err(e) = taste::save(tastes, kept.as_deref()) {
                        eprintln!("pinned: could not keep the account's taste: {e}");
                    }
                }
                let _ = answer.send(read);
            });
            if let Ok(Ok(tastes)) = answered.recv_timeout(TASTE_WAIT) {
                return tastes;
            }
        }
        taste::load(dir.as_deref()).unwrap_or_default()
    }

    /// The link to the hub is up, maybe for another account than before: offers this machine's
    /// own tastes again. Never call it while holding a Machine lock.
    pub(crate) fn share_tastes_again(&self) {
        self.tastes_shared.store(false, Ordering::SeqCst);
        let hub = self.hub.lock().unwrap().clone();
        if let Some(hub) = hub {
            share_tastes(&*hub, &self.tastes_shared, self.orchestration_dir.as_deref());
        }
    }

    /// The account's tastes as they stand: the hub's while this machine is linked, else its own.
    /// Unlike `tastes`, a hub that can't answer is an error, so a chat never reads a stale copy.
    fn tastes_now(&self) -> Result<Tastes, String> {
        match self.hub.lock().unwrap().clone() {
            Some(hub) => serde_json::from_value(hub(HubCall::GetTastes)?).map_err(|e| format!("the hub's answer did not read: {e}")),
            None => Ok(self.local_tastes()),
        }
    }

    /// The tastes a signed-out app keeps on this machine.
    pub fn local_tastes(&self) -> Tastes {
        taste::load(self.orchestration_dir.as_deref()).unwrap_or_default()
    }

    /// `Tastes::set` on this machine's own tastes.
    pub fn set_local_taste(&self, scope: &TasteScope, patch: TastePatch) -> Result<Tastes, String> {
        taste::change(self.orchestration_dir.as_deref(), |tastes| tastes.set(scope, patch))
    }

    /// `Tastes::reset` on this machine's own tastes.
    pub fn reset_local_taste_field(&self, scope: &TasteScope, field: TasteField) -> Result<Tastes, String> {
        taste::change(self.orchestration_dir.as_deref(), |tastes| {
            tastes.reset(scope, field);
            Ok(())
        })
    }

    fn record_usage(&self, account: accounts::AccountId, report: Usage) {
        let usage = {
            let mut all = self.usage.lock().unwrap();
            let known = all.entry(account.clone()).or_default();
            *known = report.over(*known, unix_now());
            *known
        };
        self.emit(Event::Usage { usage, account });
    }

    /// Reads a Go account's allowance off the statusline path. The statusline asks for this and
    /// returns at once, so a slow or unreachable gateway never delays the user's own statusline.
    fn refresh_go_usage(&self, account: accounts::AccountId) {
        let Some(account) = self.accounts.get(&account) else { return };
        if account.auth != Auth::OpenCodeGo {
            return;
        }
        let Some(key) = self.accounts.key(&account) else { return };
        let now = Instant::now();
        {
            let mut refreshed = self.usage_refreshed.lock().unwrap();
            if refreshed.get(&account.id).is_some_and(|at| now.duration_since(*at) < Duration::from_secs(15)) {
                return;
            }
            refreshed.insert(account.id.clone(), now);
        }
        let Some(machine) = self.me.upgrade() else { return };
        let id = account.id.clone();
        std::thread::spawn(move || {
            if let Ok(usage) = crate::opencode_go::usage(&key) {
                machine.record_usage(id, usage);
            }
        });
    }

    /// Answers the hook socket. Hook events and usage update the tabs and project tools act on
    /// them; every other message, a connector session's callbacks and the reminders tools, goes to `other`.
    pub fn listen(&self, other: impl Fn(Message) -> Result<String, String> + Send + Sync + 'static) -> std::io::Result<()> {
        let me = self.me.clone();
        hooks::listen(self.launch.sock.clone(), move |message| {
            let Some(machine) = me.upgrade() else { return Err("pinned is shutting down".into()) };
            match message {
                Message::Agent(event) => {
                    let (notices, typed) = if event.hook_feedback { machine.take_notices(&event) } else { Default::default() };
                    machine.record_event(event.clone());
                    // After the event: a Stop hook that hands a request over continues the turn, and
                    // the turn that just ended did not answer it.
                    machine.mark_typed(&typed);
                    if !notices.is_empty() && event.status == AgentStatus::Done {
                        if let Some(tab) = &event.tab {
                            let _ = machine.tab(tab, |t| t.turn_pending = Some(Instant::now()));
                        }
                    }
                    Ok(merge_context(&machine.recall(&event), &notices))
                }
                Message::Usage { usage, account } => {
                    machine.record_usage(account.unwrap_or_else(accounts::default_account), usage);
                    Ok(String::new())
                }
                Message::RefreshUsage { account } => {
                    machine.refresh_go_usage(account);
                    Ok(String::new())
                }
                Message::Delegate { tab, task, tool, request } => machine.start_delegate(tab, task, tool, *request),
                Message::TaskStatus { tab, task, id } => machine.task_status(tab, task, &id),
                Message::TaskCancel { tab, task, id, reason } => machine.task_cancel(tab, task, &id, &reason),
                Message::Capabilities { tab, task, search } => machine.capabilities(tab.as_deref(), task.as_deref(), &search),
                Message::TaskReview { tab, task, id, verdict, note } => machine.task_review(tab, task, &id, &verdict, note),
                Message::DelegationStats { days } => machine.delegation_stats(days),
                Message::Project { tab, tool } => machine.project_tool(&tab, tool),
                message => other(message),
            }
        })
    }

    /// Clears the shutdown flag and reopens sessions saved by the last `quit`.
    pub fn reopen_sessions(&self) -> Vec<TabId> {
        self.quitting.store(false, Ordering::SeqCst);
        self.restore_tabs()
    }

    /// Ends every session as the machine shuts down, keeping them saved for the next launch.
    /// Tauri calls this again on ExitRequested after the tray's Quit or an update's restart, so
    /// only the first call runs: a second would find no tabs and save none.
    pub fn quit(&self) {
        self.save_sessions();
        if self.quitting.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stamp_parent_sessions();
        self.delegates.stop_for_restart(self.launch.path_env.as_deref());
        self.delegates.kill_remembered();
        let path = self.launch.path_env.clone();
        let (tabs, owned) = {
            let tabs = self.tabs.lock().unwrap();
            let owned: Vec<PathBuf> = tabs.values().filter_map(|tab| tab.made.clone()).collect();
            let ids: Vec<String> = tabs.keys().cloned().collect();
            (ids, owned)
        };
        for tab in &tabs {
            self.delegates.drop_kept_worktrees(tab, &owned, path.as_deref());
        }
        // The hub socket is still up. Clients drop these ids before a call can land on a cleared map.
        self.emit(Event::Tabs { tabs: vec![] });
        self.tabs.lock().unwrap().clear();
        self.device_hub.stop();
    }

    fn caller(tab: Option<String>, task: Option<String>) -> crate::delegate::Caller {
        crate::delegate::Caller { tab: tab.filter(|id| !id.is_empty()), task: task.filter(|id| !id.is_empty()) }
    }

    /// `delegate_task`, `task_status`, and `task_cancel` need the session that owns the work.
    /// Without `PINNED_TAB` or `PINNED_TASK` every such process would share one caller key.
    fn require_caller(tab: Option<String>, task: Option<String>) -> Result<crate::delegate::Caller, String> {
        let caller = Self::caller(tab, task);
        if caller.tab.is_none() && caller.task.is_none() {
            Err("PINNED_TAB or PINNED_TASK must be set to delegate, check, or cancel a task".into())
        } else {
            Ok(caller)
        }
    }

    fn task_status(&self, tab: Option<String>, task: Option<String>, id: &str) -> Result<String, String> {
        let caller = Self::require_caller(tab, task)?;
        let mut status = self.delegates.status(&caller, id)?;
        if let Some(remote) = self.delegates.get(id).and_then(|t| t.remote) {
            let steps = match (caller.tab.as_deref(), self.delegates.remote_due(id, Instant::now(), REMOTE_LOOK)) {
                (Some(tab), Some(due)) => self.look_remote(tab, id, &due),
                _ => None,
            };
            status = self.delegates.status(&caller, id)?;
            if let Some(steps) = steps {
                status["recentSteps"] = steps;
            }
            return Ok(with_remote(status, &remote).to_string());
        }
        // A thread's own session runs the child, so its steps are in the thread's transcript.
        if let Some(thread) = self.delegates.get(id).and_then(|t| t.thread.filter(|thread| thread.runs && !t.state.terminal())) {
            status["recentSteps"] = serde_json::json!(self.thread_steps(&thread.tab));
        }
        Ok(status.to_string())
    }

    /// Who `caller` is, as a task's record names its delegator.
    fn delegator(&self, caller: &crate::delegate::Caller) -> crate::delegate::Delegator {
        use crate::delegate::{Delegator, DelegatorKind};
        let Some(tab) = caller.tab.as_deref() else { return Delegator::default() };
        let Ok((role, global)) = self.tab(tab, |t| (role_of(&t.env), global::is_global(&t.env))) else { return Delegator::default() };
        let kind = match (global, &role) {
            (true, _) => DelegatorKind::GlobalOrchestrator,
            (false, Some((Role::Orchestrator, _))) => DelegatorKind::Captain,
            (false, Some((Role::Worker, _))) => DelegatorKind::Worker,
            (false, _) => DelegatorKind::Session,
        };
        let folder = match (global, role) {
            (true, _) => None,
            (false, Some((_, project))) => Some(project),
            (false, None) => self.project_of_tab(tab).ok(),
        };
        let (project, project_name) = match folder {
            Some(folder) => {
                let repo = git_remote(&folder).and_then(|remote| repo_key(&remote));
                let name = repo.as_deref().and_then(repo_name).map(str::to_owned).or_else(|| folder.file_name().map(|n| n.to_string_lossy().into_owned()));
                (Some(repo.unwrap_or_else(|| folder.display().to_string())), name)
            }
            None => (None, None),
        };
        Delegator { kind, tab: Some(tab.to_owned()), project, project_name }
    }

    /// The pull request a worker thread watched merged, so the task it ran was used.
    fn accept_merged(&self, tab: &str, number: Option<u64>) {
        let Some(task) = self.tab(tab, |t| t.task.clone()).ok().flatten() else { return };
        let pr = number.map_or_else(|| "its pull request".to_owned(), |n| format!("its PR #{n}"));
        if self.delegates.accept_auto(&task, format!("{pr} merged")) {
            self.send_records();
        }
    }

    /// Sends the hub the records of tasks that ended or changed since it last took them, in the
    /// background, one send at a time. A machine with no hub keeps them for its own totals.
    pub(crate) fn send_records(&self) {
        if self.hub.lock().unwrap().is_none() || self.records_retry.lock().unwrap().is_some_and(|at| Instant::now() < at) {
            return;
        }
        if self.sending_records.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.me.clone();
        std::thread::spawn(move || {
            let Some(machine) = me.upgrade() else { return };
            machine.record_now();
            machine.sending_records.store(false, Ordering::SeqCst);
        });
    }

    /// Sends the hub the records it lacks, on this thread, until it holds them all or refuses.
    fn record_now(&self) {
        loop {
            let records = self.delegates.unrecorded(RECORDS_PER_SEND);
            if records.is_empty() {
                return;
            }
            match self.call_hub(HubCall::RecordTasks { records: records.clone() }) {
                Ok(_) => self.delegates.mark_recorded(&records),
                Err(e) => {
                    eprintln!("pinned: the hub did not take {} task records: {e}", records.len());
                    *self.records_retry.lock().unwrap() = Some(Instant::now() + RECORDS_RETRY);
                    return;
                }
            }
        }
    }

    fn task_review(&self, tab: Option<String>, task: Option<String>, id: &str, verdict: &str, note: Option<String>) -> Result<String, String> {
        let caller = Self::require_caller(tab, task)?;
        let verdict = crate::delegate::Verdict::parse(verdict)?;
        let answer = self.delegates.review(&caller, id, verdict, note)?;
        self.send_records();
        Ok(answer.to_string())
    }

    /// The account's delegation totals when the hub keeps them, else this machine's own.
    fn delegation_stats(&self, days: Option<u64>) -> Result<String, String> {
        let days = crate::delegate::stats_days(days)?;
        let from_hub = self.call_hub(HubCall::DelegationStats { days }).ok().and_then(|value| serde_json::from_value::<crate::delegate::DelegationStats>(value).ok());
        let (stats, scope) = match from_hub {
            Some(stats) => (stats, "every machine of the account"),
            None => (self.local_delegation_stats(days), "this machine"),
        };
        let mut brief = stats.brief();
        brief["scope"] = json!(scope);
        Ok(brief.to_string())
    }

    /// The totals of the tasks this machine ran, for a signed-out app and a machine with no hub.
    pub fn local_delegation_stats(&self, days: u32) -> crate::delegate::DelegationStats {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        crate::delegate::aggregate(&self.delegates.records(), now, days)
    }

    /// The task the tab's thread shows, while that task is still working. `runs` asks only for
    /// one the tab's own session runs.
    fn thread_task(&self, tab: &str, runs: bool) -> Option<String> {
        let task = self.tab(tab, |t| t.task.clone()).ok().flatten()?;
        let info = self.delegates.get(&task)?;
        (!info.state.terminal() && (!runs || info.thread.is_some_and(|thread| thread.runs))).then_some(task)
    }

    fn task_cancel(&self, tab: Option<String>, task: Option<String>, id: &str, reason: &str) -> Result<String, String> {
        let caller = Self::require_caller(tab, task)?;
        let remote = self.delegates.get(id).filter(|t| !t.state.terminal()).and_then(|t| t.remote);
        if let (Some(remote), Some(tab)) = (remote, caller.tab.as_deref()) {
            self.delegates.status(&caller, id)?;
            if !remote.machine.is_empty() {
                let cancel = HubCall::RemoteTask { machine: remote.machine.clone(), tab: tab.to_owned(), task: id.to_owned(), cancel: Some(reason.to_owned()) };
                let answer = self.call_hub(cancel).map_err(|e| format!("{} did not take the cancel: {}", remote.name, needs_update(e)))?;
                self.settle_from(tab, id, &remote, &answer);
            }
        }
        self.delegates.cancel(&caller, id, reason, self.launch.path_env.as_deref()).map(|value| value.to_string())
    }

    /// `delegate_task` with `target.machine` or `target.box`: the hub runs it there, and this
    /// machine keeps the task, under its own id, for the caller's `task_status` and notice.
    fn delegate_elsewhere(&self, caller: &crate::delegate::Caller, tool: Option<String>, mut request: crate::delegate::Request, limits: taste::Limits) -> Result<String, String> {
        use crate::delegate::{resolve_access, Claim, NewTask, Parent, RemoteTarget};
        let tab = caller.tab.clone().ok_or("only a thread delegates to another machine or a box, and this is a headless task")?;
        let (account, cwd, ceiling, session) = self.tab(&tab, |t| (t.account.id.clone(), t.cwd.clone(), t.child_access(), t.claude_session.clone()))?;
        let access = resolve_access(ceiling, &request)?;
        let repo = request.target.boxed.as_ref().and_then(|b| b.repo.clone()).or_else(|| git_remote(&cwd));
        if repo.is_none() {
            return Err("this thread's folder has no git origin, so no other machine or box can find its project".into());
        }
        let account = request.target.account.clone().filter(|id| !id.is_empty()).unwrap_or(account);
        let provider = self.accounts.get(&account).map_or(Provider::Claude, |a| a.provider);
        let place = request.target.machine.clone().unwrap_or_else(|| "an Upstash Box".into());
        // Held here to the caller's limits, and sent with them, so the worker there is held too.
        let effort = limits.admit(request.target.model.as_deref(), request.target.effort.as_deref(), &self.claude_models())?;
        let (budget_ms, limit_ms) = crate::delegate::run_limits(request.budget_ms, request.limit_ms, limits.time_limit_ms())?;
        (request.target.effort, request.budget_ms, request.limit_ms) = (effort.clone(), Some(budget_ms), Some(limit_ms));
        let held = taste::Limits { time_limit_min: limit_ms.div_ceil(60_000).try_into().unwrap_or(u32::MAX), ..limits };
        let delegator = self.delegator(caller);
        let task = match self.delegates.claim(NewTask {
            parent: Parent::Tab(tab.clone()),
            tool,
            title: request.title.clone(),
            account,
            provider,
            model: request.target.model.clone(),
            cwd,
            mode: request.mode,
            access,
            isolated: false,
            caller_key: caller.key(),
            client_request_id: request.client_request_id.clone(),
            role: request.role.clone(),
        }, Some(limits.parallel)) {
            Claim::Existing(id) => return self.remote_started(&id),
            Claim::Full(working) => return Err(workers_full(working)),
            Claim::Fresh(task) => *task,
        };
        self.delegates.set_limits(&task.id, budget_ms, limit_ms);
        self.delegates.hold_to(&task.id, held);
        self.delegates.describe(&task.id, delegator.clone(), effort);
        self.delegates.note_parent(&task.id, session);
        self.delegates.set_remote(&task.id, RemoteTarget { machine: String::new(), name: place, tab: None, looked: None });
        let asked = HubCall::Delegate { tab, task: task.id.clone(), request: Box::new(request), repo, access: ceiling, delegator, limits: held };
        match self.call_hub(asked) {
            Ok(answer) => {
                let machine = answer["machine"].as_str().unwrap_or_default().to_owned();
                let name = answer["machineName"].as_str().map_or_else(|| machine.clone(), str::to_owned);
                let tab = answer["tabId"].as_str().map(str::to_owned);
                self.delegates.set_remote(&task.id, RemoteTarget { machine, name, tab, looked: Some(Instant::now()) });
                self.arm_delegate_pass();
                self.remote_started(&task.id)
            }
            Err(error) => {
                let error = needs_update(error);
                self.delegates.fail(&task.id, error.clone(), self.launch.path_env.as_deref());
                Err(error)
            }
        }
    }

    fn remote_started(&self, id: &str) -> Result<String, String> {
        let body = self.delegates.started_json(id)?;
        Ok(match self.delegates.get(id).and_then(|t| t.remote) {
            Some(remote) => with_remote(body, &remote),
            None => body,
        }
        .to_string())
    }

    /// Asks the machine that runs a remote task how it goes. A finished task ends here too; a
    /// working one gives its latest steps. None while that machine can't answer, as while its box boots.
    fn look_remote(&self, tab: &str, id: &str, remote: &crate::delegate::RemoteTarget) -> Option<Value> {
        if remote.machine.is_empty() {
            return None;
        }
        let asked = HubCall::RemoteTask { machine: remote.machine.clone(), tab: tab.to_owned(), task: id.to_owned(), cancel: None };
        let answer = self.call_hub(asked).ok()?;
        self.settle_from(tab, id, remote, &answer);
        Some(answer["recentSteps"].clone()).filter(|steps| steps.is_array())
    }

    /// Ends the remote task when `answer`, the status its machine gave, says it ended.
    fn settle_from(&self, tab: &str, id: &str, remote: &crate::delegate::RemoteTarget, answer: &Value) {
        let state = answer["workState"].as_str().unwrap_or_default();
        if !matches!(state, "result_available" | "failed" | "cancelled") {
            return;
        }
        let end = crate::delegate::RemoteEnd {
            state: state.to_owned(),
            summary: answer["summary"].as_str().map(str::to_owned),
            error: answer["error"].as_str().map(str::to_owned),
        };
        if self.delegates.settle_remote(id, tab, &remote.machine, &end) {
            self.emit_tabs();
        }
    }

    /// `Call::RemoteEnded`: the machine `from` ran this machine's task `task` for `tab`, and it ended.
    fn remote_ended(&self, from: &str, tab: &str, task: &str, end: &crate::delegate::RemoteEnd) -> bool {
        if !self.delegates.settle_remote(task, tab, from, end) {
            return false;
        }
        self.maybe_deliver(tab);
        self.emit_tabs();
        true
    }

    /// `Call::RemoteTask`: the status of the task started here for `task` of `tab` on `origin`, or
    /// its state once cancelled.
    fn remote_task(&self, origin: &str, tab: &str, task: &str, cancel: Option<&str>) -> Result<Value, String> {
        let key = crate::delegate::Origin::key(origin, tab);
        let id = self.delegates.for_origin(&key, task).ok_or_else(|| format!("no task {task} of that thread runs here"))?;
        let caller = crate::delegate::Caller { tab: Some(key), task: None };
        if let Some(reason) = cancel {
            return self.delegates.cancel(&caller, &id, reason, self.launch.path_env.as_deref());
        }
        let mut status = self.delegates.status(&caller, &id)?;
        if let Some(thread) = self.delegates.get(&id).and_then(|t| t.thread.filter(|thread| thread.runs && !t.state.terminal())) {
            status["recentSteps"] = serde_json::json!(self.thread_steps(&thread.tab));
        }
        Ok(status)
    }

    /// Sends the end of each task that a tab on another machine delegated here back through the
    /// hub. An end the hub could not pass on waits for the next pass, at most every `REMOTE_LOOK`.
    fn send_remote_ends(&self, now: Instant) {
        let fresh: Vec<_> = self.delegates.claim_remote_ends().into_iter().map(|(_, machine, tab, task, end)| HubCall::RemoteEnded { to: machine, tab, task, end }).collect();
        // A worker box goes once its end got through, so the tasks' records go first.
        if !fresh.is_empty() && self.hub.lock().unwrap().is_some() {
            self.record_now();
        }
        let held = std::mem::take(&mut *self.remote_ends.lock().unwrap());
        let (again, mut waiting): (Vec<_>, Vec<_>) = held.into_iter().partition(|(at, _)| now.saturating_duration_since(*at) >= REMOTE_LOOK);
        for call in fresh.into_iter().chain(again.into_iter().map(|(_, call)| call)) {
            if let Err(e) = self.call_hub(call.clone()) {
                eprintln!("pinned: could not send a delegated task's end back: {e}");
                waiting.push((now, call));
            }
        }
        self.remote_ends.lock().unwrap().extend(waiting);
    }

    /// A cancel or a quit landed while startup still had no child process. `unplaced` is a worktree
    /// directory that is not on the task yet, so cancelling the task would not remove it.
    fn startup_stopped(&self, id: &str, unplaced: Option<&Path>) -> bool {
        let stop = self.quitting.load(Ordering::SeqCst) || self.delegates.is_terminal(id);
        if !stop {
            return false;
        }
        if let Some(dir) = unplaced {
            branches::drop_worktree(dir, self.launch.path_env.as_deref());
        }
        if !self.delegates.is_terminal(id) {
            let _ = self.delegates.cancel_tree(id, "pinned is shutting down", self.launch.path_env.as_deref());
        }
        true
    }

    /// The parent tab must still be open. A task claimed for a tab that closed is cancelled, so
    /// the catalog wait cannot leave a child whose parent is already gone.
    fn ensure_parent(&self, tab: Option<&str>, task: Option<&str>) -> Result<(), String> {
        let Some(tab) = tab.filter(|id| !id.is_empty()) else { return Ok(()) };
        if self.tabs.lock().unwrap().contains_key(tab) {
            return Ok(());
        }
        if let Some(id) = task {
            if !self.delegates.is_terminal(id) {
                let _ = self.delegates.cancel_tree(id, "the parent tab closed", self.launch.path_env.as_deref());
            }
        }
        Err(format!("no tab {tab}"))
    }

    fn start_delegate(&self, tab: Option<String>, parent_task: Option<String>, tool: Option<String>, request: crate::delegate::Request) -> Result<String, String> {
        use crate::delegate::Parent;
        let caller = Self::require_caller(tab, parent_task)?;
        if let Some(tab) = caller.tab.as_deref() {
            if self.tab(tab, |t| global::is_global(&t.env))? {
                return Err(NO_PROJECT_WORK.into());
            }
        }
        if let Some(error) = crate::delegate::delegate_block(caller.task.is_some()) {
            return Err(error.to_owned());
        }
        if let Some(id) = self.delegates.requested(&caller.key(), request.client_request_id.as_deref()) {
            return self.remote_started(&id);
        }
        let limits = self.caller_limits(caller.tab.as_deref().unwrap_or_default())?;
        if request.target.machine.is_some() || request.target.boxed.is_some() {
            return self.delegate_elsewhere(&caller, tool, request, limits);
        }
        let delegator = self.delegator(&caller);
        let parallel = Some(limits.parallel);
        let side = if let Some(tab) = caller.tab.clone() {
            let (account, model, provider, auth, cwd, ceiling) = self.tab(&tab, |t| (t.account.id.clone(), t.model.clone(), t.account.provider, t.account.auth, t.cwd.clone(), t.child_access()))?;
            ChildParent { parent: Parent::Tab(tab.clone()), caller_key: caller.key(), client_request_id: request.client_request_id.clone(), tab: Some(tab), origin: None, account: Some(account), inherited_model: model, provider, auth, cwd, ceiling, limits, parallel, delegator }
        } else {
            let id = caller.task.clone().unwrap_or_default();
            let task = self.delegates.lineage(&id).ok_or_else(|| format!("no task {id}"))?;
            let auth = self.accounts.get(&task.account).map(|account| account.auth).unwrap_or_default();
            ChildParent { parent: Parent::Task(id), caller_key: caller.key(), client_request_id: request.client_request_id.clone(), tab: None, origin: None, account: Some(task.account), inherited_model: task.model, provider: task.provider, auth, cwd: task.cwd, ceiling: task.access, limits, parallel, delegator }
        };
        self.launch_child(side, tool, request)
    }

    /// Starts a worker here for a tab on another machine, which the hub sent as `Call::Delegate`.
    /// It works in this machine's checkout of `repo`, cloned with `token` when there is none, on
    /// the project's account unless the request names one.
    fn start_for_origin(&self, origin: crate::delegate::Origin, mut request: crate::delegate::Request, repo: Option<String>, token: Option<String>) -> Result<String, String> {
        let key = origin.parent_key();
        if let Some(id) = self.delegates.for_origin(&key, &origin.task) {
            return self.delegates.started_json(&id).map(|value| value.to_string());
        }
        let repo = repo.ok_or("the thread that asked works in a folder with no git origin, so this machine can't find its project")?;
        let cwd = self.find_folder("", Some(&repo), token.as_deref())?;
        let account = self.account_for(None, &cwd)?;
        // The caller's limits, held no looser than this machine's own for the project.
        let limits = origin.limits.narrow(self.tastes().resolve(&captain_scope(&history::project_of(&cwd))).limits());
        (request.target.machine, request.target.boxed) = (None, None);
        let caller_key = crate::delegate::Caller { tab: Some(key.clone()), task: None }.key();
        let side = ChildParent {
            parent: crate::delegate::Parent::Tab(key),
            caller_key,
            client_request_id: Some(origin.task.clone()),
            tab: None,
            account: Some(account.id),
            inherited_model: None,
            provider: account.provider,
            auth: account.auth,
            cwd,
            ceiling: origin.access,
            limits,
            // The caller's machine counted it against Workers at once.
            parallel: None,
            delegator: origin.delegator.clone(),
            origin: Some(origin),
        };
        self.launch_child(side, None, request)
    }

    fn launch_child(&self, side: ChildParent, tool: Option<String>, request: crate::delegate::Request) -> Result<String, String> {
        use crate::delegate::{driver_args, resolve_access, spawn, ChildSpec, Claim, Isolation, NewTask, SpawnSpec};
        let ChildParent { parent, caller_key, client_request_id, tab: parent_tab, origin, account: account_id, inherited_model, provider: parent_provider, auth: parent_auth, mut cwd, ceiling, limits, parallel, delegator } = side;
        let account = if let Some(id) = request.target.account.clone().filter(|id| !id.is_empty()) {
            self.accounts.get(&id).ok_or_else(|| format!("no account {id} on this machine"))?
        } else if let Some(id) = account_id {
            self.accounts.get(&id).unwrap_or_else(|| Account::builtin(Provider::Claude))
        } else {
            Account::builtin(Provider::Claude)
        };
        if let Some(kind) = request.target.driver_kind.as_deref() {
            crate::delegate::require_driver(&account.id, account.provider, kind)?;
        }
        let bin = self.launch.bin(account.provider);
        if !bin.is_file() {
            return Err(format!("{} is not installed on this machine", account.provider.spec().name));
        }
        if !self.status(&account).signed_in {
            return Err(format!("account {} is not signed in on this machine", account.id));
        }
        let access = resolve_access(ceiling, &request)?;
        // The same CLI can mean a Claude login or a Go key. A model id from one is not the other's.
        let inherited_model = if parent_auth == account.auth { inherited_model } else { None };
        let mut model = crate::delegate::child_model(parent_provider, account.provider, inherited_model, request.target.model.clone());
        let mut effort = limits.admit(model.as_deref(), request.target.effort.as_deref(), &self.claude_models())?;
        let (budget_ms, limit_ms) = crate::delegate::run_limits(request.budget_ms, request.limit_ms, limits.time_limit_ms())?;
        self.ensure_parent(parent_tab.as_deref(), None)?;
        let isolated = request.isolation == Isolation::Worktree;
        let task = match self.delegates.claim(NewTask {
            parent,
            tool,
            title: request.title.clone(),
            account: account.id.clone(),
            provider: account.provider,
            model: model.clone(),
            cwd: cwd.clone(),
            mode: request.mode,
            access,
            isolated: false,
            caller_key,
            client_request_id,
            role: request.role.clone(),
        }, parallel) {
            Claim::Existing(id) => return self.delegates.started_json(&id).map(|value| value.to_string()),
            Claim::Full(working) => return Err(workers_full(working)),
            Claim::Fresh(task) => *task,
        };
        self.delegates.set_limits(&task.id, budget_ms, limit_ms);
        self.delegates.hold_to(&task.id, taste::Limits { time_limit_min: limit_ms.div_ceil(60_000).try_into().unwrap_or(u32::MAX), ..limits });
        self.delegates.describe(&task.id, delegator, effort.clone());
        if let Some(tab) = parent_tab.as_deref() {
            let session = self.tab(tab, |t| t.claude_session.clone()).ok().flatten();
            self.delegates.note_parent(&task.id, session);
        }
        self.ensure_parent(parent_tab.as_deref(), Some(&task.id))?;
        if account.provider == Provider::Cursor {
            if let Some(name) = model.clone() {
                let ids = self.delegates.cursor_ids(&bin, self.launch.path_env.as_deref());
                model = Some(crate::delegate::map_cursor_model(&ids, &name, effort.as_deref()));
                self.delegates.set_model(&task.id, model.clone());
            }
            effort = None;
        }
        self.ensure_parent(parent_tab.as_deref(), Some(&task.id))?;
        // A child called from a tab is a worker thread under it. Claude Code runs in that thread's
        // own session; Cursor and Codex report no turns to pinned, so they run headless and the
        // thread shows their steps. A child for a tab on another machine is a thread here.
        let from = parent_tab.clone().or_else(|| origin.as_ref().map(crate::delegate::Origin::parent_key));
        let runs_in_thread = from.is_some() && account.provider == Provider::Claude;
        if isolated {
            match branches::new_worktree(&cwd, self.launch.path_env.as_deref()) {
                Ok(dir) => {
                    if self.startup_stopped(&task.id, Some(&dir)) {
                        return self.delegates.started_json(&task.id).map(|value| value.to_string());
                    }
                    cwd = dir;
                    self.delegates.place(&task.id, cwd.clone(), !runs_in_thread);
                }
                Err(error) => {
                    self.delegates.fail(&task.id, error.clone(), self.launch.path_env.as_deref());
                    return Err(error);
                }
            }
        }
        if self.startup_stopped(&task.id, None) {
            return self.delegates.started_json(&task.id).map(|value| value.to_string());
        }
        let brief = crate::delegate::granted_brief(&request.task, &request.tools);
        if let Some(from) = from.as_deref() {
            let child = ThreadChild { from, origin: origin.clone(), task: &task.id, title: &request.title, cwd: &cwd, made: (isolated && runs_in_thread).then(|| cwd.clone()), model: model.clone() };
            if runs_in_thread {
                return self.run_in_thread(child, account, effort.as_deref(), access, &brief);
            }
            self.watch_thread(child, account.clone());
        }
        let argv = driver_args(
            account.provider,
            &SpawnSpec {
                task: &brief,
                model: model.as_deref(),
                effort: effort.as_deref(),
                access,
                mcp_config: self.launch.mcp_config.as_deref(),
                exe: &self.launch.exe,
                sock: &self.launch.sock,
                task_id: &task.id,
                reminders: self.launch.reminders,
            },
        );
        let mut env = self.accounts.env(&account);
        env.push((SOCK_ENV.to_owned(), self.launch.sock.display().to_string()));
        env.push((TASK_ENV.to_owned(), task.id.clone()));
        let id = task.id.clone();
        let me = self.me.clone();
        let path_env = self.launch.path_env.clone();
        let mut strip: Vec<&str> = INHERITED_SESSION_ENV.to_vec();
        if account.auth == Auth::OpenCodeGo {
            strip.extend_from_slice(&crate::opencode_go::STRIP_ENV);
        }
        let pid = match spawn(
            ChildSpec { bin: &bin, argv: &argv, cwd: &cwd, env: &env, path: self.launch.path_env.as_deref(), strip: &strip },
            move |event| {
                let Some(machine) = me.upgrade() else { return };
                if let Some(tab) = machine.delegates.apply(&id, event, path_env.as_deref()) {
                    machine.maybe_deliver(&tab);
                }
            },
        ) {
            Ok(pid) => pid,
            Err(error) => {
                self.delegates.fail(&task.id, error.clone(), self.launch.path_env.as_deref());
                return Err(error);
            }
        };
        self.delegates.remember_pid(&task.id, pid);
        if self.quitting.load(Ordering::SeqCst) {
            self.delegates.kill_remembered();
        }
        self.arm_delegate_pass();
        self.delegates.started_json(&task.id).map(|value| value.to_string())
    }

    /// Opens the worker thread a delegated Claude Code child runs in: a Worker tab under `from`,
    /// on `account` with the task's model, effort and access, started on the brief. The task ends
    /// with the thread's first finished turn.
    fn run_in_thread(&self, child: ThreadChild, account: Account, effort: Option<&str>, access: crate::delegate::Access, brief: &str) -> Result<String, String> {
        use crate::delegate::Access;
        // A thread is titled after its prompt's first line, so the title leads the prompt.
        let prompt = if brief.lines().next().is_some_and(|line| line.trim() == child.title) { brief.to_owned() } else { format!("{}\n\n{brief}", child.title) };
        // A prompt starting with a dash would read as an option.
        let prompt = if prompt.starts_with('-') { format!("\n{prompt}") } else { prompt };
        let mut launch = vec!["--name".to_owned(), child.title.to_owned()];
        if let Some(effort) = effort {
            launch.extend(["--effort".to_owned(), effort.to_owned()]);
        }
        match access {
            Access::Full => {}
            Access::Auto => launch.extend(["--permission-mode".to_owned(), "auto".to_owned()]),
            Access::ReadOnly => launch.extend(["--permission-mode".to_owned(), "plan".to_owned()]),
        }
        launch.push(prompt);
        let id = self.add_child_tab(&child, account, launch, access == Access::Full);
        let _ = self.tab(&id, |t| {
            t.made = child.made.clone();
            if access == Access::ReadOnly {
                t.permission_mode = Some("plan".into());
            }
        });
        self.delegates.link_thread(child.task, crate::delegate::Thread { tab: id.clone(), runs: true });
        if let Err(error) = self.start_claude(&id, UNWATCHED.0, UNWATCHED.1) {
            self.delegates.fail(child.task, error.clone(), self.launch.path_env.as_deref());
            self.close_tab(&id);
            return Err(error);
        }
        // Cancel, the limit and quit stop the task by this pid, which is the thread's session.
        // A new session clears the title, so it is set again.
        if let Some(pid) = self.tab(&id, |t| {
            t.title = Some(child.title.to_owned());
            t.session.as_ref().and_then(pty::Session::pid)
        }).ok().flatten() {
            self.delegates.remember_pid(child.task, pid);
        }
        self.arm_delegate_pass();
        self.emit_tabs();
        self.delegates.started_json(child.task).map(|value| value.to_string())
    }

    /// Opens the worker thread that shows a headless child under `from`. It does not start while
    /// the child works; it then resumes the child's conversation, when there is one.
    fn watch_thread(&self, child: ThreadChild, account: Account) {
        let id = self.add_child_tab(&child, account, vec![], false);
        let _ = self.tab(&id, |t| t.status = Some(AgentStatus::Working));
        self.delegates.link_thread(child.task, crate::delegate::Thread { tab: id, runs: false });
        self.emit_tabs();
    }

    fn add_child_tab(&self, child: &ThreadChild, account: Account, launch: Vec<String>, bypass: bool) -> TabId {
        let project = match child.origin {
            Some(_) => history::project_of(child.cwd),
            None => self.orchestrated(child.from).unwrap_or_else(|_| history::project_of(child.cwd)),
        };
        let id = self.add_tab(child.cwd.to_owned(), account, launch, role_env(Role::Worker, &project), bypass, None);
        let _ = self.tab(&id, |t| {
            t.spawned_by = child.origin.is_none().then(|| child.from.to_owned());
            t.origin = child.origin.clone();
            t.task = Some(child.task.to_owned());
            t.title = Some(child.title.to_owned());
            t.model = child.model.clone();
        });
        id
    }

    /// A worker thread's turn ended. When the tab runs a task still working, that ends it, and
    /// the task's notice tells the spawner. Returns false when the news is the worker's to report.
    fn settle_thread_task(&self, tab: &str, event: &AgentEvent) -> bool {
        let Ok(Some(task)) = self.tab(tab, |t| t.task.clone()) else { return false };
        let Some(info) = self.delegates.get(&task) else { return false };
        if info.state.terminal() || !info.thread.as_ref().is_some_and(|thread| thread.runs) {
            return false;
        }
        let error = match event.status {
            AgentStatus::Done => None,
            AgentStatus::Failed => Some(event.message.clone().unwrap_or_else(|| "the turn failed".into())),
            _ => return false,
        };
        if let Some(session) = self.tab(tab, |t| t.claude_session.clone()).ok().flatten() {
            self.delegates.set_conversation(&task, session);
        }
        // The meter reads a working task only; this reads the turn that just ended before it settles.
        self.meter_workers();
        let parent = self.delegates.settle_thread(&task, self.last_reply(tab), error, self.thread_steps(tab), self.launch.path_env.as_deref());
        // A thread for another machine's tab sends its end back through the hub on the next pass.
        if self.tab(tab, |t| t.origin.is_some()).unwrap_or(false) {
            self.arm_delegate_pass();
            return true;
        }
        // The notice waits for a spawner that is still open; a closed one leaves it to the worker's report.
        match parent {
            Some(parent) if self.tabs.lock().unwrap().contains_key(&parent) => {
                self.maybe_deliver(&parent);
                true
            }
            _ => false,
        }
    }

    /// Whether the worker's turn that just ended left background work it started pending, which
    /// Claude Code resumes the worker with once it hands back. Notes since when the worker waits.
    fn waits_on_background(&self, tab: &str) -> bool {
        let started = self.tab(tab, |t| t.started).ok().flatten();
        let pending = self.transcript(tab).ok().and_then(|file| thread::pending_background(&file, started).ok()).unwrap_or_default();
        let wait = (!pending.is_empty()).then(|| BackgroundWait { since: Instant::now(), shells_only: pending.iter().all(|b| *b == thread::Background::Shell) });
        let _ = self.tab(tab, |t| t.background_wait = wait);
        if wait.is_some() {
            self.arm_delegate_pass();
        }
        wait.is_some()
    }

    /// Ends the wait of a worker left waiting only on background shells and quiet for as long as a
    /// headless child is given: a shell such as a dev server never hands back, so the reply before
    /// it is the worker's last.
    fn end_quiet_waits(&self, now: Instant) {
        let quiet = Duration::from_millis(crate::delegate::QUIET_AFTER_REPLY_MS);
        let ended: Vec<(String, PathBuf)> = {
            let mut tabs = self.tabs.lock().unwrap();
            let ripe = tabs.iter_mut().filter(|(_, t)| t.background_wait.is_some_and(|w| w.shells_only && now.saturating_duration_since(w.since) >= quiet));
            ripe.filter_map(|(id, t)| {
                t.background_wait = None;
                t.status = Some(AgentStatus::Done);
                t.updated = Some(unix_now());
                Some((id.clone(), role_of(&t.env)?.1))
            })
            .collect()
        };
        for (tab, project) in ended {
            let event = AgentEvent { status: AgentStatus::Done, cwd: None, message: None, tab: Some(tab.clone()), session: None, title: None, resume_at: None, prompt: None, snapshot: None, permission_mode: None, hook_feedback: false };
            if !self.settle_thread_task(&tab, &event) {
                self.report_worker(&tab, &project, &event, Some(AgentStatus::Working));
            }
            self.maybe_deliver(&tab);
            self.emit_tabs();
        }
    }

    /// The last steps of a thread's conversation, as `task_status` lists them.
    fn thread_steps(&self, tab: &str) -> Vec<thread::Item> {
        let Some(page) = self.transcript(tab).ok().and_then(|file| thread::read(&file, 0).ok()) else { return vec![] };
        let skip = page.items.len().saturating_sub(crate::delegate::STEP_KEEP);
        page.items.into_iter().skip(skip).collect()
    }

    /// A headless child's thread follows its task: it shows the end, then resumes the child's
    /// conversation when opened. Its worktree becomes the thread's.
    fn follow_headless_threads(&self) {
        let watched: Vec<(String, String)> = self.tabs.lock().unwrap().iter().filter(|(_, t)| t.status == Some(AgentStatus::Working) && t.session.is_none()).filter_map(|(id, t)| Some((id.clone(), t.task.clone()?))).collect();
        let mut changed = false;
        for (tab, task) in watched {
            let Some(info) = self.delegates.get(&task) else { continue };
            if !info.state.terminal() || info.thread.as_ref().is_none_or(|thread| thread.runs) {
                continue;
            }
            let failed = matches!(info.state, crate::delegate::WorkState::Failed | crate::delegate::WorkState::Cancelled);
            let launch = info.conversation.as_deref().map(|id| crate::delegate::resume_args(info.provider, id)).unwrap_or_default();
            let made = (info.isolated && info.conversation.is_some()).then(|| info.cwd.clone());
            let _ = self.tab(&tab, |t| {
                t.status = Some(if failed { AgentStatus::Failed } else { AgentStatus::Done });
                t.message = info.error.clone();
                t.updated = Some(unix_now());
                t.launch = launch;
                t.made = made;
            });
            changed = true;
        }
        if changed {
            self.emit_tabs();
        }
    }

    fn parent_idle(&self, tab: &str) -> bool {
        self.tab(tab, |t| t.notice_idle()).unwrap_or(false)
    }

    /// Types one finished task's notice now. The tab must own the task. The idle gate does not apply.
    fn deliver_task(&self, tab: &str, task: &str) -> Result<(), String> {
        if !self.tab(tab, |t| t.draft.empty())? {
            return Err("The prompt has unsent text. Send or clear it (Ctrl-U), then try again.".into());
        }
        let caller = crate::delegate::Caller { tab: Some(tab.to_owned()), task: None };
        let text = self.delegates.claim_task_notice(&caller, task)?;
        if let Err(error) = self.type_prompt(tab, &text) {
            self.delegates.release_notice(&[task.to_owned()]);
            return Err(error);
        }
        Ok(())
    }

    /// The queued messages `typed` started their Capitão's turn, which clients then see working.
    fn mark_typed(&self, typed: &[String]) {
        if self.sent.lock().unwrap().typed(typed) {
            self.emit_tabs();
        }
    }

    fn maybe_deliver(&self, tab: &str) {
        if !self.parent_idle(tab) {
            return;
        }
        let Some((text, ids)) = self.delegates.claim_notice(tab) else { return };
        if self.write_notice(tab, &text) != Ok(true) {
            self.delegates.release_notice(&ids);
            self.delegates.release_reports(tab);
        } else {
            let typed = self.delegates.drop_claimed_reports(tab);
            self.sent.lock().unwrap().typed(&typed);
            self.emit_tabs();
        }
    }

    /// Writes `text` into a raw session. `Ok(false)` when the session has not taken the terminal,
    /// which leaves the notice unclaimed so a later idle can type it. Queuing it is not delivery.
    fn write_notice(&self, tab: &str, text: &str) -> Result<bool, String> {
        self.tab(tab, |t| {
            if !t.notice_idle() {
                return Ok(false);
            }
            match t.session.as_mut() {
                Some(s) if s.raw() => {
                    s.write(&paste(text)).map_err(|e| e.to_string())?;
                    t.turn_pending = Some(Instant::now());
                    Ok(true)
                }
                Some(_) => Ok(false),
                None => Err(format!("tab {tab} has no running session")),
            }
        })?
    }

    fn arm_delegate_pass(&self) {
        if self.delegate_pass_armed.swap(true, Ordering::Relaxed) {
            return;
        }
        let me = self.me.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let Some(machine) = me.upgrade() else { return };
            if machine.quitting.load(Ordering::Relaxed) {
                continue;
            }
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                machine.delegates.enforce_limits(Instant::now(), machine.launch.path_env.as_deref());
                machine.send_remote_ends(Instant::now());
                machine.meter_workers();
                if !machine.delegates.unrecorded(1).is_empty() {
                    machine.send_records();
                }
                machine.follow_headless_threads();
                machine.end_quiet_waits(Instant::now());
                let tabs: Vec<String> = machine.tabs.lock().unwrap().keys().cloned().collect();
                for tab in tabs {
                    if !machine.tab(&tab, Tab::reload_if_idle).unwrap_or(false) {
                        machine.maybe_deliver(&tab);
                    }
                }
            }));
        });
    }

    fn capabilities(&self, tab: Option<&str>, task: Option<&str>, search: &str) -> Result<String, String> {
        use crate::delegate::{capabilities, codex_constraints, cursor_models, Access, Inherited, ListedAccount};
        let (account, model, access) = if let Some(tab) = tab.filter(|tab| !tab.is_empty()) {
            self.tab(tab, |t| (t.account.clone(), t.model.clone(), t.child_access())).unwrap_or_else(|_| (Account::builtin(Provider::Claude), None, Access::Auto))
        } else if let Some(lineage) = task.filter(|task| !task.is_empty()).and_then(|id| self.delegates.lineage(id)) {
            let account = self.accounts.get(&lineage.account).unwrap_or_else(|| Account::builtin(lineage.provider));
            (account, lineage.model, lineage.access)
        } else {
            (Account::builtin(Provider::Claude), None, Access::Auto)
        };
        let parent = Inherited { account: account.id, model, access };
        let listed = self
            .accounts
            .list()
            .into_iter()
            .map(|account| {
                let status = self.status(&account);
                let bin = self.launch.bin(account.provider);
                let found = bin.is_file();
                let (models, mut constraints) = match account.provider {
                    Provider::Claude if account.auth == Auth::OpenCodeGo => {
                        (crate::opencode_go::models(), vec!["Runs through Claude Code against the OpenCode Go gateway.".into()])
                    }
                    Provider::Claude => (crate::delegate::claude_choices(&self.claude_models()), vec![]),
                    Provider::Codex => (vec![], codex_constraints()),
                    Provider::Cursor => {
                        let list = self.delegates.cursor_catalog(&bin, self.launch.path_env.as_deref());
                        let mut constraints = crate::delegate::cursor_constraints();
                        if found && list.trim().is_empty() {
                            constraints.push("the model list is not ready yet".into());
                        }
                        (cursor_models(&list), constraints)
                    }
                };
                if !status.signed_in {
                    constraints.push("not signed in on this machine".into());
                }
                if !found {
                    constraints.push(format!("{} is not installed on this machine", account.provider.spec().name));
                }
                let limited = self.usage.lock().unwrap().get(&account.id).is_some_and(|usage| {
                    [usage.five_hour, usage.seven_day, usage.monthly].iter().flatten().any(|window| window.used_percentage >= 99.0)
                });
                if limited {
                    constraints.push("a usage limit stopped this account".into());
                }
                ListedAccount { id: account.id, provider: account.provider, name: account.name, signed_in: status.signed_in, bin_found: found, limited, models, constraints }
            })
            .collect::<Vec<_>>();
        let search = (!search.is_empty()).then_some(search);
        Ok(capabilities(&parent, &listed, search).to_string())
    }

    /// Opens a finished delegated task's conversation as a thread nested under `tab`, its owner.
    fn open_delegated(&self, tab: &str, task: &str) -> Result<TabId, String> {
        use crate::delegate::Access;
        if let Some(thread) = self.delegates.get(task).and_then(|t| t.thread).filter(|thread| self.tabs.lock().unwrap().contains_key(&thread.tab)) {
            return Ok(thread.tab);
        }
        let info = self.delegates.open(&crate::delegate::Caller { tab: Some(tab.to_owned()), task: None }, task)?;
        let account = self.accounts.get(&info.account).ok_or_else(|| format!("no account {} on this machine", info.account))?;
        let bypass = info.access == Access::Full;
        let mut launch = crate::delegate::resume_args(info.provider, &info.conversation);
        let plan = info.access == Access::ReadOnly && info.provider == Provider::Claude;
        if plan {
            launch.extend(["--permission-mode".to_owned(), "plan".to_owned()]);
        }
        let session = (info.provider == Provider::Claude).then(|| info.conversation.clone());
        let id = self.add_tab(info.cwd.clone(), account, launch, vec![], bypass, session);
        let _ = self.tab(&id, |t| t.spawned_by = Some(tab.to_owned()));
        if plan {
            let _ = self.tab(&id, |t| t.permission_mode = Some("plan".into()));
        }
        if info.isolated {
            let _ = self.tab(&id, |t| t.made = Some(info.cwd));
        }
        self.emit_tabs();
        Ok(id)
    }

    fn record_event(&self, event: AgentEvent) {
        if let Some(tab) = &event.tab {
            let new_session = event.session.as_ref().is_some_and(|s| self.record_session(tab, s.clone(), event.cwd.as_deref()));
            let (before, role, waiting) = self.tab(tab, |t| (t.status, role_of(&t.env), t.background_wait.is_some())).unwrap_or_default();
            // Claude Code's idle notice after the turn end the worker waits past.
            if waiting && event.status == AgentStatus::NeedsInput && event.message.as_deref().is_some_and(|m| m.contains("waiting for your input")) {
                return;
            }
            let worker = role.as_ref().is_some_and(|(role, _)| *role == Role::Worker);
            let held = worker && event.status == AgentStatus::Done && self.waits_on_background(tab);
            self.record_status(tab, &event);
            if held {
                let _ = self.tab(tab, |t| t.status = Some(AgentStatus::Working));
            } else if event.status == AgentStatus::Done {
                self.note_question(tab);
            }
            match role {
                Some((Role::Worker, _)) if held => {}
                Some((Role::Worker, project)) => {
                    if !self.settle_thread_task(tab, &event) {
                        self.report_worker(tab, &project, &event, before);
                    }
                }
                Some((Role::Orchestrator, project)) if matches!(event.status, AgentStatus::Done | AgentStatus::Failed) => self.captain_turn_ended(tab, &project, &event),
                _ => {}
            }
            self.record_turn(tab, &event);
            let turn_ended = matches!(event.status, AgentStatus::Done | AgentStatus::Failed);
            if new_session || turn_ended {
                self.refresh_stats(tab, turn_ended);
            }
            self.maybe_deliver(tab);
            self.continue_when_limit_lifts(tab);
            self.emit_tabs();
        }
        self.emit(Event::Agent { event });
    }

    /// Runs `f` on the tab `id`, failing once the tab is closed.
    fn tab<R>(&self, id: &str, f: impl FnOnce(&mut Tab) -> R) -> Result<R, String> {
        self.tabs.lock().unwrap().get_mut(id).map(f).ok_or_else(|| format!("no tab {id}"))
    }

    fn add_tab(&self, cwd: PathBuf, account: Account, launch: Vec<String>, env: Vec<(String, String)>, bypass: bool, claude_session: Option<String>) -> TabId {
        let n = self.next_tab.fetch_add(1, Ordering::Relaxed);
        let id = format!("tab-{:x}-{n}", self.boot);
        let tab = Tab {
            cwd,
            account,
            program: Program::Agent,
            session: None,
            starting: false,
            started: None,
            launch,
            env,
            bypass,
            claude_session,
            tally: Tally::default(),
            last_found_transcript: None,
            interrupted: false,
            resume_at: None,
            held_at_limit: false,
            model: None,
            made: None,
            spawned_by: None,
            origin: None,
            task: None,
            turn: None,
            status: None,
            updated: None,
            title: None,
            message: None,
            question: None,
            held: vec![],
            turn_pending: None,
            background_wait: None,
            draft: Draft::default(),
            owes_reload: false,
            permission_mode: None,
            permission_unknown: false,
            screen: Arc::default(),
        };
        self.tabs.lock().unwrap().insert(id.clone(), tab);
        id
    }

    fn emit(&self, event: Event) {
        let listeners: Vec<Arc<Listener>> = self.listeners.lock().unwrap().iter().map(|(_, l)| l.clone()).collect();
        for listener in listeners {
            listener(&event);
        }
        (self.events)(event);
    }

    fn emit_tabs(&self) {
        self.emit(Event::Tabs { tabs: self.tabs() });
    }

    fn exited(&self, tab: &str, session: u64) {
        let Ok(screen) = self.tab(tab, |t| t.screen.clone()) else { return };
        if screen.lock().unwrap().session != session {
            return;
        }
        self.end_session(tab);
        let _ = self.tab(tab, |t| (t.session, t.background_wait) = (None, None));
        if let Some(task) = self.thread_task(tab, true) {
            self.delegates.fail(&task, "its thread's session ended".into(), self.launch.path_env.as_deref());
        }
        self.emit(Event::Exit { tab: tab.to_owned() });
        self.emit_tabs();
    }

    /// Writes the tabs whose claude session is still going, in the order they were opened.
    /// Does nothing once `quit` has run: the tabs are gone by then, and the file keeps them for the next launch.
    fn save_sessions(&self) {
        let Some(file) = self.sessions_file.as_ref().filter(|_| !self.quitting.load(Ordering::SeqCst)) else { return };
        let saved: Vec<sessions::Saved> = {
            let tabs = self.tabs.lock().unwrap();
            let parent_session = |t: &Tab| tabs.get(t.spawned_by.as_ref()?)?.claude_session.clone();
            let mut open: Vec<(u32, sessions::Saved)> = tabs.iter().filter_map(|(id, t)| Some((tab_number(id), sessions::Saved { spawned_by: parent_session(t), ..t.saved()? }))).collect();
            open.sort_by_key(|(n, _)| *n);
            open.into_iter().map(|(_, s)| s).collect()
        };
        if let Err(e) = sessions::save(file, &saved) {
            eprintln!("pinned: sessions did not save: {e}");
        }
    }

    /// Hooks report the session on every event, since /clear or /resume inside claude swaps it.
    /// True when the session is new to the tab.
    fn record_session(&self, tab: &str, session: String, reported_cwd: Option<&str>) -> bool {
        let Ok((new, changed)) = self.tab(tab, |t| {
            let new = t.claude_session.as_ref() != Some(&session);
            if new {
                t.tally = Tally::default();
            }
            let mut changed = new;
            // An orchestrator steps into its workers' worktrees to review them, and stays in the project.
            let orchestrator = role_of(&t.env).is_some_and(|(role, _)| role == Role::Orchestrator);
            if let Some(worktree) = reported_cwd.filter(|_| !orchestrator).and_then(|c| sessions::worktree(&t.cwd, Path::new(c))) {
                t.cwd = worktree;
                changed = true;
            }
            t.claude_session = Some(session);
            (new, changed)
        }) else {
            return false;
        };
        if changed {
            self.save_sessions();
            self.stamp_parent_sessions();
        }
        new
    }

    /// The notices the hook adds to the session's context, and the orchestrator requests among them.
    fn take_notices(&self, event: &AgentEvent) -> (String, Vec<String>) {
        let Some(tab) = event.tab.as_deref() else { return Default::default() };
        let wants = event.status == AgentStatus::Done || (event.status == AgentStatus::Working && event.prompt.is_some());
        if !wants {
            return Default::default();
        }
        let Some((text, _)) = self.delegates.claim_notice(tab) else { return Default::default() };
        let typed = self.delegates.drop_claimed_reports(tab);
        (text, typed)
    }

    fn stamp_parent_sessions(&self) {
        let sessions = {
            let tabs = self.tabs.lock().unwrap();
            tabs.iter().filter_map(|(id, tab)| tab.claude_session.clone().map(|session| (id.clone(), session))).collect()
        };
        self.delegates.stamp_sessions(&sessions);
    }

    /// Reads what the tab's transcript gained, off the caller's thread, then sends the tabs again.
    /// After a turn ends it reads again until that turn's duration shows up, or gives up.
    fn refresh_stats(&self, tab: &str, turn_ended: bool) {
        let (Some(machine), tab) = (self.me.upgrade(), tab.to_owned()) else { return };
        std::thread::spawn(move || {
            let before = machine.tab(&tab, |t| t.tally.turns).ok();
            for _ in 0..TURN_LOG_WAIT.0 {
                let after = machine.read_stats(&tab);
                if !turn_ended || after.is_none() || after != before {
                    break;
                }
                std::thread::sleep(TURN_LOG_WAIT.1);
            }
            machine.emit_tabs();
        });
    }

    /// Reads what the tab's transcript gained since the last read, outside the tabs lock, and
    /// returns the turns counted so far. `None` when the tab has no transcript to read.
    fn read_stats(&self, tab: &str) -> Option<u32> {
        let (session, mut tally, account) = self.tab(tab, |t| Some((t.claude_session.clone()?, t.tally.clone(), t.account.clone()))).ok()??;
        let file = self.find_transcript(&account, &session)?;
        tally.advance(&file).ok()?;
        let turns = tally.turns;
        self.tab(tab, |t| {
            if t.claude_session.as_ref() == Some(&session) {
                t.tally = tally;
            }
        })
        .ok()?;
        Some(turns)
    }

    fn record_status(&self, tab: &str, event: &AgentEvent) {
        let changed = self.tab(tab, |t| {
            let before = (t.interrupted, t.resume_at, t.held_at_limit);
            match event.status {
                AgentStatus::Working | AgentStatus::Done => {
                    t.interrupted = event.status == AgentStatus::Working;
                    t.resume_at = None;
                    t.held_at_limit = false;
                }
                AgentStatus::Failed => {
                    t.interrupted = true;
                    t.held_at_limit &= t.resume_at == event.resume_at;
                    t.resume_at = event.resume_at;
                }
                AgentStatus::NeedsInput => {}
            }
            let idle_notice = event.status == AgentStatus::NeedsInput && t.status == Some(AgentStatus::Done);
            if !idle_notice {
                t.updated = Some(unix_now());
            }
            if matches!(event.status, AgentStatus::Done | AgentStatus::Failed) {
                t.turn_pending = None;
            }
            if matches!(event.status, AgentStatus::Working | AgentStatus::Failed) {
                t.background_wait = None;
            }
            if event.status != AgentStatus::NeedsInput {
                t.question = None;
            }
            t.status = Some(event.status);
            if event.title.is_some() {
                t.title = event.title.clone();
            }
            t.message = event.message.clone().filter(|_| event.status == AgentStatus::NeedsInput);
            let mode_before = (t.permission_unknown, t.permission_mode.clone());
            if event.permission_mode.is_some() {
                t.permission_mode = event.permission_mode.clone();
                t.permission_unknown = false;
            }
            before != (t.interrupted, t.resume_at, t.held_at_limit) || mode_before != (t.permission_unknown, t.permission_mode.clone())
        });
        if changed == Ok(true) {
            self.save_sessions();
        }
    }

    /// Starts a turn on a prompt, or on its end saves what the session changed since.
    fn record_turn(&self, tab: &str, event: &AgentEvent) {
        use AgentStatus::*;
        let Some(tree) = event.snapshot.clone() else { return };
        match event.status {
            Working => {
                let _ = self.tab(tab, |t| t.turn = Some((tree, event.prompt.clone())));
            }
            Done | Failed => {
                let Ok(Some(((before, prompt), cwd, session))) =
                    self.tab(tab, |t| Some((t.turn.take()?, t.cwd.clone(), t.claude_session.clone()?)))
                else {
                    return;
                };
                let (Some(dir), Some((files, patch, truncated))) = (&self.history_dir, history::diff(&cwd, &before, &tree)) else { return };
                let turn = history::Turn { session, title: event.title.clone(), prompt, ended: unix_now(), files, patch, truncated, tree: Some(tree), before: Some(before) };
                history::append(&history::file_for(dir, &history::project_of(&cwd)), &turn);
            }
            NeedsInput => {}
        }
    }

    fn end_session(&self, tab: &str) {
        if self.quitting.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.tab(tab, |t| (t.interrupted, t.resume_at) = (false, None));
        let ended = self.tab(tab, |t| {
            t.tally = Tally::default();
            t.claude_session.take()
        });
        if ended.ok().flatten().is_some() {
            self.save_sessions();
        }
    }

    /// Types `CONTINUE_PROMPT` into the tab's session once its usage limit lifts. Safe to call again
    /// for the same limit: whichever waiter fires first clears `resume_at`, so the rest stand down.
    fn continue_when_limit_lifts(&self, tab: &str) {
        let Ok(Some(at)) = self.tab(tab, |t| t.resume_at) else { return };
        let (Some(machine), tab) = (self.me.upgrade(), tab.to_owned()) else { return };
        // A short sleep loop, since a laptop asleep past `at` must fire on wake.
        std::thread::spawn(move || {
            while unix_now() < at + LIMIT_MARGIN_SECS {
                std::thread::sleep(std::time::Duration::from_secs(15));
                if machine.tab(&tab, |t| t.resume_at) != Ok(Some(at)) {
                    return;
                }
            }
            let fired = machine.tab(&tab, |t| !t.held_at_limit && t.session.is_some() && t.resume_at == Some(at) && t.resume_at.take().is_some());
            if fired != Ok(true) {
                return;
            }
            let type_into = |data: &str| machine.tab(&tab, |t| t.session.as_mut().map(|s| s.write(data.as_bytes())));
            let _ = type_into(sessions::CONTINUE_PROMPT);
            // Enter arrives separately, or the TUI takes it as part of a paste.
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = type_into("\r");
            machine.save_sessions();
            machine.emit_tabs();
        });
    }

    /// Clones `repo` into `dest`, as T3 Code's Add project clones to the folder picked, and opens it as a project.
    fn clone_into(&self, repo: &str, dest: &str, token: Option<&str>) -> Result<PathBuf, String> {
        let target = projects::expand(dest, &home());
        if std::fs::read_dir(&target).is_ok_and(|mut entries| entries.next().is_some()) {
            return Err(format!("{} already holds files", target.display()));
        }
        clone(repo, &target, token)?;
        Ok(self.open_project(target))
    }

    /// The tabs whose conversations hold `query`, as T3 Code's thread search: a prompt's match before
    /// an answer's, then the tab that heard from its session last.
    pub fn search_threads(&self, query: &str) -> Vec<ThreadMatch> {
        if query.trim().chars().count() < 2 {
            return vec![];
        }
        let sessions: Vec<(TabId, Account, String, u64)> = self.tabs.lock().unwrap().iter().filter_map(|(id, t)| Some((id.clone(), t.account.clone(), t.claude_session.clone()?, t.updated.unwrap_or(0)))).collect();
        let mut found: Vec<(ThreadMatch, u64)> = sessions
            .into_iter()
            .filter_map(|(tab, account, session, updated)| {
                let found = thread::search(&self.find_transcript(&account, &session)?, query).ok()??;
                Some((ThreadMatch { tab, found }, updated))
            })
            .collect();
        found.sort_by_key(|(m, updated)| (m.found.source != "user", std::cmp::Reverse(*updated)));
        found.into_iter().take(50).map(|(m, _)| m).collect()
    }

    fn find_folder(&self, folder: &str, repo: Option<&str>, token: Option<&str>) -> Result<PathBuf, String> {
        let dir = PathBuf::from(folder);
        if !folder.is_empty() && dir.is_dir() {
            return Ok(dir);
        }
        let Some(repo) = repo.map(str::trim).filter(|r| !r.is_empty()) else {
            return Err(format!("{folder} does not exist on this machine"));
        };
        let known: Vec<PathBuf> = projects::recent(&self.projects.lock().unwrap()).into_iter().map(|p| p.path.clone()).collect();
        let same = |dir: &Path| git_remote(dir).is_some_and(|remote| same_repo(&remote, repo));
        if let Some(found) = known.into_iter().find(|p| same(p)) {
            return Ok(found);
        }
        let name = repo_name(repo).ok_or_else(|| format!("{repo} names no repository to clone"))?;
        let target = self.clones.join(name);
        if !target.exists() {
            clone(repo, &target, token)?;
        } else if !same(&target) {
            return Err(format!("{} exists and is not a clone of {repo}", target.display()));
        }
        Ok(self.open_project(target))
    }
}

/// Copies `<session>.jsonl` and the `<session>` folder of its subagents from `from` into `into`.
fn copy_tree(from: &Path, into: &Path, session: &str) -> std::io::Result<()> {
    let name = format!("{session}.jsonl");
    std::fs::copy(from.join(&name), into.join(&name))?;
    copy_dir(&from.join(session), &into.join(session))
}

fn copy_dir(from: &Path, into: &Path) -> std::io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(into)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = into.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
#[cfg(unix)]
const HOME_ENV: &str = "HOME";
// Where Claude Code looks for ~/.claude on Windows; HOME there is Git Bash's, when set at all.
#[cfg(windows)]
const HOME_ENV: &str = "USERPROFILE";

fn test_binary() -> bool {
    std::env::current_exe().ok().is_some_and(|exe| exe.components().any(|part| part.as_os_str() == "deps"))
}

pub(crate) fn home() -> PathBuf {
    PathBuf::from(std::env::var_os(HOME_ENV).unwrap_or_default())
}

impl Machine {
    /// Beside the price list, in the machine's settings folder.
    fn prices_file(&self) -> Option<PathBuf> {
        self.rates_file.as_ref().map(|f| f.with_file_name("usage-prices.json"))
    }
}

fn json<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// A machine as a tool names it: an id or a name, with or without the `@` of a tag.
fn machine_named(machine: &str) -> &str {
    machine.trim().trim_start_matches('@')
}

/// How many conversation items a `read_session` returns: 20 when not asked, at most 50.
fn read_items(items: Option<usize>) -> usize {
    items.unwrap_or(reading::DEFAULT_ITEMS).clamp(1, READ_ITEMS_MAX)
}

const READ_ITEMS_MAX: usize = 50;

/// Whether `path` is absolute on macOS, Linux or Windows. A folder on another machine follows that
/// machine's rules, not this one's.
fn is_absolute_anywhere(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/') || path.starts_with(r"\\") || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
}

/// The hub's error, or which side needs an update when it is an older hub's or machine's refusal.
/// A remote task's status or start, naming the machine that runs it and its thread there.
fn with_remote(mut body: Value, remote: &crate::delegate::RemoteTarget) -> Value {
    body["machine"] = json!(remote.name);
    body["tabId"] = json!(remote.tab);
    body
}

fn needs_update(e: String) -> String {
    if crate::link::is_unknown_command(&e) {
        "o hub ou a máquina precisa atualizar o pinned".to_owned()
    } else {
        e
    }
}

/// A tool's answer as one line of JSON, which the session reads and the chat can render.
fn text_json<T: Serialize>(value: T) -> Result<String, String> {
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

fn canonical(path: &str) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.into())
}

/// How many memories a search returns when not told.
const SEARCH_LIMIT: usize = 8;

/// Most of a worker's last message an orchestrator is sent, or of a Capitão's last message the
/// global orchestrator is sent; the rest stays in the thread.
const REPORT_CHARS: usize = 4000;

/// `text` cut to `max` characters, with a note that says where the rest is.
fn truncate(text: &str, max: usize, rest: &str) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}\n[cut short: {rest}]", &text[..cut]),
        None => text.to_owned(),
    }
}

/// Opening order: the number after the last `-`. `tab-1` and `tab-<boot>-1` both order as 1.
fn tab_number(id: &str) -> u32 {
    id.rsplit('-').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// A value chosen when the process starts, different for each `Machine` in the process.
fn fresh_boot() -> u64 {
    use sha2::{Digest, Sha256};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let bytes = Sha256::digest(format!("{}:{nanos}:{n}", std::process::id()));
    u64::from_le_bytes(bytes[..8].try_into().unwrap())
}

/// The terminal's input line as far as the keys show it.
#[derive(Clone, Copy, Debug, Default)]
struct Draft {
    /// Characters typed into the line.
    chars: usize,
    /// Up or down put an earlier prompt in the line. Its length is unknown, so backspace cannot empty it.
    recalled: bool,
}

impl Draft {
    fn empty(&self) -> bool {
        self.chars == 0 && !self.recalled
    }
}

/// Follows the terminal's input line. Printable text adds a character, including newlines inside
/// a bracketed paste. Backspace removes one, floored at 0. Ctrl-C, Ctrl-U, and Enter clear the line.
/// Enter on a non-empty line is a submit. Up and down recall a prompt, so the line stays occupied
/// until Enter, Ctrl-U, or Ctrl-C. Other escape sequences, such as focus reports, are ignored.
fn note_draft(draft: &mut Draft, data: &[u8]) -> bool {
    let mut submitted = false;
    let mut pasting = false;
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            0x1b => {
                let Some(len) = escape_len(&data[i + 1..]) else { break };
                match &data[i + 1..i + 1 + len] {
                    b"[200~" => pasting = true,
                    b"[201~" => pasting = false,
                    b"[A" | b"[B" | b"OA" | b"OB" => draft.recalled = true,
                    _ => {}
                }
                i += len;
            }
            b'\r' | b'\n' if pasting => draft.chars += 1,
            b'\r' => {
                submitted |= !draft.empty();
                *draft = Draft::default();
            }
            0x03 | 0x15 => *draft = Draft::default(),
            0x08 | 0x7f => draft.chars = draft.chars.saturating_sub(1),
            0x80..=0xbf => {}
            byte if byte >= 0x20 => draft.chars += 1,
            _ => {}
        }
        i += 1;
    }
    submitted
}

fn escape_len(rest: &[u8]) -> Option<usize> {
    if rest.is_empty() {
        return Some(0);
    }
    match rest[0] {
        b'[' => rest[1..].iter().position(|b| (0x40..=0x7e).contains(b)).map(|at| at + 2),
        b']' => {
            let mut i = 1;
            while i < rest.len() {
                if rest[i] == 0x07 {
                    return Some(i + 1);
                }
                if rest[i] == 0x1b && rest.get(i + 1) == Some(&b'\\') {
                    return Some(i + 2);
                }
                i += 1;
            }
            None
        }
        b'O' | b'N' => rest.get(1).map(|_| 2),
        _ => Some(1),
    }
}

/// `text` as a bracketed paste and Enter, so its lines reach claude as one prompt that it submits.
/// Control characters other than newlines and tabs are dropped, so the text can't end the paste early.
fn paste(text: &str) -> Vec<u8> {
    let text: String = text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
    [b"\x1b[200~", text.as_bytes(), b"\x1b[201~\r"].concat()
}

pub fn git_remote(dir: &Path) -> Option<String> {
    let out = process::command("git")
        .arg("-C")
        .arg(dir)
        .args(["remote", "get-url", "origin"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let url = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !url.is_empty()).then_some(url)
}

/// The folder name a clone of `repo` gets under `~/pinned`.
pub fn repo_name(repo: &str) -> Option<&str> {
    let name = repo.trim_end_matches('/').rsplit(['/', ':']).next()?.trim_end_matches(".git");
    (!name.is_empty() && name != "." && name != "..").then_some(name)
}

/// Offers the tastes in `dir`, a signed-out app's or the old cost style's seed, to the account,
/// which takes them only while it has none. The defaults are not offered: the hub answers them to
/// an account with none anyway, and taking them would refuse a later machine's own.
fn share_tastes(hub: &dyn Fn(HubCall) -> Result<Value, String>, shared: &AtomicBool, dir: Option<&Path>) {
    if shared.load(Ordering::SeqCst) {
        return;
    }
    let Some(tastes) = taste::load(dir).filter(|own| *own != Tastes::default()) else { return };
    if hub(HubCall::ImportTastes { tastes }).is_ok() {
        shared.store(true, Ordering::SeqCst);
    }
}

/// The taste a Capitão in `folder` follows: its project's, or every Capitão's when the folder has
/// no origin to name the project by.
fn captain_scope(folder: &Path) -> TasteScope {
    git_remote(folder).and_then(|remote| repo_key(&remote)).map_or(TasteScope::Captains, |repo| TasteScope::Project { repo })
}

/// A repository's key in the tastes, as in `github.com/acme/site`, from any of its URLs.
pub fn repo_key(url: &str) -> Option<String> {
    https_url(url).map(|https| https.trim_start_matches("https://").trim_end_matches(".git").to_lowercase())
}

/// The https address of a repository as written, whichever of its URLs is given.
fn https_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest.to_owned(),
        None => url.replacen(':', "/", 1),
    };
    let rest = rest.rsplit_once('@').map_or(rest.as_str(), |(_, host_path)| host_path);
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let host = parts.next()?.split(':').next()?.to_lowercase();
    let path: Vec<&str> = parts.collect();
    (path.len() >= 2).then(|| format!("https://{host}/{}.git", path.join("/")))
}

fn same_repo(a: &str, b: &str) -> bool {
    a == b || repo_key(a).is_some_and(|key| Some(key) == repo_key(b))
}

/// Clones `repo`, and if that fails, its https form: a machine without ssh access to the
/// host, such as a VPS holding only another repository's deploy key, can still clone a public one.
fn clone(repo: &str, into: &Path, token: Option<&str>) -> Result<(), String> {
    let https = https_url(repo);
    // With the account's token a GitHub repository clones over https, private or not, and keeps the
    // token in the clone's own config, as actions/checkout does, so the session can fetch and push.
    if let (Some(token), Some(https)) = (token, https.as_ref().filter(|url| url.starts_with("https://github.com/"))) {
        let auth = github::auth_config(token);
        clone_once(https, into, Some(&auth))?;
        return git_config(into, &auth);
    }
    let first = clone_once(repo, into, None);
    match (first, https.filter(|https| https != repo)) {
        (Err(e), Some(https)) => clone_once(&https, into, None).map_err(|again| format!("{e}; {again}")),
        (first, _) => first,
    }
}

fn git_config(repo: &Path, (key, value): &(&str, String)) -> Result<(), String> {
    let out = process::command("git").arg("-C").arg(repo).args(["config", "--local", key, value]).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    out.status.success().then_some(()).ok_or_else(|| format!("git config failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
}

/// Clones without a terminal: a repository that needs a password fails instead of waiting for one.
fn clone_once(repo: &str, into: &Path, config: Option<&(&str, String)>) -> Result<(), String> {
    if let Some(parent) = into.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut git = process::command("git");
    // Through the environment, so the token stays out of the process list.
    if let Some((key, value)) = config {
        git.env("GIT_CONFIG_COUNT", "1").env("GIT_CONFIG_KEY_0", key).env("GIT_CONFIG_VALUE_0", value);
    }
    let out = git
        .arg("clone")
        .arg(repo)
        .arg(into)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", std::env::var("GIT_SSH_COMMAND").unwrap_or_else(|_| "ssh -o BatchMode=yes".into()))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git clone {repo}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("git clone {repo} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// The files `name` goes by on PATH: on Windows the native installer's binary first, then npm's shim.
fn bin_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd")]
    } else {
        vec![name.to_owned()]
    }
}

/// The first of `name`'s files found on `path_env`, or where the native installer puts it.
fn find_bin(name: &str, path_env: Option<&str>) -> PathBuf {
    let dirs: Vec<PathBuf> = path_env.map(|path| std::env::split_paths(path).collect()).unwrap_or_default();
    let names = bin_names(name);
    names
        .iter()
        .flat_map(|name| dirs.iter().map(move |dir| dir.join(name)))
        .find(|p| p.is_file())
        .unwrap_or_else(|| home().join(".local").join("bin").join(&names[0]))
}

/// Apps opened from Finder inherit launchd's bare PATH. Ask the user's login shell for the
/// PATH their terminal sees, so claude and the tools it runs (git, node, brew) resolve.
#[cfg(unix)]
fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let out = process::command(shell)
        .args(["-ilc", "printf '\\n__PINNED_PATH__%s__PINNED_PATH__' \"$PATH\""])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split("__PINNED_PATH__").nth(1).map(str::to_owned)
}

/// A Windows app inherits the user's whole PATH already.
#[cfg(windows)]
fn login_shell_path() -> Option<String> {
    None
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A version 4 UUID for a forked conversation, as Claude Code names sessions, drawn from the clock and
/// the session it came from.
fn new_session_id(from: &str) -> String {
    use sha2::{Digest, Sha256};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut b: [u8; 16] = Sha256::digest(format!("{from}:{nanos}:{}", std::process::id()))[..16].try_into().unwrap();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// Writes `text` to `path` inside `cwd`, creating its folders, and returns the path relative to `cwd`.
/// A path that climbs out of the folder is refused.
fn save_file(cwd: &Path, path: &str, text: &str) -> Result<String, String> {
    let relative = Path::new(path.trim());
    if path.trim().is_empty() || relative.is_absolute() || relative.components().any(|c| !matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir)) {
        return Err("enter a path inside the workspace".into());
    }
    let target = cwd.join(relative);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    std::fs::write(&target, text).map_err(|e| format!("could not write {path}: {e}"))?;
    Ok(relative.to_string_lossy().into_owned())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::agent::{session_env, Subject};
    #[cfg(unix)]
    use crate::connectors::Delivery;
    use serde_json::json;
    use std::time::{Duration, Instant};

    pub(crate) fn launch() -> Launch {
        Launch {
            exe: "/opt/pinned/bin/pinned".into(),
            bins: BTreeMap::new(),
            path_env: None,
            settings: "s".into(),
            mcp_config: None,
            reminders: true,
            plugin_dir: "p".into(),
            tutorial_helper: "t".into(),
            sock: "k".into(),
            vector_search: false,
        }
    }

    fn machine() -> Arc<Machine> {
        Machine::new(launch(), None, Some("/repo".into()), |_| {})
    }

    #[test]
    fn call_hub_reaches_the_installed_hub_and_refuses_while_unlinked() {
        let machine = machine();
        assert_eq!(machine.call_hub(HubCall::Overview).unwrap_err(), "this machine is not linked to a hub");
        let overview = Arc::new(AtomicBool::new(false));
        let seen = overview.clone();
        machine.set_hub(Arc::new(move |call| {
            seen.store(matches!(call, HubCall::Overview), Ordering::SeqCst);
            Ok(json!({"ok": true}))
        }));
        assert_eq!(machine.call_hub(HubCall::Overview).unwrap(), json!({"ok": true}));
        assert!(overview.load(Ordering::SeqCst), "the hub was not handed the Overview");
    }

    #[test]
    fn a_restarting_update_runs_the_restart_that_install_scheduled() {
        let machine = machine();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        machine.schedule_restart(Box::new(move || flag.store(true, Ordering::SeqCst)));
        machine.restart_if_replacing(&UpdateStatus::Updating { to: "9.9.9".into() });
        assert!(!ran.load(Ordering::SeqCst), "an update that is still downloading restarted");
        machine.restart_if_replacing(&UpdateStatus::Restarting { to: "9.9.9".into() });
        assert!(ran.load(Ordering::SeqCst), "a release that is ready to restart left the scheduled restart sitting");
    }

    #[test]
    fn a_restarted_machine_does_not_reuse_tab_ids() {
        let first = machine();
        let old = first.open_tab(None, false).unwrap();
        let second = machine();
        let new_id = second.open_tab(None, false).unwrap();
        let later = second.open_tab(None, false).unwrap();
        assert_ne!(old, new_id, "the new process reused {old}");
        assert_eq!(second.view(&old).unwrap_err(), format!("no tab {old}"));
        assert_eq!((second.view(&new_id).unwrap().opened, second.view(&later).unwrap().opened), (1, 2));
        assert_eq!((tab_number("tab-1"), tab_number("tab-abcdef-3")), (1, 3));
    }

    #[test]
    fn quitting_announces_an_empty_tab_list() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let events = seen.clone();
        let m = Machine::new(launch(), None, Some("/repo".into()), move |event| {
            if let Event::Tabs { tabs } = event {
                events.lock().unwrap().push(tabs.len());
            }
        });
        m.open_tab(None, false).unwrap();
        m.quit();
        assert_eq!(seen.lock().unwrap().last().copied(), Some(0));
    }

    #[test]
    #[cfg(unix)]
    fn a_worker_tree_survives_a_restart() {
        let root = temp("restore-tree");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        let before = accounts_machine(&root);
        let open = |session: &str, parent: Option<&TabId>| {
            let tab = before.add_tab(app.clone(), Account::builtin(Provider::Claude), vec![], role_env(Role::Worker, &app), false, Some(session.into()));
            before.tab(&tab, |t| t.spawned_by = parent.cloned()).unwrap();
            tab
        };
        let root_tab = open("s-root", None);
        let worker = open("s-worker", Some(&root_tab));
        open("s-nested", Some(&worker));
        before.save_sessions();
        let saved: Vec<_> = sessions::load(&root.join("config/sessions.json")).into_iter().map(|s| (s.session, s.spawned_by)).collect();
        assert_eq!(saved, [("s-root".into(), None), ("s-worker".into(), Some("s-root".into())), ("s-nested".into(), Some("s-worker".into()))]);

        let after = accounts_machine(&root);
        let tabs = after.restore_tabs();
        let parents: Vec<_> = tabs.iter().map(|id| after.view(id).unwrap().spawned_by).collect();
        assert_eq!(parents, [None, Some(tabs[0].clone()), Some(tabs[1].clone())], "the restored tabs nest under their restored parents");
        let _ = std::fs::remove_dir_all(root);
    }

    /// The tray's Quit and an update's restart call `quit`, then Tauri's ExitRequested calls it again.
    #[test]
    #[cfg(unix)]
    fn quitting_twice_keeps_the_saved_threads() {
        let root = temp("quit-twice");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        let before = accounts_machine(&root);
        for session in ["s1", "s2"] {
            before.add_tab(app.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, Some(session.into()));
        }
        before.quit();
        before.quit();
        let tab = before.add_tab(app.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, Some("s3".into()));
        before.close_tab(&tab);
        let saved: Vec<String> = sessions::load(&root.join("config/sessions.json")).into_iter().map(|s| s.session).collect();
        assert_eq!(saved, ["s1", "s2"], "nothing after the first quit rewrites the threads it saved");

        let after = accounts_machine(&root);
        assert_eq!(after.restore_tabs().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_restored_tab_never_becomes_its_own_parent() {
        let root = temp("restore-self-parent");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("config/sessions.json"),
            format!(r#"[{{"cwd": "{}", "session": "s1", "env": [], "bypass": false, "interrupted": false, "resume_at": null, "spawned_by": "s1"}}]"#, app.display()),
        )
        .unwrap();
        let m = accounts_machine(&root);
        let tabs = m.restore_tabs();
        assert_eq!(tabs.len(), 1);
        assert_eq!(m.view(&tabs[0]).unwrap().spawned_by, None, "a saved parent equal to the tab's own session is dropped");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_restored_interrupted_tab_reports_work_left() {
        let root = temp("restore-interrupted");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("config/sessions.json"),
            format!(r#"[{{"cwd": "{}", "session": "s1", "env": [], "bypass": false, "interrupted": true, "resume_at": null}}]"#, app.display()),
        )
        .unwrap();
        let m = accounts_machine(&root);
        let tabs = m.restore_tabs();
        assert_eq!(tabs.len(), 1);
        let view = m.view(&tabs[0]).unwrap();
        assert!(view.interrupted, "the UI prompts continuations from this flag");
        assert!(!view.running);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn starting_a_restored_tab_twice_keeps_its_claude_session() {
        let root = temp("restore-twice");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("config/sessions.json"),
            format!(r#"[{{"cwd": "{}", "session": "s1", "env": [], "bypass": false, "interrupted": false, "resume_at": null}}]"#, app.display()),
        )
        .unwrap();
        let m = accounts_machine(&root);
        let tabs = m.restore_tabs();
        assert_eq!(tabs.len(), 1);
        let id = tabs[0].clone();
        m.start_claude(&id, 80, 24).unwrap();
        m.start_claude(&id, 80, 24).unwrap();
        let view = m.tabs().into_iter().find(|tab| tab.id == id).unwrap();
        assert_eq!(view.session.as_deref(), Some("s1"));
        assert!(view.running);
        let _ = std::fs::remove_dir_all(root);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-machine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(dir).unwrap()
    }

    #[cfg(unix)]
    fn sh(script: &str) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[cfg(unix)]
    fn running(m: &Machine, env: Vec<(String, String)>, script: &str) -> TabId {
        let id = m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], env, false, None);
        m.spawn(&id, sh(script), pty::size(80, 24)).unwrap();
        id
    }

    fn viewer() -> (Arc<Mutex<Vec<u8>>>, Sink) {
        let seen = Arc::new(Mutex::new(vec![]));
        let sink = seen.clone();
        (seen, Box::new(move |b: &[u8]| sink.lock().unwrap().extend_from_slice(b)))
    }

    fn text(seen: &Mutex<Vec<u8>>) -> String {
        String::from_utf8_lossy(&seen.lock().unwrap()).into_owned()
    }

    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        assert!(process::command("git").arg("-C").arg(dir).args(args).status().unwrap().success(), "git {args:?}");
    }

    #[cfg(unix)]
    fn slack_env(connector: &str, thread: &str) -> Vec<(String, String)> {
        session_env(connector, &Subject::SlackThread { channel: "C0123ABCD".into(), thread: thread.into() })
    }

    #[cfg(unix)]
    fn delivery(connector: &str, thread: &str, messages: &[&str]) -> Delivery {
        Delivery {
            connector: connector.into(),
            channel: "C0123ABCD".into(),
            thread: thread.into(),
            messages: messages.iter().map(|m| m.to_string()).collect(),
            cursor: "1792400090.000000".into(),
            mentioned: vec![],
        }
    }

    #[test]
    fn a_saved_file_stays_inside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(save_file(dir.path(), " plans/ship.md ", "# Ship\n").unwrap(), "plans/ship.md");
        assert_eq!(std::fs::read_to_string(dir.path().join("plans/ship.md")).unwrap(), "# Ship\n");
        for bad in ["", "../out.md", "/etc/x", "a/../../b"] {
            assert!(save_file(dir.path(), bad, "x").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn forked_sessions_get_fresh_version_4_ids() {
        let (a, b) = (new_session_id("s"), new_session_id("s"));
        assert_ne!(a, b);
        assert!(a.len() == 36 && a.as_bytes()[14] == b'4' && "89ab".contains(a.as_bytes()[19] as char), "{a}");
    }

    #[test]
    fn a_paste_is_bracketed_submitted_and_cannot_close_itself() {
        assert_eq!(paste("[Slack reply from ana in the thread]\nyes,\tdo it"), b"\x1b[200~[Slack reply from ana in the thread]\nyes,\tdo it\x1b[201~\r");
        assert_eq!(paste("a\x1b[201~\rb\x07"), b"\x1b[200~a[201~b\x1b[201~\r");
    }

    #[cfg(unix)]
    #[test]
    fn a_delivery_finds_the_live_tab_of_its_thread() {
        let m = machine();
        running(&m, session_env("c2", &Subject::LinearIssue { issue: "uuid".into() }), "sleep 30");
        running(&m, slack_env("c9", "1792400050.000200"), "sleep 30");
        running(&m, slack_env("c2", "1792499999.000100"), "sleep 30");
        m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], slack_env("c2", "1792400050.000200"), false, None);
        let d = delivery("c2", "1792400050.000200", &["hi"]);
        assert_eq!(m.live_tab(&d.env()), None, "the thread's only tab has no running session");
        assert!(!m.deliver(&d.env(), &d.text()));
        let own = running(&m, slack_env("c2", "1792400050.000200"), "sleep 30");
        assert_eq!(m.view(&own).unwrap().opened, 5);
        assert_eq!(m.live_tab(&d.env()), Some(own));
        let newer = running(&m, slack_env("c2", "1792400050.000200"), "sleep 30");
        assert_eq!(m.live_tab(&d.env()), Some(newer));
        assert_eq!(m.live_tab(&[]), None, "an empty environment matches no session");
    }

    /// The session is a real pty running cat with the terminal in raw mode, so the file holds
    /// exactly the bytes the session received.
    #[cfg(unix)]
    #[test]
    fn a_delivery_reaches_the_session_through_its_pty() {
        let file = std::env::temp_dir().join(format!("pinned-delivery-{}.bin", std::process::id()));
        let m = machine();
        let tab = running(&m, slack_env("c2", "1792400050.000200"), &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));

        let d = delivery("c2", "1792400050.000200", &["[Slack reply from ana in the thread]\nyes, create the task", "[Slack reply from bruno in the thread]\nand ship it"]);
        assert_eq!(m.call(Call::Deliver { env: d.env(), text: d.text() }), Ok(json!(true)));
        let expected: &[u8] = b"\x1b[200~[Slack reply from ana in the thread]\nyes, create the task\n\n[Slack reply from bruno in the thread]\nand ship it\x1b[201~\r";
        wait_for("cat to write", || std::fs::read(&file).is_ok_and(|b| b.len() >= expected.len()));
        assert_eq!(std::fs::read(&file).unwrap(), expected);
        let elsewhere = delivery("c2", "1792499999.000100", &["elsewhere"]);
        assert!(!m.deliver(&elsewhere.env(), &elsewhere.text()));
        let _ = std::fs::remove_file(file);
    }

    /// Until claude takes the terminal, the pty is in cooked mode: it echoes what is typed and
    /// turns the submitting \r into \n, so a prompt typed that early never submits. The session
    /// here draws first and takes the terminal a while later, as claude does, and the prompt still
    /// arrives whole, and only then.
    #[test]
    #[cfg(unix)]
    fn a_prompt_typed_before_the_session_takes_the_terminal_waits_for_it() {
        let file = std::env::temp_dir().join(format!("pinned-early-prompt-{}.bin", std::process::id()));
        let m = machine();
        let tab = running(&m, vec![], &format!("sleep 0.2; echo drawn; sleep 0.3; stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();

        assert_eq!(m.call(Call::Prompt { tab: tab.clone(), text: "typed early".into() }), Ok(json!(null)));
        wait_for("the session to draw", || text(&seen).contains("drawn"));
        assert_eq!(m.call(Call::Prompt { tab: tab.clone(), text: "typed after drawing".into() }), Ok(json!(null)));
        wait_for("stty", || text(&seen).contains("ready"));
        let expected: &[u8] = b"\x1b[200~typed early\x1b[201~\r\x1b[200~typed after drawing\x1b[201~\r";
        wait_for("cat to write", || std::fs::read(&file).is_ok_and(|b| b.len() >= expected.len()));
        assert_eq!(std::fs::read(&file).unwrap(), expected);
        assert!(text(&seen).starts_with("drawn\r\nready"), "the terminal echoed a prompt before the session took it: {:?}", text(&seen));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn a_screen_keeps_only_the_last_256_kib_of_its_session() {
        let mut screen = Screen::default();
        let before = screen.restart();
        screen.push(before, b"the session before");
        let session = screen.restart();
        let output: Vec<u8> = (0..300 * 1024).map(|i| (i % 251) as u8).collect();
        for chunk in output.chunks(16 * 1024) {
            screen.push(session, chunk);
        }
        screen.push(before, b"a killed session's last words");
        let (seen, sink) = viewer();
        screen.attach("late".into(), sink);
        assert_eq!(seen.lock().unwrap().len(), 256 * 1024);
        assert_eq!(*seen.lock().unwrap(), output[output.len() - 256 * 1024..]);
    }

    #[cfg(unix)]
    #[test]
    fn stopping_a_turn_types_what_the_apps_stop_button_does() {
        let m = machine();
        let typed = temp("stop-turn").join("typed");
        let tab = running(&m, vec![], &format!("stty raw -echo; dd bs=1 count=6 2>/dev/null | od -An -tx1 > {}; sleep 30", typed.display()));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(m.call(Call::StopTurn { tab: tab.clone() }), Ok(Value::Null));
        wait_for("the keys", || std::fs::read_to_string(&typed).is_ok_and(|t| !t.trim().is_empty()));
        assert_eq!(std::fs::read_to_string(&typed).unwrap().split_whitespace().collect::<Vec<_>>(), ["18", "0b", "18", "0b", "1b", "1b"]);
        assert_eq!(m.call(Call::StopTurn { tab: "tab-none".into() }), Err("no tab tab-none".into()));
        m.close_tab(&tab);
    }

    #[cfg(unix)]
    #[test]
    fn a_late_viewer_gets_the_screen_then_live_output_across_a_restart() {
        let m = machine();
        let tab = running(&m, vec![], "printf 'first\\n'; read line; printf 'got %s\\n' \"$line\"; sleep 30");
        let (early, sink) = viewer();
        m.attach(&tab, "early".into(), sink).unwrap();
        wait_for("first", || text(&early).contains("first"));

        let (late, sink) = viewer();
        m.attach(&tab, "late".into(), sink).unwrap();
        assert_eq!(text(&late), "first\r\n");
        m.write_input(&tab, b"x\r").unwrap();
        wait_for("the answer", || text(&late).contains("got x"));
        assert_eq!(text(&late), "first\r\nx\r\ngot x\r\n");
        assert_eq!(text(&early), "first\r\nx\r\ngot x\r\n");

        m.detach(&tab, "early");
        m.spawn(&tab, sh("printf 'second\\n'; sleep 30"), pty::size(80, 24)).unwrap();
        wait_for("second", || text(&late).contains("second"));
        assert_eq!(text(&late), "first\r\nx\r\ngot x\r\nsecond\r\n");
        assert_eq!(text(&early), "first\r\nx\r\ngot x\r\n");
        let (again, sink) = viewer();
        m.attach(&tab, "late".into(), sink).unwrap();
        assert_eq!(text(&again), "second\r\n", "a restart starts a fresh screen");
    }

    #[cfg(unix)]
    #[test]
    fn a_session_that_exits_reports_it_and_stops_running() {
        let events = Arc::new(Mutex::new(vec![]));
        let seen = events.clone();
        let m = Machine::new(launch(), None, Some("/repo".into()), move |e| seen.lock().unwrap().push(serde_json::to_value(e).unwrap()));
        let tab = running(&m, vec![], "exit 0");
        wait_for("the exit", || events.lock().unwrap().iter().any(|e| e["kind"] == "exit"));
        assert_eq!(events.lock().unwrap().iter().find(|e| e["kind"] == "exit"), Some(&json!({ "kind": "exit", "tab": tab })));
        assert!(!m.view(&tab).unwrap().running);
        assert_eq!(events.lock().unwrap().last().unwrap()["tabs"][0]["running"], false);
    }

    #[cfg(windows)]
    #[test]
    fn a_session_runs_in_conpty_and_reports_its_exit() {
        let events = Arc::new(Mutex::new(vec![]));
        let seen_events = events.clone();
        let m = Machine::new(launch(), None, Some("/repo".into()), move |e| seen_events.lock().unwrap().push(serde_json::to_value(e).unwrap()));
        let tab = m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/c", "echo pinned on windows"]);
        m.spawn(&tab, cmd, pty::size(80, 24)).unwrap();
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        let start = Instant::now();
        while !text(&seen).contains("pinned on windows") {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for the echo, saw {:?}", text(&seen));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!text(&seen).contains("\x1b[6n"), "the cursor request reached the viewer");
        wait_for("the exit", || events.lock().unwrap().iter().any(|e| e["kind"] == "exit"));
        assert_eq!(events.lock().unwrap().iter().find(|e| e["kind"] == "exit"), Some(&json!({ "kind": "exit", "tab": tab })));
        assert!(!m.view(&tab).unwrap().running);
    }

    #[cfg(unix)]
    #[test]
    fn claude_is_found_on_path_or_where_the_native_installer_puts_it() {
        let root = temp("find-claude");
        let (empty, bin) = (root.join("empty"), root.join("bin"));
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(bin.join("claude.d")).unwrap();
        let path = std::env::join_paths([&empty, &bin]).unwrap().into_string().unwrap();
        assert_eq!(find_bin("claude", Some(&path)), PathBuf::from(std::env::var("HOME").unwrap()).join(".local/bin/claude"), "a folder is not claude");
        std::fs::write(bin.join("claude"), "").unwrap();
        assert_eq!(find_bin("claude", Some(&path)), bin.join("claude"));
        assert_eq!(find_bin("claude", None), PathBuf::from(std::env::var("HOME").unwrap()).join(".local/bin/claude"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn claude_is_the_native_exe_before_npms_shim_anywhere_on_path() {
        let root = temp("find-claude");
        let (npm, native) = (root.join("npm"), root.join("native"));
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::create_dir_all(&native).unwrap();
        for shim in ["claude", "claude.cmd", "claude.ps1"] {
            std::fs::write(npm.join(shim), "").unwrap();
        }
        std::fs::write(native.join("claude.exe"), "").unwrap();
        let path = std::env::join_paths([&npm, &native]).unwrap().into_string().unwrap();
        assert_eq!(find_bin("claude", Some(&path)), native.join("claude.exe"));
        std::fs::remove_file(native.join("claude.exe")).unwrap();
        assert_eq!(find_bin("claude", Some(&path)), npm.join("claude.cmd"));
        assert_eq!(find_bin("claude", None), PathBuf::from(std::env::var("USERPROFILE").unwrap()).join(r".local\bin\claude.exe"));
        let _ = std::fs::remove_dir_all(root);
    }

    fn assistant_line(text: &str) -> String {
        format!(
            "{{\"type\":\"assistant\",\"uuid\":\"{text}\",\"timestamp\":\"2020-01-01T00:00:00Z\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
        )
    }

    fn thread_events(heard: &Mutex<Vec<Value>>) -> Vec<Value> {
        heard.lock().unwrap().iter().filter(|event| event["kind"] == "thread").cloned().collect()
    }

    fn texts_from(m: &Machine, tab: &str, from: u64) -> Vec<String> {
        let page = serde_json::to_value(m.thread(tab, from).unwrap()).unwrap();
        page["items"].as_array().unwrap().iter().map(|item| item["text"].as_str().unwrap_or_default().to_owned()).collect()
    }

    fn ticked_by_hand(m: &Machine) {
        m.threads_watched.store(true, Ordering::Relaxed);
    }

    fn watching(name: &str) -> (PathBuf, Arc<Machine>, Arc<Mutex<Vec<Value>>>, TabId, PathBuf) {
        let root = temp(name);
        let m = Machine::new(launch(), Some(root.join("config")), None, |_| {});
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        let heard = Arc::new(Mutex::new(vec![]));
        let seen = heard.clone();
        m.subscribe(Box::new(move |event| seen.lock().unwrap().push(serde_json::to_value(event).unwrap())));
        let tab = m.add_tab(root.join("app"), m.accounts.get("work").unwrap(), vec![], vec![], false, Some("s1".into()));
        ticked_by_hand(&m);
        m.call(Call::WatchThreads { tabs: vec![tab.clone()] }).unwrap();
        let dir = root.join("config/accounts/work/projects/demo");
        std::fs::create_dir_all(&dir).unwrap();
        (root, m, heard, tab, dir)
    }

    #[test]
    fn a_grown_transcript_names_only_its_tab() {
        let (root, m, heard, tab, dir) = watching("thread-nudge");
        let file = dir.join("s1.jsonl");
        std::fs::write(&file, assistant_line("ready")).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        heard.lock().unwrap().clear();
        let todo = r#"{"type":"assistant","uuid":"a1","timestamp":"t1","message":{"content":[{"type":"tool_use","id":"w1","name":"TodoWrite","input":{"todos":[{"content":"Ship the fix","status":"in_progress"}]}}]}}"#;
        std::fs::write(&file, format!("{}{todo}\n", assistant_line("ready"))).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })]);
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard).len(), 1, "an unchanged file is quiet");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_newly_watched_tab_is_nudged_once() {
        let (root, m, heard, tab, dir) = watching("thread-first-look");
        std::fs::write(dir.join("s1.jsonl"), assistant_line("written before the watch")).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_new_session_on_a_watched_tab_nudges() {
        let (root, m, heard, tab, dir) = watching("thread-new-session");
        std::fs::write(dir.join("s1.jsonl"), assistant_line("ready")).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        heard.lock().unwrap().clear();
        m.tabs.lock().unwrap().get_mut(&tab).unwrap().claude_session = Some("s2".into());
        std::fs::write(dir.join("s2.jsonl"), assistant_line("fresh")).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })]);
        std::fs::write(dir.join("s2.jsonl"), format!("{}{}", assistant_line("fresh"), assistant_line("next"))).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard).len(), 2, "the new session's own file is watched from then on");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_line_finished_after_a_partial_first_look_nudges() {
        let (root, m, heard, tab, dir) = watching("thread-partial");
        let file = dir.join("s1.jsonl");
        let whole = format!("{}{}", assistant_line("ready"), assistant_line("visible"));
        let cut = assistant_line("ready").len() + 20;
        std::fs::write(&file, &whole[..cut]).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        heard.lock().unwrap().clear();
        std::fs::write(&file, &whole).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })]);
        let read = m.thread(&tab, assistant_line("ready").len() as u64).unwrap();
        assert_eq!(serde_json::to_value(&read).unwrap()["items"][0]["text"], "visible");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_moved_transcript_is_found_again() {
        let (root, m, heard, tab, dir) = watching("thread-move");
        let other = dir.parent().unwrap().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(dir.join("s1.jsonl"), assistant_line("ready")).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        heard.lock().unwrap().clear();
        std::fs::remove_file(dir.join("s1.jsonl")).unwrap();
        std::fs::write(other.join("s1.jsonl"), format!("{}{}", assistant_line("ready"), assistant_line("moved"))).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })]);
        assert_eq!(texts_from(&m, &tab, assistant_line("ready").len() as u64), vec!["moved"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unwatched_tab_is_not_nudged() {
        let (root, m, heard, watched, dir) = watching("thread-unwatched");
        let background = m.add_tab(root.join("app"), m.accounts.get("work").unwrap(), vec![], vec![], false, Some("s2".into()));
        std::fs::write(dir.join("s1.jsonl"), assistant_line("ready")).unwrap();
        std::fs::write(dir.join("s2.jsonl"), assistant_line("ready")).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        std::fs::write(dir.join("s1.jsonl"), format!("{}{}", assistant_line("ready"), assistant_line("visible"))).unwrap();
        std::fs::write(dir.join("s2.jsonl"), format!("{}{}", assistant_line("ready"), assistant_line("hidden"))).unwrap();
        m.nudge_thread_watchers(&mut looks);
        let nudge = json!({ "kind": "thread", "tab": watched });
        assert_eq!(thread_events(&heard), vec![nudge.clone(), nudge], "the first look and the growth of {watched}, nothing for {background}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_first_watch_starts_the_watcher_which_nudges_within_a_tick() {
        let root = temp("thread-loop");
        let heard = Arc::new(Mutex::new(vec![]));
        let seen = heard.clone();
        let m = Machine::new(launch(), Some(root.join("config")), None, move |event| seen.lock().unwrap().push(serde_json::to_value(event).unwrap()));
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        let tab = m.add_tab(root.join("app"), m.accounts.get("work").unwrap(), vec![], vec![], false, Some("s1".into()));
        let dir = root.join("config/accounts/work/projects/demo");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s1.jsonl");
        std::fs::write(&file, assistant_line("ready")).unwrap();
        m.call(Call::WatchThreads { tabs: vec![tab.clone()] }).unwrap();
        wait_for("the first look", || thread_events(&heard).len() == 1);
        let started = Instant::now();
        std::fs::write(&file, format!("{}{}", assistant_line("ready"), assistant_line("visible"))).unwrap();
        wait_for("the nudge", || thread_events(&heard).len() == 2);
        assert!(started.elapsed() < Duration::from_millis(500), "the nudge took {:?}, the poll is 1200 ms", started.elapsed());
        std::thread::sleep(Duration::from_millis(150));
        let nudge = json!({ "kind": "thread", "tab": tab });
        assert_eq!(thread_events(&heard), vec![nudge.clone(), nudge], "one look, one growth, then quiet");
        m.quit();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_shorter_transcript_nudges_and_reads_as_a_reset() {
        let (root, m, heard, tab, dir) = watching("thread-shrink");
        let file = dir.join("s1.jsonl");
        let long = format!("{}{}", assistant_line("ready"), assistant_line("gone"));
        std::fs::write(&file, &long).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        heard.lock().unwrap().clear();
        std::fs::write(&file, assistant_line("ready")).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })]);
        let page = serde_json::to_value(m.thread(&tab, long.len() as u64).unwrap()).unwrap();
        assert_eq!(page["reset"], true);
        assert_eq!(texts_from(&m, &tab, long.len() as u64), vec!["ready"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_transcript_missing_for_one_tick_nudges_when_it_returns_longer() {
        let (root, m, heard, tab, dir) = watching("thread-miss");
        let file = dir.join("s1.jsonl");
        std::fs::write(&file, assistant_line("ready")).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        heard.lock().unwrap().clear();
        std::fs::remove_file(&file).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), Vec::<Value>::new(), "a missing file is quiet");
        std::fs::write(&file, assistant_line("ready")).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), Vec::<Value>::new(), "the same file back is quiet");
        std::fs::write(&file, format!("{}{}", assistant_line("ready"), assistant_line("after"))).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })]);
        assert_eq!(texts_from(&m, &tab, assistant_line("ready").len() as u64), vec!["after"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_cached_transcript_is_not_scanned_again() {
        let (root, m, heard, tab, _) = watching("thread-cache");
        let projects = root.join("config/accounts/work/projects");
        for i in 0..300 {
            std::fs::create_dir_all(projects.join(format!("p{i:03}"))).unwrap();
        }
        let file = projects.join("p001/s1.jsonl");
        std::fs::write(&file, assistant_line("ready")).unwrap();
        let scans = || thread::PROJECT_SCANS.lock().unwrap().iter().filter(|path| path.as_path() == projects).count();
        let before = scans();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(scans(), before + 1, "the first look searches the project directories");
        std::fs::write(&file, format!("{}{}", assistant_line("ready"), assistant_line("visible"))).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(scans(), before + 1, "a file that is still there is not searched again");
        let nudge = json!({ "kind": "thread", "tab": tab });
        assert_eq!(thread_events(&heard), vec![nudge.clone(), nudge]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reads_of_a_tab_search_the_projects_once_until_its_transcript_moves() {
        let (root, m, _, tab, dir) = watching("thread-read-cache");
        let projects = root.join("config/accounts/work/projects");
        let scans = || thread::PROJECT_SCANS.lock().unwrap().iter().filter(|path| path.as_path() == projects).count();
        std::fs::write(dir.join("s1.jsonl"), assistant_line("ready")).unwrap();
        let before = scans();
        for _ in 0..5 {
            assert_eq!(texts_from(&m, &tab, 0), vec!["ready"]);
        }
        assert_eq!(scans(), before + 1, "five reads search once");
        let other = projects.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::rename(dir.join("s1.jsonl"), other.join("s1.jsonl")).unwrap();
        assert_eq!(texts_from(&m, &tab, 0), vec!["ready"], "a moved transcript is found again");
        assert_eq!(scans(), before + 2);
        std::fs::remove_file(other.join("s1.jsonl")).unwrap();
        assert_eq!(texts_from(&m, &tab, 0), Vec::<String>::new(), "a deleted transcript reads as none");
        std::fs::write(dir.join("s1.jsonl"), format!("{}{}", assistant_line("ready"), assistant_line("back"))).unwrap();
        assert_eq!(texts_from(&m, &tab, 0), vec!["ready", "back"], "a transcript written again is found again");
        std::fs::write(dir.join("s2.jsonl"), assistant_line("fresh")).unwrap();
        m.tab(&tab, |t| t.claude_session = Some("s2".into())).unwrap();
        assert_eq!(texts_from(&m, &tab, 0), vec!["fresh"], "a new session reads its own transcript");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_transcript_not_written_yet_is_searched_again_after_a_wait_or_a_new_status() {
        let (root, m, heard, tab, _) = watching("thread-unwritten");
        let projects = root.join("config/accounts/work/projects");
        for i in 0..300 {
            std::fs::create_dir_all(projects.join(format!("p{i:03}"))).unwrap();
        }
        let scans = || thread::PROJECT_SCANS.lock().unwrap().iter().filter(|path| path.as_path() == projects).count();
        let before = scans();
        let mut looks = HashMap::new();
        for _ in 0..10 {
            m.nudge_thread_watchers(&mut looks);
        }
        assert_eq!(scans(), before + 1, "ten ticks at the prompt search once");
        m.tab(&tab, |t| t.status = Some(AgentStatus::Working)).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(scans(), before + 2, "a turn that starts searches at once");
        std::fs::write(projects.join("p007/s1.jsonl"), assistant_line("first")).unwrap();
        std::thread::sleep(THREAD_LOOK_EVERY * 2);
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })], "the transcript written after the turn started nudges");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_transcript_not_found_is_searched_again_after_40_ms_then_twice_as_long_up_to_the_poll() {
        let (root, m, _, _, _) = watching("thread-backoff");
        let projects = root.join("config/accounts/work/projects");
        let account = m.accounts.get("work").unwrap();
        let scans = || thread::PROJECT_SCANS.lock().unwrap().iter().filter(|path| path.as_path() == projects).count();
        let searched = |look: &mut TranscriptLook, session: &str, status: Option<AgentStatus>, now: Instant| {
            let before = scans();
            m.locate_transcript(&account, session, status, look, now);
            scans() > before
        };
        let next_search = |look: &mut TranscriptLook, session: &str, status: Option<AgentStatus>, after: Instant| {
            (1..=3000).find(|ms| searched(look, session, status, after + Duration::from_millis(*ms))).unwrap()
        };
        let mut look = TranscriptLook::default();
        let mut at = Instant::now();
        assert!(searched(&mut look, "s1", None, at), "the first look searches");
        let mut gaps = vec![];
        for _ in 0..7 {
            let gap = next_search(&mut look, "s1", None, at);
            gaps.push(gap);
            at += Duration::from_millis(gap);
        }
        assert_eq!(gaps, [40, 80, 160, 320, 640, 1200, 1200]);
        at += Duration::from_millis(1);
        assert!(searched(&mut look, "s1", Some(AgentStatus::Working), at), "a new status searches at once");
        assert_eq!(next_search(&mut look, "s1", Some(AgentStatus::Working), at), 40, "and waits 40 ms again");
        at += Duration::from_millis(40 + 1);
        assert!(searched(&mut look, "s2", Some(AgentStatus::Working), at), "a new session searches at once");
        assert_eq!(next_search(&mut look, "s2", Some(AgentStatus::Working), at), 40, "and waits 40 ms again");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_dropped_hub_link_keeps_this_machines_own_watch() {
        let (root, m, heard, local, dir) = watching("thread-local-viewer");
        let remote = m.add_tab(root.join("app"), m.accounts.get("work").unwrap(), vec![], vec![], false, Some("s2".into()));
        m.set_watched_threads("client-7", std::slice::from_ref(&remote));
        std::fs::write(dir.join("s1.jsonl"), assistant_line("local")).unwrap();
        std::fs::write(dir.join("s2.jsonl"), assistant_line("remote")).unwrap();
        m.drop_remote_thread_watchers();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": local })]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_loop_skips_its_ticks_while_the_machine_quits_and_ticks_again_after_a_reopen() {
        let m = machine();
        let ticks = Arc::new(AtomicU32::new(0));
        let counted = ticks.clone();
        m.every(Duration::from_millis(1), move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        wait_for("a tick", || ticks.load(Ordering::SeqCst) > 0);
        m.quit();
        wait_for("the ticks to stop", || {
            let before = ticks.load(Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(10));
            ticks.load(Ordering::SeqCst) == before
        });
        let stopped = ticks.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(ticks.load(Ordering::SeqCst), stopped, "no tick while the machine quits");
        m.reopen_sessions();
        wait_for("a tick after the reopen", || ticks.load(Ordering::SeqCst) > stopped);
    }

    #[test]
    fn a_tick_with_nothing_watched_waits_for_no_tab() {
        let (root, m, _, tab, dir) = watching("thread-idle");
        std::fs::write(dir.join("s1.jsonl"), assistant_line("ready")).unwrap();
        m.set_watched_threads(LOCAL_VIEWER, &[]);
        let tick = |m: &Arc<Machine>| {
            let (m, (done, finished)) = (m.clone(), std::sync::mpsc::channel());
            std::thread::spawn(move || {
                m.nudge_thread_watchers(&mut HashMap::new());
                let _ = done.send(());
            });
            finished
        };
        let held = m.tabs.lock().unwrap();
        assert!(tick(&m).recv_timeout(Duration::from_secs(5)).is_ok(), "an empty watch ticks while another thread holds the tabs");
        m.set_watched_threads(LOCAL_VIEWER, std::slice::from_ref(&tab));
        let watched = tick(&m);
        assert!(watched.recv_timeout(Duration::from_millis(50)).is_err(), "a watched tab is read under the tabs lock");
        drop(held);
        assert!(watched.recv_timeout(Duration::from_secs(5)).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_switched_account_is_read_from_its_new_home() {
        let root = temp("thread-switch");
        let heard = Arc::new(Mutex::new(vec![]));
        let seen = heard.clone();
        let m = Machine::new(
            Launch { bins: fake_bins(&root.join("bin")), ..launch() },
            Some(root.join("config")),
            None,
            move |event| seen.lock().unwrap().push(serde_json::to_value(event).unwrap()),
        );
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Personal".into(), auth: None, key: None }).unwrap();
        std::fs::write(root.join("config/accounts/personal/signed-in"), "").unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();
        let tab = m.add_tab(root.join("app"), m.accounts.get("work").unwrap(), vec![], vec![], false, Some("s1".into()));
        ticked_by_hand(&m);
        m.call(Call::WatchThreads { tabs: vec![tab.clone()] }).unwrap();
        let from = root.join("config/accounts/work/projects/demo");
        std::fs::create_dir_all(&from).unwrap();
        let original = from.join("s1.jsonl");
        std::fs::write(&original, assistant_line("ready")).unwrap();
        let mut looks = HashMap::new();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(texts_from(&m, &tab, 0), vec!["ready"]);
        m.switch_account(&tab, "personal", 80, 24).unwrap();
        let copied = root.join("config/accounts/personal/projects/demo/s1.jsonl");
        assert!(original.is_file(), "the switch leaves the old transcript");
        assert!(copied.is_file(), "the switch copies the transcript");
        heard.lock().unwrap().clear();
        std::fs::write(&copied, format!("{}{}", assistant_line("ready"), assistant_line("moved"))).unwrap();
        m.nudge_thread_watchers(&mut looks);
        assert_eq!(thread_events(&heard), vec![json!({ "kind": "thread", "tab": tab })], "a line written in the new home nudges");
        assert_eq!(texts_from(&m, &tab, assistant_line("ready").len() as u64), vec!["moved"]);
        m.close_tab(&tab);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_tab_rejects_a_path_that_is_not_a_folder() {
        let m = machine();
        let err = m.open_tab(Some("/no/such/pinned-folder".into()), false).unwrap_err();
        assert_eq!(err, "/no/such/pinned-folder is not a folder on this machine");
        assert!(m.tabs().is_empty());
    }

    #[test]
    fn a_listener_hears_events_until_it_unsubscribes() {
        let m = machine();
        let heard = Arc::new(Mutex::new(vec![]));
        let seen = heard.clone();
        let id = m.subscribe(Box::new(move |e| seen.lock().unwrap().push(serde_json::to_value(e).unwrap())));
        let tab = m.open_tab(None, false).unwrap();
        m.unsubscribe(id);
        m.close_tab(&tab);
        assert_eq!(
            *heard.lock().unwrap(),
            [json!({ "kind": "tabs", "tabs": [{
                "id": tab, "cwd": "/repo", "account": "claude", "program": "agent", "bypass": false, "running": false, "status": null,
                "title": null, "message": null, "resume_at": null, "connector": null, "opened": 1,
            }] })]
        );
    }

    #[test]
    fn hook_events_set_the_tabs_status_title_and_question() {
        let m = machine();
        let tab = m.open_tab(None, false).unwrap();
        let event = |status, title: Option<&str>, message: Option<&str>| AgentEvent {
            status,
            cwd: None,
            message: message.map(Into::into),
            tab: Some(tab.clone()),
            session: None,
            title: title.map(Into::into),
            resume_at: None,
            prompt: None,
            snapshot: None,
            permission_mode: None,
            hook_feedback: false,
        };
        m.record_event(event(AgentStatus::NeedsInput, Some("Fix the build"), Some("Claude needs your permission to use Bash")));
        let view = m.view(&tab).unwrap();
        assert_eq!(
            (view.status, view.title.as_deref(), view.message.as_deref()),
            (Some(AgentStatus::NeedsInput), Some("Fix the build"), Some("Claude needs your permission to use Bash"))
        );
        m.record_event(event(AgentStatus::Working, None, Some("ignored while working")));
        let view = m.view(&tab).unwrap();
        assert_eq!((view.status, view.title.as_deref(), view.message), (Some(AgentStatus::Working), Some("Fix the build"), None));
    }

    #[test]
    fn a_branch_switch_waits_for_claude_to_finish_the_turn() {
        let dir = temp("checkout");
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        git(&dir, &["branch", "other"]);
        let m = machine();
        let tab = m.add_tab(dir.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let working = AgentEvent { status: AgentStatus::Working, cwd: None, message: None, tab: Some(tab.clone()), session: None, title: None, resume_at: None, prompt: None, snapshot: None, permission_mode: None, hook_feedback: false };
        m.record_event(working.clone());
        let checkout = Call::Checkout { tab: tab.clone(), branch: "other".into(), create: false, from: None };
        assert!(m.call(checkout.clone()).unwrap_err().contains("claude is working"));
        m.record_event(AgentEvent { status: AgentStatus::NeedsInput, ..working.clone() });
        assert!(m.call(checkout.clone()).unwrap_err().contains("claude is working"), "a permission prompt mid-turn let the branch switch");
        m.record_event(AgentEvent { status: AgentStatus::Done, ..working.clone() });
        m.record_event(AgentEvent { status: AgentStatus::NeedsInput, ..working });
        assert_eq!(m.call(checkout), Ok(Value::Null));
        assert_eq!(m.call(Call::Workspace { tab }).unwrap()["branch"], "other");
    }

    #[test]
    fn make_folder_makes_the_missing_parents_and_clone_into_a_folder_with_files_is_refused() {
        let m = machine();
        let dir = temp("make-folder");
        let made = dir.join("a").join("b");
        assert_eq!(m.call(Call::MakeFolder { path: made.display().to_string() }), Ok(json!(made)));
        assert!(made.is_dir());
        std::fs::write(made.join("file"), "x").unwrap();
        let refused = m.call(Call::Clone { repo: "https://example.invalid/o/r.git".into(), token: None, dest: Some(made.display().to_string()) });
        assert_eq!(refused, Err(format!("{} already holds files", made.display())));
    }

    #[test]
    fn calls_answer_with_the_json_a_client_reads() {
        let m = machine();
        let tab = m.call(Call::OpenTab { path: None, bypass: true }).unwrap();
        let id = tab.as_str().unwrap().to_owned();
        assert_eq!(m.view(&id).unwrap().opened, 1);
        assert_eq!(
            m.call(Call::Tabs),
            Ok(json!([{
                "id": id, "cwd": "/repo", "account": "claude", "program": "agent", "bypass": true, "running": false, "status": null,
                "title": null, "message": null, "resume_at": null, "connector": null, "opened": 1,
            }]))
        );
        assert_eq!(
            m.call(Call::SetFolder { tab: id.clone(), path: "/nonexistent/acme".into() }),
            Err("/nonexistent/acme is not a folder on this machine".into())
        );
        assert_eq!(m.call(Call::CloseTab { tab: id.clone() }), Ok(Value::Null));
        assert_eq!(m.call(Call::StartClaude { tab: id.clone(), cols: 80, rows: 24 }), Err(format!("no tab {id}")));
        assert_eq!(m.call(Call::Deliver { env: vec![], text: "hi".into() }), Ok(json!(false)));
    }

    #[cfg(unix)]
    #[test]
    fn skipping_permissions_restarts_the_tab_on_its_conversation_and_continues_the_turn() {
        let dir = temp("bypass");
        let claude = dir.join("claude");
        std::fs::write(&claude, "#!/bin/sh\nprintf '%s|' \"$@\"; echo; exec sleep 30\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(dir.clone()), Some(dir.clone()), |_| {});
        let tab = m.add_tab(dir.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, Some("s1".into()));
        m.spawn(&tab, sh("sleep 30"), pty::size(80, 24)).unwrap();
        m.tab(&tab, |t| t.interrupted = true).unwrap();
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();

        let set = |on| m.call(Call::SetBypass { tab: tab.clone(), on, cols: 80, rows: 24 });
        assert_eq!(set(true), Ok(json!(dir.display().to_string())));
        wait_for("claude's arguments", || text(&seen).ends_with("\r\n"));
        assert_eq!(text(&seen), "--settings|s|--plugin-dir|p|--dangerously-skip-permissions|--resume|s1|Continue from where you left off.|\r\n");
        assert!(m.view(&tab).unwrap().bypass);
        assert!(sessions::load(&dir.join("sessions.json"))[0].bypass, "a relaunch keeps skipping permissions");

        let session = m.tab(&tab, |t| t.screen.clone()).unwrap().lock().unwrap().session;
        assert_eq!(set(true), Ok(json!(dir.display().to_string())));
        assert_eq!(m.tab(&tab, |t| t.screen.clone()).unwrap().lock().unwrap().session, session, "a session already skipping permissions keeps running");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_model_picked_for_a_conversation_restarts_it_with_that_model_and_keeps_it() {
        let dir = temp("model");
        let claude = dir.join("claude");
        std::fs::write(&claude, "#!/bin/sh\nprintf '%s|' \"$@\"; echo; exec sleep 30\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(dir.clone()), Some(dir.clone()), |_| {});
        let tab = m.add_tab(dir.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, Some("s1".into()));
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();

        assert_eq!(m.call(Call::SetModel { tab: tab.clone(), model: "--evil".into(), cols: 80, rows: 24, account: None }), Err("\"--evil\" is not a model name".into()));
        assert_eq!(m.call(Call::SetModel { tab: tab.clone(), model: "claude-opus-4-8".into(), cols: 80, rows: 24, account: None }), Ok(json!(dir.display().to_string())));
        wait_for("claude's arguments", || text(&seen).ends_with("\r\n"));
        assert_eq!(text(&seen), "--settings|s|--plugin-dir|p|--resume|s1|--model|claude-opus-4-8|\r\n");
        assert_eq!(sessions::load(&dir.join("sessions.json"))[0].model.as_deref(), Some("claude-opus-4-8"), "a relaunch keeps the model");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn allowing_agents_to_drive_devices_tells_each_new_session_how() {
        let dir = temp("device-agents");
        let claude = dir.join("claude");
        std::fs::write(&claude, "#!/bin/sh\nprintf '%s|' \"$@\"; echo; exec sleep 30\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: [(Provider::Claude, claude)].into_iter().collect(), ..launch() }, Some(dir.clone()), Some(dir.clone()), |_| {});
        assert_eq!(m.call(Call::DeviceAgents { allow: None }), Ok(json!(false)));
        assert_eq!(m.call(Call::DeviceAgents { allow: Some(true) }), Ok(json!(true)));
        let tab = m.add_tab(dir.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        m.start_claude(&tab, 80, 24).unwrap();
        wait_for("claude's arguments", || text(&seen).ends_with("\r\n"));
        assert_eq!(text(&seen), format!("--settings|s|--plugin-dir|p|--append-system-prompt|{}|\r\n", crate::device::AGENT_PROMPT));
        assert_eq!(m.call(Call::DeviceAgents { allow: Some(false) }), Ok(json!(false)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_thread_without_a_project_runs_in_the_machines_scratch_folder() {
        let dir = temp("scratch");
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let m = Machine::new(launch(), Some(dir.clone()), Some(project.clone()), |_| {});
        let tab = m.call(Call::OpenScratch { bypass: false }).unwrap().as_str().unwrap().to_owned();
        assert_eq!(m.view(&tab).unwrap().cwd, dir.join("scratch").display().to_string());
        assert!(dir.join("scratch").is_dir());
        assert_eq!(m.call(Call::Workspace { tab }).unwrap()["scratch"], true);
        let again = m.open_tab(Some(dir.join("scratch").display().to_string()), false).unwrap();
        assert_eq!(m.call(Call::Workspace { tab: again }).unwrap()["scratch"], true);
        let other = m.open_tab(None, false).unwrap();
        assert_eq!(m.call(Call::Workspace { tab: other }).unwrap()["scratch"], false);
        assert_eq!(m.list_projects().iter().map(|p| p.path.clone()).collect::<Vec<_>>(), [project.display().to_string()], "the scratch folder joined the projects");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_session_held_at_its_usage_limit_stays_held_until_the_next_turn() {
        let m = machine();
        let tab = m.open_tab(None, false).unwrap();
        let failed = |resume_at| AgentEvent { status: AgentStatus::Failed, cwd: None, message: None, tab: Some(tab.clone()), session: None, title: None, resume_at, prompt: None, snapshot: None, permission_mode: None, hook_feedback: false };
        m.record_status(&tab, &failed(Some(1792400000)));
        m.call(Call::AutoResume { tab: tab.clone(), on: false }).unwrap();
        assert!(m.view(&tab).unwrap().held);
        m.record_status(&tab, &failed(Some(1792400000)));
        assert!(m.view(&tab).unwrap().held, "the same limit reported again let it go");
        m.record_status(&tab, &failed(Some(1792500000)));
        assert!(!m.view(&tab).unwrap().held, "a new limit kept the old hold");
        m.call(Call::AutoResume { tab: tab.clone(), on: false }).unwrap();
        m.record_status(&tab, &AgentEvent { status: AgentStatus::Working, ..failed(None) });
        let view = m.view(&tab).unwrap();
        assert_eq!((view.held, view.resume_at), (false, None));
    }

    #[test]
    fn cancel_task_from_the_owning_tab_cancels_and_another_tab_is_refused() {
        let m = machine();
        let account = Account::builtin(Provider::Claude);
        let tab = m.add_tab("/repo".into(), account.clone(), vec![], vec![], false, None);
        let other = m.add_tab("/repo".into(), account.clone(), vec![], vec![], false, None);
        let task = m.delegates.begin(crate::delegate::NewTask {
            parent: crate::delegate::Parent::Tab(tab.clone()),
            tool: None,
            title: "Look".into(),
            account: account.id.clone(),
            provider: Provider::Claude,
            model: None,
            cwd: "/repo".into(),
            mode: crate::delegate::Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: false,
            caller_key: format!("tab:{tab}"),
            client_request_id: None,
            role: None,
        });
        let body = m.call(Call::CancelTask { tab: tab.clone(), task: task.id.clone() }).unwrap();
        assert_eq!(body["workState"], "cancelled");
        assert_eq!(body["error"], json!("cancelled from the card"));
        let refused = m.call(Call::CancelTask { tab: other, task: task.id.clone() });
        assert_eq!(refused, Err(format!("task {} belongs to another session", task.id)));
    }

    #[test]
    #[cfg(unix)]
    fn a_parent_close_drops_a_kept_worktree_unless_an_opened_tab_owns_it() {
        let dir = temp("owned-worktree");
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let loose = branches::new_worktree(&repo, None).unwrap();
        let owned = branches::new_worktree(&repo, None).unwrap();
        let m = machine();
        let account = Account::builtin(Provider::Claude);
        let parent = m.add_tab(std::fs::canonicalize(&repo).unwrap(), account.clone(), vec![], vec![], false, None);
        let spec = |title: &str, cwd: PathBuf| crate::delegate::NewTask {
            parent: crate::delegate::Parent::Tab(parent.clone()),
            tool: None,
            title: title.into(),
            account: account.id.clone(),
            provider: Provider::Claude,
            model: None,
            cwd,
            mode: crate::delegate::Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: true,
            caller_key: format!("tab:{parent}"),
            client_request_id: None,
            role: None,
        };
        let loose_task = m.delegates.begin(spec("Loose", loose.clone()));
        let owned_task = m.delegates.begin(spec("Owned", owned.clone()));
        let init = |id: &str, session: &str| {
            m.delegates.apply(id, crate::delegate::ChildEvent::Line(json!({"type":"system","subtype":"init","session_id":session}).to_string()), None);
            m.delegates.apply(id, crate::delegate::ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        };
        init(&loose_task.id, "loose-1");
        init(&owned_task.id, "owned-1");
        let opened = m.open_delegated(&parent, &owned_task.id).unwrap();
        assert_eq!(m.tab(&opened, |t| t.made.clone()).unwrap(), Some(owned.clone()));
        assert_eq!(m.view(&opened).unwrap().spawned_by, Some(parent.clone()), "an opened task nests under the thread that delegated it");
        m.close_tab(&parent);
        assert!(!loose.exists(), "closing the parent left a worktree no tab owns");
        assert!(owned.is_dir(), "closing the parent removed a worktree an opened tab owns");
        m.close_tab(&opened);
        assert!(!owned.exists(), "closing the opened tab left its worktree");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn quit_drops_a_kept_worktree() {
        let dir = temp("quit-worktree");
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let kept = branches::new_worktree(&repo, None).unwrap();
        let m = machine();
        let account = Account::builtin(Provider::Claude);
        let parent = m.add_tab(std::fs::canonicalize(&repo).unwrap(), account.clone(), vec![], vec![], false, None);
        let task = m.delegates.begin(crate::delegate::NewTask {
            parent: crate::delegate::Parent::Tab(parent.clone()),
            tool: None,
            title: "Kept".into(),
            account: account.id.clone(),
            provider: Provider::Claude,
            model: None,
            cwd: kept.clone(),
            mode: crate::delegate::Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: true,
            caller_key: format!("tab:{parent}"),
            client_request_id: None,
            role: None,
        });
        m.delegates.apply(&task.id, crate::delegate::ChildEvent::Line(json!({"type":"system","subtype":"init","session_id":"keep-me"}).to_string()), None);
        m.delegates.apply(&task.id, crate::delegate::ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        assert!(kept.is_dir(), "the worktree was not created");
        m.quit();
        assert!(!kept.exists(), "quit left a worktree that still had a conversation");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn quit_keeps_a_worktree_an_opened_tab_owns() {
        let dir = temp("quit-owned-worktree");
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let loose = branches::new_worktree(&repo, None).unwrap();
        let owned = branches::new_worktree(&repo, None).unwrap();
        let m = machine();
        let account = Account::builtin(Provider::Claude);
        let parent = m.add_tab(std::fs::canonicalize(&repo).unwrap(), account.clone(), vec![], vec![], false, None);
        let spec = |title: &str, cwd: PathBuf| crate::delegate::NewTask {
            parent: crate::delegate::Parent::Tab(parent.clone()),
            tool: None,
            title: title.into(),
            account: account.id.clone(),
            provider: Provider::Claude,
            model: None,
            cwd,
            mode: crate::delegate::Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: true,
            caller_key: format!("tab:{parent}"),
            client_request_id: None,
            role: None,
        };
        let loose_task = m.delegates.begin(spec("Loose", loose.clone()));
        let owned_task = m.delegates.begin(spec("Owned", owned.clone()));
        let init = |id: &str, session: &str| {
            m.delegates.apply(id, crate::delegate::ChildEvent::Line(json!({"type":"system","subtype":"init","session_id":session}).to_string()), None);
            m.delegates.apply(id, crate::delegate::ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        };
        init(&loose_task.id, "loose-1");
        init(&owned_task.id, "owned-1");
        let opened = m.open_delegated(&parent, &owned_task.id).unwrap();
        assert_eq!(m.tab(&opened, |t| t.made.clone()).unwrap(), Some(owned.clone()));
        m.quit();
        assert!(!loose.exists(), "quit left a worktree no tab owns");
        assert!(owned.is_dir(), "quit removed a worktree an opened tab owns");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_new_thread_moves_to_a_new_worktree_and_back_dropping_it_unused() {
        let dir = temp("workspace");
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let claude = dir.join("claude");
        std::fs::write(&claude, "#!/bin/sh\npwd; exec sleep 30\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, None, None, |_| {});
        let tab = m.add_tab(std::fs::canonicalize(&repo).unwrap(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();

        let set = |worktree: Option<&str>| m.call(Call::SetWorkspace { tab: tab.clone(), worktree: worktree.map(Into::into), cols: 80, rows: 24 });
        let tree = PathBuf::from(set(Some("new")).unwrap().as_str().unwrap());
        assert!(tree.starts_with(std::fs::canonicalize(&repo).unwrap().join(".claude/worktrees")), "{}", tree.display());
        wait_for("claude in the worktree", || text(&seen).contains(".claude/worktrees/pinned-"));
        assert_eq!(m.call(Call::Workspace { tab: tab.clone() }).unwrap()["worktree"], true);
        assert!(set(Some("/elsewhere")).unwrap_err().contains("is not a worktree of"));
        assert_eq!(PathBuf::from(set(None).unwrap().as_str().unwrap()), std::fs::canonicalize(&repo).unwrap());
        assert!(!tree.exists(), "the unused worktree stayed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn restarting_claude_resumes_the_tabs_conversation() {
        let dir = temp("restart");
        let claude = dir.join("claude");
        std::fs::write(&claude, "#!/bin/sh\nprintf '%s|' \"$@\"; echo; exec sleep 30\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(dir.clone()), Some(dir.clone()), |_| {});
        let tab = m.add_tab(dir.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, Some("s1".into()));
        m.spawn(&tab, sh("sleep 30"), pty::size(80, 24)).unwrap();
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();

        assert_eq!(m.call(Call::RestartClaude { tab: tab.clone(), cols: 80, rows: 24 }), Ok(json!(dir.display().to_string())));
        wait_for("claude's arguments", || text(&seen).ends_with("\r\n"));
        assert_eq!(text(&seen), "--settings|s|--plugin-dir|p|--resume|s1|\r\n");
        assert!(!m.view(&tab).unwrap().bypass);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_run_opens_in_its_folder_with_this_machines_binary() {
        let dir = temp("run-folder");
        let m = Machine::new(Launch { bins: fake_bins(&dir.join(".bin")), ..launch() }, None, Some("/repo".into()), |_| {});
        let run = RunSpec {
            folder: dir.display().to_string(),
            repo: Some("git@github.com:acme/app.git".into()),
            args: vec!["--name".into(), "ENG-7".into(), "Run `'{{pinned}}' stage coding` first".into()],
            env: vec![("PINNED_CONNECTOR".into(), "c1".into())],
            token: None,
            account: None,
            budget_usd: None,
        };
        let id = m.open_run(&run).unwrap();
        let view = m.view(&id).unwrap();
        assert_eq!((view.cwd, view.connector.as_deref(), view.running), (dir.display().to_string(), Some("c1"), false));
        assert_eq!(m.tab(&id, |t| t.launch.clone()).unwrap(), ["--name", "ENG-7", "Run `'/opt/pinned/bin/pinned' stage coding` first"]);
        let elsewhere = RunSpec { folder: "/nonexistent/acme".into(), repo: None, ..run };
        assert_eq!(m.open_run(&elsewhere), Err("/nonexistent/acme does not exist on this machine".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_run_resuming_a_session_reads_its_transcript_from_the_start() {
        let dir = temp("run-resume");
        let m = machine();
        let run = RunSpec { folder: dir.display().to_string(), repo: None, args: vec!["--resume".into(), "s1".into()], env: vec![], token: None, account: None, budget_usd: None };
        let id = m.open_run(&run).unwrap();
        assert_eq!(m.tab(&id, |t| t.launch.windows(2).find(|w| w[0] == "--resume").map(|w| w[1].clone())).unwrap(), Some("s1".into()));
        assert_eq!(m.tab(&id, |t| t.claude_session.clone()).unwrap().as_deref(), Some("s1"));
        assert_eq!(m.thread(&id, 0).unwrap().session.as_deref(), Some("s1"));
        assert_eq!(m.view(&id).unwrap().session.as_deref(), Some("s1"), "the tab's view lacks the session clients key it by");
        let fresh = m.open_run(&RunSpec { args: vec![], ..run }).unwrap();
        assert_eq!(m.tab(&fresh, |t| t.claude_session.clone()).unwrap(), None);
        assert_eq!(m.view(&fresh).unwrap().session, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_resume_that_finds_no_conversation_is_refused_and_a_live_one_stays() {
        let dir = temp("resume-missing");
        let missing = dir.join("missing");
        std::fs::write(&missing, "#!/bin/sh\necho 'No conversation found with session ID' >&2\necho 'No conversation found with session ID'\nexit 1\n").unwrap();
        let staying = dir.join("staying");
        std::fs::write(&staying, "#!/bin/sh\necho started\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&missing, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&staying, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let run = RunSpec {
            folder: dir.display().to_string(),
            repo: None,
            args: vec!["--resume".into(), "missing".into()],
            env: vec![],
            token: None,
            account: None,
            budget_usd: None,
        };
        let refused = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, missing)]), ..launch() }, Some(dir.clone()), Some(dir.clone()), |_| {});
        let err = refused.run(&run).unwrap_err();
        assert!(err.contains("No conversation found"), "{err}");
        assert!(refused.tabs().is_empty(), "the refused resume left a tab");
        let staying_machine = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, staying)]), ..launch() }, Some(dir.clone()), Some(dir.clone()), |_| {});
        let id = staying_machine.run(&run).unwrap();
        assert!(staying_machine.view(&id).unwrap().running);
        staying_machine.close_tab(&id);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_resume_is_recognized_when_the_tui_places_each_word() {
        let screen = "No\u{1b}[12Gconversation\u{1b}[25Gfound with session ID";
        assert!(Machine::resume_refused(screen));
        assert!(!Machine::resume_refused("Resumed the conversation and found the bug"));
    }

    #[test]
    #[cfg(unix)]
    fn a_resume_that_paints_then_exits_is_still_refused() {
        let dir = temp("resume-late");
        let missing = dir.join("missing");
        std::fs::write(
            &missing,
            "#!/bin/sh\necho started\nsleep 0.6\nprintf 'No\\033[12Gconversation\\033[25Gfound with session ID\\n'\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&missing, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let run = RunSpec {
            folder: dir.display().to_string(),
            repo: None,
            args: vec!["--resume".into(), "missing".into()],
            env: vec![],
            token: None,
            account: None,
            budget_usd: None,
        };
        let refused = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, missing)]), ..launch() }, Some(dir.clone()), Some(dir.clone()), |_| {});
        let err = refused.run(&run).unwrap_err();
        assert!(err.contains("No conversation found"), "{err}");
        assert!(refused.tabs().is_empty(), "the refused resume left a tab");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stop_hook_leaves_tab_done_with_claude_session_on_tabs_event() {
        let env = vec![(CONNECTOR_ENV.to_owned(), "c1".into())];
        let tabs_seen = Arc::new(Mutex::new(vec![]));
        let seen = tabs_seen.clone();
        let m = Machine::new(launch(), None, Some("/repo".into()), move |event| {
            if let MachineEvent::Tabs { tabs } = event {
                seen.lock().unwrap().extend(tabs);
            }
        });
        let tab = m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], env, false, None);
        let working = AgentEvent {
            status: AgentStatus::Working,
            cwd: Some("/repo".into()),
            message: None,
            tab: Some(tab.clone()),
            session: Some("claude-s1".into()),
            title: None,
            resume_at: None,
            prompt: Some("fix the bug".into()),
            snapshot: None,
            permission_mode: None,
            hook_feedback: false,
        };
        m.record_event(working.clone());
        m.record_event(AgentEvent { status: AgentStatus::Done, prompt: None, snapshot: None, ..working });
        let view = m.view(&tab).unwrap();
        assert_eq!(view.status, Some(AgentStatus::Done));
        assert_eq!(view.session.as_deref(), Some("claude-s1"));
        assert_eq!(view.connector.as_deref(), Some("c1"));
        let last = tabs_seen.lock().unwrap().last().unwrap().clone();
        assert_eq!(last.id, tab);
        assert_eq!(last.status, Some(AgentStatus::Done));
        assert_eq!(last.session.as_deref(), Some("claude-s1"));
    }

    #[test]
    fn claude_does_not_inherit_the_session_pinned_was_started_from() {
        std::env::set_var("CLAUDE_CODE_SESSION_ID", "outer-session");
        std::env::set_var("CLAUDECODE", "1");
        let cmd = launch().command(Program::Agent, &Account::builtin(Provider::Claude), "tab-1", Path::new("/repo"), false, vec![], &[], &[]);
        assert_eq!(cmd.get_env("CLAUDE_CODE_SESSION_ID"), None);
        assert_eq!(cmd.get_env("CLAUDECODE"), None);
        assert_eq!(cmd.get_env("PINNED_TAB").and_then(|v| v.to_str()), Some("tab-1"));
    }

    #[test]
    fn each_shared_plugin_folder_is_its_own_plugin_dir() {
        let dir = temp("plugin-argv");
        let plugin = dir.join("toy@demo");
        std::fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        std::fs::write(plugin.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#).unwrap();
        let cmd = launch().command(Program::Agent, &Account::builtin(Provider::Claude), "tab-1", Path::new("/repo"), false, vec![], &[], std::slice::from_ref(&plugin));
        let args: Vec<String> = cmd.get_argv().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let flags: Vec<_> = args.windows(2).filter(|w| w[0] == "--plugin-dir").map(|w| w[1].as_str()).collect();
        assert_eq!(flags, vec!["p", plugin.to_str().unwrap()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_run_that_cannot_start_leaves_no_tab() {
        let dir = temp("run-unstartable");
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, "/nonexistent/claude".into())]), ..launch() }, None, Some("/repo".into()), |_| {});
        let run = RunSpec { folder: dir.display().to_string(), repo: None, args: vec![], env: vec![], token: None, account: None, budget_usd: None };
        assert!(m.run(&run).is_err());
        assert_eq!(m.tabs(), vec![]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Fake `claude` and `codex` that answer their status command like the real ones, signed in on
    /// the default home and on a home holding a `signed-in` file, and otherwise print what they
    /// were started with.
    #[cfg(unix)]
    fn fake_bins(dir: &Path) -> BTreeMap<Provider, PathBuf> {
        std::fs::create_dir_all(dir).unwrap();
        let script = |name: &str, status: &str, home: &str, signed_in: &str, signed_out: &str| {
            let path = dir.join(name);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\nif [ \"$1 $2\" = \"{status}\" ]; then\n  [ -n \"${home}\" ] && [ ! -e \"${home}/signed-in\" ] && {{ echo '{signed_out}'; exit 1; }}\n  echo '{signed_in}'; exit 0\nfi\n\
                     if [ \"$1 $2\" = \"auth logout\" ]; then exit 0; fi\n\
                     printf '%s|%s|%s|%s|%s\\n' \"$(basename \"$0\")\" \"$*\" \"$CLAUDE_CONFIG_DIR\" \"$CODEX_HOME\" \"$PINNED_ACCOUNT\"\nsleep 30\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
            path
        };
        BTreeMap::from([
            (Provider::Claude, script("claude", "auth status", "CLAUDE_CONFIG_DIR", r#"{"loggedIn":true,"email":"ana@acme.dev","orgName":"Acme","subscriptionType":"max"}"#, r#"{"loggedIn":false}"#)),
            (Provider::Codex, script("codex", "login status", "CODEX_HOME", "Logged in using ChatGPT", "Not logged in")),
            (Provider::Cursor, script("cursor-agent", "status ", "CURSOR_UNUSED", "Logged in as ana@acme.dev", "Not logged in")),
        ])
    }

    #[cfg(unix)]
    fn accounts_machine(root: &Path) -> Arc<Machine> {
        Machine::new(Launch { bins: fake_bins(&root.join("bin")), ..launch() }, Some(root.join("config")), None, |_| {})
    }

    /// Starts the tab and returns the first line its session printed.
    #[cfg(unix)]
    fn first_line(m: &Machine, tab: &str) -> String {
        let (seen, sink) = viewer();
        m.attach(tab, "test".into(), sink).unwrap();
        m.start_claude(tab, 200, 24).unwrap();
        wait_for("the first line", || text(&seen).contains('\n'));
        text(&seen)
    }

    #[test]
    #[cfg(unix)]
    fn a_tab_runs_its_projects_account_in_that_accounts_home() {
        let root = temp("account-tabs");
        let m = accounts_machine(&root);
        let (work_app, side_app, own_app) = (root.join("work-app"), root.join("side-app"), root.join("own-app"));
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }).unwrap();
        for (app, account) in [(&work_app, Some("work")), (&side_app, Some("side")), (&own_app, None)] {
            std::fs::create_dir_all(app).unwrap();
            m.open_project(app.clone());
            m.call(Call::SetProjectAccount { path: app.display().to_string(), account: account.map(Into::into) }).unwrap();
        }
        let open = |app: &Path, bypass| m.open_tab(Some(app.display().to_string()), bypass).unwrap();
        let config = root.join("config").display().to_string();
        assert_eq!(first_line(&m, &open(&work_app, false)), format!("claude|--settings s --plugin-dir p|{config}/accounts/work||work\r\n"));
        assert_eq!(
            first_line(&m, &open(&side_app, true)),
            format!("codex|-c notify=[\"/opt/pinned/bin/pinned\", \"hook\", \"done\"] --dangerously-bypass-approvals-and-sandbox||{config}/accounts/side|side\r\n")
        );
        assert_eq!(first_line(&m, &open(&own_app, true)), "claude|--settings s --plugin-dir p --dangerously-skip-permissions|||claude\r\n");
        assert_eq!(m.tabs().into_iter().map(|t| t.account).collect::<Vec<_>>(), ["work", "side", "claude"]);

        let moved = m.open_tab(Some(own_app.display().to_string()), false).unwrap();
        m.set_folder(&moved, &work_app.display().to_string()).unwrap();
        assert_eq!(m.view(&moved).unwrap().account, "work", "a tab moved to another project takes that project's account");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Writes a Go account and its key without validation, so a launch test stays offline.
    #[cfg(unix)]
    fn write_go_account(root: &Path) {
        let config = root.join("config");
        std::fs::create_dir_all(config.join("accounts/go")).unwrap();
        std::fs::write(config.join("accounts.json"), r#"[{"id":"go","provider":"claude","name":"Go","auth":"opencode-go"}]"#).unwrap();
        std::fs::write(config.join("accounts/go/opencode-go.key"), "oc_sk_test").unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn a_go_account_launches_claude_with_the_go_gateway_and_strips_a_parent_login() {
        let root = temp("go-launch");
        write_go_account(&root);
        let claude = root.join("claude");
        std::fs::write(
            &claude,
            "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"$ANTHROPIC_BASE_URL\" \"$ANTHROPIC_AUTH_TOKEN\" \"$PINNED_OPENCODE_GO_KEY\" \"$CLAUDE_CONFIG_DIR\"\nsleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(root.join("config")), None, |_| {});
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        m.open_project(app.clone());
        m.call(Call::SetProjectAccount { path: app.display().to_string(), account: Some("go".into()) }).unwrap();
        let tab = m.open_tab(Some(app.display().to_string()), false).unwrap();
        let printed = first_line(&m, &tab);
        let config = root.join("config").display().to_string();
        assert_eq!(
            printed,
            format!("https://opencode.ai/inference/go/anthropic|oc_sk_test|oc_sk_test|{config}/accounts/go\r\n")
        );
        assert!(crate::opencode_go::STRIP_ENV.contains(&"ANTHROPIC_API_KEY"));
        assert!(crate::opencode_go::STRIP_ENV.contains(&"CLAUDE_CODE_USE_BEDROCK"));
        assert!(INHERITED_SESSION_ENV.contains(&"PINNED_OPENCODE_GO_KEY"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_malformed_go_home_refuses_preparation() {
        let root = temp("go-home-bad");
        write_go_account(&root);
        let m = accounts_machine(&root);
        std::fs::write(root.join("config/accounts/go/.claude.json"), "not json").unwrap();
        let account = m.accounts.get("go").unwrap();
        assert!(m.prepare_go_home(&account).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn moving_between_a_login_and_a_go_account_drops_the_other_models_id() {
        let root = temp("go-switch-model");
        write_go_account(&root);
        let m = accounts_machine(&root);
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        m.open_project(app.clone());
        let tab = m.add_tab(app.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        m.tab(&tab, |t| t.model = Some("claude-opus-5-5".into())).unwrap();
        m.call(Call::SwitchAccount { tab: tab.clone(), account: "go".into(), cols: 80, rows: 24 }).unwrap();
        assert_eq!(m.tab(&tab, |t| (t.account.id.clone(), t.model.clone())).unwrap(), ("go".into(), None));
        m.tab(&tab, |t| t.model = Some("kimi-k3".into())).unwrap();
        m.call(Call::SwitchAccount { tab: tab.clone(), account: "claude".into(), cols: 80, rows: 24 }).unwrap();
        assert_eq!(m.tab(&tab, |t| (t.account.id.clone(), t.model.clone())).unwrap(), ("claude".into(), None));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_model_from_another_account_moves_the_same_tab_and_sets_both() {
        let root = temp("go-set-model-account");
        write_go_account(&root);
        let m = accounts_machine(&root);
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        m.open_project(app.clone());
        let tab = m.add_tab(app, Account::builtin(Provider::Claude), vec![], vec![], false, None);
        m.tab(&tab, |t| t.model = Some("claude-opus-5-5".into())).unwrap();
        m.call(Call::SetModel { tab: tab.clone(), model: "kimi-k3".into(), cols: 80, rows: 24, account: Some("go".into()) }).unwrap();
        assert_eq!(m.tab(&tab, |t| (t.account.id.clone(), t.model.clone())).unwrap(), ("go".into(), Some("kimi-k3".into())));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_headless_claude_runs_under_the_tabs_claude_account() {
        let root = temp("account-headless");
        let claude = root.join("claude");
        std::fs::write(&claude, "#!/bin/sh\necho \"{\\\"title\\\":\\\"as $PINNED_ACCOUNT\\\"}\"\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(root.join("config")), None, |_| {});
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }).unwrap();
        let title = |account: &str| {
            let tab = m.add_tab(root.clone(), m.accounts.get(account).unwrap(), vec![], vec![], false, None);
            m.call(Call::RegenerateTitle { tab, previous: String::new(), thread: "USER: hi".into() })
        };
        assert_eq!(title("work"), Ok(json!("as work")));
        assert_eq!(title("side"), Ok(json!("as claude")), "a Codex tab asks the built-in Claude Code login");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn account_calls_answer_with_the_json_a_client_reads() {
        let root = temp("account-calls");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let m = accounts_machine(&root);
        m.open_project(app.clone());
        let work = json!({
            "id": "work", "provider": "claude", "name": "Work", "auth": "login", "provider_name": "Claude Code", "builtin": false,
            "signed_in": false, "email": null, "org": null, "plan": null,
        });
        // Claude Code logins carry the picker's models; the rest of each account reads as before.
        let without_models = |mut value: Value| {
            let first = |account: &mut Value| account.as_object_mut().unwrap().remove("models").map(|models| models[0]["slug"].clone());
            let firsts = match value.as_array_mut() {
                Some(list) => list.iter_mut().map(first).collect(),
                None => vec![first(&mut value)],
            };
            (value, firsts)
        };
        let (added, firsts) = without_models(m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap());
        assert_eq!((added, firsts), (work.clone(), vec![Some(json!("claude-fable-5-2"))]));
        assert_eq!(m.call(Call::AddAccount { provider: Provider::Codex, name: "work".into(), auth: None, key: None }), Err("an account work already exists".into()));
        let (listed, firsts) = without_models(m.call(Call::ListAccounts).unwrap());
        let fable = Some(json!("claude-fable-5-2"));
        assert_eq!(firsts, [fable.clone(), None, None, fable]);
        assert_eq!(
            Ok::<_, String>(listed),
            Ok(json!([
                {
                    "id": "claude", "provider": "claude", "name": "Claude Code", "auth": "login", "provider_name": "Claude Code", "builtin": true,
                    "signed_in": true, "email": "ana@acme.dev", "org": "Acme", "plan": "max",
                },
                {
                    "id": "codex", "provider": "codex", "name": "Codex", "auth": "login", "provider_name": "Codex", "builtin": true,
                    "signed_in": true, "email": null, "org": null, "plan": null,
                },
                {
                    "id": "cursor", "provider": "cursor", "name": "Cursor", "auth": "login", "provider_name": "Cursor", "builtin": true,
                    "signed_in": true, "email": "ana@acme.dev", "org": null, "plan": null,
                },
                work,
            ]))
        );
        let path = app.display().to_string();
        assert_eq!(m.call(Call::SetProjectAccount { path: path.clone(), account: Some("work".into()) }), Ok(json!({ "path": path, "name": "app", "account": "work" })));
        assert_eq!(m.call(Call::SetProjectAccount { path: path.clone(), account: Some("home".into()) }), Err("no account home on this machine".into()));
        assert_eq!(
            m.call(Call::SetProjectAccount { path: "/nonexistent/app".into(), account: None }),
            Err("/nonexistent/app is not a project on this machine".into())
        );
        assert_eq!(m.call(Call::RemoveAccount { account: "claude".into() }), Err("claude is built in and can't be removed".into()));
        assert_eq!(m.call(Call::RemoveAccount { account: "work".into() }), Ok(Value::Null));
        assert_eq!(m.call(Call::ListProjects), Ok(json!([{ "path": path, "name": "app", "account": "claude" }])), "a project whose account is gone runs under the default one");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_run_starts_only_under_a_claude_account_signed_in_here_and_in_its_home() {
        let root = temp("run-account");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let m = accounts_machine(&root);
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }).unwrap();
        m.open_project(app.clone());
        let run = |account: Option<&str>| {
            let spec = RunSpec { folder: app.display().to_string(), repo: None, args: vec!["Fix ENG-7".into()], env: vec![], token: None, account: account.map(Into::into), budget_usd: None };
            m.call(Call::Run { run: spec })
        };
        assert_eq!(run(Some("work")), Err("account work is not signed in on this machine".into()));
        assert_eq!(run(Some("side")), Err("account side runs Codex, which can't run a connector's session".into()));
        assert_eq!(run(Some("home")), Err("no account home on this machine".into()));
        assert_eq!(m.tabs(), vec![], "a refused run leaves no tab");

        let config = root.join("config").display().to_string();
        std::fs::write(format!("{config}/accounts/work/signed-in"), "").unwrap();
        let tab = run(Some("work")).unwrap();
        let (seen, sink) = viewer();
        m.attach(tab.as_str().unwrap(), "test".into(), sink).unwrap();
        wait_for("the session", || text(&seen).contains('\n'));
        assert_eq!(text(&seen), format!("claude|--settings s --plugin-dir p Fix ENG-7|{config}/accounts/work||work\r\n"));

        m.call(Call::SetProjectAccount { path: app.display().to_string(), account: Some("work".into()) }).unwrap();
        let tab = run(None).unwrap();
        assert_eq!(m.view(tab.as_str().unwrap()).unwrap().account, "work", "a connector that names no account runs under its project's");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn usage_is_kept_for_each_account_its_sessions_report() {
        use std::io::{BufRead, Write};
        let root = temp("usage");
        let events = Arc::new(Mutex::new(vec![]));
        let seen = events.clone();
        let m = Machine::new(Launch { sock: root.join("sock"), ..launch() }, None, None, move |e| seen.lock().unwrap().push(serde_json::to_value(e).unwrap()));
        m.listen(|_| Err("not for this test".into())).unwrap();
        let report = |line: &str| {
            let mut sock = std::os::unix::net::UnixStream::connect(root.join("sock")).unwrap();
            writeln!(sock, "{line}").unwrap();
            std::io::BufReader::new(sock).read_line(&mut String::new()).unwrap();
        };
        report(r#"{"kind":"usage","five_hour":{"used_percentage":23.5,"resets_at":4102444800}}"#);
        report(r#"{"kind":"usage","account":"work","seven_day":{"used_percentage":90.0,"resets_at":4102444800}}"#);
        report(r#"{"kind":"usage","account":"work","five_hour":{"used_percentage":5.0,"resets_at":4102444800}}"#);
        let window = |used: f64| json!({ "used_percentage": used, "resets_at": 4102444800u64 });
        assert_eq!(
            serde_json::to_value(m.usage()).unwrap(),
            json!({ "claude": { "five_hour": window(23.5) }, "work": { "five_hour": window(5.0), "seven_day": window(90.0) } })
        );
        assert_eq!(
            events.lock().unwrap().last(),
            Some(&json!({ "kind": "usage", "usage": { "five_hour": window(5.0), "seven_day": window(90.0) }, "account": "work" }))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_project_syncs_the_claude_files_of_its_account() {
        let root = temp("sync-account");
        let m = accounts_machine(&root);
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }).unwrap();
        for (name, account) in [("own", None), ("work", Some("work")), ("side", Some("side"))] {
            let app = root.join(name);
            std::fs::create_dir_all(&app).unwrap();
            git(&app, &["init", "-q"]);
            git(&app, &["remote", "add", "origin", &format!("git@github.com:acme/{name}.git")]);
            m.open_project(app.clone());
            m.set_project_account(&app.display().to_string(), account.map(Into::into)).unwrap();
        }
        let homes: Vec<(String, Option<PathBuf>)> = m.checkouts().into_iter().map(|c| (c.project.name, c.claude_home)).collect();
        assert_eq!(
            homes.into_iter().collect::<BTreeMap<_, _>>(),
            BTreeMap::from([
                ("own".into(), Some(home().join(".claude"))),
                ("side".into(), None),
                ("work".into(), Some(root.join("config/accounts/work"))),
            ])
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn signing_in_runs_the_providers_login_in_the_accounts_home_and_is_never_resumed() {
        let root = temp("sign-in");
        let m = accounts_machine(&root);
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }).unwrap();
        let config = root.join("config").display().to_string();
        let tab = m.call(Call::SignIn { account: "work".into() }).unwrap();
        let id = tab.as_str().unwrap().to_owned();
        let (seen, sink) = viewer();
        m.attach(&id, "test".into(), sink).unwrap();
        wait_for("the login", || text(&seen).contains('\n'));
        assert_eq!(text(&seen), format!("claude|auth login|{config}/accounts/work||work\r\n"));
        let view = m.view(&id).unwrap();
        assert_eq!((view.cwd, view.account.as_str(), view.program, view.running), (format!("{config}/accounts/work"), "work", Program::SignIn, true));
        m.record_session(&id, "s1".into(), None);
        assert_eq!(sessions::load(&root.join("config/sessions.json")), vec![], "a login is not a session to resume");

        let side = m.sign_in("side").unwrap();
        let (seen, sink) = viewer();
        m.attach(&side, "test".into(), sink).unwrap();
        wait_for("the login", || text(&seen).contains('\n'));
        assert_eq!(text(&seen), format!("codex|login||{config}/accounts/side|side\r\n"));
        assert_eq!(m.call(Call::SignIn { account: "home".into() }), Err("no account home on this machine".into()));
        assert_eq!(m.tabs().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn files_from_v0_7_0_load_and_run_under_the_default_account() {
        let root = temp("v070");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/projects.json"), format!(r#"[{{"path": "{}", "last_opened": 1760000000}}]"#, app.display())).unwrap();
        std::fs::write(
            root.join("config/sessions.json"),
            format!(r#"[{{"cwd": "{}", "session": "s1", "env": [], "bypass": false, "interrupted": false, "resume_at": null}}]"#, app.display()),
        )
        .unwrap();
        let m = accounts_machine(&root);
        assert_eq!(m.list_projects().into_iter().map(|p| p.account).collect::<Vec<_>>(), ["claude"]);
        let tabs = m.restore_tabs();
        assert_eq!(m.view(&tabs[0]).unwrap().account, "claude");
        assert_eq!(first_line(&m, &tabs[0]), "claude|--settings s --plugin-dir p --resume s1|||claude\r\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_codex_session_reopens_with_resume_not_claudes_flag() {
        let root = temp("codex-resume");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let m = accounts_machine(&root);
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }).unwrap();
        std::fs::write(
            root.join("config/sessions.json"),
            format!(r#"[{{"cwd": "{}", "session": "thr-9", "env": [], "bypass": false, "interrupted": false, "resume_at": null, "account": "side"}}]"#, app.display()),
        )
        .unwrap();
        let tabs = m.restore_tabs();
        assert_eq!(tabs.len(), 1);
        let line = first_line(&m, &tabs[0]);
        assert!(line.contains(" resume thr-9"), "codex was not resumed with `resume`: {line}");
        assert!(!line.contains("--resume"), "codex was passed claude's --resume: {line}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_codex_tab_keeps_the_notify_command_from_its_codex_home() {
        let root = temp("codex-user-notify");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let m = accounts_machine(&root);
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }).unwrap();
        let script = root.join("user-notify");
        let home = root.join("config/accounts/side");
        std::fs::write(
            home.join("config.toml"),
            format!("notify = [\n  \"{}\",\n  \"--ping\",\n]\n[tui]\nnotify = [\"nope\"]\n", script.display()),
        )
        .unwrap();
        m.open_project(app.clone());
        m.call(Call::SetProjectAccount { path: app.display().to_string(), account: Some("side".into()) }).unwrap();
        let tab = m.open_tab(Some(app.display().to_string()), false).unwrap();
        let line = first_line(&m, &tab);
        assert!(line.contains(&script.display().to_string()), "the account's notify was dropped: {line}");
        assert!(line.contains("--ping"), "the notify argument was dropped: {line}");
        assert!(line.contains("\"hook\", \"done\", \"--\""), "pinned's hook does not run first: {line}");
        assert!(!line.contains("nope"), "a notify inside a table was used: {line}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_limited_session_moves_to_another_signed_in_account_and_continues_there() {
        let root = temp("switch-account");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let m = accounts_machine(&root);
        for (provider, name) in [(Provider::Claude, "Work"), (Provider::Claude, "Spare"), (Provider::Codex, "Side")] {
            m.call(Call::AddAccount { provider, name: name.into(), auth: None, key: None }).unwrap();
        }
        m.open_project(app.clone());
        m.call(Call::SetProjectAccount { path: app.display().to_string(), account: Some("work".into()) }).unwrap();
        let config = root.join("config").display().to_string();
        let work = PathBuf::from(format!("{config}/accounts/work/projects/-app"));
        std::fs::create_dir_all(work.join("s1/subagents")).unwrap();
        std::fs::write(work.join("s1.jsonl"), "{}\n").unwrap();
        std::fs::write(work.join("s1/subagents/agent-a.jsonl"), "{}\n").unwrap();
        let tab = m.open_tab(Some(app.display().to_string()), false).unwrap();
        m.tab(&tab, |t| (t.claude_session, t.interrupted, t.resume_at) = (Some("s1".into()), true, Some(unix_now() + 3600))).unwrap();
        let switch = |account: &str| m.call(Call::SwitchAccount { tab: tab.clone(), account: account.into(), cols: 80, rows: 24 });

        assert_eq!(switch("side"), Err("account side runs Codex, which can't resume a Claude Code conversation".into()));
        assert_eq!(switch("home"), Err("no account home on this machine".into()));
        assert_eq!(switch("spare"), Err("account spare is not signed in on this machine".into()));
        m.tab(&tab, |t| t.status = Some(AgentStatus::Working)).unwrap();
        std::fs::write(format!("{config}/accounts/spare/signed-in"), "").unwrap();
        assert_eq!(switch("spare"), Err("claude is working in this tab. Switch accounts when the turn ends.".into()));
        m.tab(&tab, |t| t.status = Some(AgentStatus::Failed)).unwrap();

        assert_eq!(switch("spare"), Ok(json!(app.display().to_string())));
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("the resumed session", || text(&seen).contains('\n'));
        assert_eq!(text(&seen), format!("claude|--settings s --plugin-dir p --resume s1 Continue from where you left off.|{config}/accounts/spare||spare\r\n"));
        let view = m.view(&tab).unwrap();
        assert_eq!((view.account.as_str(), view.resume_at), ("spare", None));
        let spare = PathBuf::from(format!("{config}/accounts/spare/projects/-app"));
        assert!(spare.join("s1.jsonl").is_file() && spare.join("s1/subagents/agent-a.jsonl").is_file(), "the transcript and its subagents follow the conversation");
        let kept: Value = serde_json::from_slice(&std::fs::read(root.join("config/sessions.json")).unwrap()).unwrap();
        assert_eq!(kept[0]["account"], json!("spare"), "a restart resumes it under the new account");
        assert_eq!(
            m.call(Call::RemoveAccount { account: "spare".into() }),
            Err("close the tabs that run under spare before removing it".into()),
            "its saved session would not resume after a restart"
        );
        m.close_tab(&tab);
        assert_eq!(m.call(Call::RemoveAccount { account: "spare".into() }), Ok(Value::Null));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_session_resumes_in_the_account_it_ran_under() {
        let root = temp("resume-account");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/accounts.json"), r#"[{"id": "work", "provider": "claude", "name": "Work"}]"#).unwrap();
        let saved = |session: &str, account: &str| format!(r#"{{"cwd": "{}", "session": "{session}", "env": [], "account": "{account}"}}"#, app.display());
        std::fs::write(root.join("config/sessions.json"), format!("[{}, {}]", saved("s1", "work"), saved("s2", "gone"))).unwrap();
        let m = accounts_machine(&root);
        let tabs = m.restore_tabs();
        assert_eq!(tabs.len(), 1, "a session whose account is gone can't resume anywhere");
        let config = root.join("config").display().to_string();
        assert_eq!(first_line(&m, &tabs[0]), format!("claude|--settings s --plugin-dir p --resume s1|{config}/accounts/work||work\r\n"));
        let kept: Value = serde_json::from_slice(&std::fs::read(root.join("config/sessions.json")).unwrap()).unwrap();
        assert_eq!((&kept[0]["session"], &kept[0]["account"]), (&json!("s1"), &json!("work")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ssh_and_https_urls_of_one_repository_match() {
        let ssh = "git@github.com:FelipeGazapina/pinned.git";
        assert_eq!(repo_key(ssh).as_deref(), Some("github.com/felipegazapina/pinned"));
        for same in ["https://github.com/FelipeGazapina/pinned", "https://github.com/FelipeGazapina/pinned.git/", "ssh://git@github.com:22/FelipeGazapina/pinned.git", "https://token@github.com/felipegazapina/pinned.git"] {
            assert!(same_repo(ssh, same), "{same}");
        }
        assert!(!same_repo(ssh, "git@github.com:FelipeGazapina/other.git"));
        assert!(!same_repo(ssh, "git@gitlab.com:FelipeGazapina/pinned.git"));
        assert_eq!(repo_key("pinned"), None);
        assert_eq!(https_url(ssh).as_deref(), Some("https://github.com/FelipeGazapina/pinned.git"));
    }

    #[test]
    #[cfg(unix)]
    fn a_run_without_its_folder_takes_the_project_cloned_from_the_same_origin() {
        let root = temp("run-origin");
        let (ours, other) = (root.join("app-checkout"), root.join("site"));
        for (dir, origin) in [(&ours, "git@github.com:acme/app.git"), (&other, "git@github.com:acme/site.git")] {
            std::fs::create_dir_all(dir).unwrap();
            git(dir, &["init", "-q"]);
            git(dir, &["remote", "add", "origin", origin]);
        }
        let m = Machine::new(Launch { bins: fake_bins(&root.join("bin")), ..launch() }, None, Some("/repo".into()), |_| {});
        m.open_project(other);
        m.open_project(ours.clone());
        let run = RunSpec { folder: "/Users/someone-else/app".into(), repo: Some("git@github.com:acme/app.git".into()), args: vec![], env: vec![], token: None, account: None, budget_usd: None };
        let id = m.open_run(&run).unwrap();
        assert_eq!(m.view(&id).unwrap().cwd, ours.display().to_string());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_repository_clones_once_and_later_clones_find_the_first() {
        let root = temp("clone");
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        git(&source, &["init", "-q"]);
        git(&source, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "first"]);
        let into = root.join("clones/source");
        assert_eq!(clone(&source.display().to_string(), &into, None), Ok(()));
        assert!(into.join(".git").is_dir());
        assert!(clone(&source.display().to_string(), &into, None).is_err(), "git refuses to clone over a folder");
        assert!(clone(&root.join("missing").display().to_string(), &root.join("clones/missing"), None).unwrap_err().contains("git clone"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_clone_is_named_like_git_names_it() {
        assert_eq!(repo_name("git@github.com:acme/app.git"), Some("app"));
        assert_eq!(repo_name("https://github.com/acme/app/"), Some("app"));
        assert_eq!(repo_name("git@host:app.git"), Some("app"));
        assert_eq!(repo_name("https://github.com/acme/.."), None);
    }

    #[test]
    #[cfg(unix)]
    fn a_cursor_child_of_a_claude_tab_does_not_inherit_the_claude_model() {
        let root = temp("cross-model");
        let argv = root.join("argv");
        let cursor = root.join("cursor-agent");
        let body = r#"#!/bin/sh
if [ "$1" = status ]; then echo 'Logged in as ana@acme.dev'; exit 0; fi
if [ "$1" = "--list-models" ]; then sleep 0.3; printf '%s\n' 'composer-2.5-high - Composer High'; exit 0; fi
printf '%s\n' "$@" > "ARGV"
echo '{"type":"result","result":"ok"}'
"#
        .replace("ARGV", &argv.display().to_string());
        std::fs::write(&cursor, body).unwrap();
        std::fs::set_permissions(&cursor, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Cursor, cursor)]), ..launch() }, None, Some(root.clone()), |_| {});
        let tab = m.add_tab(root.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        m.tab(&tab, |t| t.model = Some("claude-opus-5-5".into())).unwrap();
        let plain = crate::delegate::parse_request(&json!({ "task": "look", "target": { "providerInstanceId": "cursor" }, "mode": "async" })).unwrap();
        m.start_delegate(Some(tab.clone()), None, None, plain).unwrap();
        wait_for("plain argv", || std::fs::read(&argv).is_ok_and(|b| !b.is_empty()));
        let lines: Vec<String> = String::from_utf8(std::fs::read(&argv).unwrap()).unwrap().lines().map(str::to_owned).collect();
        assert_eq!(lines, vec!["-p", "--output-format", "stream-json", "--trust", "look"]);
        let _ = std::fs::remove_file(&argv);
        let mapped = crate::delegate::parse_request(&json!({
            "task": "look",
            "target": { "providerInstanceId": "cursor", "model": "composer-2.5", "options": { "effort": "high" } },
            "mode": "async",
        }))
        .unwrap();
        m.start_delegate(Some(tab), None, None, mapped).unwrap();
        wait_for("mapped argv", || std::fs::read(&argv).is_ok_and(|b| String::from_utf8_lossy(&b).contains("composer-2.5-high")));
        let lines: Vec<String> = String::from_utf8(std::fs::read(&argv).unwrap()).unwrap().lines().map(str::to_owned).collect();
        assert_eq!(lines, vec!["-p", "--output-format", "stream-json", "--trust", "--model", "composer-2.5-high", "look"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_delegated_task_keeps_the_budget_it_asked_for() {
        let root = temp("budget");
        let cursor = root.join("cursor-agent");
        let body = "#!/bin/sh\nif [ \"$1\" = status ]; then echo 'Logged in as ana@acme.dev'; exit 0; fi\necho '{\"type\":\"result\",\"result\":\"ok\"}'\n";
        std::fs::write(&cursor, body).unwrap();
        std::fs::set_permissions(&cursor, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Cursor, cursor)]), ..launch() }, None, Some(root.clone()), |_| {});
        let tab = m.add_tab(root.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let request = crate::delegate::parse_request(&json!({ "task": "look", "target": { "providerInstanceId": "cursor" }, "budgetMs": 90000, "limitMs": 600000 })).unwrap();
        let started: Value = serde_json::from_str(&m.start_delegate(Some(tab.clone()), None, None, request).unwrap()).unwrap();
        let id = started["taskId"].as_str().unwrap().to_owned();
        let status: Value = serde_json::from_str(&m.task_status(Some(tab), None, &id).unwrap()).unwrap();
        assert_eq!((status["budgetSecs"].as_u64(), status["limitSecs"].as_u64()), (Some(90), Some(600)), "{status}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_worker_may_start_a_delegated_task() {
        let root = temp("worker-delegate");
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, root.join("missing-claude"))]), ..launch() }, None, None, |_| {});
        let tab = m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], role_env(Role::Worker, Path::new("/repo")), false, None);
        let request = crate::delegate::parse_request(&json!({ "task": "look" })).unwrap();
        assert_eq!(m.start_delegate(Some(tab), None, None, request).unwrap_err(), "Claude Code is not installed on this machine", "the worker got past every check up to the missing binary");
        assert!(m.delegates.running().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_delegated_child_cannot_start_another_pinned_worker() {
        let m = machine();
        let request = crate::delegate::parse_request(&json!({ "task": "look" })).unwrap();
        assert_eq!(m.start_delegate(None, Some("task-1".into()), None, request).unwrap_err(), "a delegated child cannot start another pinned worker");
        assert!(m.delegates.running().is_empty());
    }

    #[test]
    fn a_task_caller_the_registry_does_not_know_is_refused() {
        let m = Machine::new(launch(), None, None, |_| {});
        let request = crate::delegate::parse_request(&json!({ "task": "look" })).unwrap();
        assert_eq!(m.start_delegate(None, Some("task-gone".into()), None, request), Err("a delegated child cannot start another pinned worker".into()));
        assert!(m.delegates.running().is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn a_parent_tab_that_closes_during_startup_does_not_start_the_child() {
        let root = temp("parent-gone");
        let argv = root.join("argv");
        let listing = root.join("listing");
        let cursor = root.join("cursor-agent");
        let body = r#"#!/bin/sh
if [ "$1" = status ]; then echo 'Logged in as ana@acme.dev'; exit 0; fi
if [ "$1" = "--list-models" ]; then touch "LISTING"; sleep 0.4; printf '%s\n' 'composer-2.5 - Composer'; exit 0; fi
printf '%s\n' "$@" > "ARGV"
echo '{"type":"result","result":"ok"}'
"#
        .replace("LISTING", &listing.display().to_string())
        .replace("ARGV", &argv.display().to_string());
        std::fs::write(&cursor, body).unwrap();
        std::fs::set_permissions(&cursor, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Cursor, cursor)]), ..launch() }, None, Some(root.clone()), |_| {});
        let tab = m.add_tab(root.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        m.close_tab(&tab);
        let gone = crate::delegate::parse_request(&json!({ "task": "look", "target": { "providerInstanceId": "cursor" }, "mode": "async" })).unwrap();
        let err = m.start_delegate(Some(tab), None, None, gone).unwrap_err();
        assert!(err.contains("no tab"), "{err}");

        let tab = m.add_tab(root.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let mapped = crate::delegate::parse_request(&json!({
            "task": "look",
            "target": { "providerInstanceId": "cursor", "model": "composer-2.5" },
            "mode": "async",
        }))
        .unwrap();
        let started = m.clone();
        let parent = tab.clone();
        let handle = std::thread::spawn(move || started.start_delegate(Some(parent), None, None, mapped));
        wait_for("the model list", || listing.is_file());
        assert!(m.close_tab(&tab));
        let err = handle.join().unwrap().unwrap_err();
        assert!(err.contains("no tab"), "{err}");
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!argv.exists(), "the child started after its parent tab closed");
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    fn blocking_git(dir: &Path, started: &Path, go: &Path) -> String {
        let real = String::from_utf8(crate::process::command("/bin/sh").args(["-c", "command -v git"]).output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned();
        let bin = dir.join("git-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let body = format!(
            "#!/bin/sh\nexport PATH=\"/usr/bin:/bin:/opt/homebrew/bin\"\nif [ \"$1\" = worktree ] && [ \"$2\" = add ]; then\n  /usr/bin/touch \"{}\"\n  while [ ! -f \"{}\" ]; do /bin/sleep 0.02; done\nfi\nexec \"{}\" \"$@\"\n",
            started.display(),
            go.display(),
            real
        );
        std::fs::write(bin.join("git"), body).unwrap();
        std::fs::set_permissions(bin.join("git"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        bin.display().to_string()
    }

    #[cfg(unix)]
    fn worktree_left_during_setup(quit: bool) {
        let root = temp(if quit { "wt-quit" } else { "wt-cancel" });
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let started = root.join("adding");
        let go = root.join("go");
        let argv = root.join("argv");
        let claude = root.join("claude");
        let body = r#"#!/bin/sh
if [ "$1" = auth ] && [ "$2" = status ]; then echo '{"loggedIn":true,"email":"ana@acme.dev"}'; exit 0; fi
printf '%s\n' "$@" > "ARGV"
echo '{"type":"result","result":"ok"}'
"#
        .replace("ARGV", &argv.display().to_string());
        std::fs::write(&claude, body).unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let path = blocking_git(&root, &started, &go);
        let m = Machine::new(
            Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), path_env: Some(path), ..launch() },
            None,
            Some(repo.clone()),
            |_| {},
        );
        let tab = m.add_tab(repo.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let request = crate::delegate::parse_request(&json!({ "task": "look", "isolation": "worktree", "mode": "async" })).unwrap();
        let worker = m.clone();
        let parent = tab.clone();
        let handle = std::thread::spawn(move || worker.start_delegate(Some(parent), None, None, request));
        let began = Instant::now();
        while !started.is_file() {
            if handle.is_finished() {
                panic!("startup ended before worktree add: {:?}", handle.join().unwrap());
            }
            assert!(began.elapsed() < Duration::from_secs(15), "timed out waiting for worktree add");
            std::thread::sleep(Duration::from_millis(20));
        }
        if quit {
            m.quit();
        } else {
            let id = m.delegates.running().into_iter().next().expect("the task was claimed");
            m.delegates
                .cancel(&crate::delegate::Caller { tab: Some(tab), task: None }, &id, "stop", m.launch().path_env.as_deref())
                .unwrap();
        }
        std::fs::write(&go, "").unwrap();
        let body = handle.join().unwrap().expect("startup returned an error");
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["workState"], "cancelled");
        let trees = repo.join(".claude/worktrees");
        let left = std::fs::read_dir(&trees).map(|entries| entries.count()).unwrap_or(0);
        assert_eq!(left, 0, "a worktree was left behind");
        assert!(!argv.is_file(), "the child started after startup was abandoned");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_cancel_during_worktree_setup_drops_the_directory_and_cancels_the_task() {
        worktree_left_during_setup(false);
    }

    #[test]
    #[cfg(unix)]
    fn a_quit_during_worktree_setup_drops_the_directory_and_cancels_the_task() {
        worktree_left_during_setup(true);
    }

    #[test]
    #[cfg(unix)]
    fn a_plan_parent_hands_its_child_read_only_even_when_bypass_is_on() {
        let root = temp("plan-child");
        let argv = root.join("argv");
        let claude = root.join("claude");
        let body = r#"#!/bin/sh
if [ "$1" = auth ] && [ "$2" = status ]; then echo '{"loggedIn":true,"email":"ana@acme.dev"}'; exit 0; fi
printf '%s\n' "$@" > "ARGV"
echo '{"type":"result","result":"ok"}'
"#
        .replace("ARGV", &argv.display().to_string());
        std::fs::write(&claude, body).unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, None, Some(root.clone()), |_| {});
        let tab = m.add_tab(root.clone(), Account::builtin(Provider::Claude), vec![], vec![], true, None);
        m.record_event(AgentEvent { permission_mode: Some("plan".into()), ..done_event(&tab) });
        assert_eq!(argv_of(&m, &tab, &argv), ["--permission-mode", "plan"]);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The access flags the child thread a `delegate_task` from `tab` starts with.
    #[cfg(unix)]
    fn argv_of(m: &Machine, tab: &str, argv: &Path) -> Vec<String> {
        let _ = std::fs::remove_file(argv);
        let request = crate::delegate::parse_request(&json!({ "task": "look", "mode": "async" })).unwrap();
        m.start_delegate(Some(tab.to_owned()), None, None, request).unwrap();
        wait_for("child argv", || std::fs::read(argv).is_ok_and(|bytes| !bytes.is_empty()));
        let lines: Vec<String> = String::from_utf8(std::fs::read(argv).unwrap()).unwrap().lines().map(str::to_owned).collect();
        let mode = lines.windows(2).find(|w| w[0] == "--permission-mode").map(|w| w.to_vec()).unwrap_or_default();
        lines.iter().filter(|arg| *arg == "--dangerously-skip-permissions").cloned().chain(mode).collect()
    }

    #[cfg(unix)]
    fn claude_recording_argv(root: &Path) -> PathBuf {
        let argv = root.join("argv");
        let claude = root.join("claude");
        let body = r#"#!/bin/sh
if [ "$1" = auth ] && [ "$2" = status ]; then echo '{"loggedIn":true,"email":"ana@acme.dev"}'; exit 0; fi
printf '%s\n' "$@" > "ARGV"
echo '{"type":"result","result":"ok"}'
"#
        .replace("ARGV", &argv.display().to_string());
        std::fs::write(&claude, body).unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        claude
    }

    #[test]
    #[cfg(unix)]
    fn shift_tab_makes_the_child_read_only_until_a_hook_reports_the_mode() {
        let root = temp("shift-tab-mode");
        let claude = claude_recording_argv(&root);
        let argv = root.join("argv");
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, None, Some(root.clone()), |_| {});
        let tab = m.add_tab(root.clone(), Account::builtin(Provider::Claude), vec![], vec![], true, None);
        let full = ["--dangerously-skip-permissions"].map(str::to_owned).to_vec();
        let plan = ["--permission-mode", "plan"].map(str::to_owned).to_vec();
        m.record_event(AgentEvent { permission_mode: Some("default".into()), ..done_event(&tab) });
        assert_eq!(argv_of(&m, &tab, &argv), full);
        m.write_input(&tab, b"\x1b[Z").unwrap();
        assert_eq!(argv_of(&m, &tab, &argv), plan);
        m.record_event(AgentEvent { permission_mode: Some("default".into()), ..done_event(&tab) });
        assert_eq!(argv_of(&m, &tab, &argv), full);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_restored_plan_session_starts_its_child_read_only() {
        let root = temp("restored-plan");
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        let claude = claude_recording_argv(&root);
        let argv = root.join("argv");
        sessions::save(
            &root.join("sessions.json"),
            &[sessions::Saved {
                cwd: repo.clone(),
                session: "s1".into(),
                env: vec![],
                bypass: true,
                interrupted: false,
                resume_at: None,
                held: false,
                account: "claude".into(),
                model: None,
                permission_mode: Some("plan".into()),
                spawned_by: None,
            }],
        )
        .unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(root.clone()), Some(repo), |_| {});
        let tab = m.restore_tabs().pop().expect("the plan session restored");
        assert_eq!(argv_of(&m, &tab, &argv), ["--permission-mode", "plan"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn an_opened_thread_takes_its_access_from_the_task() {
        let root = temp("open-access");
        let argv = root.join("argv");
        let body = r#"#!/bin/sh
printf '%s\n' "$@" > "ARGV"
exec sleep 30
"#
        .replace("ARGV", &argv.display().to_string());
        let claude = root.join("claude");
        let codex = root.join("codex");
        for bin in [&claude, &codex] {
            std::fs::write(bin, &body).unwrap();
            std::fs::set_permissions(bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        let m = Machine::new(
            Launch { bins: BTreeMap::from([(Provider::Claude, claude), (Provider::Codex, codex)]), ..launch() },
            None,
            Some(root.clone()),
            |_| {},
        );
        let open = |provider: Provider, parent_bypass: bool, access: crate::delegate::Access, session: &str| {
            let account = Account::builtin(provider);
            let parent = m.add_tab(root.clone(), account.clone(), vec![], vec![], parent_bypass, None);
            let task = m.delegates.begin(crate::delegate::NewTask {
                parent: crate::delegate::Parent::Tab(parent.clone()),
                tool: None,
                title: "Opened".into(),
                account: account.id,
                provider,
                model: None,
                cwd: root.clone(),
                mode: crate::delegate::Mode::Async,
                access,
                isolated: false,
                caller_key: format!("tab:{parent}"),
                client_request_id: None,
                role: None,
            });
            let started = match provider {
                Provider::Claude | Provider::Cursor => json!({"type":"system","subtype":"init","session_id":session}).to_string(),
                Provider::Codex => json!({"type":"thread.started","thread_id":session}).to_string(),
            };
            m.delegates.apply(&task.id, crate::delegate::ChildEvent::Line(started), None);
            if provider == Provider::Claude {
                m.delegates.apply(&task.id, crate::delegate::ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
            } else {
                m.delegates.apply(&task.id, crate::delegate::ChildEvent::Exited { code: Some(0), stderr: String::new() }, None);
            }
            let opened = m.open_delegated(&parent, &task.id).unwrap();
            let _ = std::fs::remove_file(&argv);
            m.start_claude(&opened, 80, 24).unwrap();
            wait_for("opened argv", || std::fs::read(&argv).is_ok_and(|bytes| !bytes.is_empty()));
            let lines: Vec<String> = String::from_utf8(std::fs::read(&argv).unwrap()).unwrap().lines().map(str::to_owned).collect();
            let mode = m.tab(&opened, |t| (t.bypass, t.permission_mode.clone())).unwrap();
            (lines, mode)
        };
        let pair = |args: &[String], flag: &str, value: &str| args.windows(2).any(|window| window[0] == flag && window[1] == value);

        let (lines, (bypass, mode)) = open(Provider::Claude, true, crate::delegate::Access::ReadOnly, "plan-session");
        assert!(!bypass, "a read-only thread copied the parent's bypass");
        assert_eq!(mode.as_deref(), Some("plan"));
        assert!(pair(&lines, "--resume", "plan-session"), "{lines:?}");
        assert!(pair(&lines, "--permission-mode", "plan"), "{lines:?}");
        assert!(!lines.iter().any(|arg| arg == "--dangerously-skip-permissions"), "{lines:?}");

        let (lines, (bypass, mode)) = open(Provider::Claude, false, crate::delegate::Access::Full, "full-session");
        assert!(bypass);
        assert_eq!(mode, None);
        assert!(lines.iter().any(|arg| arg == "--dangerously-skip-permissions"), "{lines:?}");
        assert!(!lines.iter().any(|arg| arg == "--permission-mode"), "{lines:?}");

        let (lines, (bypass, mode)) = open(Provider::Claude, true, crate::delegate::Access::Auto, "auto-session");
        assert!(!bypass, "an auto thread copied the parent's bypass");
        assert_eq!(mode, None);
        assert!(pair(&lines, "--resume", "auto-session"), "{lines:?}");
        assert!(!lines.iter().any(|arg| arg == "--dangerously-skip-permissions"), "{lines:?}");
        assert!(!lines.iter().any(|arg| arg == "--permission-mode"), "{lines:?}");

        let (lines, (bypass, mode)) = open(Provider::Codex, true, crate::delegate::Access::ReadOnly, "codex-session");
        assert!(!bypass, "a Codex thread copied the parent's bypass");
        assert_eq!(mode, None);
        assert!(pair(&lines, "resume", "codex-session"), "{lines:?}");
        assert!(!lines.iter().any(|arg| arg == "--permission-mode" || arg == "--dangerously-bypass-approvals-and-sandbox"), "{lines:?}");
        m.quit();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delegate_calls_without_a_tab_or_task_are_refused() {
        let m = machine();
        let request = crate::delegate::parse_request(&json!({ "task": "look" })).unwrap();
        for err in [
            m.start_delegate(None, None, None, request).unwrap_err(),
            m.task_status(None, None, "task-1").unwrap_err(),
            m.task_cancel(Some(String::new()), Some(String::new()), "task-1", "stop").unwrap_err(),
        ] {
            assert!(err.contains("PINNED_TAB") && err.contains("PINNED_TASK"), "{err}");
        }
        assert!(m.capabilities(None, None, "").is_ok());
    }

    #[test]
    fn a_model_claudes_menu_adds_reaches_the_picker_and_delegate_task_and_outlives_a_restart() {
        let dir = std::env::temp_dir().join(format!("pinned-model-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let menu = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/web-sweep/screens/claude-2.1.321-model-menu.txt")).unwrap();
        let lines: Vec<String> = menu.replace("Fable 5.2", "Fable 5.3").lines().map(String::from).collect();
        let picker = |m: &Machine| -> Vec<String> {
            let accounts = m.call(Call::ListAccounts).unwrap();
            let claude = accounts.as_array().unwrap().iter().find(|a| a["id"] == "claude").unwrap().clone();
            claude["models"].as_array().unwrap().iter().take(5).map(|model| format!("{} {}", model["name"].as_str().unwrap(), model["legacy"])).collect()
        };
        let catalog = |m: &Machine| -> Vec<String> {
            let value: Value = serde_json::from_str(&m.capabilities(None, None, "fable").unwrap()).unwrap();
            value["providers"][0]["models"].as_array().unwrap().iter().map(|model| model["id"].as_str().unwrap().to_owned()).collect()
        };

        let m = Machine::new(launch(), Some(dir.clone()), Some("/repo".into()), |_| {});
        assert_eq!(picker(&m)[0], "Claude Fable 5.2 false");
        assert_eq!(m.call(Call::LearnModelMenu { lines: vec!["$ ls".into()] }).unwrap_err(), "claude's /model menu is not on that screen");
        let learned = m.call(Call::LearnModelMenu { lines }).unwrap();
        assert_eq!(learned[0]["slug"], "claude-fable-5-3");
        let expected = ["Claude Fable 5.3 false", "Claude Opus 5.5 false", "Claude Sonnet 5.5 false", "Claude Haiku 5.5 false", "Claude Fable 5.2 true"];
        assert_eq!(picker(&m), expected);
        assert_eq!(catalog(&m), ["claude-fable-5-3", "claude-fable-5-2", "claude-fable-5-1", "claude-fable-5"]);
        drop(m);

        let again = Machine::new(launch(), Some(dir.clone()), Some("/repo".into()), |_| {});
        assert_eq!(picker(&again), expected, "a restarted machine keeps what its claude showed");
        assert_eq!(catalog(&again)[0], "claude-fable-5-3");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn capabilities_for_a_task_use_that_task_not_builtin_claude() {
        use crate::delegate::{Access, Mode, NewTask, Parent};
        let m = machine();
        let task = m.delegates.begin(NewTask {
            parent: Parent::Tab("tab-9".into()),
            tool: None,
            title: "Look".into(),
            account: "codex".into(),
            provider: Provider::Codex,
            model: Some("gpt-5.4".into()),
            cwd: std::env::temp_dir(),
            mode: Mode::Async,
            access: Access::Full,
            isolated: false,
            caller_key: "tab:tab-9".into(),
            client_request_id: None,
            role: None,
        });
        let value: serde_json::Value = serde_json::from_str(&m.capabilities(None, Some(&task.id), "").unwrap()).unwrap();
        assert_eq!(value["inheritedProviderInstanceId"], "codex");
        assert_eq!(value["inheritedModel"], "gpt-5.4");
        assert_eq!(value["runtimeMode"], "full-access");
        let missing: serde_json::Value = serde_json::from_str(&m.capabilities(None, None, "").unwrap()).unwrap();
        assert_eq!(missing["inheritedProviderInstanceId"], "claude");
        assert_eq!(missing["runtimeMode"], "auto");
    }

    #[cfg(unix)]
    fn finish_task(m: &Machine, parent: crate::delegate::Parent, title: &str) -> String {
        use crate::delegate::{ChildEvent, Mode, NewTask};
        let task = m.delegates.begin(NewTask {
            parent,
            tool: None,
            title: title.into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: Some("composer-2.5".into()),
            cwd: std::env::temp_dir(),
            mode: Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: false,
            caller_key: "tab:notice".into(),
            client_request_id: None,
            role: None,
        });
        m.delegates.apply(&task.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
        task.id
    }

    #[cfg(unix)]
    fn pty_file(name: &str) -> (Arc<Machine>, String, PathBuf) {
        let file = std::env::temp_dir().join(format!("pinned-{name}-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let m = machine();
        let tab = running(&m, vec![], &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        (m, tab, file)
    }

    #[cfg(unix)]
    fn read_pty(file: &Path) -> String {
        String::from_utf8_lossy(&std::fs::read(file).unwrap_or_default()).into_owned()
    }

    #[cfg(unix)]
    fn reload_machine(name: &str) -> (Arc<Machine>, PathBuf, Arc<Mutex<Vec<MachineEvent>>>) {
        let root = temp(name);
        let events = Arc::new(Mutex::new(vec![]));
        let heard = events.clone();
        let m = Machine::new(launch(), Some(root.join("config")), Some(root.clone()), move |e| heard.lock().unwrap().push(e));
        (m, root, events)
    }

    #[cfg(unix)]
    fn typing_tab(m: &Machine, account: Account, file: &Path) -> TabId {
        let tab = m.add_tab("/repo".into(), account, vec![], vec![], false, None);
        m.spawn(&tab, sh(&format!("stty raw -echo; echo ready; exec cat > '{}'", file.display())), pty::size(80, 24)).unwrap();
        let (seen, sink) = viewer();
        m.attach(&tab, format!("viewer-{tab}"), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        tab
    }

    fn personal_skills(m: &Machine, account: &Account) -> skills::Place {
        skills::Place::Writable(skills::Writable::ClaudePersonal { home: m.accounts.home(account) })
    }

    fn skills_changed_events(events: &Mutex<Vec<MachineEvent>>) -> Vec<Vec<skills::Scope>> {
        events.lock().unwrap().iter().filter_map(|e| if let MachineEvent::SkillsChanged { scopes } = e { Some(scopes.clone()) } else { None }).collect()
    }

    #[test]
    #[cfg(unix)]
    fn an_idle_claude_tab_reloads_its_skills_at_once_and_stays_idle() {
        let (m, root, events) = reload_machine("reload-idle");
        let claude = Account::builtin(Provider::Claude);
        let tab = typing_tab(&m, claude.clone(), &root.join("typed.bin"));
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        let nudged = m.skills_changed(&[personal_skills(&m, &claude)]);
        assert_eq!(nudged, [skills::Nudged { tab: tab.clone(), title: None, nudge: skills::Delivery::Reloaded }]);
        wait_for("the reload command", || read_pty(&root.join("typed.bin")).contains("/reload-skills"));
        assert_eq!(m.tab(&tab, |t| (t.owes_reload, t.turn_pending.is_none(), t.notice_idle())).unwrap(), (false, true, true));
        assert_eq!(skills_changed_events(&events), [vec![skills::Scope::Personal]]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_busy_tab_reloads_right_after_its_turn_ends() {
        let (m, root, _) = reload_machine("reload-busy");
        let claude = Account::builtin(Provider::Claude);
        let file = root.join("typed.bin");
        let tab = typing_tab(&m, claude.clone(), &file);
        m.record_event(AgentEvent { status: AgentStatus::Working, ..done_event(&tab) });
        let nudged = m.skills_changed(&[personal_skills(&m, &claude)]);
        assert_eq!(nudged[0].nudge, skills::Delivery::AfterTurn);
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!read_pty(&file).contains("/reload-skills"), "the reload interrupted a turn: {}", read_pty(&file));
        m.record_event(done_event(&tab));
        wait_for("the reload after Done", || read_pty(&file).contains("/reload-skills"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_draft_holds_the_reload_until_it_is_cleared() {
        let (m, root, _) = reload_machine("reload-draft");
        let claude = Account::builtin(Provider::Claude);
        let file = root.join("typed.bin");
        let tab = typing_tab(&m, claude.clone(), &file);
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        m.write_input(&tab, b"xyz").unwrap();
        assert_eq!(m.skills_changed(&[personal_skills(&m, &claude)])[0].nudge, skills::Delivery::AfterTurn);
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!read_pty(&file).contains("/reload-skills"), "the reload was typed over the draft: {}", read_pty(&file));
        m.write_input(&tab, b"\x7f\x7f\x7f").unwrap();
        wait_for("the reload once the draft is gone", || read_pty(&file).contains("/reload-skills"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn codex_and_cursor_tabs_see_a_change_at_their_next_start() {
        let (m, root, _) = reload_machine("reload-next-start");
        let codex = typing_tab(&m, Account::builtin(Provider::Codex), &root.join("codex.bin"));
        let cursor = typing_tab(&m, Account::builtin(Provider::Cursor), &root.join("cursor.bin"));
        for tab in [&codex, &cursor] {
            m.tab(tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        }
        let nudged = m.skills_changed(&[skills::Place::Writable(skills::Writable::Agents)]);
        let got: Vec<(TabId, skills::Delivery)> = nudged.into_iter().map(|n| (n.tab, n.nudge)).collect();
        assert_eq!(got, [(codex.clone(), skills::Delivery::NextStart), (cursor.clone(), skills::Delivery::NextStart)]);
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(read_pty(&root.join("codex.bin")) + &read_pty(&root.join("cursor.bin")), "");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_sync_pull_of_user_skills_reloads_only_that_homes_tabs() {
        let (m, root, events) = reload_machine("reload-pull");
        let claude = Account::builtin(Provider::Claude);
        let bloom = Account { id: "bloom".into(), provider: Provider::Claude, name: "Bloom".into(), auth: Auth::Login };
        let (mine, other) = (typing_tab(&m, claude.clone(), &root.join("claude.bin")), typing_tab(&m, bloom, &root.join("bloom.bin")));
        for tab in [&mine, &other] {
            m.tab(tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        }
        let share = sync::PluginShare { claude_home: m.accounts.home(&claude), dir: root.join("config/synced-plugins") };
        let touched = skills::pulled_by_plugin_sync(&share, &["user-skills/notes/SKILL.md".into(), "user-skills/notes/references/x.md".into()]);
        let nudged = m.skills_changed(&touched);
        assert_eq!(nudged.iter().map(|n| n.tab.clone()).collect::<Vec<_>>(), [mine]);
        wait_for("the reload in the pulled home's tab", || read_pty(&root.join("claude.bin")).contains("/reload-skills"));
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(read_pty(&root.join("bloom.bin")), "");
        assert_eq!(skills_changed_events(&events), [vec![skills::Scope::Personal]]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn typed_text_holds_a_notice_across_done_until_backspaces_clear_it() {
        let (m, tab, file) = pty_file("notice-typed");
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        m.write_input(&tab, b"xyz").unwrap();
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Ping");
        m.maybe_deliver(&tab);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let early = read_pty(&file);
        assert!(early.contains("xyz"), "the pty did not receive the keystrokes: {early:?}");
        assert!(!early.contains("[pinned]"), "the notice was typed over a draft: {early:?}");
        m.record_event(done_event(&tab));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!read_pty(&file).contains("[pinned]"), "Done cleared a draft: {}", read_pty(&file));
        m.write_input(&tab, b"\x7f\x7f\x7f").unwrap();
        let needle = format!("[pinned] Delegated task {id} \"Ping\"");
        wait_for("the notice after the draft is cleared", || read_pty(&file).contains(&needle));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    #[cfg(unix)]
    fn a_recalled_prompt_holds_the_notice_through_backspace_until_ctrl_u() {
        let (m, tab, file) = pty_file("notice-recall");
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        m.write_input(&tab, b"\x1b[A").unwrap();
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Recall");
        m.write_input(&tab, b"\x7f").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!read_pty(&file).contains("[pinned]"), "the notice was typed onto a recalled prompt: {}", read_pty(&file));
        m.write_input(&tab, b"\x15").unwrap();
        let needle = format!("[pinned] Delegated task {id} \"Recall\"");
        wait_for("the notice after ctrl-u", || read_pty(&file).contains(&needle));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    #[cfg(unix)]
    fn two_deliverers_paste_a_notice_once() {
        let (m, tab, file) = pty_file("notice-once");
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Once");
        let (left, right) = (m.clone(), m.clone());
        let (tab_left, tab_right) = (tab.clone(), tab.clone());
        std::thread::scope(|scope| {
            scope.spawn(|| left.maybe_deliver(&tab_left));
            scope.spawn(|| right.maybe_deliver(&tab_right));
        });
        let needle = format!("[pinned] Delegated task {id} \"Once\"");
        wait_for("the one notice", || read_pty(&file).contains(&needle));
        assert_eq!(read_pty(&file).matches(&needle).count(), 1, "the notice was pasted twice: {}", read_pty(&file));
        let _ = std::fs::remove_file(file);
    }

    #[cfg(unix)]
    fn done_event(tab: &str) -> AgentEvent {
        AgentEvent {
            status: AgentStatus::Done,
            cwd: None,
            message: None,
            tab: Some(tab.into()),
            session: None,
            title: None,
            resume_at: None,
            prompt: None,
            snapshot: None,
            permission_mode: None,
            hook_feedback: false,
        }
    }

    #[test]
    fn escape_sequences_are_not_a_draft_and_enter_submits() {
        let mut draft = Draft::default();
        for keys in [&b"\x1b[I"[..], b"\x1b[O", b"\x1b", b"\x1b[C", b"\x1b[D", b"\x1b[?1;2c", b"\x1b[1;1R", b"\x1b]11;rgb:0/0/0\x07"] {
            assert!(!note_draft(&mut draft, keys), "{keys:?}");
            assert!(draft.empty(), "{keys:?} is not text");
        }
        for keys in [b"\x1b[A".as_slice(), b"\x1b[B", b"\x1bOA", b"\x1bOB"] {
            for clear in [b"\r".as_slice(), b"\x15", b"\x03"] {
                draft = Draft::default();
                assert!(!note_draft(&mut draft, keys), "{keys:?}");
                assert!(!note_draft(&mut draft, b"\x7f\x7f\x7f"));
                assert!(!draft.empty(), "backspace after {keys:?} edits the recalled prompt, which is still in the line");
                assert_eq!(note_draft(&mut draft, clear), clear == b"\r", "{clear:?} after {keys:?}");
                assert!(draft.empty(), "{clear:?} clears the recalled prompt");
            }
        }
        draft = Draft::default();
        note_draft(&mut draft, "é".as_bytes());
        assert_eq!(draft.chars, 1, "one character, however many bytes");
        note_draft(&mut draft, b"\x7f");
        assert_eq!(draft.chars, 0);
        assert!(!note_draft(&mut draft, b"\x1b[200~line one\rline two\x1b[201~"), "a newline inside a paste is not Enter");
        assert_eq!(draft.chars, 17);
        note_draft(&mut draft, b"\x03");
        assert_eq!(draft.chars, 0);
        assert!(!note_draft(&mut draft, b"\x1b[200~ab\x1b[201~"));
        assert_eq!(draft.chars, 2, "a bracketed paste counts the text inside it");
        assert!(note_draft(&mut draft, b"\r"));
        assert_eq!(draft.chars, 0);
        assert!(!note_draft(&mut draft, b"\r"), "enter on an empty line is not a submit");
        note_draft(&mut draft, b"ab");
        assert!(!note_draft(&mut draft, b"\x08"));
        assert_eq!(draft.chars, 1);
        assert!(!note_draft(&mut draft, b"\x7f\x7f"));
        assert_eq!(draft.chars, 0, "backspace stops at an empty line");
    }

    #[test]
    #[cfg(unix)]
    fn a_codex_tab_gets_a_second_notice_only_after_the_done_that_follows_the_first() {
        let (m, tab, file) = pty_file("notice-second");
        m.tab(&tab, |t| {
            t.account = Account::builtin(Provider::Codex);
            t.status = Some(AgentStatus::Done);
        })
        .unwrap();
        let first = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "First");
        m.maybe_deliver(&tab);
        let first_needle = format!("[pinned] Delegated task {first} \"First\"");
        wait_for("the first notice", || read_pty(&file).contains(&first_needle));
        let second = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Second");
        m.maybe_deliver(&tab);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mid = read_pty(&file);
        assert!(!mid.contains("Second"), "a second notice was pasted before the first turn ended: {mid}");
        m.record_event(done_event(&tab));
        let second_needle = format!("[pinned] Delegated task {second} \"Second\"");
        wait_for("the second notice after Done", || read_pty(&file).contains(&second_needle));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    #[cfg(unix)]
    fn a_delivery_and_a_notice_do_not_share_an_idle_gap() {
        let env = vec![("PINNED_ISSUE".into(), "abc".into())];
        let file = std::env::temp_dir().join(format!("pinned-notice-deliver-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let m = machine();
        let tab = m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], env.clone(), false, None);
        m.spawn(&tab, sh(&format!("stty raw -echo; echo ready; exec cat > '{}'", file.display())), pty::size(80, 24)).unwrap();
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        assert!(m.deliver(&env, "from slack"));
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "After");
        m.maybe_deliver(&tab);
        wait_for("the slack prompt", || read_pty(&file).contains("from slack"));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!read_pty(&file).contains("[pinned]"), "the notice shared the slack prompt's turn: {}", read_pty(&file));
        m.record_event(done_event(&tab));
        let needle = format!("[pinned] Delegated task {id} \"After\"");
        wait_for("the notice after the delivery's Done", || read_pty(&file).contains(&needle));

        let file_b = std::env::temp_dir().join(format!("pinned-notice-deliver-b-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&file_b);
        let tab_b = m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], env.clone(), false, None);
        m.spawn(&tab_b, sh(&format!("stty raw -echo; echo ready; exec cat > '{}'", file_b.display())), pty::size(80, 24)).unwrap();
        let (seen_b, sink_b) = viewer();
        m.attach(&tab_b, "test-b".into(), sink_b).unwrap();
        wait_for("stty b", || text(&seen_b).contains("ready"));
        m.tab(&tab_b, |t| t.status = Some(AgentStatus::Done)).unwrap();
        let id_b = finish_task(&m, crate::delegate::Parent::Tab(tab_b.clone()), "Before");
        m.maybe_deliver(&tab_b);
        let needle_b = format!("[pinned] Delegated task {id_b} \"Before\"");
        wait_for("the notice first", || read_pty(&file_b).contains(&needle_b));
        assert!(m.deliver(&env, "from linear"));
        wait_for("a delivery types at once", || read_pty(&file_b).contains("from linear"));
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(&file_b);
    }

    #[test]
    #[cfg(unix)]
    fn the_users_own_prompt_goes_in_mid_turn_and_after_an_interrupt() {
        let (m, tab, file) = pty_file("prompt-mid-turn");
        m.record_event(AgentEvent { status: AgentStatus::Working, ..done_event(&tab) });
        assert_eq!(m.call(Call::Prompt { tab: tab.clone(), text: "steer this".into() }), Ok(json!(null)));
        wait_for("the prompt typed mid-turn", || read_pty(&file).contains("steer this"));
        m.write_input(&tab, b"\x1b").unwrap();
        assert_eq!(m.call(Call::Prompt { tab: tab.clone(), text: "after esc".into() }), Ok(json!(null)));
        wait_for("the prompt after an interrupt with no Done", || read_pty(&file).contains("after esc"));
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    #[cfg(unix)]
    fn a_slash_command_holds_its_notice_until_done() {
        for (name, account) in [("slash-claude", Account::builtin(Provider::Claude)), ("slash-codex", Account::builtin(Provider::Codex))] {
            let (m, tab, file) = pty_file(name);
            m.tab(&tab, |t| {
                t.account = account;
                t.status = Some(AgentStatus::Done);
            })
            .unwrap();
            m.write_input(&tab, b"/model\r").unwrap();
            let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Slash");
            m.maybe_deliver(&tab);
            m.tab(&tab, |t| t.turn_pending = Some(Instant::now() - Duration::from_secs(5))).unwrap();
            m.maybe_deliver(&tab);
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(!read_pty(&file).contains("[pinned]"), "{name} typed the notice before Done: {}", read_pty(&file));
            m.record_event(done_event(&tab));
            let needle = format!("[pinned] Delegated task {id} \"Slash\"");
            wait_for("the notice after Done", || read_pty(&file).contains(&needle));
            let _ = std::fs::remove_file(file);
        }
    }

    #[test]
    #[cfg(unix)]
    fn focus_reports_do_not_hold_a_notice() {
        let (m, tab, file) = pty_file("notice-focus");
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        for bytes in [b"\x1b[I".as_slice(), b"\x1b[O".as_slice(), b"\x1b".as_slice(), b"\x1b[Z".as_slice(), b"\x1b[C".as_slice(), b"\x1b[D".as_slice(), b"\r".as_slice()] {
            m.write_input(&tab, bytes).unwrap();
        }
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Focus");
        m.maybe_deliver(&tab);
        let needle = format!("[pinned] Delegated task {id} \"Focus\"");
        wait_for("the notice after focus reports", || read_pty(&file).contains(&needle));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    #[cfg(unix)]
    fn deliver_task_types_a_held_notice_and_another_tab_cannot() {
        let (m, tab, file) = pty_file("deliver-task");
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        m.write_input(&tab, b"xyz").unwrap();
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Held");
        m.maybe_deliver(&tab);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!read_pty(&file).contains("[pinned]"), "the notice was typed over a draft: {}", read_pty(&file));
        let other = m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let refused = m.call(Call::DeliverTask { tab: other, task: id.clone() });
        assert_eq!(refused, Err(format!("task {id} belongs to another session")));
        assert!(!read_pty(&file).contains("[pinned]"), "another tab typed the notice: {}", read_pty(&file));
        let blocked = m.call(Call::DeliverTask { tab: tab.clone(), task: id.clone() });
        assert_eq!(blocked, Err("The prompt has unsent text. Send or clear it (Ctrl-U), then try again.".into()));
        assert!(!read_pty(&file).contains("[pinned]"), "Send to session submitted the draft: {}", read_pty(&file));
        m.write_input(&tab, b"\x15").unwrap();
        let needle = format!("[pinned] Delegated task {id} \"Held\"");
        wait_for("the notice after the draft is cleared", || read_pty(&file).contains(&needle));
        let second = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Clear");
        assert_eq!(m.call(Call::DeliverTask { tab: tab.clone(), task: second.clone() }), Ok(json!(null)));
        let second_needle = format!("[pinned] Delegated task {second} \"Clear\"");
        wait_for("Send to session typed the notice", || read_pty(&file).contains(&second_needle));
        let again = m.call(Call::DeliverTask { tab, task: id });
        assert!(again.is_err(), "a second send typed it again: {again:?}");
        let _ = std::fs::remove_file(file);
    }

    #[test]
    #[cfg(unix)]
    fn a_grandchild_is_not_typed_into_the_root_tab() {
        let (m, tab, file) = pty_file("notice-grand");
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        let parent = m.delegates.begin(crate::delegate::NewTask {
            parent: crate::delegate::Parent::Tab(tab.clone()),
            tool: None,
            title: "Parent".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: std::env::temp_dir(),
            mode: crate::delegate::Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: false,
            caller_key: format!("tab:{tab}"),
            client_request_id: None,
            role: None,
        });
        let child = finish_task(&m, crate::delegate::Parent::Task(parent.id.clone()), "Grandchild");
        m.maybe_deliver(&tab);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!read_pty(&file).contains("Grandchild"), "a grandchild notice was typed: {}", read_pty(&file));
        assert!(!read_pty(&file).contains(&child), "a grandchild id was typed: {}", read_pty(&file));
        m.delegates.apply(&parent.id, crate::delegate::ChildEvent::Line(json!({"type":"result","result":"PARENT"}).to_string()), None);
        m.maybe_deliver(&tab);
        let needle = format!("[pinned] Delegated task {} \"Parent\"", parent.id);
        wait_for("the parent notice", || read_pty(&file).contains(&needle));
        assert!(!read_pty(&file).contains("Grandchild"), "the parent notice included the grandchild: {}", read_pty(&file));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    #[cfg(unix)]
    fn a_cursor_tab_with_no_turn_hook_never_receives_a_notice() {
        let file = std::env::temp_dir().join(format!("pinned-notice-cursor-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let m = machine();
        let tab = m.add_tab("/repo".into(), Account::builtin(Provider::Cursor), vec![], vec![], false, None);
        m.spawn(&tab, sh(&format!("stty raw -echo; echo ready; exec cat > '{}'", file.display())), pty::size(80, 24)).unwrap();
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Cursor");
        m.maybe_deliver(&tab);
        std::thread::sleep(std::time::Duration::from_millis(300));
        m.maybe_deliver(&tab);
        let got = read_pty(&file);
        assert!(!got.contains(&id), "a Cursor tab with no turn end was typed into: {got}");
        assert!(!got.contains("[pinned]"), "a Cursor tab received a notice: {got}");
        let _ = std::fs::remove_file(file);
    }

    #[test]
    #[cfg(unix)]
    fn a_codex_notify_marks_the_tab_done_and_the_notice_is_typed() {
        let root = temp("codex-notify");
        let sock = std::env::temp_dir().join(format!("pinned-codex-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let file = root.join("notice.bin");
        let exe = root.join("pinned-hook");
        let codex = root.join("codex");
        std::fs::write(&exe, r#"#!/usr/bin/env python3
import json, os, socket, sys
payload = json.loads(sys.argv[-1])
if payload.get("type") != "agent-turn-complete":
    sys.exit(0)
msg = {"kind":"agent","status":"done","cwd":payload.get("cwd"),"message":None,"tab":os.environ.get("PINNED_TAB"),"session":payload.get("thread-id"),"title":None,"resume_at":None,"prompt":None,"snapshot":None}
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(os.environ["PINNED_SOCK"])
s.sendall((json.dumps(msg) + "\n").encode())
"#).unwrap();
        std::fs::write(&codex, r#"#!/usr/bin/env python3
import json, os, subprocess, sys
payload = {"type":"agent-turn-complete","thread-id":"thr-9","turn-id":"turn-9","cwd":"/repo","input-messages":["ping"],"last-assistant-message":"ok"}
for arg in sys.argv[1:]:
    if arg.startswith("notify="):
        subprocess.run(json.loads(arg[len("notify="):]) + [json.dumps(payload)], check=False)
        break
os.system("stty raw -echo")
print("ready", flush=True)
out = open(os.environ["NOTICE_FILE"], "wb")
while True:
    data = sys.stdin.buffer.read(1)
    if not data:
        break
    out.write(data)
    out.flush()
"#).unwrap();
        for path in [&exe, &codex] {
            std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        let m = Machine::new(
            Launch { exe, bins: BTreeMap::from([(Provider::Codex, codex)]), sock: sock.clone(), ..launch() },
            None,
            Some(root.clone()),
            |_| {},
        );
        m.listen(|_| Err("not a connector".into())).unwrap();
        let tab = m.add_tab(
            root.clone(),
            Account::builtin(Provider::Codex),
            vec![],
            vec![("NOTICE_FILE".into(), file.display().to_string()), ("CODEX_HOME".into(), root.display().to_string())],
            false,
            None,
        );
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        m.start_claude(&tab, 80, 24).unwrap();
        wait_for("codex ready", || text(&seen).contains("ready"));
        wait_for("codex done", || m.view(&tab).unwrap().status == Some(AgentStatus::Done));
        assert_eq!(m.view(&tab).unwrap().session.as_deref(), Some("thr-9"));
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "From Codex");
        m.maybe_deliver(&tab);
        let needle = format!("[pinned] Delegated task {id} \"From Codex\"");
        wait_for("the codex notice", || read_pty(&file).contains(&needle));
        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A fake `claude` that records, for each start, its folder, role, project and arguments in
    /// `<claude>.<tab>`, NUL-separated since a prompt holds newlines.
    #[cfg(unix)]
    fn recording_claude(dir: &Path) -> PathBuf {
        let claude = dir.join("claude");
        let script = "#!/bin/sh\nprintf '%s\\0' \"$(pwd -P)\" \"$PINNED_ROLE\" \"$PINNED_PROJECT\" \"$@\" > \"$0.tmp.$PINNED_TAB\" && mv \"$0.tmp.$PINNED_TAB\" \"$0.$PINNED_TAB\"\nexec sleep 30\n";
        std::fs::write(&claude, script).unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        claude
    }

    #[cfg(unix)]
    fn started(claude: &Path, tab: &str) -> Vec<String> {
        let file = PathBuf::from(format!("{}.{tab}", claude.display()));
        wait_for("claude to start", || file.exists());
        let text = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_file(file);
        text.trim_end_matches('\0').split('\0').map(String::from).collect()
    }

    #[cfg(unix)]
    fn ask(sock: &Path, line: &str) -> Result<String, String> {
        use std::io::{BufRead, Write};
        let mut stream = crate::ipc::UnixStream::connect(sock).unwrap();
        writeln!(stream, "{line}").unwrap();
        let mut reply = String::new();
        std::io::BufReader::new(stream).read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap()
    }

    fn hook(tab: &str, status: AgentStatus) -> AgentEvent {
        AgentEvent { status, cwd: None, message: None, tab: Some(tab.into()), session: Some("s1".into()), title: None, resume_at: None, prompt: None, snapshot: None, permission_mode: None, hook_feedback: false }
    }

    /// A Capitão whose session is a pty running cat in raw mode, which writes what it is typed
    /// to `file`, and which has finished a turn: idle.
    #[cfg(unix)]
    fn idle_captain(m: &Machine, path: &str, file: &Path) -> TabId {
        let tab = m.open_orchestrator(path).unwrap();
        let (seen, sink) = viewer();
        m.spawn(&tab, sh(&format!("stty raw -echo; echo ready; exec cat > '{}'", file.display())), pty::size(80, 24)).unwrap();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        m.record_event(hook(&tab, AgentStatus::Done));
        tab
    }

    #[cfg(unix)]
    fn typed(file: &Path, expected: &[u8]) -> Vec<u8> {
        wait_for("cat to write", || std::fs::read(file).is_ok_and(|b| b.len() >= expected.len()));
        std::thread::sleep(Duration::from_millis(100));
        std::fs::read(file).unwrap()
    }

    #[cfg(unix)]
    fn enqueue(m: &Machine, path: &str, text: &str, request_id: &str) -> Result<Value, String> {
        m.call(Call::Enqueue { path: path.into(), text: text.into(), request_id: request_id.into(), sender: String::new(), busy: None })
    }

    #[test]
    #[cfg(unix)]
    fn a_message_for_an_idle_capitao_is_typed_now_once_and_the_rest_wait_for_its_turn_to_end() {
        let dir = temp("enqueue-idle");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let file = dir.join("typed.bin");
        let m = Machine::new(launch(), Some(dir.join("config")), None, |_| {});
        let path = project.display().to_string();
        let tab = idle_captain(&m, &path, &file);

        assert_eq!(enqueue(&m, &path, "Ship the form.", "r-1"), Ok(json!({ "outcome": "entregue", "tab": tab })));
        assert_eq!(typed(&file, b"\x1b[200~[Orchestrator] Ship the form.\x1b[201~\r"), b"\x1b[200~[Orchestrator] Ship the form.\x1b[201~\r");
        assert_eq!(enqueue(&m, &path, "Ship the form.", "r-1"), Ok(json!({ "outcome": "entregue", "tab": tab })), "a repeat answers the first outcome");
        assert_eq!(enqueue(&m, &dir.display().to_string(), "Other.", "r-1"), Err("request_id r-1 was already used for another message".into()));

        assert_eq!(enqueue(&m, &path, "Then the footer.", "r-2"), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 1 })), "typed over the turn the first message started");
        assert_eq!(enqueue(&m, &path, "And the docs.", "r-3"), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 2 })));
        assert_eq!(std::fs::read(&file).unwrap().len(), b"\x1b[200~[Orchestrator] Ship the form.\x1b[201~\r".len(), "a queued message was typed while the Capitão worked");
        assert_eq!(m.sent.lock().unwrap().in_flight(&project).iter().map(|s| s.request_id.as_str()).collect::<Vec<_>>(), ["r-1"]);
        assert_eq!(m.view(&tab).unwrap().queued, 2, "the Capitão's view does not count its queued messages");

        m.record_event(hook(&tab, AgentStatus::Done));
        let both = b"\x1b[200~[Orchestrator] Ship the form.\x1b[201~\r\x1b[200~[Orchestrator] Then the footer.\n[Orchestrator] And the docs.\x1b[201~\r";
        assert_eq!(typed(&file, both), both);
        assert_eq!(m.view(&tab).unwrap().queued, 0, "typed messages still count as queued");
        assert_eq!(m.sent.lock().unwrap().in_flight(&project).iter().map(|s| s.request_id.as_str()).collect::<Vec<_>>(), ["r-2", "r-3"], "the drained messages started the next turn");
        assert_eq!(m.sent.lock().unwrap().find("r-1").map(|s| s.progress), Some(Progress::Answered), "the turn r-1 started ended");
        assert_eq!(enqueue(&m, &path, "Then the footer.", "r-2"), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 1 })), "a repeat after the drain still answers the first outcome and types nothing");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(std::fs::read(&file).unwrap(), both);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_stop_hook_hands_over_a_queued_message_and_its_turn_is_the_next_one() {
        let dir = temp("enqueue-hook");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let sock = dir.join("pinned.sock");
        let m = Machine::new(Launch { sock: sock.clone(), ..launch() }, Some(dir.join("config")), None, |_| {});
        m.listen(|_| Err("not the machine's".into())).unwrap();
        let path = project.display().to_string();
        let tab = idle_captain(&m, &path, &dir.join("typed.bin"));
        m.record_event(hook(&tab, AgentStatus::Working));
        assert_eq!(enqueue(&m, &path, "Check the build.", "r-1"), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 1 })));
        assert!(m.sent.lock().unwrap().in_flight(&project).is_empty());
        let done = serde_json::to_string(&Message::Agent(AgentEvent { hook_feedback: true, ..hook(&tab, AgentStatus::Done) })).unwrap();
        assert_eq!(ask(&sock, &done), Ok("[Orchestrator] Check the build.".into()));
        assert_eq!(m.sent.lock().unwrap().in_flight(&project).iter().map(|s| s.request_id.as_str()).collect::<Vec<_>>(), ["r-1"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_busy_capitao_refuses_when_the_settings_say_so_and_a_refusal_is_not_remembered() {
        let dir = temp("enqueue-refuse");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let m = Machine::new(launch(), Some(dir.join("config")), None, |_| {});
        let path = project.display().to_string();
        let tab = idle_captain(&m, &path, &dir.join("typed.bin"));
        m.record_event(hook(&tab, AgentStatus::Working));
        let refuse = global::Settings { busy: BusyPolicy::Refuse, ..global::Settings::default() };
        m.call(Call::SetGlobalOrchestratorSettings { settings: refuse }).unwrap();

        assert_eq!(enqueue(&m, &path, "Ship it.", "r-1"), Ok(json!({ "outcome": "ocupado", "tab": tab })));
        assert!(m.delegates.claim_notice(&tab).is_none(), "a refused message was queued");
        let queue = Call::Enqueue { path: path.clone(), text: "Ship it.".into(), request_id: "r-1".into(), sender: "Almirante".into(), busy: Some(BusyPolicy::Queue) };
        assert_eq!(m.call(queue), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 1 })), "the call's policy wins, and the refused id delivers on a retry");
        assert_eq!(m.delegates.claim_notice(&tab).map(|(text, _)| text), Some("[Almirante] Ship it.".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_stopped_capitao_starts_on_the_message_and_one_without_a_conversation_restarts_on_it() {
        let dir = temp("enqueue-start");
        let (project, config) = (dir.join("site"), dir.join("config"));
        std::fs::create_dir_all(&project).unwrap();
        let claude = recording_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(config), None, |_| {});
        let path = project.display().to_string();
        let capitao = format!("{}\n\n{}\n\n{}\n\n{}", Role::Orchestrator.prompt(), taste::Taste::default().brief(), taste::Taste::default().limits().hint(), global::captain_prompt("Orchestrator", global::ReportPolicy::Auto));

        let opened = enqueue(&m, &path, "Ship the form.", "r-1").unwrap();
        let tab = opened["tab"].as_str().unwrap().to_owned();
        assert_eq!(opened, json!({ "outcome": "iniciado", "tab": tab }), "the project had no Capitão");
        assert_eq!(started(&claude, &tab), [&path, "orchestrator", &path, "--settings", "s", "--plugin-dir", "p", "[Orchestrator] Ship the form.", "--append-system-prompt", &capitao]);
        assert_eq!(enqueue(&m, &path, "Then the footer.", "r-2"), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 1 })), "the message it started on was not taken yet");

        let api = dir.join("api");
        std::fs::create_dir_all(&api).unwrap();
        let api_path = api.display().to_string();
        let fresh = m.open_orchestrator(&api_path).unwrap();
        m.start_claude(&fresh, 80, 24).unwrap();
        started(&claude, &fresh);
        assert_eq!(enqueue(&m, &api_path, "- Fix the README.", "r-3"), Ok(json!({ "outcome": "iniciado", "tab": fresh })));
        assert_eq!(started(&claude, &fresh)[7..9], ["[Orchestrator] - Fix the README.", "--append-system-prompt"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_restored_capitao_resumes_its_conversation_on_the_message() {
        let dir = temp("enqueue-resume");
        let (project, config) = (dir.join("site"), dir.join("config"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        let path = project.display().to_string();
        std::fs::write(config.join("projects.json"), json!([{ "path": path }]).to_string()).unwrap();
        let env = json!([["PINNED_ROLE", "orchestrator"], ["PINNED_PROJECT", path]]);
        std::fs::write(config.join("sessions.json"), json!([{ "cwd": path, "session": "s1", "env": env }]).to_string()).unwrap();
        let claude = recording_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(config), None, |_| {});
        let tab = m.restore_tabs().remove(0);
        assert_eq!(enqueue(&m, &path, "Where are we?", "r-1"), Ok(json!({ "outcome": "iniciado", "tab": tab })));
        assert_eq!(started(&claude, &tab)[7..11], ["--resume", "s1", "[Orchestrator] Where are we?", "--append-system-prompt"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_queued_message_survives_a_restart_and_its_request_id_still_answers_the_first_outcome() {
        let dir = temp("enqueue-restart");
        let (project, config) = (dir.join("site"), dir.join("config"));
        std::fs::create_dir_all(&project).unwrap();
        let path = project.display().to_string();
        let before = Machine::new(launch(), Some(config.clone()), None, |_| {});
        let tab = idle_captain(&before, &path, &dir.join("typed.bin"));
        before.record_event(hook(&tab, AgentStatus::Working));
        assert_eq!(enqueue(&before, &path, "Ship it.", "r-1"), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 1 })));
        before.quit();

        let after = Machine::new(launch(), Some(config), None, |_| {});
        let restored = after.restore_tabs().remove(0);
        assert_ne!(restored, tab);
        assert_eq!(enqueue(&after, &path, "Ship it.", "r-1"), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 1 })));
        assert_eq!(after.delegates.claim_notice(&restored).map(|(text, _)| text), Some("[Orchestrator] Ship it.".into()), "the queue did not follow the Capitão's conversation");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn the_global_orchestrator_opens_once_in_its_own_folder_which_is_no_project_but_keeps_memory() {
        let dir = temp("global-tab");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let (sock, config) = (dir.join("pinned.sock"), dir.join("config"));
        let claude = recording_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), sock: sock.clone(), ..launch() }, Some(config.clone()), None, |_| {});
        m.listen(|_| Err("not the machine's".into())).unwrap();
        let path = m.open_project(project.clone()).display().to_string();

        let global = m.call(Call::OpenGlobalOrchestrator).unwrap();
        assert_eq!(m.call(Call::OpenGlobalOrchestrator).unwrap(), global, "a second global orchestrator opened");
        let global = global.as_str().unwrap().to_owned();
        let folder = config.join("home/.pinned/orquestrador");
        let view = m.view(&global).unwrap();
        assert_eq!((view.cwd.as_str(), view.orchestrator_global, view.role, view.running), (folder.to_str().unwrap(), true, None, false));
        assert_eq!(m.list_projects().iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), [path.as_str()], "the global folder became a project");

        let tool = |tab: &str, tool: &str| ask(&sock, &format!(r#"{{"kind":"project","tab":"{tab}","tool":{{{tool}}}}}"#));
        assert_eq!(tool(&global, r#""name":"memory_save","title":"Machines","text":"The Mac mini hosts the orchestrator.""#), Ok("Saved #1.".into()));
        let found = m.call(Call::MemorySearch { path: folder.display().to_string(), query: "".into(), limit: None }).unwrap();
        assert_eq!(found.as_array().unwrap().iter().map(|m| m["title"].clone()).collect::<Vec<_>>(), [json!("Machines")]);
        assert_eq!(m.list_projects().len(), 1);

        let send = format!(r#""name":"send_to_captain","project":"{path}","message":"Ship the form.","request_id":"r-1""#);
        let sent: Value = serde_json::from_str(&tool(&global, &send).unwrap()).unwrap();
        assert_eq!(sent["outcome"], json!("iniciado"));
        assert_eq!(started(&claude, sent["tab"].as_str().unwrap())[7], "[Orchestrator] Ship the form.");
        let elsewhere = r#""name":"send_to_captain","project":"github.com/o/elsewhere","message":"x","request_id":"r-2""#;
        assert_eq!(tool(&global, elsewhere), Err("github.com/o/elsewhere is not a project on this machine, and the hub can't list the other machines' projects (this machine is not linked to a hub): name it as <machine id>:<path>".into()));
        assert_eq!(tool(&global, r#""name":"agents""#), Err("the global orchestrator does no project work: hand it to the project's Capitão with send_to_captain".into()));

        let plain = m.open_tab(Some(path.clone()), false).unwrap();
        assert_eq!(tool(&plain, &send.replace("r-1", "r-3")), Err("only the global orchestrator has this tool".into()));
        assert_eq!(tool(&plain, r#""name":"tell_orchestrator","text":"Halfway.""#), Err("only a project's Capitão reports to the global orchestrator".into()));
        let capitao = sent["tab"].as_str().unwrap();
        assert_eq!(tool(capitao, r#""name":"tell_orchestrator","text":"Halfway.""#), Err("this machine is not linked to a hub".into()), "a machine that hosts no orchestrator reports through the hub");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn send_to_captain_relays_a_project_on_another_machine_and_keeps_one_named_by_this_machines_id_here() {
        let dir = temp("global-relay");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let claude = recording_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(dir.join("config")), None, |_| {});
        let path = m.open_project(project).display().to_string();
        let overview: Arc<Mutex<Option<Value>>> = Arc::default();
        let relayed: Arc<Mutex<Vec<Value>>> = Arc::default();
        let (seen_overview, seen_relays) = (overview.clone(), relayed.clone());
        m.set_hub(Arc::new(move |call| match call {
            HubCall::Overview => seen_overview.lock().unwrap().clone().ok_or_else(|| "this hub runs pinned 0.17.19, which can't answer that yet".to_owned()),
            relay @ HubCall::Relay { .. } => {
                let mut relays = seen_relays.lock().unwrap();
                relays.push(serde_json::to_value(relay).unwrap());
                Ok(json!({ "outcome": "na_fila", "tab": "tab-b-1", "position": relays.len() }))
            }
            _ => Err("not this test's".into()),
        }));
        let relay = |request_id: &str, machine: &str, path: &str| {
            json!({ "cmd": "relay", "request_id": request_id, "to": { "kind": "captain", "machine": machine, "path": path, "text": "Ship it.", "sender": "Orchestrator", "busy": "queue" } })
        };

        assert_eq!(m.send_to_captain("m-book:/w/site", None, "Ship it.", "r-1"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 1 }));
        assert_eq!(m.send_to_captain("/w/site", Some("m-book"), "Ship it.", "r-2"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 2 }));
        assert_eq!(m.send_to_captain("github.com/o/site", None, "Ship it.", "r-3"), Err("github.com/o/site is not a project on this machine, and the hub can't list the other machines' projects (this hub runs pinned 0.17.19, which can't answer that yet): name it as <machine id>:<path>".into()));
        let mine = format!("m-mini:{path}");
        assert_eq!(m.send_to_captain(&mine, None, "Ship it.", "r-4"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 3 }), "without the hub's id for it, this machine's own id names another");
        assert_eq!(*relayed.lock().unwrap(), [relay("r-1", "m-book", "/w/site"), relay("r-2", "m-book", "/w/site"), relay("r-4", "m-mini", &path)]);

        m.set_hub_id("m-mini".into(), "mini".into());
        let here = m.send_to_captain(&mine, None, "Ship it.", "r-5").unwrap();
        assert_eq!(started(&claude, here.tab())[7], "[Orchestrator] Ship it.");
        assert_eq!(relayed.lock().unwrap().len(), 3, "a project named by this machine's id went to the hub");

        let checkout = |machine: &str, name: &str, online: bool| json!({ "machine": machine, "machine_name": name, "online": online, "path": "/w/site", "captain": "livre" });
        let site = |checkouts: Vec<Value>| {
            let machines = [("m-book", "MacBook"), ("m-old", "iMac"), ("m-air", "Air")].map(|(id, name)| json!({ "id": id, "name": name, "online": true }));
            json!({ "projects": [{ "key": "github.com/o/site", "name": "site", "repo": "github.com/o/site", "checkouts": checkouts }], "waiting": [], "machines": machines })
        };
        *overview.lock().unwrap() = Some(site(vec![checkout("m-book", "MacBook", true), checkout("m-old", "iMac", false)]));
        assert_eq!(m.send_to_captain("github.com/o/site", None, "Ship it.", "r-6"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 4 }));
        *overview.lock().unwrap() = Some(site(vec![checkout("m-book", "MacBook", true), checkout("m-air", "Air", true)]));
        assert_eq!(m.send_to_captain("github.com/o/site", None, "Ship it.", "r-7"), Err("github.com/o/site is on MacBook, Air: pass machine to pick one".into()));
        assert_eq!(m.send_to_captain("github.com/o/site", Some("air"), "Ship it.", "r-8"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 5 }));
        assert_eq!(m.send_to_captain("github.com/o/site", Some("@MacBook"), "Ship it.", "r-9"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 6 }));
        assert_eq!(m.send_to_captain("Air:/w/site", None, "Ship it.", "r-10"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 7 }));
        assert_eq!(m.send_to_captain("@imac:/w/new", None, "Ship it.", "r-11"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 8 }), "a folder the overview doesn't list yet");
        assert_eq!(m.send_to_captain("m-book:/w/site", None, "Ship it.", "r-12"), Ok(Handoff::Queued { tab: "tab-b-1".into(), position: 9 }));
        assert_eq!(m.send_to_captain("github.com/o/site", Some("@Studio"), "Ship it.", "r-13"), Err("github.com/o/site is not a project on Studio".into()));
        let targets: Vec<(Value, Value)> = relayed.lock().unwrap()[3..].iter().map(|r| (r["to"]["machine"].clone(), r["to"]["path"].clone())).collect();
        let to = |machine: &str, path: &str| (json!(machine), json!(path));
        assert_eq!(targets, [to("m-book", "/w/site"), to("m-air", "/w/site"), to("m-book", "/w/site"), to("m-air", "/w/site"), to("m-old", "/w/new"), to("m-book", "/w/site")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn read_captain_reads_a_capitao_on_another_machine_through_the_hub() {
        let dir = temp("global-read");
        let m = Machine::new(launch(), Some(dir.join("config")), None, |_| {});
        let asked: Arc<Mutex<Vec<Value>>> = Arc::default();
        let seen = asked.clone();
        m.set_hub(Arc::new(move |call| match call {
            HubCall::Overview => Ok(json!({
                "projects": [{ "key": "github.com/o/site", "name": "site", "repo": "github.com/o/site", "checkouts": [{ "machine": "m-book", "machine_name": "MacBook", "online": true, "path": "/w/site", "captain": "trabalhando", "captain_tab": "tab-b-1" }] }],
                "waiting": [],
                "machines": [{ "id": "m-book", "name": "MacBook", "online": true }, { "id": "m-old", "name": "iMac", "online": true }],
            })),
            HubCall::ReadCaptain { machine, path } => {
                seen.lock().unwrap().push(json!([machine, path]));
                match machine.as_str() {
                    "m-old" => Err("this hub runs pinned 0.17.19, which can't answer that: unknown variant `read_captain`".into()),
                    _ => Ok(json!({ "state": "working", "next": "it is working; read again in a while", "title": "Ship the form", "conversation": { "items": [], "cursor": 3, "reset": false }, "project": path })),
                }
            }
            _ => Err("not this test's".into()),
        }));
        let read = m.read_captain("github.com/o/site", None).map(|text| serde_json::from_str::<Value>(&text).unwrap());
        assert_eq!(read, Ok(json!({ "state": "working", "next": "it is working; read again in a while", "title": "Ship the form", "conversation": { "items": [], "cursor": 3, "reset": false }, "project": "/w/site" })));
        let notes = dir.join("notes").display().to_string();
        assert_eq!(m.read_captain(&notes, Some("@imac")), Err("o hub ou a máquina precisa atualizar o pinned".into()), "a folder the overview doesn't list, on a machine named by name");
        assert_eq!(m.read_captain(r"m-win:C:\w\site", None).map(|text| serde_json::from_str::<Value>(&text).unwrap()["project"].clone()), Ok(json!(r"C:\w\site")), "a Windows folder, from any machine");
        assert_eq!(m.read_captain("m-book:/w/api", None).map(|text| serde_json::from_str::<Value>(&text).unwrap()["project"].clone()), Ok(json!("/w/api")), "a Unix folder, from any machine");
        assert_eq!(*asked.lock().unwrap(), [json!(["m-book", "/w/site"]), json!(["m-old", notes]), json!(["m-win", r"C:\w\site"]), json!(["m-book", "/w/api"])]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_report_starts_the_global_orchestrator_on_its_line_once() {
        let dir = temp("global-report");
        let claude = recording_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(dir.join("config")), None, |_| {});
        let report = CaptainReport { machine: "m1".into(), machine_name: "MacBook".into(), project: "site".into(), path: "/w/site".into(), tab: "tab-1a-3".into(), text: "Done: PR #12 is open.".into(), answers: vec![] };
        let call = || m.call(Call::OrchestratorReport { request_id: "report-1".into(), report: report.clone() });
        let first = call().unwrap();
        let global = first["tab"].as_str().unwrap().to_owned();
        assert_eq!(first, json!({ "outcome": "iniciado", "tab": global }));
        let folder = dir.join("config/home/.pinned/orquestrador").display().to_string();
        assert_eq!(started(&claude, &global), [&folder, "global-orchestrator", "", "--settings", "s", "--plugin-dir", "p", "[pinned] Capitão site (MacBook) [m1 tab-1a-3]: Done: PR #12 is open.", "--append-system-prompt", &format!("{}\n\n{}", global::prompt("Orchestrator", &crate::account::hostname()), taste::Taste::default().brief())]);
        assert_eq!(call(), Ok(first), "a repeated report answers the first outcome");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A machine whose projects' sessions run under the `work` account, so their transcripts live
    /// in the test's config folder.
    #[cfg(unix)]
    fn work_machine(dir: &Path, claude: &Path, sock: &Path) -> Arc<Machine> {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("accounts.json"), r#"[{"id": "work", "provider": "claude", "name": "Work"}]"#).unwrap();
        Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.to_owned())]), sock: sock.to_owned(), ..launch() }, Some(config), None, |_| {})
    }

    #[cfg(unix)]
    fn said(dir: &Path, session: &str, text: &str) {
        let folder = dir.join("config/accounts/work/projects/-site");
        std::fs::create_dir_all(&folder).unwrap();
        let lines = [
            r#"{"type":"user","uuid":"u1","timestamp":"2026-11-09T10:00:00Z","message":{"content":"[Orchestrator] Ship the form."}}"#.to_owned(),
            json!({ "type": "assistant", "uuid": "a1", "timestamp": "2026-11-09T10:05:00Z", "message": { "content": [{ "type": "text", "text": text }] } }).to_string(),
        ];
        std::fs::write(folder.join(format!("{session}.jsonl")), lines.map(|l| l + "\n").concat()).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn a_message_from_the_global_orchestrator_comes_back_as_the_capitaos_report_when_its_turn_ends() {
        let dir = temp("global-e2e");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let (claude, sock) = (recording_claude(&dir), dir.join("pinned.sock"));
        let m = work_machine(&dir, &claude, &sock);
        m.listen(|_| Err("not the machine's".into())).unwrap();
        m.set_global_settings(global::Settings { host: true, ..global::Settings::default() }).unwrap();
        let path = m.open_project(project.clone()).display().to_string();
        m.set_project_account(&path, Some("work".into())).unwrap();
        let global_tab = m.call(Call::OpenGlobalOrchestrator).unwrap().as_str().unwrap().to_owned();
        let tool = |tool: &str| ask(&sock, &format!(r#"{{"kind":"project","tab":"{global_tab}","tool":{{{tool}}}}}"#));
        let host = crate::account::hostname();
        // As the link does on the hub's hello; the hub itself is down, so every view is this machine's.
        m.set_hub_id("m-mini".into(), host.clone());
        let projects = |captain: &str, tab: &str| {
            let checkout = json!({ "machine": "m-mini", "machine_name": host, "online": true, "version": env!("CARGO_PKG_VERSION"), "path": path, "captain": captain, "captain_tab": tab, "workers": 0 });
            let machine = json!({ "id": "m-mini", "name": host, "online": true, "version": env!("CARGO_PKG_VERSION") });
            json!({ "projects": [{ "key": format!("m-mini:{path}"), "name": "site", "checkouts": [checkout] }], "waiting": [], "machines": [machine] })
        };
        let overview = || serde_json::from_str::<Value>(&tool(r#""name":"projects""#).unwrap()).unwrap();

        let sent: Value = serde_json::from_str(&tool(&format!(r#""name":"send_to_captain","project":"{path}","message":"Ship the form.","request_id":"r-1""#)).unwrap()).unwrap();
        let capitao = sent["tab"].as_str().unwrap().to_owned();
        assert_eq!(sent, json!({ "outcome": "iniciado", "tab": capitao }));
        assert_eq!(started(&claude, &capitao)[7], "[Orchestrator] Ship the form.");
        assert_eq!(overview(), projects("trabalhando", &capitao), "the message started its turn before any hook said so");

        said(&dir, "s1", "The form validates now. PR #12 is open.");
        let done = serde_json::to_string(&Message::Agent(AgentEvent { hook_feedback: true, ..hook(&capitao, AgentStatus::Done) })).unwrap();
        ask(&sock, &done).unwrap();
        let report = format!("[pinned] Capitão site ({host}) [m-mini {capitao}]: The form validates now. PR #12 is open.");
        assert_eq!(started(&claude, &global_tab)[7..], [report, "--append-system-prompt".into(), format!("{}\n\n{}", global::prompt("Orchestrator", &host), taste::Taste::default().brief())], "the report started the global orchestrator");
        assert_eq!(m.sent.lock().unwrap().find("r-1").map(|s| s.progress), Some(Progress::Answered));

        assert_eq!(overview(), projects("livre", &capitao));
        assert_eq!(tool(r#""name":"waiting""#), Ok("[]".into()));

        let read: Value = serde_json::from_str(&tool(&format!(r#""name":"read_captain","project":"m-mini:{path}""#)).unwrap()).unwrap();
        let conversation = json!([
            { "kind": "user", "at": "2026-11-09T10:00:00Z", "text": "[Orchestrator] Ship the form." },
            { "kind": "text", "at": "2026-11-09T10:05:00Z", "text": "The form validates now. PR #12 is open." },
        ]);
        assert_eq!((&read["state"], &read["project"], &read["conversation"]["items"]), (&json!("done"), &json!(path), &conversation));
        assert_eq!(
            tool(r#""name":"read_captain","project":"github.com/o/elsewhere""#),
            Err("github.com/o/elsewhere is not a project on this machine, and the hub can't list the other machines' projects (this machine is not linked to a hub): name it as <machine id>:<path>".into())
        );

        let plain = m.open_tab(Some(path.clone()), false).unwrap();
        m.start_claude(&plain, 80, 24).unwrap();
        assert_eq!(started(&claude, &plain), [&path, "", "", "--settings", "s", "--plugin-dir", "p"], "a plain thread got a system prompt");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_capitao_off_the_host_reports_through_the_hub_and_tool_only_reports_nothing_by_itself() {
        let dir = temp("global-relay");
        let (site, api) = (dir.join("site"), dir.join("api"));
        std::fs::create_dir_all(&site).unwrap();
        std::fs::create_dir_all(&api).unwrap();
        let (claude, sock) = (recording_claude(&dir), dir.join("pinned.sock"));
        let m = work_machine(&dir, &claude, &sock);
        let relayed = Arc::new(Mutex::new(vec![]));
        let hub = relayed.clone();
        m.set_hub(Arc::new(move |call| match call {
            HubCall::Relay { .. } => {
                hub.lock().unwrap().push(serde_json::to_value(&call).unwrap());
                Ok(json!({ "outcome": "entregue", "tab": "tab-g-1" }))
            }
            _ => Err("this hub runs pinned 0.17.19, which can't answer that yet: the global orchestrator's relay is not on the hub".into()),
        }));
        let path = m.open_project(site.clone()).display().to_string();
        m.set_project_account(&path, Some("work".into())).unwrap();
        let host = crate::account::hostname();
        let relays = || {
            let calls = relayed.lock().unwrap().clone();
            calls.into_iter().map(|mut call| {
                assert!(call["request_id"].as_str().unwrap().starts_with("report:"), "a report's id may meet a message's: {call}");
                call["request_id"] = json!("report:*");
                call
            }).collect::<Vec<_>>()
        };

        let capitao = enqueue(&m, &path, "Ship the form.", "r-1").unwrap()["tab"].as_str().unwrap().to_owned();
        started(&claude, &capitao);
        m.record_event(hook(&capitao, AgentStatus::Working));
        assert_eq!(m.project_tool(&capitao, ProjectTool::TellOrchestrator { text: "Which form?".into() }), Ok(r#"{"outcome":"entregue","tab":"tab-g-1"}"#.into()));
        said(&dir, "s1", "Shipped.");
        m.record_event(hook(&capitao, AgentStatus::Done));
        wait_for("the report", || relayed.lock().unwrap().len() == 2);
        let report = |text: &str, answers: Value| json!({ "cmd": "relay", "request_id": "report:*", "to": { "kind": "orchestrator", "report": { "machine": "", "machine_name": host, "project": "site", "path": path, "tab": capitao, "text": text, "answers": answers } } });
        let mut asked = report("Which form?", json!([]));
        asked["to"]["report"].as_object_mut().unwrap().remove("answers");
        assert_eq!(relays(), [asked, report("Shipped.", json!(["r-1"]))]);
        assert_eq!(m.projects_overview().map(|o| o.projects.len()), Ok(1), "the hub's refusal did not fall back to this machine");

        m.set_global_settings(global::Settings { reports: global::ReportPolicy::ToolOnly, ..global::Settings::default() }).unwrap();
        let api_path = api.display().to_string();
        let other = enqueue(&m, &api_path, "Bump the API.", "r-2").unwrap()["tab"].as_str().unwrap().to_owned();
        started(&claude, &other);
        m.record_event(hook(&other, AgentStatus::Done));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(relayed.lock().unwrap().len(), 2, "a tool-only Capitão reported by itself");
        assert_eq!(m.sent.lock().unwrap().find("r-2").map(|s| s.progress), Some(Progress::Answered));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    fn append_transcript(dir: &Path, session: &str, lines: &[Value]) {
        use std::io::Write;
        let folder = dir.join("config/accounts/work/projects/-site");
        std::fs::create_dir_all(&folder).unwrap();
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(folder.join(format!("{session}.jsonl"))).unwrap();
        file.write_all(lines.iter().map(|l| format!("{l}\n")).collect::<String>().as_bytes()).unwrap();
    }

    /// A turn's lines: the prompt, then the reply, or a tool call when it wrote no text.
    #[cfg(unix)]
    fn turn_lines(n: u32, prompt: &str, reply: Option<&str>) -> Vec<Value> {
        let at = format!("2026-11-09T16:{n:02}:00Z");
        let said = match reply {
            Some(text) => json!({ "type": "text", "text": text }),
            None => json!({ "type": "tool_use", "id": format!("t{n}"), "name": "Bash", "input": { "command": "ls" } }),
        };
        vec![
            json!({ "type": "user", "uuid": format!("u{n}"), "timestamp": at, "message": { "content": prompt } }),
            json!({ "type": "assistant", "uuid": format!("a{n}"), "timestamp": at, "message": { "content": [said] } }),
        ]
    }

    #[test]
    #[cfg(unix)]
    fn each_report_reaches_the_global_orchestrator_once_however_long_the_capitaos_transcript() {
        let dir = temp("report-once");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let (claude, sock) = (recording_claude(&dir), dir.join("pinned.sock"));
        let m = work_machine(&dir, &claude, &sock);
        m.set_global_settings(global::Settings { host: true, ..global::Settings::default() }).unwrap();
        let path = m.open_project(project.clone()).display().to_string();
        m.set_project_account(&path, Some("work".into())).unwrap();

        // The transcript passed one 8 MiB page long ago, and its first page ends on an old reply.
        let padding = "x".repeat(64 << 10);
        let mut old = turn_lines(0, "Earlier.", Some("OLD reply."));
        old.extend((0..140).map(|i| json!({ "type": "user", "uuid": format!("p{i}"), "timestamp": "2026-11-09T15:00:00Z", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t0", "content": padding }] } })));
        append_transcript(&dir, "s1", &old);

        let capitao = idle_captain(&m, &path, &dir.join("capitao.bin"));
        let global = m.call(Call::OpenGlobalOrchestrator).unwrap().as_str().unwrap().to_owned();
        let inbox = dir.join("global.bin");
        let (seen, sink) = viewer();
        m.spawn(&global, sh(&format!("stty raw -echo; echo ready; exec cat > '{}'", inbox.display())), pty::size(80, 24)).unwrap();
        m.attach(&global, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        let global_idle = || m.record_event(AgentEvent { session: Some("g1".into()), ..hook(&global, AgentStatus::Done) });
        global_idle();
        let reports = || -> Vec<String> {
            read_pty(&inbox).split("\x1b[200~").skip(1).map(|pasted| pasted.split_once("]: ").unwrap().1.trim_end_matches("\x1b[201~\r").to_owned()).collect()
        };
        let mut n = 0;
        let mut turn = |request_id: Option<&str>, reply: Option<&str>| {
            n += 1;
            let prompt = match request_id {
                Some(id) => {
                    assert_eq!(enqueue(&m, &path, id, id).unwrap()["outcome"], "entregue");
                    format!("[Orquestrador] {id}")
                }
                None => "Felipe asks directly.".to_owned(),
            };
            m.record_event(hook(&capitao, AgentStatus::Working));
            append_transcript(&dir, "s1", &turn_lines(n, &prompt, reply));
            m.record_event(hook(&capitao, AgentStatus::Done));
        };

        turn(Some("r-1"), Some("Reply one."));
        wait_for("the first report", || reports().len() == 1);
        m.record_event(hook(&capitao, AgentStatus::Done));
        global_idle();
        turn(None, Some("Reply two."));
        turn(Some("r-3"), None);
        wait_for("the second report", || reports().len() == 2);
        global_idle();
        turn(Some("r-4"), Some("Reply four."));
        wait_for("the third report", || reports().len() == 3);
        global_idle();

        let delivered = m.sent.lock().unwrap().in_flight(&m.global_folder());
        let first = CaptainReport { machine: String::new(), machine_name: crate::account::hostname(), project: "site".into(), path: path.clone(), tab: capitao.clone(), text: "Reply one.".into(), answers: vec!["r-1".into()] };
        let retried = m.call(Call::OrchestratorReport { request_id: delivered[0].request_id.clone(), report: first });
        assert_eq!(retried, Ok(json!({ "outcome": "entregue", "tab": global })), "a retry answers the first outcome");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(reports(), ["Reply one.", "(no reply)", "Reply four."], "each turn's own reply, each once");
        assert_eq!(m.sent.lock().unwrap().in_flight(&m.global_folder()).len(), 3);
        let read: Value = serde_json::from_str(&m.read_captain(&path, None).unwrap()).unwrap();
        assert_eq!(read["conversation"]["items"].as_array().unwrap().last().unwrap()["text"], "Reply four.", "read_captain reads the end of the transcript");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_turn_that_ends_asking_the_user_waits_on_them_until_the_next_turn_and_across_a_restart() {
        let dir = temp("waits-question");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let (claude, sock) = (recording_claude(&dir), dir.join("pinned.sock"));
        let m = work_machine(&dir, &claude, &sock);
        let path = m.open_project(project.clone()).display().to_string();
        m.set_project_account(&path, Some("work".into())).unwrap();
        let tab = m.open_tab(Some(path.clone()), false).unwrap();
        let waiting = |m: &Machine| m.waiting().unwrap().into_iter().map(|w| (w.kind, w.message)).collect::<Vec<_>>();
        let asks = |question: &str| vec![(global::WaitingKind::NeedsInput, Some(question.to_owned()))];

        m.record_event(hook(&tab, AgentStatus::Working));
        append_transcript(&dir, "s1", &turn_lines(1, "Merge the fix.", Some("Merged. Quer que eu faça a release?")));
        m.record_event(hook(&tab, AgentStatus::Done));
        assert_eq!(waiting(&m), asks("Quer que eu faça a release?"));
        m.record_event(AgentEvent { message: Some("Claude is waiting for your input".into()), ..hook(&tab, AgentStatus::NeedsInput) });
        assert_eq!(waiting(&m), asks("Quer que eu faça a release?"), "Claude Code's idle notice a minute later");

        m.record_event(hook(&tab, AgentStatus::Working));
        assert_eq!(waiting(&m), [], "the user answered");
        append_transcript(&dir, "s1", &turn_lines(2, "Sim.", Some("Released v0.17.21.")));
        m.record_event(hook(&tab, AgentStatus::Done));
        assert_eq!(waiting(&m), [], "a turn that asked nothing");

        m.record_event(hook(&tab, AgentStatus::Working));
        append_transcript(&dir, "s1", &turn_lines(3, "And #89?", Some("O #89 conflita com o main. Fecho o #89?")));
        m.record_event(hook(&tab, AgentStatus::Done));
        drop(m);
        let restarted = work_machine(&dir, &claude, &sock);
        assert_eq!(restarted.restore_tabs().len(), 1);
        assert_eq!(waiting(&restarted), asks("Fecho o #89?"), "the question outlives a restart");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_capitao_that_claude_code_calls_idle_after_its_turn_is_free_and_takes_a_message_now() {
        let dir = temp("enqueue-idle-notice");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let file = dir.join("typed.bin");
        let m = Machine::new(launch(), Some(dir.join("config")), None, |_| {});
        let path = project.display().to_string();
        let tab = idle_captain(&m, &path, &file);
        let state = || global::CaptainState::of(Some(&m.view(&tab).unwrap()));

        m.record_event(hook(&tab, AgentStatus::NeedsInput));
        assert_eq!(state(), global::CaptainState::Idle, "the idle notification a minute after the turn");
        assert_eq!(enqueue(&m, &path, "Ship the form.", "r-1"), Ok(json!({ "outcome": "entregue", "tab": tab })));
        assert_eq!(typed(&file, b"\x1b[200~[Orchestrator] Ship the form.\x1b[201~\r"), b"\x1b[200~[Orchestrator] Ship the form.\x1b[201~\r");

        m.record_event(hook(&tab, AgentStatus::Working));
        m.record_event(hook(&tab, AgentStatus::NeedsInput));
        assert_eq!(state(), global::CaptainState::Working, "a permission prompt mid-turn");
        assert_eq!(enqueue(&m, &path, "Then the footer.", "r-2"), Ok(json!({ "outcome": "na_fila", "tab": tab, "position": 1 })));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn the_global_orchestrators_folder_gets_no_capitao_and_its_tab_starts_no_worker() {
        let dir = temp("global-nocap");
        let (sock, config) = (dir.join("pinned.sock"), dir.join("config"));
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, dir.join("missing-claude"))]), sock: sock.clone(), ..launch() }, Some(config.clone()), None, |_| {});
        m.listen(|_| Err("not the machine's".into())).unwrap();
        let global = m.call(Call::OpenGlobalOrchestrator).unwrap().as_str().unwrap().to_owned();
        let folder = config.join("home/.pinned/orquestrador").display().to_string();

        assert_eq!(enqueue(&m, &folder, "Ship it.", "r-1"), Err(format!("{folder} is the global orchestrator's folder, which has no Capitão")));
        assert_eq!(m.list_projects().len(), 0, "the global folder became a project");
        assert_eq!(m.tabs().iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [global.as_str()], "a Capitão opened in the global folder");

        let request = crate::delegate::parse_request(&json!({ "task": "look" })).unwrap();
        let delegate = serde_json::to_string(&Message::Delegate { tab: Some(global.clone()), task: None, tool: None, request: Box::new(request) }).unwrap();
        assert_eq!(ask(&sock, &delegate), Err("the global orchestrator does no project work: hand it to the project's Capitão with send_to_captain".into()));
        assert!(m.delegates.running().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn global_orchestrator_settings_are_saved_on_the_machine() {
        let dir = temp("global-settings");
        let m = Machine::new(launch(), Some(dir.clone()), None, |_| {});
        assert_eq!(m.call(Call::GlobalOrchestratorSettings), Ok(json!({ "name": "Orchestrator", "host": false, "busy": "queue", "reports": "auto", "group_reports": true, "read_aloud": false })));
        let settings = global::Settings { name: " Almirante ".into(), host: true, busy: BusyPolicy::Refuse, reports: global::ReportPolicy::ToolOnly, group_reports: false, read_aloud: true };
        let saved = json!({ "name": "Almirante", "host": true, "busy": "refuse", "reports": "tool_only", "group_reports": false, "read_aloud": true });
        assert_eq!(m.call(Call::SetGlobalOrchestratorSettings { settings }), Ok(saved.clone()));
        let again = Machine::new(launch(), Some(dir.clone()), None, |_| {});
        assert_eq!(again.call(Call::GlobalOrchestratorSettings), Ok(saved));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn an_orchestrator_cannot_edit_and_starts_with_one_prompt() {
        let dir = temp("orchestrator-args");
        let project = dir.join("app");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let claude = recording_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(dir.join("config")), None, |_| {});
        m.call(Call::DeviceAgents { allow: Some(true) }).unwrap();
        let path = project.display().to_string();
        let tab = m.call(Call::OpenOrchestrator { path: path.clone() }).unwrap().as_str().unwrap().to_owned();
        m.start_claude(&tab, 80, 24).unwrap();
        let args = started(&claude, &tab);
        let prompt = format!("{}\n\n{}\n\n{}\n\n{}\n\n{}", crate::device::AGENT_PROMPT, Role::Orchestrator.prompt(), taste::Taste::default().brief(), taste::Taste::default().limits().hint(), global::captain_prompt("Orchestrator", global::ReportPolicy::Auto));
        assert_eq!(args, [&path, "orchestrator", &path, "--settings", "s", "--plugin-dir", "p", "--append-system-prompt", &prompt]);
        assert_eq!(m.view(&tab).unwrap().role, Some(Role::Orchestrator));
        let review = project.join(".claude/worktrees/pinned-1/src").display().to_string();
        m.record_event(AgentEvent { status: AgentStatus::Working, cwd: Some(review), message: None, tab: Some(tab.clone()), session: Some("s1".into()), title: None, resume_at: None, prompt: None, snapshot: None, permission_mode: None, hook_feedback: false });
        assert_eq!(m.view(&tab).unwrap().cwd, path, "reviewing a worker's worktree moved the orchestrator into it");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_capitao_starts_with_its_projects_taste_and_the_global_orchestrator_with_its_own() {
        let dir = temp("taste-brief");
        let (site, config) = (dir.join("site"), dir.join("config"));
        std::fs::create_dir_all(&site).unwrap();
        git(&site, &["init", "-q"]);
        git(&site, &["remote", "add", "origin", "git@github.com:acme/site.git"]);
        let claude = recording_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(config.clone()), None, |_| {});
        let account = json!({
            "orchestrator": { "strategy": "strongest_first", "ladder": [{ "model": "claude-opus-5-5" }], "parallel": 6, "autonomy": "report_at_end", "review_diffs": false, "second_opinion": true },
            "projects": { "github.com/acme/site": { "parallel": 5, "autonomy": "report_at_end", "context": ["Deploys go through the VPS."], "tier": "mid", "budget_cents": 200 } },
        });
        let asked = Arc::new(AtomicU32::new(0));
        let (answer, count) = (account.clone(), asked.clone());
        m.set_hub(Arc::new(move |call| match call {
            HubCall::GetTastes => {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(answer.clone())
            }
            _ => Err("not this test's".into()),
        }));
        let path = site.display().to_string();
        let tab = m.call(Call::OpenOrchestrator { path: path.clone() }).unwrap().as_str().unwrap().to_owned();
        m.start_claude(&tab, 80, 24).unwrap();
        let site_brief = "Run up to 5 workers at once.
Start each worker on glm-5.3, and when it fails twice move it one rung up the ladder: claude-sonnet-5-5, then claude-opus-5-5.
Send hard work, such as architecture calls or a bug whose cause is unknown, straight to claude-opus-5-5.
Tell the user only at the end, with the result.
Review each worker's diff before you accept its work.
Deploys go through the VPS.";
        let cap = "pinned enforces these delegation limits on delegate_task: at most 5 workers at once, models up to the mid tier, effort up to max, 480 minutes per worker, US$ 2.00 per worker. A call past them is refused, and a worker past its time or budget is stopped; if a limit blocks you, tell the user to raise it in Settings › Way of working rather than working around it. The tier covers the model each worker runs on, not the models it picks for its own subagents.";
        let capitao = format!("{}\n\n{site_brief}\n\n{cap}\n\n{}", Role::Orchestrator.prompt(), global::captain_prompt("Orchestrator", global::ReportPolicy::Auto));
        assert_eq!(started(&claude, &tab), [&path, "orchestrator", &path, "--settings", "s", "--plugin-dir", "p", "--append-system-prompt", &capitao]);
        assert_eq!(asked.load(Ordering::SeqCst), 1, "the Capitão's start asked the hub once");
        let kept: Value = serde_json::from_slice(&std::fs::read(config.join("taste.json")).unwrap()).unwrap();
        assert_eq!(kept["projects"], account["projects"], "the hub's answer is kept for when it can't answer");

        m.set_hub(Arc::new(|_| Err("not connected to the hub".into())));
        m.relaunch(&tab, 80, 24).unwrap();
        assert_eq!(started(&claude, &tab).last(), Some(&capitao), "offline, the Capitão starts with the taste kept from the hub");

        let global_tab = m.call(Call::OpenGlobalOrchestrator).unwrap().as_str().unwrap().to_owned();
        m.start_claude(&global_tab, 80, 24).unwrap();
        let orchestrator_brief = "Run up to 6 workers at once.
Start each worker on claude-opus-5-5 directly, without trying a cheaper model first.
Send hard work, such as architecture calls or a bug whose cause is unknown, straight to claude-opus-5-5.
Tell the user only at the end, with the result.
Have another model review a risky change before it goes ahead.";
        let global_prompt = format!("{}\n\n{orchestrator_brief}", global::prompt("Orchestrator", &crate::account::hostname()));
        assert_eq!(started(&claude, &global_tab).last(), Some(&global_prompt));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_machines_own_taste_goes_to_the_account_before_its_first_read_and_again_on_each_connect() {
        let dir = temp("taste-share");
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("orchestration.json"), r#"{"style":"quality","max_workers":4}"#).unwrap();
        let m = Machine::new(launch(), Some(config.clone()), None, |_| {});
        let calls = Arc::new(Mutex::new(vec![]));
        let seen = calls.clone();
        m.set_hub(Arc::new(move |call| {
            seen.lock().unwrap().push(serde_json::to_value(&call).unwrap()["cmd"].clone());
            match call {
                HubCall::ImportTastes { tastes } => Ok(json!(tastes.captains.profile() == taste::Profile::Aggressive)),
                HubCall::GetTastes => Ok(json!({})),
                _ => Err("not this test's".into()),
            }
        }));
        assert_eq!(m.tastes(), Tastes::default(), "the account's answer wins");
        assert_eq!(m.tastes(), Tastes::default());
        assert_eq!(*calls.lock().unwrap(), [json!("import_tastes"), json!("get_tastes"), json!("get_tastes")], "the seeded style went up once, before the first read");
        m.share_tastes_again();
        assert_eq!(calls.lock().unwrap().len(), 3, "the account's defaults, kept in taste.json, are not offered as this machine's own");

        std::fs::write(config.join("taste.json"), serde_json::to_vec(&Tastes { captains: taste::Profile::Economical.preset().unwrap(), ..Tastes::default() }).unwrap()).unwrap();
        m.share_tastes_again();
        assert_eq!(calls.lock().unwrap().last(), Some(&json!("import_tastes")), "a new link offers this machine's own taste again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_silent_hub_holds_a_sessions_start_only_briefly_and_its_late_answer_is_kept() {
        let dir = temp("taste-slow");
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        let m = Machine::new(launch(), Some(config.clone()), None, |_| {});
        let kept = Tastes { orchestrator: taste::Profile::Economical.preset().unwrap(), ..Tastes::default() };
        taste::save(&kept, Some(&config)).unwrap();
        let late = Tastes { orchestrator: taste::Profile::Aggressive.preset().unwrap(), ..Tastes::default() };
        let answer = serde_json::to_value(&late).unwrap();
        m.set_hub(Arc::new(move |call| match call {
            HubCall::GetTastes => {
                std::thread::sleep(Duration::from_secs(2));
                Ok(answer.clone())
            }
            _ => Err("not this test's".into()),
        }));
        let asked = Instant::now();
        assert_eq!(m.tastes(), kept, "the start went on with the taste kept in taste.json");
        assert!(asked.elapsed() < Duration::from_millis(1500), "the start waited {:?}", asked.elapsed());
        wait_for("the late answer to be kept", || taste::load(Some(&config)) == Some(late.clone()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_signed_out_machine_changes_its_own_taste_file() {
        let dir = temp("taste-local");
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("orchestration.json"), r#"{"style":"economical","max_workers":4}"#).unwrap();
        let m = Machine::new(launch(), Some(config.clone()), None, |_| {});
        assert_eq!(m.local_tastes().captains.profile(), taste::Profile::Economical, "the old cost style seeded the Capitães");
        let site = TasteScope::Project { repo: "github.com/acme/site".into() };
        let set = m.set_local_taste(&site, TastePatch { parallel: Some(5), review_diffs: Some(false), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&set.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 5, "review_diffs": false } }));
        let reset = m.reset_local_taste_field(&site, TasteField::ReviewDiffs).unwrap();
        assert_eq!(serde_json::to_value(&reset.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 5 } }));
        assert_eq!(m.local_tastes(), reset);
        assert_eq!(m.local_tastes().captains.profile(), taste::Profile::Economical, "moving the worker cap kept the seeded taste");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_taste_chat_starts_on_its_scope_and_its_session_alone_changes_the_taste_file() {
        let dir = temp("taste-chat");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        let (sock, config) = (dir.join("pinned.sock"), dir.join("config"));
        let claude = recording_claude(&dir);
        let events = Arc::new(Mutex::new(vec![]));
        let seen = events.clone();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), sock: sock.clone(), ..launch() }, Some(config.clone()), None, move |e| {
            seen.lock().unwrap().push(serde_json::to_value(e).unwrap())
        });
        m.listen(|_| Err("not the machine's".into())).unwrap();
        let path = m.open_project(project.clone()).display().to_string();

        let site = TasteScope::Project { repo: "github.com/acme/site".into() };
        let chat = m.call(Call::OpenTasteChat { scope: site.clone(), path: Some(path.clone()) }).unwrap().as_str().unwrap().to_owned();
        let view = m.view(&chat).unwrap();
        assert_eq!((view.cwd.as_str(), view.taste_chat, view.role, view.running), (path.as_str(), true, None, false));
        m.start_claude(&chat, 80, 24).unwrap();
        let args = started(&claude, &chat);
        assert!(args.iter().any(|a| a.starts_with("Help me adjust the way of working of the Captain of the project github.com/acme/site in pinned (scope project:github.com/acme/site).")), "{args:?}");

        let tool = |tab: &str, tool: &str| ask(&sock, &format!(r#"{{"kind":"project","tab":"{tab}","tool":{{{tool}}}}}"#));
        let set: Value = serde_json::from_str(&tool(&chat, r#""name":"taste_set","scope":"project:github.com/acme/site","patch":{"parallel":2}"#).unwrap()).unwrap();
        assert_eq!((&set["scope"], &set["own"], &set["effective"]["parallel"], &set["inherits_from"]), (&json!("project:github.com/acme/site"), &json!({ "parallel": 2 }), &json!(2), &json!("captains")));
        assert!(set["brief"].as_str().unwrap().starts_with("Run up to 2 workers at once.\n"), "{set}");
        let written: Value = serde_json::from_slice(&std::fs::read(config.join("taste.json")).unwrap()).unwrap();
        assert_eq!(written["projects"], json!({ "github.com/acme/site": { "parallel": 2 } }), "the page's own read, get_tastes, reads this file");
        assert_eq!(m.local_tastes().resolve(&site).parallel, 2);
        assert_eq!(events.lock().unwrap().iter().filter(|e| e["kind"] == "tastes-changed").count(), 1);
        let got: Value = serde_json::from_str(&tool(&chat, r#""name":"taste_get","scope":"project:github.com/acme/site""#).unwrap()).unwrap();
        assert_eq!(got, set);

        let reset: Value = serde_json::from_str(&tool(&chat, r#""name":"taste_reset","scope":"project:github.com/acme/site","field":"parallel""#).unwrap()).unwrap();
        assert_eq!((&reset["own"], &reset["effective"]["parallel"]), (&json!({}), &json!(3)));
        assert_eq!(tool(&chat, r#""name":"taste_set","scope":"captains","patch":{"parallel":101}"#), Err("parallel workers go from 1 to 100".into()));
        assert_eq!(tool(&chat, r#""name":"taste_get","scope":"site""#), Err("unknown scope \"site\": use orchestrator, captains or project:<repo>, as in project:github.com/acme/site".into()));

        let plain = m.open_tab(Some(path.clone()), false).unwrap();
        assert_eq!(tool(&plain, r#""name":"taste_set","scope":"captains","patch":{"parallel":1}"#), Err("only a thread opened with Chat to adjust on the Way of working page has this tool".into()));
        assert_eq!(m.local_tastes().captains.parallel, 3, "a plain thread changed the taste");

        let captains = m.call(Call::OpenTasteChat { scope: TasteScope::Captains, path: None }).unwrap().as_str().unwrap().to_owned();
        assert_eq!(m.view(&captains).unwrap().cwd, m.scratch.display().to_string(), "a scope with no project opens without one");
        assert_eq!(m.call(Call::OpenTasteChat { scope: TasteScope::Captains, path: Some(dir.join("gone").display().to_string()) }), Err(format!("{} is not a folder on this machine", dir.join("gone").display())));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_projects_orchestrator_opens_once_and_only_under_claude_code() {
        let root = temp("orchestrator-once");
        let m = accounts_machine(&root);
        let (app, api, side) = (root.join("app"), root.join("api"), root.join("side"));
        for p in [&app, &api, &side] {
            std::fs::create_dir_all(p).unwrap();
        }
        let open = |p: &Path| m.call(Call::OpenOrchestrator { path: p.display().to_string() });
        let app_tab = open(&app).unwrap();
        assert_eq!(open(&app).unwrap(), app_tab, "a second orchestrator opened for the project");
        let api_tab = open(&api).unwrap();
        assert_ne!(api_tab, app_tab);
        assert_eq!(m.view(api_tab.as_str().unwrap()).unwrap().opened, 2);
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }).unwrap();
        m.open_project(side.clone());
        m.call(Call::SetProjectAccount { path: side.display().to_string(), account: Some("side".into()) }).unwrap();
        assert_eq!(open(&side), Err("account side runs Codex, which can't be a project's orchestrator".into()));
        assert_eq!(open(&root.join("gone")), Err(format!("{} is not a folder on this machine", root.join("gone").display())));
        assert_eq!(m.tabs().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_resumed_orchestrator_starts_with_its_role_again() {
        let dir = temp("orchestrator-resume");
        let (project, config) = (dir.join("app"), dir.join("config"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        let path = project.display().to_string();
        std::fs::write(config.join("projects.json"), json!([{ "path": path }]).to_string()).unwrap();
        let env = json!([["PINNED_ROLE", "orchestrator"], ["PINNED_PROJECT", path]]);
        std::fs::write(config.join("sessions.json"), json!([{ "cwd": path, "session": "s1", "env": env }]).to_string()).unwrap();
        let claude = recording_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(config), None, |_| {});
        let tabs = m.restore_tabs();
        m.start_claude(&tabs[0], 80, 24).unwrap();
        let prompt = format!("{}\n\n{}\n\n{}\n\n{}", Role::Orchestrator.prompt(), taste::Taste::default().brief(), taste::Taste::default().limits().hint(), global::captain_prompt("Orchestrator", global::ReportPolicy::Auto));
        assert_eq!(started(&claude, &tabs[0]), [&path, "orchestrator", &path, "--settings", "s", "--plugin-dir", "p", "--resume", "s1", "--append-system-prompt", prompt.as_str()]);
        assert_eq!(m.call(Call::OpenOrchestrator { path }), Ok(json!(tabs[0])), "the restored orchestrator is not the project's");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A Claude Code that answers its sign-in check, records each tab's folder, role, project,
    /// account home and arguments, then stays up as that tab's session.
    #[cfg(unix)]
    fn thread_claude(dir: &Path) -> PathBuf {
        let claude = dir.join("claude");
        let script = "#!/bin/sh\nif [ \"$1 $2\" = \"auth status\" ]; then echo '{\"loggedIn\":true,\"email\":\"ana@acme.dev\"}'; exit 0; fi\nprintf '%s\\0' \"$(pwd -P)\" \"$PINNED_ROLE\" \"$PINNED_PROJECT\" \"$CLAUDE_CONFIG_DIR\" \"$@\" > \"$0.tmp.$PINNED_TAB\" && mv \"$0.tmp.$PINNED_TAB\" \"$0.$PINNED_TAB\"\nexec sleep 30\n";
        std::fs::write(&claude, script).unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        claude
    }

    #[cfg(unix)]
    fn delegate_task(m: &Machine, from: &str, args: Value) -> Value {
        let request = crate::delegate::parse_request(&args).unwrap();
        serde_json::from_str(&m.start_delegate(Some(from.to_owned()), None, None, request).unwrap()).unwrap()
    }

    /// A Captain of the project `app` on a machine whose Captains have `limits`, with the `work`
    /// account. Returns the machine, its folder and the Captain's tab.
    #[cfg(unix)]
    fn limited_captain(name: &str, limits: TastePatch) -> (Arc<Machine>, PathBuf, String) {
        let dir = temp(name);
        let project = dir.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, thread_claude(&dir))]), ..launch() }, Some(dir.join("config")), None, |_| {});
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        m.set_local_taste(&TasteScope::Captains, limits).unwrap();
        let path = m.open_project(project).display().to_string();
        let captain = m.open_orchestrator(&path).unwrap();
        (m, dir, captain)
    }

    #[cfg(unix)]
    fn refused(m: &Machine, from: &str, args: Value) -> String {
        m.start_delegate(Some(from.to_owned()), None, None, crate::delegate::parse_request(&args).unwrap()).unwrap_err()
    }

    #[cfg(unix)]
    fn work(model: Option<&str>, effort: Option<&str>) -> Value {
        json!({ "task": "Fix the parser.", "title": "Parser", "target": { "providerInstanceId": "work", "model": model, "options": { "effort": effort } } })
    }

    #[test]
    #[cfg(unix)]
    fn a_call_above_the_tier_or_the_most_effort_is_refused_and_one_without_effort_gets_the_default() {
        let limits = TastePatch { tier: Some(taste::Tier::Mid), default_effort: Some(taste::Effort::Low), max_effort: Some(taste::Effort::High), ..TastePatch::default() };
        let (m, dir, captain) = limited_captain("limits-tier", limits);
        assert_eq!(
            refused(&m, &captain, work(Some("claude-opus-5-5"), None)),
            "claude-opus-5-5 is a high-tier model; this project allows up to mid. Pick a mid model or raise Tier in Settings › Way of working"
        );
        assert_eq!(
            refused(&m, &captain, work(None, None)),
            "name a model: this project allows models up to the mid tier, and a worker without one runs on its account's default. Pick a mid model from orchestrator_capabilities, or raise Tier in Settings › Way of working"
        );
        assert_eq!(
            refused(&m, &captain, work(Some("claude-sonnet-5-5"), Some("max"))),
            "effort max is above high, the most this project allows. Ask for high or less, or raise Max effort in Settings › Way of working"
        );
        assert_eq!(m.delegates.working_cwds(), Vec::<PathBuf>::new(), "a refused call starts nothing");

        let child = delegate_task(&m, &captain, work(Some("claude-sonnet-5-5"), None))["tabId"].as_str().unwrap().to_owned();
        let args = started(&dir.join("claude"), &child);
        assert!(args.windows(2).any(|pair| pair == ["--effort", "low"]), "the default effort: {args:?}");
        let child = delegate_task(&m, &captain, work(Some("claude-haiku-5-5"), Some("high")))["tabId"].as_str().unwrap().to_owned();
        let args = started(&dir.join("claude"), &child);
        assert!(args.windows(2).any(|pair| pair == ["--effort", "high"]), "the effort asked for, at the most: {args:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn the_tier_cap_holds_a_model_claudes_menu_added_at_its_familys_tier() {
        let (m, dir, captain) = limited_captain("limits-learned", TastePatch { tier: Some(taste::Tier::Low), ..TastePatch::default() });
        let menu = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/web-sweep/screens/claude-2.1.321-model-menu.txt")).unwrap();
        let lines = menu.replace("Fable 5.2", "Fable 5.3").replace("Haiku 5.5", "Haiku 6").lines().map(String::from).collect();
        m.call(Call::LearnModelMenu { lines }).unwrap();
        assert_eq!(
            refused(&m, &captain, work(Some("claude-fable-5-3"), None)),
            "claude-fable-5-3 is a mid-tier model; this project allows up to low. Pick a low model or raise Tier in Settings › Way of working"
        );
        let child = delegate_task(&m, &captain, work(Some("claude-haiku-6"), None))["tabId"].as_str().unwrap().to_owned();
        let args = started(&dir.join("claude"), &child);
        assert!(args.windows(2).any(|pair| pair == ["--model", "claude-haiku-6"]), "a learned Haiku runs under a low cap: {args:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn the_worker_past_workers_at_once_is_refused_until_one_ends() {
        let (m, dir, captain) = limited_captain("limits-parallel", TastePatch { parallel: Some(2), ..TastePatch::default() });
        let first = delegate_task(&m, &captain, work(Some("claude-sonnet-5-5"), None));
        delegate_task(&m, &captain, work(Some("claude-sonnet-5-5"), None));
        assert_eq!(
            refused(&m, &captain, work(Some("claude-sonnet-5-5"), None)),
            "2 workers already run in this project, the most it allows at once. Wait for one to finish, or raise Workers at once in Settings › Way of working."
        );
        m.task_cancel(Some(captain.clone()), None, first["taskId"].as_str().unwrap(), "not needed").unwrap();
        assert_eq!(delegate_task(&m, &captain, work(Some("claude-sonnet-5-5"), None))["workState"], "working", "an ended worker frees its place");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_worker_past_the_time_limit_is_stopped_and_task_status_says_why() {
        let (m, dir, captain) = limited_captain("limits-time", TastePatch { time_limit_min: Some(30), ..TastePatch::default() });
        let asked = json!({ "task": "Fix the parser.", "title": "Parser", "target": { "providerInstanceId": "work", "model": "claude-sonnet-5-5" }, "limitMs": 7_200_000 });
        let task = delegate_task(&m, &captain, asked)["taskId"].as_str().unwrap().to_owned();
        let status = || serde_json::from_str::<Value>(&m.task_status(Some(captain.clone()), None, &task).unwrap()).unwrap();
        assert_eq!((status()["limitSecs"].as_u64(), status()["budgetSecs"].as_u64()), (Some(1800), Some(900)), "a longer limitMs is cut to the time limit");
        m.delegates.enforce_limits(Instant::now() + Duration::from_secs(1801), None);
        let ended = status();
        assert_eq!((ended["workState"].as_str(), ended["error"].as_str()), (Some("cancelled"), Some("pinned stopped it after 30 min, its limit")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_worker_past_its_budget_is_stopped_and_task_status_shows_what_it_spent() {
        let (m, dir, captain) = limited_captain("limits-budget", TastePatch { budget_cents: Some(50), ..TastePatch::default() });
        let config = dir.join("config");
        std::fs::write(config.join("usage-model-rates.json"), "{}").unwrap();
        std::fs::write(config.join("usage-prices.json"), r#"{"prices":{"claude-sonnet-5-5":{"input":3,"output":15}}}"#).unwrap();
        let reply = delegate_task(&m, &captain, work(Some("claude-sonnet-5-5"), None));
        let (task, child) = (reply["taskId"].as_str().unwrap().to_owned(), reply["tabId"].as_str().unwrap().to_owned());
        let transcripts = config.join("accounts/work/projects/app");
        std::fs::create_dir_all(&transcripts).unwrap();
        let transcript = transcripts.join("s-child.jsonl");
        let spent = |id: &str, output: u64| json!({ "type": "assistant", "requestId": id, "message": { "id": id, "model": "claude-sonnet-5-5", "content": [], "usage": { "input_tokens": 0, "output_tokens": output } } });
        std::fs::write(&transcript, format!("{}\n", spent("m1", 10_000))).unwrap();
        m.record_event(hook_event(&child, "working", &transcript, Some("Parser")));
        let status = || serde_json::from_str::<Value>(&m.task_status(Some(captain.clone()), None, &task).unwrap()).unwrap();

        m.meter_workers();
        let working = status();
        let cents = working["usage"]["cost_usd"].as_f64().map(|usd| (usd * 100.0).round());
        assert_eq!((working["workState"].as_str(), working["usage"]["tokens_out"].as_u64(), cents), (Some("working"), Some(10_000), Some(15.0)), "{working}");
        append(&transcript, &[spent("m2", 30_000)]);
        m.meter_workers();
        let ended = status();
        assert_eq!(
            (ended["workState"].as_str(), ended["error"].as_str(), ended["usage"]["tokens_out"].as_u64()),
            (Some("cancelled"), Some("pinned stopped it after it spent US$ 0.60, its budget of US$ 0.50"), Some(40_000))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_workers_subagents_and_its_next_session_count_toward_its_budget() {
        let (m, dir, captain) = limited_captain("limits-subagents", TastePatch { budget_cents: Some(50), ..TastePatch::default() });
        let config = dir.join("config");
        std::fs::write(config.join("usage-model-rates.json"), "{}").unwrap();
        std::fs::write(config.join("usage-prices.json"), r#"{"prices":{"claude-sonnet-5-5":{"input":3,"output":15}}}"#).unwrap();
        let reply = delegate_task(&m, &captain, work(Some("claude-sonnet-5-5"), None));
        let (task, child) = (reply["taskId"].as_str().unwrap().to_owned(), reply["tabId"].as_str().unwrap().to_owned());
        let transcripts = config.join("accounts/work/projects/app");
        std::fs::create_dir_all(transcripts.join("s-child/subagents")).unwrap();
        let spent = |id: &str, output: u64| json!({ "type": "assistant", "requestId": id, "message": { "id": id, "model": "claude-sonnet-5-5", "content": [], "usage": { "input_tokens": 0, "output_tokens": output } } });
        let transcript = transcripts.join("s-child.jsonl");
        std::fs::write(&transcript, format!("{}\n", spent("m1", 10_000))).unwrap();
        std::fs::write(transcripts.join("s-child/subagents/agent-a1.jsonl"), format!("{}\n", spent("m2", 20_000))).unwrap();
        m.record_event(hook_event(&child, "working", &transcript, Some("Parser")));
        let status = || serde_json::from_str::<Value>(&m.task_status(Some(captain.clone()), None, &task).unwrap()).unwrap();

        m.meter_workers();
        let working = status();
        let cents = working["usage"]["cost_usd"].as_f64().map(|usd| (usd * 100.0).round());
        assert_eq!((working["workState"].as_str(), working["usage"]["tokens_out"].as_u64(), cents), (Some("working"), Some(30_000), Some(45.0)), "the subagent's tokens count: {working}");

        let next = transcripts.join("s-next.jsonl");
        std::fs::write(&next, format!("{}\n", spent("m3", 10_000))).unwrap();
        let cleared = json!({ "session_id": "s-next", "transcript_path": next, "prompt": null });
        m.record_event(AgentEvent { tab: Some(child.clone()), ..hooks::event_from(Some("working"), &cleared.to_string()).unwrap() });
        m.meter_workers();
        let ended = status();
        assert_eq!(
            (ended["workState"].as_str(), ended["error"].as_str(), ended["usage"]["tokens_out"].as_u64()),
            (Some("cancelled"), Some("pinned stopped it after it spent US$ 0.60, its budget of US$ 0.50"), Some(40_000)),
            "/clear starts a new session, not a new budget"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_retried_call_gets_its_task_back_at_the_workers_at_once_limit_and_the_users_threads_do_not_count() {
        let (m, dir, captain) = limited_captain("limits-retry", TastePatch { parallel: Some(1), ..TastePatch::default() });
        for _ in 0..2 {
            m.open_tab(Some(dir.join("app").display().to_string()), false).unwrap();
        }
        let asked = json!({ "task": "Fix the parser.", "title": "Parser", "target": { "providerInstanceId": "work", "model": "claude-sonnet-5-5" }, "clientRequestId": "r1" });
        let first = delegate_task(&m, &captain, asked.clone());
        assert_eq!(first["workState"], "working", "the user's own threads in the project are not workers");
        assert_eq!(delegate_task(&m, &captain, asked)["taskId"], first["taskId"], "the retry is the task already started");
        assert_eq!(
            refused(&m, &captain, work(Some("claude-sonnet-5-5"), None)),
            "1 worker already runs in this project, the most it allows at once. Wait for one to finish, or raise Workers at once in Settings › Way of working."
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_workers_last_turn_is_metered_before_its_task_ends() {
        let (m, dir, captain) = limited_captain("limits-last-turn", TastePatch::default());
        let config = dir.join("config");
        std::fs::write(config.join("usage-model-rates.json"), "{}").unwrap();
        std::fs::write(config.join("usage-prices.json"), r#"{"prices":{"claude-sonnet-5-5":{"input":3,"output":15}}}"#).unwrap();
        let reply = delegate_task(&m, &captain, work(Some("claude-sonnet-5-5"), None));
        let (task, child) = (reply["taskId"].as_str().unwrap().to_owned(), reply["tabId"].as_str().unwrap().to_owned());
        let transcripts = config.join("accounts/work/projects/app");
        std::fs::create_dir_all(&transcripts).unwrap();
        let transcript = transcripts.join("s-child.jsonl");
        std::fs::write(&transcript, "").unwrap();
        m.record_event(hook_event(&child, "working", &transcript, Some("Parser")));
        append(&transcript, &[json!({ "type": "assistant", "requestId": "r1", "message": { "id": "m1", "model": "claude-sonnet-5-5", "content": [{ "type": "text", "text": "Fixed." }], "usage": { "input_tokens": 1000, "output_tokens": 2000 } } })]);
        m.record_event(hook_event(&child, "done", &transcript, None));
        let status: Value = serde_json::from_str(&m.task_status(Some(captain.clone()), None, &task).unwrap()).unwrap();
        let cents = status["usage"]["cost_usd"].as_f64().map(|usd| (usd * 100.0).round());
        assert_eq!(
            (status["workState"].as_str(), status["usage"]["tokens_in"].as_u64(), status["usage"]["tokens_out"].as_u64(), cents),
            (Some("result_available"), Some(1000), Some(2000), Some(3.0)),
            "the turn that ended the task counts: {status}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_workers_own_worker_is_held_to_the_limits_it_started_under() {
        let (m, dir, captain) = limited_captain("limits-narrow", TastePatch { tier: Some(taste::Tier::Mid), ..TastePatch::default() });
        let worker = delegate_task(&m, &captain, work(Some("claude-sonnet-5-5"), None))["tabId"].as_str().unwrap().to_owned();
        m.set_local_taste(&TasteScope::Captains, TastePatch { tier: Some(taste::Tier::High), ..TastePatch::default() }).unwrap();
        assert_eq!(delegate_task(&m, &captain, work(Some("claude-opus-5-5"), None))["workState"], "working", "the Captain has the raised tier");
        assert_eq!(
            refused(&m, &worker, work(Some("claude-opus-5-5"), None)),
            "claude-opus-5-5 is a high-tier model; this project allows up to mid. Pick a mid model or raise Tier in Settings › Way of working",
            "the worker keeps the tier it started under"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_captains_delegate_task_is_a_worker_thread_on_the_account_model_and_effort_it_names() {
        let dir = temp("child-thread");
        let project = dir.join("app");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        git(&project, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let claude = thread_claude(&dir);
        let config = dir.join("config");
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(config.clone()), None, |_| {});
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        let path = m.open_project(project.clone()).display().to_string();
        let captain = m.open_orchestrator(&path).unwrap();

        let reply = delegate_task(&m, &captain, json!({
            "task": "Add a --json flag to the CLI.",
            "title": "JSON output",
            "target": { "providerInstanceId": "work", "model": "claude-sonnet-5-5", "options": { "effort": "high" } },
            "isolation": "worktree",
        }));
        let (task, child) = (reply["taskId"].as_str().unwrap().to_owned(), reply["tabId"].as_str().unwrap().to_owned());
        assert_eq!((reply["workState"].as_str(), reply["providerInstanceId"].as_str(), reply["model"].as_str()), (Some("working"), Some("work"), Some("claude-sonnet-5-5")));
        let view = m.view(&child).unwrap();
        assert_eq!(
            (view.role, view.spawned_by.as_deref(), view.account.as_str(), view.title.as_deref(), view.running),
            (Some(Role::Worker), Some(captain.as_str()), "work", Some("JSON output"), true),
            "the child is a running worker thread under the Captain"
        );
        let args = started(&claude, &child);
        let tree = PathBuf::from(&args[0]);
        assert_eq!(tree.parent(), Some(project.join(".claude/worktrees").as_path()), "{}", tree.display());
        assert_eq!(reply["cwd"].as_str(), Some(args[0].as_str()));
        assert_eq!(
            args[1..],
            [
                "worker",
                &path,
                &config.join("accounts/work").display().to_string(),
                "--settings",
                "s",
                "--plugin-dir",
                "p",
                "--name",
                "JSON output",
                "--effort",
                "high",
                "--permission-mode",
                "auto",
                "JSON output\n\nAdd a --json flag to the CLI.",
                "--model",
                "claude-sonnet-5-5",
                "--append-system-prompt",
                Role::Worker.prompt(),
            ]
        );

        assert_eq!(m.delegates.working_cwds(), [tree.clone()], "the thread and its task are one worker");
        let checkout = &m.local_overview().projects[0].checkouts[0];
        assert_eq!((checkout.path.as_str(), checkout.workers), (path.as_str(), 1), "projects() counts the child");
        let agents = m.project_tool(&captain, ProjectTool::Agents).unwrap();
        assert_eq!(agents, format!("{child}: JSON output (starting) in {}", tree.display()));
        assert!(m.thread(&captain, 0).unwrap().agents.is_empty(), "the child shows as a thread, not as a card on the Captain");
        let caller = |tab: &str| m.task_status(Some(tab.to_owned()), None, &task).map(|s| serde_json::from_str::<Value>(&s).unwrap());
        assert_eq!(caller(&captain).unwrap()["workState"], "working");

        let transcripts = config.join("accounts/work/projects/app");
        std::fs::create_dir_all(&transcripts).unwrap();
        std::fs::write(transcripts.join("s-child.jsonl"), assistant_line("Added --json; cargo test passes.")).unwrap();
        m.record_event(AgentEvent { cwd: Some(args[0].clone()), session: Some("s-child".into()), ..hook(&child, AgentStatus::Working) });
        m.record_event(AgentEvent { session: Some("s-child".into()), ..hook(&child, AgentStatus::Done) });

        let status = caller(&captain).unwrap();
        assert_eq!(
            (status["workState"].as_str(), status["summary"].as_str(), status["error"].as_str(), status["tabId"].as_str(), status["conversationId"].as_str()),
            (Some("result_available"), Some("Added --json; cargo test passes."), None, Some(child.as_str()), Some("s-child"))
        );
        assert_eq!(status["recentSteps"][0]["text"], "Added --json; cargo test passes.");
        let read = reading::read(&m.view(&child).unwrap(), m.thread(&child, 0).and_then(json), reading::DEFAULT_ITEMS);
        assert_eq!(read["conversation"]["items"][0]["text"], "Added --json; cargo test passes.", "{read}");
        assert_eq!(m.view(&child).unwrap().running, true, "the thread stays open after its task ends");
        assert_eq!(m.delegates.working_cwds(), Vec::<PathBuf>::new(), "an ended task no longer counts toward workers at once");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A Captain's thread child on the `work` account, with its transcript file and its folder.
    #[cfg(unix)]
    fn captain_with_thread_child(name: &str) -> (Arc<Machine>, PathBuf, String, String, String, PathBuf) {
        let dir = temp(name);
        let project = dir.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let claude = thread_claude(&dir);
        let config = dir.join("config");
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(config.clone()), None, |_| {});
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        let path = m.open_project(project).display().to_string();
        let captain = m.open_orchestrator(&path).unwrap();
        let reply = delegate_task(&m, &captain, json!({ "task": "Compare the screens with the boards.", "title": "Board check", "target": { "providerInstanceId": "work" } }));
        let (task, child) = (reply["taskId"].as_str().unwrap().to_owned(), reply["tabId"].as_str().unwrap().to_owned());
        let transcripts = config.join("accounts/work/projects/app");
        std::fs::create_dir_all(&transcripts).unwrap();
        let transcript = transcripts.join("s-child.jsonl");
        std::fs::write(&transcript, json!({ "type": "custom-title", "customTitle": "Board check", "sessionId": "s-child" }).to_string() + "\n").unwrap();
        (m, dir, captain, task, child, transcript)
    }

    #[test]
    #[cfg(unix)]
    fn a_captains_finished_thread_task_reaches_the_hub_with_its_usage_and_the_stats_answer_from_there() {
        let (m, dir, captain, task, child, transcript) = captain_with_thread_child("thread-record");
        let usage = |id: &str| json!({ "type": "assistant", "message": { "id": id, "model": "claude-sonnet-5-5", "content": [{ "type": "text", "text": "Matches the boards." }], "usage": { "input_tokens": 12, "cache_read_input_tokens": 3000, "cache_creation_input_tokens": 400, "output_tokens": 250 } } });
        append(&transcript, &[usage("m1"), usage("m1"), usage("m2")]);
        // An Agent-tool subagent of the worker writes its own transcript beside the worker's.
        let subagents = transcript.with_extension("").join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        std::fs::write(subagents.join("agent-a1.jsonl"), format!("{}\n", usage("m-sub"))).unwrap();
        let rates = json!({ "claude-sonnet-5-5": { "input_cost_per_token": 3e-6, "output_cost_per_token": 15e-6, "cache_read_input_token_cost": 0.3e-6, "cache_creation_input_token_cost": 3.75e-6 } });
        std::fs::write(dir.join("config/usage-model-rates.json"), rates.to_string()).unwrap();
        let kept: Arc<Mutex<Vec<crate::delegate::TaskRecord>>> = Arc::default();
        let hub_records = kept.clone();
        m.set_hub(Arc::new(move |call| match call {
            HubCall::RecordTasks { records } => {
                hub_records.lock().unwrap().extend(records);
                Ok(Value::Null)
            }
            HubCall::DelegationStats { days } => Ok(json!(crate::delegate::aggregate(&hub_records.lock().unwrap(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64, days))),
            _ => Err("not this test's".into()),
        }));
        m.record_event(AgentEvent { session: Some("s-child".into()), ..hook(&child, AgentStatus::Working) });
        m.record_event(AgentEvent { session: Some("s-child".into()), ..hook(&child, AgentStatus::Done) });
        m.send_records();
        let start = Instant::now();
        while kept.lock().unwrap().is_empty() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let record = kept.lock().unwrap().first().cloned().expect("the finished task reached the hub");
        assert_eq!(
            json!([record.id, record.title, record.delegator.kind, record.delegator.tab, record.delegator.project_name, record.account, record.outcome, record.thread, record.usage.tokens_in, record.usage.tokens_out]),
            json!([task, "Board check", "captain", captain, "app", "work", "done", child, 3 * 3412, 750]),
            "the turn that ended is metered before the task settles with its subagent's, and m1's two lines count once"
        );
        let cost = record.usage.cost_usd.expect("priced from the rates file");
        assert!((cost - (36.0 * 3e-6 + 9000.0 * 0.3e-6 + 1200.0 * 3.75e-6 + 750.0 * 15e-6)).abs() < 1e-9, "{cost}");
        assert!(m.delegates.unrecorded(10).is_empty(), "the hub took it");

        let answer: Value = serde_json::from_str(&m.delegation_stats(None).unwrap()).unwrap();
        assert_eq!((answer["scope"].as_str(), answer["all"]["runs"].as_u64(), answer["byModel"]["work's default"]["runs"].as_u64()), (Some("every machine of the account"), Some(1), Some(1)), "{answer}");
        m.set_hub(Arc::new(|_| Err("this hub runs pinned 0.17.20, which can't answer that".into())));
        let local: Value = serde_json::from_str(&m.delegation_stats(Some(30)).unwrap()).unwrap();
        assert_eq!((local["scope"].as_str(), local["days"].as_u64(), local["all"]["runs"].as_u64()), (Some("this machine"), Some(30), Some(1)), "{local}");
        assert_eq!(m.delegation_stats(Some(2)).unwrap_err(), "days must be 1, 7 or 30, not 2");

        let review = m.task_review(Some(captain.clone()), None, &task, "accepted", Some("merged".into())).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&review).unwrap()["review"]["verdict"], "accepted");
        assert_eq!(m.task_review(Some(captain), None, &task, "kept", None).unwrap_err(), "verdict must be accepted or redone, not \"kept\"");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_global_orchestrator_a_captain_and_a_plain_thread_each_name_themselves_as_delegator() {
        use crate::delegate::{Caller, DelegatorKind};
        let dir = temp("delegator");
        let project = dir.join("site");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q"]);
        git(&project, &["remote", "add", "origin", "git@github.com:Acme/Site.git"]);
        let m = Machine::new(launch(), Some(dir.join("config")), None, |_| {});
        let path = m.open_project(project.clone()).display().to_string();
        let captain = m.open_orchestrator(&path).unwrap();
        let global = m.open_global_orchestrator().unwrap();
        let plain = m.add_tab(project.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let of = |tab: &str| {
            let d = m.delegator(&Caller { tab: Some(tab.to_owned()), task: None });
            (d.kind, d.project, d.project_name)
        };
        let site = (Some("github.com/acme/site".to_owned()), Some("site".to_owned()));
        assert_eq!(of(&captain), (DelegatorKind::Captain, site.0.clone(), site.1.clone()));
        assert_eq!(of(&global), (DelegatorKind::GlobalOrchestrator, None, None));
        assert_eq!(of(&plain), (DelegatorKind::Session, site.0, site.1));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What the hook reports for `payload`, as Claude Code's hook process would send it from `tab`.
    #[cfg(unix)]
    fn hook_event(tab: &str, status: &str, transcript: &Path, prompt: Option<&str>) -> AgentEvent {
        let payload = json!({ "session_id": "s-child", "transcript_path": transcript, "prompt": prompt });
        AgentEvent { tab: Some(tab.into()), ..hooks::event_from(Some(status), &payload.to_string()).unwrap() }
    }

    #[cfg(unix)]
    fn append(file: &Path, lines: &[Value]) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(file).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_thread_that_ends_a_turn_with_a_background_shell_pending_finishes_after_the_turn_the_shell_resumes() {
        let (m, dir, captain, task, child, transcript) = captain_with_thread_child("thread-background");
        let said = |text: &str| json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": text }] } });
        let notification = "<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n<summary>Background command \"cargo test\" completed</summary>\n</task-notification>";
        let status = || serde_json::from_str::<Value>(&m.task_status(Some(captain.clone()), None, &task).unwrap()).unwrap();
        let notices = || m.delegates.claim_notice(&captain).map(|(text, _)| text);

        m.record_event(hook_event(&child, "working", &transcript, Some("Board check\n\nCompare the screens with the boards.")));
        append(&transcript, &[
            json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "content": "Command running in background with ID: b1." }] }, "toolUseResult": { "backgroundTaskId": "b1" } }),
            said("Looking at the screenshots."),
        ]);
        m.record_event(hook_event(&child, "done", &transcript, None));
        m.record_event(AgentEvent { message: Some("Claude is waiting for your input".into()), ..hook_event(&child, "needs-input", &transcript, None) });
        assert_eq!((status()["workState"].as_str(), notices()), (Some("working"), None), "a turn end with the shell pending is not the end");
        let view = m.view(&child).unwrap();
        assert_eq!((view.status, view.title.as_deref()), (Some(AgentStatus::Working), Some("Board check")));

        append(&transcript, &[json!({ "type": "attachment", "attachment": { "type": "queued_command", "prompt": notification, "commandMode": "task-notification" } })]);
        m.record_event(hook_event(&child, "working", &transcript, Some(notification)));
        assert_eq!(m.view(&child).unwrap().title.as_deref(), Some("Board check"), "the resume renamed the thread");
        append(&transcript, &[said("All four screens match the boards.")]);
        m.record_event(hook_event(&child, "done", &transcript, None));

        let notice = notices().expect("the task's end notice");
        assert!(notice.contains(&format!("Delegated task {task} \"Board check\"")) && notice.contains("finished: result available"), "{notice}");
        assert_eq!(notices(), None, "the notice came once");
        let ended = status();
        assert_eq!((ended["workState"].as_str(), ended["summary"].as_str()), (Some("result_available"), Some("All four screens match the boards.")));
        let view = m.view(&child).unwrap();
        assert_eq!((view.status, view.title.as_deref()), (Some(AgentStatus::Done), Some("Board check")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_thread_waiting_only_on_a_shell_ends_with_its_reply_after_30_quiet_minutes() {
        let (m, dir, captain, task, child, transcript) = captain_with_thread_child("thread-quiet-shell");
        m.record_event(hook_event(&child, "working", &transcript, Some("Board check")));
        append(&transcript, &[
            json!({ "type": "user", "message": { "content": [] }, "toolUseResult": { "backgroundTaskId": "dev" } }),
            json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "The dev server runs on :3000." }] } }),
        ]);
        m.record_event(hook_event(&child, "done", &transcript, None));
        m.end_quiet_waits(Instant::now());
        let state = || serde_json::from_str::<Value>(&m.task_status(Some(captain.clone()), None, &task).unwrap()).unwrap();
        assert_eq!(state()["workState"], "working");

        let quiet = Duration::from_millis(crate::delegate::QUIET_AFTER_REPLY_MS);
        m.end_quiet_waits(Instant::now() + quiet);
        assert_eq!((state()["workState"].as_str(), state()["summary"].as_str()), (Some("result_available"), Some("The dev server runs on :3000.")));
        assert_eq!(m.view(&child).unwrap().status, Some(AgentStatus::Done));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn every_thread_fans_out_to_worker_threads_and_agents_lists_the_tree() {
        let dir = temp("delegate");
        let project = dir.join("app");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        git(&project, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let claude = thread_claude(&dir);
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude.clone())]), ..launch() }, Some(dir.join("config")), None, |_| {});
        let path = m.open_project(project.clone()).display().to_string();
        let orchestrator = m.open_orchestrator(&path).unwrap();
        let plain = m.open_tab(Some(path.clone()), false).unwrap();
        let child = |from: &str, args: Value| delegate_task(&m, from, args)["tabId"].as_str().unwrap().to_owned();

        let worker = child(&plain, json!({ "task": "Add a --json flag to the CLI.", "title": "JSON output", "isolation": "worktree" }));
        let tree = started(&claude, &worker)[0].clone();
        let second = child(&orchestrator, json!({ "task": "- Fix the README typo.", "isolation": "worktree" }));
        let args = started(&claude, &second);
        assert_ne!(args[0], tree, "two workers share a checkout");
        assert_eq!(args[args.len() - 3], "\n- Fix the README typo.", "a brief starting with a dash reads as an option");
        let nested = child(&worker, json!({ "task": "Test the flag.", "title": "Flag tests", "isolation": "worktree" }));
        let nested_tree = started(&claude, &nested)[0].clone();
        assert_ne!(nested_tree, tree, "a nested worker shares its parent's checkout");
        assert_eq!(m.view(&nested).unwrap().spawned_by, Some(worker.clone()));
        assert_eq!(m.view(&worker).unwrap().spawned_by, Some(plain.clone()));
        assert_eq!(m.view(&second).unwrap().spawned_by, Some(orchestrator.clone()));
        let agents = |tab: &str| m.project_tool(tab, ProjectTool::Agents);
        assert_eq!(
            agents(&orchestrator),
            Ok(format!("{worker}: JSON output (starting) in {tree}\n  {nested}: Flag tests (starting) in {nested_tree}\n{second}: - Fix the README typo. (starting) in {}", args[0]))
        );
        assert_eq!(agents(&plain), Ok(format!("{worker}: JSON output (starting) in {tree}\n  {nested}: Flag tests (starting) in {nested_tree}")));
        assert_eq!(agents(&worker), Ok(format!("{nested}: Flag tests (starting) in {nested_tree}")));
        assert_eq!(agents(&nested), Ok("No workers yet.".into()));

        m.close_tab(&plain);
        assert!(m.view(&worker).unwrap().running, "closing the spawner leaves its worker thread running");
        let task = m.tab(&nested, |t| t.task.clone()).unwrap().unwrap();
        m.close_tab(&nested);
        assert_eq!(m.delegates.get(&task).map(|t| (t.state, t.error)), Some((crate::delegate::WorkState::Cancelled, Some("its thread was closed".into()))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn any_thread_in_a_project_keeps_its_memory_and_a_scratch_thread_has_none() {
        let dir = temp("memory-tools");
        let project = dir.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let sock = dir.join("pinned.sock");
        let m = Machine::new(Launch { sock: sock.clone(), ..launch() }, Some(dir.join("config")), None, |_| {});
        m.listen(|_| Err("not the machine's".into())).unwrap();
        let path = m.open_project(project.clone()).display().to_string();
        let plain = m.open_tab(Some(path.clone()), false).unwrap();
        let worktree = m.add_tab(PathBuf::from(&path).join(".claude/worktrees/pinned-1"), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let scratch = m.open_tab(Some(m.scratch.display().to_string()), false).unwrap();
        let tool = |tab: &str, tool: &str| ask(&sock, &format!(r#"{{"kind":"project","tab":"{tab}","tool":{{{tool}}}}}"#));

        assert_eq!(tool(&plain, r#""name":"memory_save","title":"Package manager","text":"Use pnpm, not npm.","kind":"convention""#), Ok("Saved #1.".into()));
        assert_eq!(tool(&worktree, r#""name":"memory_save","title":"Deploys","text":"The hub deploys from railway-hub.""#), Ok("Saved #2.".into()));
        assert_eq!(
            tool(&plain, r#""name":"memory_save","title":"package manager","text":"pnpm only""#),
            Ok("Not saved: #1 \"Package manager\" already says nearly this: Use pnpm, not npm.\nTo change it, call memory_save with id 1.".into())
        );
        assert_eq!(tool(&worktree, r#""name":"memory_search","query":"which package manager?""#), Ok("[1] Package manager (convention): Use pnpm, not npm.".into()));
        assert_eq!(tool(&plain, r#""name":"memory_save","title":"Deploys","text":"The hub deploys from version tags.","id":2"#), Ok("Updated #2.".into()));
        assert_eq!(tool(&plain, r#""name":"memory_search","query":"""#), Ok("[2] Deploys: The hub deploys from version tags.\n[1] Package manager (convention): Use pnpm, not npm.".into()));
        assert_eq!(tool(&plain, r#""name":"memory_forget","id":2"#), Ok("Forgot #2 \"Deploys\".".into()));
        assert_eq!(tool(&plain, r#""name":"memory_search","query":"deploys""#), Ok("No memory matches that.".into()));
        assert_eq!(tool(&plain, r#""name":"memory_save","title":" ","text":"x""#), Err("a memory needs a title and a text".into()));
        let none = Err("This thread is not in a project, so it has no memory. Memory belongs to a project folder opened in pinned.".into());
        assert_eq!(tool(&scratch, r#""name":"memory_search","query":"pnpm""#), none);
        assert_eq!(tool(&scratch, r#""name":"memory_save","title":"Scratch","text":"x""#), none);
        assert_eq!(tool(&plain, r#""name":"agents""#), Ok("No workers yet.".into()));

        let call = |c: Call| m.call(c);
        assert_eq!(
            call(Call::MemorySave { path: path.clone(), title: "Tests".into(), text: "Run cargo test.".into(), kind: None, id: None }).map(|v| (v["id"].clone(), v["title"].clone())),
            Ok((json!(3), json!("Tests")))
        );
        assert_eq!(
            call(Call::MemorySave { path: path.clone(), title: "TESTS".into(), text: "Run them.".into(), kind: None, id: None }),
            Err("Memory #3 \"Tests\" already says nearly this. Change that one instead.".into())
        );
        let found = call(Call::MemorySearch { path: path.clone(), query: "".into(), limit: Some(1) }).unwrap();
        assert_eq!(found.as_array().unwrap().iter().map(|m| m["title"].clone()).collect::<Vec<_>>(), [json!("Tests")]);
        assert_eq!(call(Call::MemoryStatus { path: path.clone() }), Ok(json!({ "count": 2, "vectors": "off" })));
        assert_eq!(call(Call::MemoryForget { path: path.clone(), id: 3 }).map(|v| v["text"].clone()), Ok(json!("Run cargo test.")));
        assert_eq!(call(Call::MemoryForget { path: path.clone(), id: 3 }), Err("no memory #3".into()));
        let elsewhere = dir.join("elsewhere").display().to_string();
        assert_eq!(call(Call::MemoryStatus { path: elsewhere.clone() }), Err(format!("{elsewhere} is not a project on this machine")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_prompt_in_a_project_thread_is_answered_with_its_closest_memories() {
        let dir = temp("recall");
        let project = dir.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let sock = dir.join("pinned.sock");
        let m = Machine::new(Launch { sock: sock.clone(), ..launch() }, Some(dir.join("config")), None, |_| {});
        m.listen(|_| Err("not the machine's".into())).unwrap();
        let path = m.open_project(project).display().to_string();
        let plain = m.open_tab(Some(path.clone()), false).unwrap();
        let scratch = m.open_tab(Some(m.scratch.display().to_string()), false).unwrap();
        for (title, text) in [("Deploys", "The hub deploys itself from version tags on main."), ("Package manager", "Use pnpm, not npm.")] {
            m.call(Call::MemorySave { path: path.clone(), title: title.into(), text: text.into(), kind: None, id: None }).unwrap();
        }
        let prompt = |tab: &str, status, prompt: &str| {
            let event = AgentEvent { status, cwd: None, message: None, tab: Some(tab.into()), session: None, title: None, resume_at: None, prompt: Some(prompt.into()), snapshot: None, permission_mode: None, hook_feedback: false };
            ask(&sock, &serde_json::to_string(&Message::Agent(event)).unwrap())
        };
        assert_eq!(
            prompt(&plain, AgentStatus::Working, "when does the hub deploy?"),
            Ok("Project memory pinned found for this prompt, by id (memory_search finds more; memory_save with an id corrects one):\n[1] Deploys: The hub deploys itself from version tags on main.".into())
        );
        assert_eq!(prompt(&plain, AgentStatus::Working, "thanks, that's all"), Ok(String::new()));
        assert_eq!(prompt(&plain, AgentStatus::Done, "when does the hub deploy?"), Ok(String::new()));
        assert_eq!(prompt(&scratch, AgentStatus::Working, "when does the hub deploy?"), Ok(String::new()));
        assert_eq!(m.view(&plain).unwrap().status, Some(AgentStatus::Done), "the events were not recorded");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Claude's idle Notification follows Stop and replaces Done. A child that finishes after that
    /// still has to reach the parent with no click.
    #[test]
    #[cfg(unix)]
    fn an_idle_notification_does_not_strand_a_notice() {
        let (m, tab, file) = pty_file("notice-idle-notify");
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        m.record_event(AgentEvent { status: AgentStatus::NeedsInput, message: Some("Claude is waiting for your input".into()), ..done_event(&tab) });
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "After idle");
        m.maybe_deliver(&tab);
        let needle = format!("[pinned] Delegated task {id} \"After idle\"");
        wait_for("the notice after the idle notification", || read_pty(&file).contains(&needle));
        let _ = std::fs::remove_file(file);
    }

    /// A draft blocks typing. The turn's Stop hook still hands the notice to the model.
    #[test]
    #[cfg(unix)]
    fn a_stop_hook_feeds_a_notice_the_draft_would_block() {
        let root = temp("notice-stop");
        let sock = root.join("hook.sock");
        let file = root.join("typed.bin");
        let m = Machine::new(Launch { sock: sock.clone(), ..launch() }, None, Some(root.clone()), |_| {});
        m.listen(|_| Err("not a connector".into())).unwrap();
        let tab = running(&m, vec![], &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        m.record_event(AgentEvent { status: AgentStatus::Working, ..done_event(&tab) });
        m.write_input(&tab, b"xyz").unwrap();
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Mid");
        let mut message = serde_json::to_value(Message::Agent(done_event(&tab))).unwrap();
        message["hook_feedback"] = json!(true);
        let reply = ask(&sock, &message.to_string()).unwrap();
        assert!(reply.contains(&id), "the Stop hook did not carry the notice: {reply}");
        std::thread::sleep(Duration::from_millis(200));
        let typed = read_pty(&file);
        assert!(typed.contains("xyz"), "the draft was not in the terminal: {typed}");
        assert!(!typed.contains("[pinned]"), "the notice was typed over the draft: {typed}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Submitting the draft must carry a notice that typing would have clobbered.
    #[test]
    #[cfg(unix)]
    fn a_draft_does_not_strand_a_notice() {
        let root = temp("notice-draft");
        let sock = root.join("hook.sock");
        let file = root.join("typed.bin");
        let m = Machine::new(Launch { sock: sock.clone(), ..launch() }, None, Some(root.clone()), |_| {});
        m.listen(|_| Err("not a connector".into())).unwrap();
        let tab = running(&m, vec![], &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        m.tab(&tab, |t| t.status = Some(AgentStatus::Done)).unwrap();
        m.write_input(&tab, b"xyz").unwrap();
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Draft");
        m.maybe_deliver(&tab);
        let mut message = serde_json::to_value(Message::Agent(AgentEvent {
            status: AgentStatus::Working,
            prompt: Some("ship it".into()),
            ..done_event(&tab)
        }))
        .unwrap();
        message["hook_feedback"] = json!(true);
        let reply = ask(&sock, &message.to_string()).unwrap();
        assert!(reply.contains(&id), "submitting the draft dropped the notice: {reply}");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!read_pty(&file).contains("[pinned]"), "the notice was typed over the draft: {}", read_pty(&file));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A runtime restart clears Done. The same conversation still receives the notice once it is back.
    #[test]
    #[cfg(unix)]
    fn a_notice_survives_a_parent_session_restart() {
        let file = std::env::temp_dir().join(format!("pinned-notice-restart-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let m = machine();
        let tab = running(&m, vec![], &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&tab, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        m.record_event(AgentEvent { status: AgentStatus::Working, session: Some("sess-1".into()), ..done_event(&tab) });
        let id = finish_task(&m, crate::delegate::Parent::Tab(tab.clone()), "Restart");
        m.maybe_deliver(&tab);
        std::thread::sleep(Duration::from_millis(200));
        assert!(!read_pty(&file).contains("[pinned]"), "the notice was typed mid-turn: {}", read_pty(&file));
        m.spawn(&tab, sh(&format!("stty raw -echo; echo ready; exec cat > '{}'", file.display())), pty::size(80, 24)).unwrap();
        let needle = format!("[pinned] Delegated task {id} \"Restart\"");
        wait_for("the notice after the session restart", || {
            m.maybe_deliver(&tab);
            read_pty(&file).contains(&needle)
        });
        let _ = std::fs::remove_file(file);
    }

    /// Quit kills the children. The next process still has the task, and a child that was running
    /// tells its parent it was stopped.
    #[test]
    fn a_task_registry_survives_a_pinned_restart() {
        let root = temp("notice-registry");
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        let project = root.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let m = Machine::new(launch(), Some(config.clone()), None, |_| {});
        let _first = m.add_tab(project.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        let parent = m.add_tab(project.clone(), Account::builtin(Provider::Claude), vec![], vec![], false, None);
        m.record_event(AgentEvent {
            status: AgentStatus::Done,
            cwd: None,
            message: None,
            tab: Some(parent.clone()),
            session: Some("orch-1".into()),
            title: None,
            resume_at: None,
            prompt: None,
            snapshot: None,
            permission_mode: None,
            hook_feedback: false,
        });
        m.close_tab(&_first);
        let finished = {
            use crate::delegate::{ChildEvent, Mode, NewTask};
            let task = m.delegates.begin(NewTask {
                parent: crate::delegate::Parent::Tab(parent.clone()),
                tool: None,
                title: "Kept".into(),
                account: "cursor".into(),
                provider: Provider::Cursor,
                model: None,
                cwd: project.clone(),
                mode: Mode::Async,
                access: crate::delegate::Access::Auto,
                isolated: false,
                caller_key: format!("tab:{parent}"),
                client_request_id: None,
                role: None,
            });
            m.delegates.apply(&task.id, ChildEvent::Line(json!({"type":"result","result":"DONE"}).to_string()), None);
            task.id
        };
        let running = m.delegates.begin(crate::delegate::NewTask {
            parent: crate::delegate::Parent::Tab(parent.clone()),
            tool: None,
            title: "Killed".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: project.clone(),
            mode: crate::delegate::Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: false,
            caller_key: format!("tab:{parent}"),
            client_request_id: None,
            role: None,
        });
        m.quit();
        let m2 = Machine::new(launch(), Some(config), None, |_| {});
        let tabs = m2.restore_tabs();
        assert_eq!(tabs.len(), 1, "the orchestrator session was not restored");
        let notice = m2.delegates.claim_notice(&tabs[0]).map(|(text, _)| text).unwrap_or_default();
        assert!(notice.contains(&finished), "the finished task was forgotten: {notice}");
        assert!(notice.contains("result available"), "the finished task's notice was forgotten: {notice}");
        assert!(notice.contains(&running.id), "the killed child was forgotten: {notice}");
        assert!(notice.contains("stopped by restart"), "the killed child did not say why: {notice}");
        let status: serde_json::Value = serde_json::from_str(&m2.task_status(Some(tabs[0].clone()), None, &finished).unwrap()).unwrap();
        assert_eq!(status["summary"], "DONE");
        let _ = std::fs::remove_dir_all(root);
    }

    fn runaway(m: &Machine) -> crate::delegate::Task {
        m.delegates.begin(crate::delegate::NewTask {
            parent: crate::delegate::Parent::Tab("tab-1".into()),
            tool: None,
            title: "Runaway".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: std::env::temp_dir(),
            mode: crate::delegate::Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: false,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        })
    }

    #[test]
    #[cfg(unix)]
    fn the_delegate_pass_stops_a_task_past_its_hard_limit() {
        use std::os::unix::process::CommandExt;
        let m = Machine::new(launch(), None, None, |_| {});
        let task = runaway(&m);
        m.delegates.set_limits(&task.id, 1, 2);
        let mut sleeper = crate::process::command("sleep");
        sleeper.arg("20").process_group(0);
        let child = std::sync::Mutex::new(sleeper.spawn().unwrap());
        let pid = child.lock().unwrap().id();
        m.delegates.remember_pid(&task.id, pid);
        m.arm_delegate_pass();
        wait_for("the hard limit", || m.delegates.is_terminal(&task.id));
        wait_for("the runaway child to stop", || child.lock().unwrap().try_wait().unwrap().is_some());
        let status = m.delegates.status(&crate::delegate::Caller { tab: Some("tab-1".into()), task: None }, &task.id).unwrap();
        assert_eq!(status["workState"], "cancelled", "{status}");
    }

    #[test]
    #[cfg(unix)]
    fn the_delegate_pass_still_stops_a_task_after_a_quit_that_reopens() {
        use std::os::unix::process::CommandExt;
        let m = Machine::new(launch(), None, None, |_| {});
        m.arm_delegate_pass();
        m.quit();
        std::thread::sleep(Duration::from_millis(2500));
        m.reopen_sessions();
        let task = runaway(&m);
        m.delegates.set_limits(&task.id, 1, 2);
        let mut sleeper = crate::process::command("sleep");
        sleeper.arg("20").process_group(0);
        let child = std::sync::Mutex::new(sleeper.spawn().unwrap());
        m.delegates.remember_pid(&task.id, child.lock().unwrap().id());
        m.arm_delegate_pass();
        wait_for("the hard limit after a quit that reopened", || m.delegates.is_terminal(&task.id));
        wait_for("the runaway child to stop", || child.lock().unwrap().try_wait().unwrap().is_some());
        let status = m.delegates.status(&crate::delegate::Caller { tab: Some("tab-1".into()), task: None }, &task.id).unwrap();
        assert_eq!(status["workState"], "cancelled", "{status}");
    }

    #[test]
    fn the_delegate_pass_keeps_enforcing_limits_after_a_panic() {
        let m = Machine::new(launch(), None, None, |_| {});
        let task = runaway(&m);
        m.delegates.set_limits(&task.id, 2_000, 2_500);
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _tabs = m.tabs.lock().unwrap();
            panic!("a thread panics while it holds the tab lock");
        }));
        assert!(poisoned.is_err() && m.tabs.is_poisoned());
        m.arm_delegate_pass();
        wait_for("the hard limit after a pass panicked", || m.delegates.is_terminal(&task.id));
        let status = m.delegates.status(&crate::delegate::Caller { tab: Some("tab-1".into()), task: None }, &task.id).unwrap();
        let error = status["error"].as_str().unwrap_or("");
        assert!(status["workState"] == "cancelled" && error.starts_with("pinned stopped it after ") && error.ends_with("s, its limit"), "{status}");
    }

    #[test]
    fn a_laptop_sleep_does_not_count_against_a_working_task() {
        let m = Machine::new(launch(), None, None, |_| {});
        let task = m.delegates.begin(crate::delegate::NewTask {
            parent: crate::delegate::Parent::Tab("tab-1".into()),
            tool: None,
            title: "Overnight".into(),
            account: "cursor".into(),
            provider: Provider::Cursor,
            model: None,
            cwd: std::env::temp_dir(),
            mode: crate::delegate::Mode::Async,
            access: crate::delegate::Access::Auto,
            isolated: false,
            caller_key: "tab:tab-1".into(),
            client_request_id: None,
            role: None,
        });
        m.delegates.backdate(&task.id, Duration::from_secs(9 * 60 * 60));
        m.arm_delegate_pass();
        std::thread::sleep(Duration::from_millis(1500));
        let status = m.delegates.status(&crate::delegate::Caller { tab: Some("tab-1".into()), task: None }, &task.id).unwrap();
        assert_eq!(status["workState"], "working", "{status}");
        assert_eq!(m.delegates.claim_notice("tab-1"), None, "the sleep counted toward the budget");
    }

    /// A worker report must not type over a draft. The orchestrator's Stop hook carries it.
    #[test]
    #[cfg(unix)]
    fn a_worker_report_does_not_clobber_a_draft() {
        let root = temp("notice-worker");
        let sock = root.join("hook.sock");
        let file = root.join("typed.bin");
        let project = root.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let m = Machine::new(Launch { sock: sock.clone(), ..launch() }, None, Some(root.clone()), |_| {});
        m.listen(|_| Err("not a connector".into())).unwrap();
        let orchestrator = running(&m, role_env(Role::Orchestrator, &project), &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&orchestrator, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        m.record_event(AgentEvent { status: AgentStatus::Working, ..done_event(&orchestrator) });
        m.write_input(&orchestrator, b"xyz").unwrap();
        let worker = m.add_tab(project.clone(), Account::builtin(Provider::Claude), vec![], role_env(Role::Worker, &project), false, None);
        m.tab(&worker, |t| t.title = Some("JSON output".into())).unwrap();
        m.record_event(AgentEvent { status: AgentStatus::Done, ..done_event(&worker) });
        std::thread::sleep(Duration::from_millis(200));
        let typed = read_pty(&file);
        assert!(typed.contains("xyz"), "the draft was lost: {typed}");
        assert!(!typed.contains("Worker"), "the worker report was typed over the draft: {typed}");
        let mut message = serde_json::to_value(Message::Agent(done_event(&orchestrator))).unwrap();
        message["hook_feedback"] = json!(true);
        let reply = ask(&sock, &message.to_string()).unwrap();
        assert!(reply.contains(&worker) && reply.contains("JSON output"), "the Stop hook missed the worker: {reply}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_workers_news_is_typed_into_its_projects_running_orchestrator() {
        let root = temp("report");
        let config = root.join("config");
        std::fs::create_dir_all(config.join("accounts/work/projects/-app")).unwrap();
        std::fs::write(config.join("accounts.json"), r#"[{"id": "work", "provider": "claude", "name": "Work"}]"#).unwrap();
        let transcript = [
            r#"{"type":"user","uuid":"u1","timestamp":"2026-10-29T10:00:00Z","message":{"content":"Add a --json flag."}}"#,
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-10-29T10:01:00Z","message":{"content":[{"type":"text","text":"Reading src/cli.rs first."}]}}"#,
            r#"{"type":"assistant","uuid":"a2","timestamp":"2026-10-29T10:05:00Z","message":{"content":[{"type":"text","text":"Added --json in src/cli.rs; all 12 tests pass."}]}}"#,
        ];
        std::fs::write(config.join("accounts/work/projects/-app/s1.jsonl"), transcript.map(|l| l.to_owned() + "\n").concat()).unwrap();
        let file = root.join("typed.bin");
        let m = Machine::new(launch(), Some(config), None, |_| {});
        let app = Path::new("/work/app");
        let orchestrator = running(&m, role_env(Role::Orchestrator, app), &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&orchestrator, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        let work = m.accounts.get("work").unwrap();
        let worker = m.add_tab("/work/app/.claude/worktrees/pinned-1".into(), work.clone(), vec![], role_env(Role::Worker, app), false, Some("s1".into()));
        let elsewhere = m.add_tab("/work/api".into(), work, vec![], role_env(Role::Worker, Path::new("/work/api")), false, Some("s1".into()));
        m.tab(&worker, |t| t.title = Some("JSON output".into())).unwrap();
        let event = |tab: &str, status, message: Option<&str>| AgentEvent { status, cwd: None, message: message.map(Into::into), tab: Some(tab.into()), session: None, title: None, resume_at: None, prompt: None, snapshot: None, permission_mode: None, hook_feedback: false };

        m.record_event(event(&worker, AgentStatus::Working, None));
        m.record_event(event(&worker, AgentStatus::NeedsInput, Some("Claude needs your permission to use Bash")));
        m.record_event(event(&worker, AgentStatus::Done, None));
        m.record_event(event(&worker, AgentStatus::NeedsInput, Some("Claude is waiting for your input")));
        m.record_event(event(&elsewhere, AgentStatus::Done, None));
        let expected = format!(
            "\x1b[200~[pinned] Worker {worker} \"JSON output\" needs input: Claude needs your permission to use Bash\x1b[201~\r\
             \x1b[200~[pinned] Worker {worker} \"JSON output\" finished. Its last message:\n\nAdded --json in src/cli.rs; all 12 tests pass.\x1b[201~\r"
        );
        wait_for("the reports", || std::fs::read(&file).is_ok_and(|b| b.len() >= expected.len()));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(String::from_utf8(std::fs::read(&file).unwrap()).unwrap(), expected);
        assert_eq!(truncate("añb", 2, "the whole message is in the worker's thread"), "añ\n[cut short: the whole message is in the worker's thread]");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A worker "Board check" on session s1 of the `work` account, whose news is typed into its
    /// project's running orchestrator, and a reader of what was typed there.
    #[cfg(unix)]
    fn worker_reporting_home(name: &str) -> (Arc<Machine>, PathBuf, String, PathBuf, impl Fn() -> String) {
        let root = temp(name);
        let config = root.join("config");
        std::fs::create_dir_all(config.join("accounts/work/projects/-app")).unwrap();
        std::fs::write(config.join("accounts.json"), r#"[{"id": "work", "provider": "claude", "name": "Work"}]"#).unwrap();
        let transcript = config.join("accounts/work/projects/-app/s1.jsonl");
        std::fs::write(&transcript, "").unwrap();
        let file = root.join("typed.bin");
        let m = Machine::new(launch(), Some(config), None, |_| {});
        let app = Path::new("/work/app");
        let orchestrator = running(&m, role_env(Role::Orchestrator, app), &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&orchestrator, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        let work = m.accounts.get("work").unwrap();
        let worker = m.add_tab("/work/app/.claude/worktrees/pinned-1".into(), work, vec![], role_env(Role::Worker, app), false, Some("s1".into()));
        m.tab(&worker, |t| t.title = Some("Board check".into())).unwrap();
        let typed = move || String::from_utf8(std::fs::read(&file).unwrap_or_default()).unwrap();
        (m, root, worker, transcript, typed)
    }

    /// What the hook reports for a turn of session s1, as Claude Code's hook process sends it.
    #[cfg(unix)]
    fn s1_hook(tab: &str, status: &str, transcript: &Path, prompt: Option<&str>) -> AgentEvent {
        let payload = json!({ "session_id": "s1", "transcript_path": transcript, "prompt": prompt });
        AgentEvent { tab: Some(tab.into()), ..hooks::event_from(Some(status), &payload.to_string()).unwrap() }
    }

    #[cfg(unix)]
    fn finished(worker: &str, reply: &str) -> String {
        format!("\x1b[200~[pinned] Worker {worker} \"Board check\" finished. Its last message:\n\n{reply}\x1b[201~\r")
    }

    #[test]
    #[cfg(unix)]
    fn a_message_from_another_session_keeps_the_workers_title_and_its_notice_carries_the_answer() {
        let (m, root, worker, transcript, typed) = worker_reporting_home("report-peer");
        let said = |text: &str| json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": text }] } });
        m.record_event(s1_hook(&worker, "working", &transcript, Some("Board check\n\nCompare the screens with the boards.")));
        append(&transcript, &[
            json!({ "type": "user", "turnPosition": { "promptIndex": 0, "turnIndex": 0 }, "message": { "content": "Compare the screens with the boards." } }),
            said("Desktop sidebar matches. Comparing against the boards next."),
        ]);
        m.record_event(s1_hook(&worker, "done", &transcript, None));

        let message = "<cross-session-message from=\"uds:/tmp/cc-socks/85194.sock\" from-name=\"Captain\" from-mode=\"bypass\">\nIs anything of yours still running?\n</cross-session-message>";
        append(&transcript, &[json!({ "type": "user", "isMeta": true, "turnPosition": { "promptIndex": 1, "turnIndex": 1 }, "origin": { "kind": "peer" }, "message": { "content": format!("Another Claude session sent a message:\n{message}") } })]);
        m.record_event(s1_hook(&worker, "working", &transcript, Some(message)));
        assert_eq!(m.view(&worker).unwrap().title.as_deref(), Some("Board check"), "the message renamed the thread");
        append(&transcript, &[
            json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "SendMessage", "input": { "to": "uds:/tmp/cc-socks/85194.sock", "message": "Nothing I started is still running." } }] } }),
            json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "Message sent." }] } }),
        ]);
        m.record_event(s1_hook(&worker, "done", &transcript, None));

        let expected = finished(&worker, "Desktop sidebar matches. Comparing against the boards next.") + &finished(&worker, "Nothing I started is still running.");
        wait_for("both notices", || typed().len() >= expected.len());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(typed(), expected);
        assert_eq!(m.view(&worker).unwrap().title.as_deref(), Some("Board check"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn a_workers_second_notice_carries_its_second_turn_never_the_first() {
        let (m, root, worker, transcript, typed) = worker_reporting_home("report-second");
        m.record_event(s1_hook(&worker, "working", &transcript, Some("Board check\n\nAdd the boards folder.")));
        append(&transcript, &[
            json!({ "type": "user", "turnPosition": { "promptIndex": 0, "turnIndex": 0 }, "message": { "content": "Add the boards folder." } }),
            json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "It already exists with the same shape; deleting my copy." }] } }),
        ]);
        m.record_event(s1_hook(&worker, "done", &transcript, None));

        let notification = "<task-notification>\n<task-id>w1</task-id>\n<status>completed</status>\n<summary>Wakeup fired</summary>\n</task-notification>";
        m.record_event(s1_hook(&worker, "working", &transcript, Some(notification)));
        append(&transcript, &[
            json!({ "type": "user", "turnPosition": { "promptIndex": 1, "turnIndex": 1 }, "origin": { "kind": "task-notification" }, "message": { "content": notification } }),
            json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "t2", "name": "Bash", "input": { "command": "ls boards" } }] } }),
            json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t2", "content": "main.png" }] } }),
        ]);
        m.record_event(s1_hook(&worker, "done", &transcript, None));

        let expected = finished(&worker, "It already exists with the same shape; deleting my copy.") + &finished(&worker, "(none)");
        wait_for("both notices", || typed().len() >= expected.len());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(typed(), expected);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Without a running orchestrator, a worker's news goes back to the plain thread that
    /// spawned it, so it works the same way there.
    #[test]
    #[cfg(unix)]
    fn a_workers_news_is_typed_into_its_spawner_without_an_orchestrator() {
        let root = temp("report-spawner");
        let file = root.join("typed.bin");
        let project = root.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let m = Machine::new(Launch { sock: root.join("hook.sock"), ..launch() }, None, Some(root.clone()), |_| {});
        m.listen(|_| Err("not a connector".into())).unwrap();
        let spawner = running(&m, vec![], &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
        let (seen, sink) = viewer();
        m.attach(&spawner, "test".into(), sink).unwrap();
        wait_for("stty", || text(&seen).contains("ready"));
        let worker = m.add_tab(project.clone(), Account::builtin(Provider::Claude), vec![], role_env(Role::Worker, &project), false, None);
        m.tab(&worker, |t| {
            t.title = Some("JSON output".into());
            t.spawned_by = Some(spawner.clone());
        })
        .unwrap();
        m.record_event(AgentEvent {
            status: AgentStatus::NeedsInput,
            message: Some("Claude needs your input".into()),
            tab: Some(worker.clone()),
            ..done_event(&worker)
        });
        let expected = format!("\x1b[200~[pinned] Worker {worker} \"JSON output\" needs input: Claude needs your input\x1b[201~\r");
        wait_for("the report", || std::fs::read(&file).is_ok_and(|b| b.len() >= expected.len()));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(String::from_utf8(std::fs::read(&file).unwrap()).unwrap(), expected);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A nested worker's news goes to the worker that started it, even while the project's
    /// orchestrator runs, and to the orchestrator once that worker's tab is closed.
    #[test]
    #[cfg(unix)]
    fn a_nested_workers_news_goes_to_its_parent_before_the_orchestrator() {
        let root = temp("report-parent");
        let project = root.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let m = Machine::new(Launch { sock: root.join("hook.sock"), ..launch() }, None, Some(root.clone()), |_| {});
        m.listen(|_| Err("not a connector".into())).unwrap();
        let typed_into = |env: Vec<(String, String)>, name: &str| {
            let file = root.join(name);
            let tab = running(&m, env, &format!("stty raw -echo; echo ready; exec cat > '{}'", file.display()));
            let (seen, sink) = viewer();
            m.attach(&tab, "test".into(), sink).unwrap();
            wait_for("stty", || text(&seen).contains("ready"));
            (tab, file)
        };
        let (_orchestrator, orchestrator_file) = typed_into(role_env(Role::Orchestrator, &project), "orchestrator.bin");
        let (parent, parent_file) = typed_into(role_env(Role::Worker, &project), "parent.bin");
        let nested = m.add_tab(project.clone(), Account::builtin(Provider::Claude), vec![], role_env(Role::Worker, &project), false, None);
        m.tab(&nested, |t| {
            t.title = Some("Flag tests".into());
            t.spawned_by = Some(parent.clone());
        })
        .unwrap();
        let ask = |message: &str| AgentEvent { status: AgentStatus::NeedsInput, message: Some(message.into()), tab: Some(nested.clone()), ..done_event(&nested) };

        m.record_event(ask("May I run the tests?"));
        let to_parent = format!("\x1b[200~[pinned] Worker {nested} \"Flag tests\" needs input: May I run the tests?\x1b[201~\r");
        wait_for("the parent's report", || std::fs::read(&parent_file).is_ok_and(|b| b.len() >= to_parent.len()));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(String::from_utf8(std::fs::read(&parent_file).unwrap()).unwrap(), to_parent);
        assert_eq!(std::fs::read(&orchestrator_file).unwrap_or_default(), b"", "the orchestrator got news its worker's worker owed the worker");

        m.close_tab(&parent);
        m.record_event(ask("Still there?"));
        let to_orchestrator = format!("\x1b[200~[pinned] Worker {nested} \"Flag tests\" needs input: Still there?\x1b[201~\r");
        wait_for("the orchestrator's report", || std::fs::read(&orchestrator_file).is_ok_and(|b| b.len() >= to_orchestrator.len()));
        assert_eq!(String::from_utf8(std::fs::read(&orchestrator_file).unwrap()).unwrap(), to_orchestrator);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A worker that finishes while its orchestrator has no running session (a tab restored
    /// after a restart, or one the web UI has not started) still reports home: the news waits
    /// in the queue instead of dropping.
    #[test]
    #[cfg(unix)]
    fn a_workers_news_is_queued_for_an_orchestrator_without_a_running_session() {
        let root = temp("report-queued");
        std::fs::create_dir_all(root.join("app")).unwrap();
        let m = Machine::new(launch(), None, Some(root.clone()), |_| {});
        let project = root.join("app");
        let orchestrator = m.add_tab(project.clone(), Account::builtin(Provider::Claude), vec![], role_env(Role::Orchestrator, &project), false, None);
        let worker = m.add_tab(project.clone(), Account::builtin(Provider::Claude), vec![], role_env(Role::Worker, &project), false, None);
        m.tab(&worker, |t| {
            t.title = Some("JSON output".into());
            t.spawned_by = Some(orchestrator.clone());
        })
        .unwrap();
        m.record_event(AgentEvent { status: AgentStatus::Done, tab: Some(worker.clone()), ..done_event(&worker) });
        assert!(m.delegates.has_pending(), "the finished worker's news was dropped instead of queued");
        assert_eq!(m.view(&orchestrator).unwrap().queued, 1, "the orchestrator's view does not count its worker's queued news");
        let _ = std::fs::remove_dir_all(root);
    }

    fn status(status: UpdateStatus) -> Value {
        serde_json::to_value(status).unwrap()
    }

    struct Fixed(UpdateStatus);

    impl SelfUpdate for Fixed {
        fn install(&self, machine: &Machine) -> UpdateStatus {
            machine.report(self.0.clone());
            self.0.clone()
        }
    }

    struct Counted {
        calls: Arc<AtomicU32>,
        status: UpdateStatus,
        started: Mutex<Option<std::sync::mpsc::Sender<()>>>,
        release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    }

    impl SelfUpdate for Counted {
        fn install(&self, machine: &Machine) -> UpdateStatus {
            self.calls.fetch_add(1, Ordering::SeqCst);
            machine.report(self.status.clone());
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            if let Some(release) = self.release.lock().unwrap().take() {
                let _ = release.recv();
            }
            self.status.clone()
        }
    }

    struct PanicInstall(UpdateStatus);

    impl SelfUpdate for PanicInstall {
        fn install(&self, machine: &Machine) -> UpdateStatus {
            machine.report(self.0.clone());
            panic!("installer failed");
        }
    }

    #[test]
    fn an_update_without_an_installer_cannot() {
        let m = machine();
        assert_eq!(m.update_snapshot(), None);
        assert_eq!(
            m.call(Call::UpdateSelf).unwrap(),
            status(UpdateStatus::Cannot { reason: "this build cannot install a release".into() }),
        );
    }

    #[test]
    fn a_second_press_while_updating_does_not_install_again() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let m = Machine::new(launch(), None, None, {
            let seen = seen.clone();
            move |event| seen.lock().unwrap().push(event)
        });
        let calls = Arc::new(AtomicU32::new(0));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let updating = UpdateStatus::Updating { to: "2.0.0".into() };
        m.set_installer(Arc::new(Counted {
            calls: calls.clone(),
            status: updating.clone(),
            started: Mutex::new(Some(started_tx)),
            release: Mutex::new(Some(release_rx)),
        }));
        let first = m.arc();
        let running = std::thread::spawn(move || first.call(Call::UpdateSelf));
        started_rx.recv().unwrap();
        assert_eq!(m.update_snapshot(), Some(updating.clone()));
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(updating.clone()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        match seen.lock().unwrap().as_slice() {
            [MachineEvent::Update { status }] => assert_eq!(status, &Some(updating.clone())),
            other => panic!("{other:?}"),
        }
        release_tx.send(()).unwrap();
        assert_eq!(running.join().unwrap().unwrap(), status(updating.clone()));
        assert_eq!(m.update_snapshot(), Some(updating.clone()));
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(updating));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_restart_stays_busy_so_a_second_press_does_not_install_again() {
        let calls = Arc::new(AtomicU32::new(0));
        let m = machine();
        let restarting = UpdateStatus::Restarting { to: "1.2.0".into() };
        m.set_installer(Arc::new(Counted {
            calls: calls.clone(),
            status: restarting.clone(),
            started: Mutex::new(None),
            release: Mutex::new(None),
        }));
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(restarting.clone()));
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(restarting.clone()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(m.update_snapshot(), Some(restarting));
    }

    #[test]
    fn a_panic_while_updating_can_be_pressed_again() {
        let m = machine();
        m.set_installer(Arc::new(PanicInstall(UpdateStatus::Updating { to: "9".into() })));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.call(Call::UpdateSelf))).is_err());
        assert_eq!(m.update_snapshot(), Some(UpdateStatus::Updating { to: "9".into() }));
        let current = UpdateStatus::UpToDate { version: "0.16.0".into() };
        m.set_installer(Arc::new(Fixed(current.clone())));
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(current));
    }

    #[test]
    fn a_panic_after_restarting_cannot_be_pressed_again() {
        let m = machine();
        let restarting = UpdateStatus::Restarting { to: "1.2.0".into() };
        m.set_installer(Arc::new(PanicInstall(restarting.clone())));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.call(Call::UpdateSelf))).is_err());
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(restarting.clone()));
        assert_eq!(m.update_snapshot(), Some(restarting));
    }

    #[test]
    fn a_failed_restart_can_be_pressed_again() {
        let calls = Arc::new(AtomicU32::new(0));
        let m = machine();
        let restarting = UpdateStatus::Restarting { to: "1.2.0".into() };
        m.set_installer(Arc::new(Counted {
            calls: calls.clone(),
            status: restarting.clone(),
            started: Mutex::new(None),
            release: Mutex::new(None),
        }));
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(restarting.clone()));
        let failed = UpdateStatus::Failed { to: "1.2.0".into(), reason: "the new binary exited before it connected".into() };
        m.report(failed);
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(restarting));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_terminal_report_after_restarting_can_be_pressed_again() {
        for terminal in [
            UpdateStatus::Failed { to: "1.2.0".into(), reason: "checksum mismatch".into() },
            UpdateStatus::Cannot { reason: "no release binary for macos aarch64".into() },
            UpdateStatus::UpToDate { version: "0.16.0".into() },
        ] {
            let calls = Arc::new(AtomicU32::new(0));
            let m = machine();
            let restarting = UpdateStatus::Restarting { to: "1.2.0".into() };
            m.set_installer(Arc::new(Counted {
                calls: calls.clone(),
                status: restarting.clone(),
                started: Mutex::new(None),
                release: Mutex::new(None),
            }));
            assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(restarting.clone()));
            m.report(terminal);
            assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(restarting));
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn a_finished_failure_is_still_the_phase_a_reconnect_reports() {
        let m = machine();
        let failed = UpdateStatus::Failed { to: "1.2.0".into(), reason: "checksum mismatch".into() };
        m.report(failed.clone());
        assert_eq!(m.update_snapshot(), Some(failed));
    }

    #[test]
    fn a_report_uses_the_link_flusher_when_one_is_installed() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let m = Machine::new(launch(), None, None, {
            let seen = seen.clone();
            move |event| seen.lock().unwrap().push(event)
        });
        let flushed = Arc::new(Mutex::new(None));
        m.set_flusher(Arc::new({
            let flushed = flushed.clone();
            move |event| {
                *flushed.lock().unwrap() = Some(event.clone());
                false
            }
        }));
        let current = UpdateStatus::UpToDate { version: "0.16.0".into() };
        m.set_installer(Arc::new(Fixed(current.clone())));
        assert!(!m.report(current.clone()));
        assert_eq!(m.call(Call::UpdateSelf).unwrap(), status(current.clone()));
        match flushed.lock().unwrap().clone() {
            Some(MachineEvent::Update { status }) => assert_eq!(status, Some(current)),
            other => panic!("{other:?}"),
        }
        assert!(seen.lock().unwrap().is_empty());
    }
}
