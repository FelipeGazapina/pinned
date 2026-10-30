use crate::accounts::{self, Account, Accounts, Provider};
use crate::agent::{session_of, AgentEvent, AgentStatus, Message, Usage, CONNECTOR_ENV, SOCK_ENV, TAB_ENV, TASK_ENV};
use crate::stats::{ThreadStats, Tally};
use crate::wire::{Call, MachineEvent, Program, ProjectView, RunSpec, TabId, TabView, ThreadMatch, ThreadPage, EXE_PLACEHOLDER};
use crate::panel::{DeviceHub, FileServer, Shells};
use crate::{branches, commands, favicon, files, github, history, hooks, panel, plugin, process, projects, pty, pulls, reminders, scaffold, scripts, sessions, subagents, sync, thread, titles, uploads, usage, workspace};
use portable_pty::{CommandBuilder, PtySize};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Instant, SystemTime};

/// A tab keeps this much of its session's output, replayed to a viewer that attaches late.
pub const OUTPUT_KEEP: usize = 256 * 1024;
/// Slack after a usage limit's reset time before the session is told to continue.
const LIMIT_MARGIN_SECS: u64 = 5;
const UNWATCHED: (u16, u16) = (120, 32);
/// Claude logs a turn's duration only after its Stop hook returns, within a second in practice.
const TURN_LOG_WAIT: (u32, std::time::Duration) = (25, std::time::Duration::from_millis(200));

pub type Event = MachineEvent;
/// Names a viewer of a tab, such as `local` for the app's own webview; attaching again replaces it.
pub type ViewerId = String;
/// Receives a tab's output. It runs on the session's reader thread with the tab's screen locked,
/// so it must not attach to or detach from that tab.
pub type Sink = Box<dyn Fn(&[u8]) + Send + Sync>;
pub type Listener = Box<dyn Fn(&Event) + Send + Sync>;

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
        })
    }

    pub fn bin(&self, provider: Provider) -> PathBuf {
        self.bins.get(&provider).cloned().unwrap_or_else(|| provider.spec().bin.into())
    }

    fn command(&self, program: Program, provider: Provider, tab: &str, cwd: &Path, bypass: bool, args: Vec<String>, env: &[(String, String)]) -> CommandBuilder {
        let spec = provider.spec();
        let mut cmd = CommandBuilder::new(self.bin(provider));
        for key in INHERITED_SESSION_ENV {
            cmd.env_remove(key);
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

/// What a Claude Code session sets for the processes it runs. pinned started from inside one (a
/// terminal tab of claude, or a node run from it) would pass them on, and each tab's claude
/// would then take itself for a nested session: no transcript, so `--resume` finds nothing.
pub(crate) const INHERITED_SESSION_ENV: [&str; 11] = [
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
];

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

struct Tab {
    cwd: PathBuf,
    account: Account,
    program: Program,
    session: Option<pty::Session>,
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
    /// Working tree and prompt as the running turn began, to diff against when it ends.
    turn: Option<(String, Option<String>)>,
    status: Option<AgentStatus>,
    updated: Option<u64>,
    title: Option<String>,
    message: Option<String>,
    /// Prompts typed before the session took the terminal, typed in once it does.
    held: Vec<String>,
    /// When the latest submit went in. Cleared by Done, Failed, and a new session.
    turn_pending: Option<Instant>,
    /// Printable bytes in the terminal's input line. Done does not clear it.
    draft_len: usize,
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

    fn view(&self, id: &str) -> TabView {
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
            stats: Some(self.tally.stats()).filter(|s| *s != ThreadStats::default()),
        }
    }

    /// A notice may be typed. Cursor never reports Done, so it is never idle.
    fn notice_idle(&self) -> bool {
        self.session.is_some() && self.status == Some(AgentStatus::Done) && self.turn_pending.is_none() && self.draft_len == 0
    }

    fn note_submit(&mut self) {
        self.turn_pending = Some(Instant::now());
    }
}

pub struct Machine {
    launch: Launch,
    default_cwd: PathBuf,
    /// The folder threads started without a project run in.
    scratch: PathBuf,
    tabs: Mutex<HashMap<TabId, Tab>>,
    next_tab: AtomicU32,
    sessions_file: Option<PathBuf>,
    sync_file: Option<PathBuf>,
    projects: Mutex<Vec<projects::Project>>,
    projects_file: Option<PathBuf>,
    accounts: Accounts,
    /// One file of finished turns per project.
    history_dir: Option<PathBuf>,
    /// Files clients attached, saved for sessions to read.
    uploads: Option<PathBuf>,
    /// LiteLLM's price list, downloaded at most daily for the usage page.
    rates_file: Option<PathBuf>,
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
    /// Wakes parents whose pty has gone quiet, once, for the life of the machine.
    notice_loop: AtomicBool,
    usage: Mutex<BTreeMap<accounts::AccountId, Usage>>,
    agents: Mutex<subagents::Cache>,
    delegates: crate::delegate::Delegates,
    events: Box<dyn Fn(Event) + Send + Sync>,
    /// Arcs, so `emit` can call them without holding the lock a listener might subscribe under.
    listeners: Mutex<Vec<(u64, Arc<Listener>)>>,
    next_listener: AtomicU64,
    me: Weak<Machine>,
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
        let home = home();
        let default_cwd = open
            .or_else(|| projects::recent(&projects).first().map(|p| p.path.clone()))
            .unwrap_or_else(|| home.clone());
        Arc::new_cyclic(|me| Machine {
            launch,
            default_cwd,
            scratch: dir.as_ref().map(|d| d.join("scratch")).unwrap_or_else(|| std::env::temp_dir().join("pinned-scratch")),
            tabs: Mutex::new(HashMap::new()),
            next_tab: AtomicU32::new(1),
            sessions_file: dir.as_ref().map(|d| d.join("sessions.json")),
            sync_file: dir.as_ref().map(|d| d.join("sync.json")),
            projects: Mutex::new(projects),
            projects_file,
            accounts: Accounts::load(dir.as_deref(), home.clone()),
            history_dir: dir.as_ref().map(|d| d.join("history")),
            uploads,
            rates_file: dir.as_ref().map(|d| d.join("usage-model-rates.json")),
            guides: dir.as_ref().map(|d| d.join("guides")).unwrap_or_else(|| std::env::temp_dir().join("pinned-guides")),
            shells: Shells::default(),
            device_hub: DeviceHub::default(),
            file_server: FileServer::default(),
            device_agents: dir.as_ref().map(|d| d.join("device-agents")),
            pr_watches: Default::default(),
            clones: home.join("pinned"),
            quitting: AtomicBool::new(false),
            notice_loop: AtomicBool::new(false),
            usage: Mutex::default(),
            agents: Mutex::default(),
            delegates: crate::delegate::Delegates::default(),
            events: Box::new(events),
            listeners: Mutex::new(vec![]),
            next_listener: AtomicU64::new(1),
            me: me.clone(),
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
            Call::SetModel { tab, model, cols, rows } => json(self.set_model(&tab, &model, cols, rows)?),
            Call::RestartClaude { tab, cols, rows } => json(self.restart_claude(&tab, cols, rows)?),
            Call::WriteInput { tab, data } => json(self.write_input(&tab, data.as_bytes())?),
            Call::ResizePty { tab, cols, rows } => json(self.resize_pty(&tab, cols, rows)?),
            Call::CloseTab { tab } => {
                self.close_tab(&tab);
                Ok(Value::Null)
            }
            Call::SetFolder { tab, path } => json(self.set_folder(&tab, &path)?),
            Call::ListProjects => json(self.list_projects()),
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
            Call::ListAccounts => json(self.list_accounts()),
            Call::AddAccount { provider, name } => {
                let account = self.accounts.add(provider, &name)?;
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
            Call::SignIn { account } => json(self.sign_in(&account)?),
            Call::SwitchAccount { tab, account, cols, rows } => json(self.switch_account(&tab, &account, cols, rows)?),
            Call::Thread { tab, from } => json(self.thread(&tab, from)?),
            Call::Prompt { tab, text } => json(self.type_prompt(&tab, &text)?),
            Call::Workspace { tab } => {
                let cwd = self.tab(&tab, |t| t.cwd.clone())?;
                json(workspace::Workspace { scratch: cwd == self.scratch, ..workspace::of(&cwd) })
            }
            Call::RegenerateTitle { tab, previous, thread } => {
                let cwd = self.tab(&tab, |t| t.cwd.clone())?;
                json(titles::regenerate(&self.launch.bin(Provider::Claude), &self.headless_env(&tab)?, &cwd, self.launch.path_env.as_deref(), &previous, &thread)?)
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
                json(commands::list(&self.accounts.home(&account), &cwd))
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
                json(branches::run_step(&cwd, &step, &self.launch.bin(Provider::Claude), &self.headless_env(&tab)?, self.launch.path_env.as_deref())?)
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
            Call::PageMeta { url } => json(panel::page_meta(&url)?),
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
            Call::PrGuide { tab, pr, write } => json(panel::guide(&self.tab(&tab, |t| t.cwd.clone())?, &pr, write, &self.guides, &self.launch.bin(Provider::Claude), &self.headless_env(&tab)?, self.launch.path_env.as_deref())?),
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
        }
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
        self.start_claude(tab, cols, rows)
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
        let (session, account, process) = self.tab(tab, |t| (t.claude_session.clone(), t.account.clone(), t.session.as_ref().and(t.started)))?;
        let Some(file) = session.as_deref().and_then(|s| self.find_transcript(&account, s)) else {
            return Ok(ThreadPage { session, items: vec![], next: 0, reset: false, context: None, branch: None, todos: None, agents: self.delegates.cards(tab) });
        };
        let page = thread::read(&file, from).map_err(|e| format!("could not read the transcript: {e}"))?;
        let mut agents = subagents::scan(&file.with_extension("").join("subagents"), &mut self.agents.lock().unwrap(), process);
        agents.extend(self.delegates.cards(tab));
        Ok(ThreadPage { session, items: page.items, next: page.next, reset: page.reset, context: page.context, branch: page.branch, todos: page.todos, agents })
    }

    /// Every tab, in the order they opened.
    pub fn tabs(&self) -> Vec<TabView> {
        let mut views: Vec<TabView> = self.tabs.lock().unwrap().iter().map(|(id, t)| t.view(id)).collect();
        views.sort_by_key(|v| v.opened);
        views
    }

    pub fn view(&self, tab: &str) -> Result<TabView, String> {
        self.tab(tab, |t| t.view(tab))
    }

    /// Adds a tab on the project at `path`, or the default folder, and returns its id.
    /// With `bypass` the tab's agent never asks for permission.
    pub fn open_tab(&self, path: Option<String>, bypass: bool) -> Result<TabId, String> {
        let cwd = match path {
            Some(path) if Path::new(&path) == self.scratch => self.scratch.clone(),
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

    /// Who each account's CLI says is signed in, asked of every account at once.
    pub fn list_accounts(&self) -> Vec<accounts::View> {
        std::thread::scope(|scope| {
            let asked: Vec<_> = self.accounts.list().into_iter().map(|a| scope.spawn(move || self.view_account(a))).collect();
            asked.into_iter().map(|h| h.join().unwrap()).collect()
        })
    }

    fn view_account(&self, account: Account) -> accounts::View {
        let status = self.status(&account);
        accounts::View::new(account, status)
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
        Ok(id)
    }

    /// Opens a tab running the CLI's login for the account, in the account's home, and returns its id.
    pub fn sign_in(&self, account: &str) -> Result<TabId, String> {
        let account = self.accounts.get(account).ok_or_else(|| format!("no account {account} on this machine"))?;
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

    /// Starts the tab's program, replacing a session already running there, and returns its
    /// folder. Viewers stay attached and see the new session from its first byte.
    pub fn start_claude(&self, tab: &str, cols: u16, rows: u16) -> Result<String, String> {
        let (cwd, account, program, args, env, bypass) = self.tab(tab, |t| {
            // Without launch arguments this is a fresh conversation, not the one recorded before.
            if t.launch.is_empty() {
                t.claude_session = None;
                t.tally = Tally::default();
            }
            let mut args = std::mem::take(&mut t.launch);
            if let Some(model) = t.model.clone().filter(|_| !args.iter().any(|a| a == "--model")) {
                args.extend(["--model".to_owned(), model]);
            }
            (t.cwd.clone(), t.account.clone(), t.program, args, t.env.clone(), t.bypass)
        })?;
        let mut args = args;
        if self.device_agents.as_ref().is_some_and(|f| f.exists()) {
            args.extend(["--append-system-prompt".to_owned(), crate::device::AGENT_PROMPT.to_owned()]);
        }
        self.save_sessions();
        let env = [self.accounts.env(&account), env].concat();
        let cmd = self.launch.command(program, account.provider, tab, &cwd, bypass, args, &env);
        self.spawn(tab, cmd, pty::size(cols, rows))?;
        self.continue_when_limit_lifts(tab);
        self.emit_tabs();
        Ok(cwd.display().to_string())
    }

    /// Moves the tab's conversation to `account` and restarts it there, returning its folder. The
    /// transcript is copied into the account's home, where `--resume` finds it, and a turn that
    /// stopped, as on a usage limit, continues at once instead of waiting for the old account.
    pub fn switch_account(&self, tab: &str, account: &str, cols: u16, rows: u16) -> Result<String, String> {
        let to = self.accounts.get(account).ok_or_else(|| format!("no account {account} on this machine"))?;
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
            t.account = to;
            t.resume_at = None;
            if let Some(saved) = t.saved() {
                (t.launch, t.resume_at) = saved.reopen(unix_now(), t.account.provider);
            }
        })?;
        self.start_claude(tab, cols, rows)
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
        self.start_claude(tab, cols, rows)
    }

    /// Restarts the tab's claude on its conversation with `model`, for this conversation alone, and
    /// returns its folder. Claude's own /model saves its pick as the default for every session.
    pub fn set_model(&self, tab: &str, model: &str, cols: u16, rows: u16) -> Result<String, String> {
        if model.starts_with('-') || !model.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.[]_".contains(&b)) {
            return Err(format!("{model:?} is not a model name"));
        }
        self.tab(tab, |t| {
            t.model = Some(model.to_owned());
            if let Some(saved) = t.saved() {
                (t.launch, t.resume_at) = saved.reopen(unix_now(), t.account.provider);
            }
        })?;
        self.start_claude(tab, cols, rows)
    }

    /// Starts claude again on the tab's conversation and returns its folder; a tab with no
    /// conversation yet starts a fresh one.
    pub fn restart_claude(&self, tab: &str, cols: u16, rows: u16) -> Result<String, String> {
        self.tab(tab, |t| {
            if let Some(saved) = t.saved() {
                (t.launch, t.resume_at) = saved.reopen(unix_now(), t.account.provider);
            }
        })?;
        self.start_claude(tab, cols, rows)
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
            t.started = Some(started);
            (t.status, t.title, t.message) = (None, None, None);
            t.turn_pending = None;
            t.draft_len = 0;
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
                machine.arm_notices();
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

    pub fn write_input(&self, tab: &str, data: &[u8]) -> Result<(), String> {
        if Shells::owns(tab) {
            return self.shells.write(tab, data);
        }
        let (submitted, mode_unknown) = self.tab(tab, |t| {
            let submitted = note_draft(&mut t.draft_len, data);
            if submitted {
                t.note_submit();
            }
            let mode_unknown = !t.permission_unknown && data.windows(3).any(|window| window == b"\x1b[Z");
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
            self.arm_notices();
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
        let (made, owned, last) = {
            let mut tabs = self.tabs.lock().unwrap();
            let made = tabs.get(tab).and_then(|t| t.made.clone());
            tabs.remove(tab);
            let owned: Vec<PathBuf> = tabs.values().filter_map(|t| t.made.clone()).collect();
            (made, owned, tabs.is_empty())
        };
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
        let path = dunce::canonicalize(path).unwrap_or_else(|_| path.into());
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
        let me = self.me.clone();
        let every = std::env::var("PINNED_PR_WATCH_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(every));
            let Some(machine) = me.upgrade() else { return };
            if machine.quitting.load(Ordering::Relaxed) {
                return;
            }
            machine.read_watches();
        });
    }

    fn read_watches(&self) {
        let tabs: Vec<String> = self.tabs.lock().unwrap().keys().cloned().collect();
        self.pr_watches.forget(&tabs);
        for tab in tabs {
            let Ok(cwd) = self.tab(&tab, |t| t.cwd.clone()) else { continue };
            for url in self.pr_watches.of(&tab) {
                let view = workspace::run(&cwd, self.launch.path_env.as_deref(), "gh", &["pr", "view", "--json", crate::prwatch::FIELDS, "--", &url]);
                if let Some(view) = view.ok().and_then(|v| serde_json::from_str(&v).ok()) {
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
            self.arm_notices();
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
        let ids: Vec<TabId> = sessions::load(file)
            .into_iter()
            .filter(|s| s.cwd.is_dir())
            .filter_map(|s| {
                // The transcript is in its account's home: with that account gone, nothing can resume it.
                let account = self.account_for(Some(&s.account), &s.cwd).ok()?;
                let (launch, resume_at) = s.reopen(now, account.provider);
                let id = self.add_tab(s.cwd, account, launch, s.env, s.bypass, Some(s.session));
                let _ = self.tab(&id, |t| (t.interrupted, t.resume_at, t.held_at_limit, t.model, t.permission_mode) = (s.interrupted, resume_at, s.held, s.model, s.permission_mode));
                Some(id)
            })
            .collect();
        self.emit_tabs();
        // A resumed session reports no new id, so its transcript is read here.
        for id in &ids {
            self.refresh_stats(id, false);
        }
        ids
    }

    /// Each account's usage, as its sessions last reported it.
    pub fn usage(&self) -> BTreeMap<accounts::AccountId, Usage> {
        self.usage.lock().unwrap().clone()
    }

    /// Answers the hook socket. Hook events and usage update the tabs; every other message, a
    /// connector session's callbacks and the reminders tools, goes to `other`.
    pub fn listen(&self, other: impl Fn(Message) -> Result<String, String> + Send + Sync + 'static) -> std::io::Result<()> {
        let me = self.me.clone();
        hooks::listen(self.launch.sock.clone(), move |message| {
            let Some(machine) = me.upgrade() else { return Err("pinned is shutting down".into()) };
            match message {
                Message::Agent(event) => {
                    machine.record_event(event);
                    Ok(String::new())
                }
                Message::Usage { usage: report, account } => {
                    let account = account.unwrap_or_else(accounts::default_account);
                    let usage = {
                        let mut all = machine.usage.lock().unwrap();
                        let known = all.entry(account.clone()).or_default();
                        *known = report.over(*known, unix_now());
                        *known
                    };
                    machine.emit(Event::Usage { usage, account });
                    Ok(String::new())
                }
                Message::Delegate { tab, task, tool, request } => machine.start_delegate(tab, task, tool, *request),
                Message::TaskStatus { tab, task, id } => machine.task_status(tab, task, &id),
                Message::TaskCancel { tab, task, id, reason } => machine.task_cancel(tab, task, &id, &reason),
                Message::Capabilities { tab, task, search } => machine.capabilities(tab.as_deref(), task.as_deref(), &search),
                message => other(message),
            }
        })
    }

    /// Ends every session as the machine shuts down, keeping them saved for the next launch.
    pub fn quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        let path = self.launch.path_env.clone();
        for id in self.delegates.running() {
            let _ = self.delegates.cancel_tree(&id, "pinned is shutting down", path.as_deref());
        }
        self.delegates.kill_remembered();
        let (tabs, owned) = {
            let tabs = self.tabs.lock().unwrap();
            let owned: Vec<PathBuf> = tabs.values().filter_map(|tab| tab.made.clone()).collect();
            let ids: Vec<String> = tabs.keys().cloned().collect();
            (ids, owned)
        };
        for tab in &tabs {
            self.delegates.drop_kept_worktrees(tab, &owned, path.as_deref());
        }
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
        self.delegates.status(&caller, id).map(|value| value.to_string())
    }

    fn task_cancel(&self, tab: Option<String>, task: Option<String>, id: &str, reason: &str) -> Result<String, String> {
        let caller = Self::require_caller(tab, task)?;
        self.delegates.cancel(&caller, id, reason, self.launch.path_env.as_deref()).map(|value| value.to_string())
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
        use crate::delegate::{driver_args, resolve_access, spawn, Access, ChildSpec, Claim, Isolation, NewTask, Parent, SpawnSpec};
        let caller = Self::require_caller(tab, parent_task)?;
        let parent = if let Some(tab) = caller.tab.clone() {
            Parent::Tab(tab)
        } else {
            Parent::Task(caller.task.clone().unwrap_or_default())
        };
        let (account_id, inherited_model, parent_provider, mut cwd, ceiling) = if let Some(tab) = caller.tab.as_deref() {
            self.tab(tab, |t| (Some(t.account.id.clone()), t.model.clone(), t.account.provider, t.cwd.clone(), t.child_access()))?
        } else if let Some(task) = caller.task.as_deref().and_then(|id| self.delegates.lineage(id)) {
            (Some(task.account), task.model, task.provider, task.cwd, task.access)
        } else {
            (None, None, Provider::Claude, self.default_cwd.clone(), Access::Auto)
        };
        let account = if let Some(id) = request.target.account.clone().filter(|id| !id.is_empty()) {
            self.accounts.get(&id).ok_or_else(|| format!("no account {id} on this machine"))?
        } else if let Some(id) = account_id {
            self.accounts.get(&id).unwrap_or_else(|| Account::builtin(Provider::Claude))
        } else {
            Account::builtin(Provider::Claude)
        };
        let bin = self.launch.bin(account.provider);
        if !bin.is_file() {
            return Err(format!("{} is not installed on this machine", account.provider.spec().name));
        }
        if !self.status(&account).signed_in {
            return Err(format!("account {} is not signed in on this machine", account.id));
        }
        let access = resolve_access(ceiling, &request)?;
        let mut model = crate::delegate::child_model(parent_provider, account.provider, inherited_model, request.target.model.clone());
        let mut effort = request.target.effort.clone().filter(|effort| !effort.is_empty());
        self.ensure_parent(caller.tab.as_deref(), None)?;
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
            caller_key: caller.key(),
            client_request_id: request.client_request_id.clone(),
            role: request.role.clone(),
        }) {
            Claim::Existing(id) => return self.delegates.started_json(&id).map(|value| value.to_string()),
            Claim::Fresh(task) => *task,
        };
        self.ensure_parent(caller.tab.as_deref(), Some(&task.id))?;
        if account.provider == Provider::Cursor {
            if let Some(name) = model.clone() {
                let ids = self.delegates.cursor_ids(&bin, self.launch.path_env.as_deref());
                model = Some(crate::delegate::map_cursor_model(&ids, &name, effort.as_deref()));
                self.delegates.set_model(&task.id, model.clone());
            }
            effort = None;
        }
        self.ensure_parent(caller.tab.as_deref(), Some(&task.id))?;
        if isolated {
            match branches::new_worktree(&cwd, self.launch.path_env.as_deref()) {
                Ok(dir) => {
                    if self.startup_stopped(&task.id, Some(&dir)) {
                        return self.delegates.started_json(&task.id).map(|value| value.to_string());
                    }
                    cwd = dir;
                    self.delegates.place(&task.id, cwd.clone(), true);
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
        let argv = driver_args(
            account.provider,
            &SpawnSpec {
                task: &request.task,
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
        let pid = match spawn(
            ChildSpec { bin: &bin, argv: &argv, cwd: &cwd, env: &env, path: self.launch.path_env.as_deref(), strip: &INHERITED_SESSION_ENV },
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
        self.arm_notices();
        self.delegates.started_json(&task.id).map(|value| value.to_string())
    }

    /// Idle means the session is up, the last report was Done, no submit is outstanding, and the
    /// input line is empty. A tab that never reports Done, such as Cursor, is never idle.
    fn parent_idle(&self, tab: &str) -> bool {
        self.tab(tab, |t| t.notice_idle()).unwrap_or(false)
    }

    /// Types one finished task's notice now. The tab must own the task. The idle gate does not apply.
    fn deliver_task(&self, tab: &str, task: &str) -> Result<(), String> {
        if self.tab(tab, |t| t.draft_len)? > 0 {
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

    fn maybe_deliver(&self, tab: &str) {
        if !self.parent_idle(tab) {
            return;
        }
        let Some((text, ids)) = self.delegates.claim_notice(tab) else { return };
        if self.write_notice(tab, &text) != Ok(true) {
            self.delegates.release_notice(&ids);
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

    /// Starts the once-a-second pass that types a waiting notice once its session is idle.
    fn arm_notices(&self) {
        if self.notice_loop.swap(true, Ordering::Relaxed) {
            return;
        }
        let me = self.me.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let Some(machine) = me.upgrade() else { return };
            if machine.quitting.load(Ordering::Relaxed) {
                return;
            }
            let tabs: Vec<String> = machine.tabs.lock().unwrap().keys().cloned().collect();
            for tab in tabs {
                machine.maybe_deliver(&tab);
            }
        });
    }

    fn capabilities(&self, tab: Option<&str>, task: Option<&str>, search: &str) -> Result<String, String> {
        use crate::delegate::{capabilities, codex_constraints, cursor_models, provider_models, Access, Inherited, ListedAccount};
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
                    Provider::Claude => (provider_models(Provider::Claude), vec![]),
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
                ListedAccount { id: account.id, provider: account.provider, name: account.name, signed_in: status.signed_in, bin_found: found, models, constraints }
            })
            .collect::<Vec<_>>();
        let search = (!search.is_empty()).then_some(search);
        Ok(capabilities(&parent, &listed, search).to_string())
    }

    fn open_delegated(&self, tab: &str, task: &str) -> Result<TabId, String> {
        use crate::delegate::Access;
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
            self.record_status(tab, &event);
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
        let id = format!("tab-{}", self.next_tab.fetch_add(1, Ordering::Relaxed));
        let tab = Tab {
            cwd,
            account,
            program: Program::Agent,
            session: None,
            started: None,
            launch,
            env,
            bypass,
            claude_session,
            tally: Tally::default(),
            interrupted: false,
            resume_at: None,
            held_at_limit: false,
            model: None,
            made: None,
            turn: None,
            status: None,
            updated: None,
            title: None,
            message: None,
            held: vec![],
            turn_pending: None,
            draft_len: 0,
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
        let _ = self.tab(tab, |t| t.session = None);
        self.emit(Event::Exit { tab: tab.to_owned() });
        self.emit_tabs();
    }

    /// Writes the tabs whose claude session is still going, in the order they were opened.
    fn save_sessions(&self) {
        let Some(file) = &self.sessions_file else { return };
        let saved: Vec<sessions::Saved> = {
            let tabs = self.tabs.lock().unwrap();
            let mut open: Vec<(u32, sessions::Saved)> = tabs.iter().filter_map(|(id, t)| Some((tab_number(id), t.saved()?))).collect();
            open.sort_by_key(|(n, _)| *n);
            open.into_iter().map(|(_, s)| s).collect()
        };
        sessions::save(file, &saved);
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
            if let Some(worktree) = reported_cwd.and_then(|c| sessions::worktree(&t.cwd, Path::new(c))) {
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
        }
        new
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
        let sessions: Vec<(TabId, Account, String, u64)> = self.tabs.lock().unwrap().iter().filter_map(|(id, t)| Some((id.clone(), t.account.clone(), t.claude_session.clone()?, t.view(id).updated.unwrap_or(0)))).collect();
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

fn tab_number(id: &str) -> u32 {
    id.trim_start_matches("tab-").parse().unwrap_or(0)
}

/// Counts the terminal's input line in characters. Printable text adds, including newlines inside
/// a bracketed paste. Backspace subtracts, floored at 0. Ctrl-C, Ctrl-U, and Enter clear the line.
/// Enter on a non-empty line is a submit. Up and down recall a prompt, so the line counts as
/// occupied until Enter, Ctrl-U, or Ctrl-C. Other escape sequences, such as focus reports, are ignored.
fn note_draft(draft_len: &mut usize, data: &[u8]) -> bool {
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
                    b"[A" | b"[B" | b"OA" | b"OB" => *draft_len = (*draft_len).max(1),
                    _ => {}
                }
                i += len;
            }
            b'\r' | b'\n' if pasting => *draft_len += 1,
            b'\r' => {
                submitted |= *draft_len > 0;
                *draft_len = 0;
            }
            0x03 | 0x15 => *draft_len = 0,
            0x08 | 0x7f => *draft_len = draft_len.saturating_sub(1),
            0x80..=0xbf => {}
            byte if byte >= 0x20 => *draft_len += 1,
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

fn repo_key(url: &str) -> Option<String> {
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
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::agent::{session_env, Subject};
    #[cfg(unix)]
    use crate::connectors::Delivery;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn launch() -> Launch {
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
        }
    }

    fn machine() -> Arc<Machine> {
        Machine::new(launch(), None, Some("/repo".into()), |_| {})
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
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
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
        assert_eq!(own, "tab-5");
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
                "id": "tab-1", "cwd": "/repo", "account": "claude", "program": "agent", "bypass": false, "running": false, "status": null,
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
        let working = AgentEvent { status: AgentStatus::Working, cwd: None, message: None, tab: Some(tab.clone()), session: None, title: None, resume_at: None, prompt: None, snapshot: None, permission_mode: None };
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
        assert_eq!(m.call(Call::OpenTab { path: None, bypass: true }), Ok(json!("tab-1")));
        assert_eq!(
            m.call(Call::Tabs),
            Ok(json!([{
                "id": "tab-1", "cwd": "/repo", "account": "claude", "program": "agent", "bypass": true, "running": false, "status": null,
                "title": null, "message": null, "resume_at": null, "connector": null, "opened": 1,
            }]))
        );
        assert_eq!(
            m.call(Call::SetFolder { tab: "tab-1".into(), path: "/nonexistent/acme".into() }),
            Err("/nonexistent/acme is not a folder on this machine".into())
        );
        assert_eq!(m.call(Call::CloseTab { tab: "tab-1".into() }), Ok(Value::Null));
        assert_eq!(m.call(Call::StartClaude { tab: "tab-1".into(), cols: 80, rows: 24 }), Err("no tab tab-1".into()));
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

        assert_eq!(m.call(Call::SetModel { tab: tab.clone(), model: "--evil".into(), cols: 80, rows: 24 }), Err("\"--evil\" is not a model name".into()));
        assert_eq!(m.call(Call::SetModel { tab: tab.clone(), model: "claude-opus-4-8".into(), cols: 80, rows: 24 }), Ok(json!(dir.display().to_string())));
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
        let failed = |resume_at| AgentEvent { status: AgentStatus::Failed, cwd: None, message: None, tab: Some(tab.clone()), session: None, title: None, resume_at, prompt: None, snapshot: None, permission_mode: None };
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
        let run = RunSpec { folder: dir.display().to_string(), repo: None, args: vec!["--resume".into(), "s1".into()], env: vec![], token: None, account: None };
        let id = m.open_run(&run).unwrap();
        assert_eq!(m.tab(&id, |t| t.claude_session.clone()).unwrap().as_deref(), Some("s1"));
        assert_eq!(m.thread(&id, 0).unwrap().session.as_deref(), Some("s1"));
        assert_eq!(m.view(&id).unwrap().session.as_deref(), Some("s1"), "the tab's view lacks the session clients key it by");
        let fresh = m.open_run(&RunSpec { args: vec![], ..run }).unwrap();
        assert_eq!(m.tab(&fresh, |t| t.claude_session.clone()).unwrap(), None);
        assert_eq!(m.view(&fresh).unwrap().session, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_does_not_inherit_the_session_pinned_was_started_from() {
        std::env::set_var("CLAUDE_CODE_SESSION_ID", "outer-session");
        std::env::set_var("CLAUDECODE", "1");
        let cmd = launch().command(Program::Agent, Provider::Claude, "tab-1", Path::new("/repo"), false, vec![], &[]);
        assert_eq!(cmd.get_env("CLAUDE_CODE_SESSION_ID"), None);
        assert_eq!(cmd.get_env("CLAUDECODE"), None);
        assert_eq!(cmd.get_env("PINNED_TAB").and_then(|v| v.to_str()), Some("tab-1"));
    }

    #[test]
    fn a_run_that_cannot_start_leaves_no_tab() {
        let dir = temp("run-unstartable");
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, "/nonexistent/claude".into())]), ..launch() }, None, Some("/repo".into()), |_| {});
        let run = RunSpec { folder: dir.display().to_string(), repo: None, args: vec![], env: vec![], token: None, account: None };
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
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into() }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into() }).unwrap();
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

    #[test]
    #[cfg(unix)]
    fn a_headless_claude_runs_under_the_tabs_claude_account() {
        let root = temp("account-headless");
        let claude = root.join("claude");
        std::fs::write(&claude, "#!/bin/sh\necho \"{\\\"title\\\":\\\"as $PINNED_ACCOUNT\\\"}\"\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(root.join("config")), None, |_| {});
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into() }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into() }).unwrap();
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
            "id": "work", "provider": "claude", "name": "Work", "provider_name": "Claude Code", "builtin": false,
            "signed_in": false, "email": null, "org": null, "plan": null,
        });
        assert_eq!(m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into() }), Ok(work.clone()));
        assert_eq!(m.call(Call::AddAccount { provider: Provider::Codex, name: "work".into() }), Err("an account work already exists".into()));
        assert_eq!(
            m.call(Call::ListAccounts),
            Ok(json!([
                {
                    "id": "claude", "provider": "claude", "name": "Claude Code", "provider_name": "Claude Code", "builtin": true,
                    "signed_in": true, "email": "ana@acme.dev", "org": "Acme", "plan": "max",
                },
                {
                    "id": "codex", "provider": "codex", "name": "Codex", "provider_name": "Codex", "builtin": true,
                    "signed_in": true, "email": null, "org": null, "plan": null,
                },
                {
                    "id": "cursor", "provider": "cursor", "name": "Cursor", "provider_name": "Cursor", "builtin": true,
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
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into() }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into() }).unwrap();
        m.open_project(app.clone());
        let run = |account: Option<&str>| {
            let spec = RunSpec { folder: app.display().to_string(), repo: None, args: vec!["Fix ENG-7".into()], env: vec![], token: None, account: account.map(Into::into) };
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
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into() }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into() }).unwrap();
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
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into() }).unwrap();
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into() }).unwrap();
        let config = root.join("config").display().to_string();
        let tab = m.call(Call::SignIn { account: "work".into() }).unwrap();
        assert_eq!(tab, json!("tab-1"));
        let (seen, sink) = viewer();
        m.attach("tab-1", "test".into(), sink).unwrap();
        wait_for("the login", || text(&seen).contains('\n'));
        assert_eq!(text(&seen), format!("claude|auth login|{config}/accounts/work||work\r\n"));
        let view = m.view("tab-1").unwrap();
        assert_eq!((view.cwd, view.account.as_str(), view.program, view.running), (format!("{config}/accounts/work"), "work", Program::SignIn, true));
        m.record_session("tab-1", "s1".into(), None);
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
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into() }).unwrap();
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
        m.call(Call::AddAccount { provider: Provider::Codex, name: "Side".into() }).unwrap();
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
            m.call(Call::AddAccount { provider, name: name.into() }).unwrap();
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
        let run = RunSpec { folder: "/Users/someone-else/app".into(), repo: Some("git@github.com:acme/app.git".into()), args: vec![], env: vec![], token: None, account: None };
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
        let real = String::from_utf8(std::process::Command::new("/bin/sh").args(["-c", "command -v git"]).output().unwrap().stdout)
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
            assert!(began.elapsed() < Duration::from_secs(5), "timed out waiting for worktree add");
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
        let request = crate::delegate::parse_request(&json!({ "task": "look", "mode": "async" })).unwrap();
        m.start_delegate(Some(tab), None, None, request).unwrap();
        wait_for("plan argv", || std::fs::read(&argv).is_ok_and(|b| !b.is_empty()));
        let lines: Vec<String> = String::from_utf8(std::fs::read(&argv).unwrap()).unwrap().lines().map(str::to_owned).collect();
        assert_eq!(lines, vec!["-p", "look", "--output-format", "stream-json", "--verbose", "--permission-mode", "plan"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    fn argv_of(m: &Machine, tab: &str, argv: &Path) -> Vec<String> {
        let _ = std::fs::remove_file(argv);
        let request = crate::delegate::parse_request(&json!({ "task": "look", "mode": "async" })).unwrap();
        m.start_delegate(Some(tab.to_owned()), None, None, request).unwrap();
        wait_for("child argv", || std::fs::read(argv).is_ok_and(|bytes| !bytes.is_empty()));
        String::from_utf8(std::fs::read(argv).unwrap()).unwrap().lines().map(str::to_owned).collect()
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
        let full = ["-p", "look", "--output-format", "stream-json", "--verbose", "--dangerously-skip-permissions"].map(str::to_owned).to_vec();
        let plan = ["-p", "look", "--output-format", "stream-json", "--verbose", "--permission-mode", "plan"].map(str::to_owned).to_vec();
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
            }],
        );
        let m = Machine::new(Launch { bins: BTreeMap::from([(Provider::Claude, claude)]), ..launch() }, Some(root.clone()), Some(repo), |_| {});
        let tab = m.restore_tabs().pop().expect("the plan session restored");
        let plan = ["-p", "look", "--output-format", "stream-json", "--verbose", "--permission-mode", "plan"].map(str::to_owned).to_vec();
        assert_eq!(argv_of(&m, &tab, &argv), plan);
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
        }
    }

    #[test]
    fn escape_sequences_are_not_a_draft_and_enter_submits() {
        let mut draft = 0usize;
        for keys in [&b"\x1b[I"[..], b"\x1b[O", b"\x1b", b"\x1b[C", b"\x1b[D", b"\x1b[?1;2c", b"\x1b[1;1R", b"\x1b]11;rgb:0/0/0\x07"] {
            assert!(!note_draft(&mut draft, keys), "{keys:?}");
            assert_eq!(draft, 0, "{keys:?} is not text");
        }
        for keys in [b"\x1b[A".as_slice(), b"\x1b[B", b"\x1bOA", b"\x1bOB"] {
            draft = 0;
            assert!(!note_draft(&mut draft, keys), "{keys:?}");
            assert_eq!(draft, 1, "{keys:?} recalls a prompt into the line");
            draft = 0;
            note_draft(&mut draft, b"xy");
            assert!(!note_draft(&mut draft, keys), "{keys:?}");
            assert_eq!(draft, 2, "{keys:?} must not shrink a longer draft");
        }
        draft = 0;
        note_draft(&mut draft, "é".as_bytes());
        assert_eq!(draft, 1, "one character, however many bytes");
        note_draft(&mut draft, b"\x7f");
        assert_eq!(draft, 0);
        assert!(!note_draft(&mut draft, b"\x1b[200~line one\rline two\x1b[201~"), "a newline inside a paste is not Enter");
        assert_eq!(draft, 17);
        note_draft(&mut draft, b"\x03");
        assert_eq!(draft, 0);
        assert!(!note_draft(&mut draft, b"\x1b[200~ab\x1b[201~"));
        assert_eq!(draft, 2, "a bracketed paste counts the text inside it");
        assert!(note_draft(&mut draft, b"\r"));
        assert_eq!(draft, 0);
        assert!(!note_draft(&mut draft, b"\r"), "enter on an empty line is not a submit");
        note_draft(&mut draft, b"ab");
        assert!(!note_draft(&mut draft, b"\x08"));
        assert_eq!(draft, 1);
        assert!(!note_draft(&mut draft, b"\x7f\x7f"));
        assert_eq!(draft, 0, "backspace stops at an empty line");
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
}
