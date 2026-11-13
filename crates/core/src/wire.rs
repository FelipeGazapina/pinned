//! The protocol between machines, the hub and clients: one JSON object per WebSocket text frame.
//! Every enum is internally tagged and unknown fields are ignored, so versions can drift.

use crate::accounts::{self, AccountId, Auth, Provider};
use crate::agent::{AgentEvent, AgentStatus, Role, Stage, Subject, Usage};
use crate::connectors::{BoxAccess, BoxSize, Connector, Settings};
use crate::schedule;

/// What an Upstash Box hour costs in USD. The app previews budgets from it; the hub enforces them.
pub const BOX_USD_PER_HOUR: f64 = 0.10;
use crate::branches::GitStep;
use crate::scripts::Script;
use crate::panel::{Candidates, MergeMethod, PrChange, ReviewComment, ReviewEvent, StackAction, StackHead};
use crate::workspace::{DiffScope, PrAction};
use crate::stats::{PrLink, ThreadStats};
use crate::global_orchestrator::{self as global, BusyPolicy, CaptainReport, RelayTo};
use crate::taste::{TasteField, TastePatch, TasteScope, Tastes};
use crate::{pulls, subagents, sync, thread, usage};
use base64::Engine;
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

/// A tab's id. It names one tab of one process: `tab-<boot>-<n>`, where `boot` changes when the process starts and `n` is the opening order.
pub type TabId = String;
/// The id the hub gives a machine, a uuid.
pub type MachineId = String;

/// Stands for the pinned binary in a `RunSpec`'s arguments, since the hub builds prompts without
/// knowing where each machine installed it. The machine that runs it puts its own path there.
pub const EXE_PLACEHOLDER: &str = "{{pinned}}";

/// Terminal bytes, which need not be UTF-8, carried as a base64 string.
#[derive(Clone, Debug, PartialEq)]
pub struct Bytes(pub Vec<u8>);

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(text).map(Bytes).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Bytes {
    fn schema_name() -> Cow<'static, str> {
        "Bytes".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::Bytes").into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string" })
    }
}

/// When the machine slept: the last tick before it and the first after it, in unix seconds.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct CutBySleep {
    pub slept_at: u64,
    pub woke_at: u64,
}

/// What a tab runs.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Program {
    /// The account's agent CLI, a session that may be resumed.
    #[default]
    Agent,
    /// The CLI's login for the account, never resumed.
    SignIn,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct TabView {
    pub id: TabId,
    pub cwd: String,
    /// The account the tab's sessions run under.
    #[serde(default = "accounts::default_account")]
    pub account: AccountId,
    #[serde(default)]
    pub program: Program,
    pub bypass: bool,
    pub running: bool,
    pub status: Option<AgentStatus>,
    pub title: Option<String>,
    /// What the session asks, while it needs input.
    pub message: Option<String>,
    /// Unix time a usage limit lifts, while one stopped the turn.
    pub resume_at: Option<u64>,
    /// The session waits out its usage limit without continuing by itself when it lifts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
    pub connector: Option<String>,
    /// What the connector's session works on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Subject>,
    /// The tab's number, which orders tabs by when they opened.
    pub opened: u64,
    /// Unix time the session last reported a status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<u64>,
    /// Claude Code's id for the conversation, which outlives the tab's id across restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// A turn was running when pinned quit, so the tab has work left to continue.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interrupted: bool,
    /// The turn died on an API error while the machine slept, so it stopped where the sleep cut it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_by_sleep: Option<CutBySleep>,
    /// What the conversation's work came to so far, once its transcript holds any of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<ThreadStats>,
    /// The git branch of the conversation's last message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The last pull request the conversation opened or worked on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<PrLink>,
    /// Unix seconds of the conversation's last message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<u64>,
    /// The conversation's first prompt, its first line cut short. It stands in for `title`, which
    /// a restored tab has lost until its next prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_title: Option<String>,
    /// The tab is its project's orchestrator, or a worker the orchestrator started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// The tab that started this one: the thread that delegated a worker, or the thread whose
    /// delegated task this tab reopened. The sidebar nests the tab under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_by: Option<TabId>,
    /// The machine whose tab `spawned_by` names, when that tab is on another machine: a worker a
    /// thread there delegated here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_on: Option<MachineId>,
    /// The tab is the account's global orchestrator, the chat above every project's Capitão. Its
    /// `role` is empty, so an older app shows it as a plain thread.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub orchestrator_global: bool,
    /// The tab is a Capitão running a turn that a message from the global orchestrator started,
    /// which it may not have reported yet: a message to it queues.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub orchestrator_turn: bool,
    /// How many messages wait for the tab to go idle before they are typed into it: its workers'
    /// news and the global orchestrator's requests.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub queued: u32,
    /// What the thread and the workers it delegated cost so far, on the thread that delegated
    /// them. Workers are not counted again under it: its card is the one place the total shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<crate::delegate::WorkTotal>,
    /// The tab was opened from the Way of working page to adjust the account's tastes by talking,
    /// and its session has the taste tools. Its `role` is empty, so an older app shows a plain thread.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub taste_chat: bool,
    /// What the session's last finished turn asked the user, read from its last reply. Cleared when
    /// the next turn starts or fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// While the session waits on an approval or a question mid-turn: the tool call or the question
    /// it waits on, read from its transcript, such as the command a permission prompt shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_detail: Option<String>,
    /// A process runs the tab's session now. `running` also holds for a conversation that sleeps.
    #[serde(default)]
    pub alive: bool,
    /// The conversation sleeps with no process: a message wakes it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub asleep: bool,
    /// The tab shows a headless child, which works with no session of the tab's own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub headless: bool,
    /// Why the tab's last process ended mid-turn, until a new turn or input starts it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
    /// The questionnaire the session waits on the user to answer, which `Call::AnswerQuestions`
    /// answers. While it is open the tab reads as a question mid-turn, so an app that predates it
    /// still shows the tab waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questions: Option<crate::questions::Questionnaire>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct ProjectView {
    pub path: String,
    pub name: String,
    /// The origin as `host/owner/name`, which names the project on every machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The account its sessions run under, the default one unless the user picked another.
    #[serde(default = "accounts::default_account")]
    pub account: AccountId,
    /// The Convex deployment the folder's `.env.local` or `.env` names in `CONVEX_DEPLOYMENT`, as
    /// written: `dev:quick-lemur-789`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convex: Option<String>,
}

/// Everything a client may ask a machine to do, local or through the hub. The comment on each
/// variant names what `Machine::call` answers.
///
/// `{"cmd":"start_claude","tab":"tab-1","cols":120,"rows":32}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Call {
    /// `Vec<TabView>`.
    Tabs,
    /// The new tab's id. `path: null` opens the machine's default folder.
    OpenTab { path: Option<String>, bypass: bool },
    /// The new tab's id, in the machine's folder for threads without a project.
    OpenScratch { bypass: bool },
    /// Null. Whether the tab's session continues by itself once the usage limit that stopped it
    /// lifts, which it does unless turned off.
    AutoResume { tab: TabId, on: bool },
    /// The folder claude runs in. Restarts claude when it is already running.
    StartClaude { tab: TabId, cols: u16, rows: u16 },
    /// Null. The tab is on the client's screen: a session that sleeps starts at this size, and
    /// one that runs stays awake. The client sends it again while the tab stays there.
    Wake { tab: TabId, cols: u16, rows: u16 },
    /// The folder claude runs in. Restarts claude with `bypass` turned `on` or off, resuming the
    /// tab's conversation; a session already running that way is left alone.
    SetBypass { tab: TabId, on: bool, cols: u16, rows: u16 },
    /// The folder claude runs in. Restarts claude on the tab's conversation with `--model model`,
    /// which picks it for that conversation alone. With `account`, moves the conversation to that
    /// account first when it is another one, so a model from another provider keeps the thread.
    SetModel {
        tab: TabId,
        model: String,
        cols: u16,
        rows: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<AccountId>,
    },
    /// The folder claude runs in. Restarts claude on the tab's conversation, so its skills,
    /// plugins and MCP servers load again.
    RestartClaude { tab: TabId, cols: u16, rows: u16 },
    /// `null`. Keystrokes as xterm reports them.
    WriteInput { tab: TabId, data: String },
    /// `null`.
    ResizePty { tab: TabId, cols: u16, rows: u16 },
    /// `null`.
    CloseTab { tab: TabId },
    /// `null`, once the tab's turn is stopped as the app's Stop button stops it: claude's background
    /// agents and workflows first, then the turn, then the turn their "stopped" notices wake.
    StopTurn { tab: TabId },
    /// `null`, once `reply` closes the questionnaire `id` open on the tab. Refused when it is
    /// closed already, or the reply does not fit its questions.
    AnswerQuestions { tab: TabId, id: String, reply: crate::questions::QuestionsReply },
    /// The tab's new folder, canonical. Stands in for the folder dialog on a remote machine;
    /// the client then sends `StartClaude`.
    SetFolder { tab: TabId, path: String },
    /// `Vec<ProjectView>`, most recent first.
    ListProjects,
    /// `Vec<history::Turn>`, newest first, only `session`'s when given.
    ProjectHistory {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },
    /// The folder's `origin` remote, or `null`.
    GitRemote { path: String },
    /// `projects::Folders`: the subfolders matching a path as typed so far, such as `~/pro`.
    Browse { path: String },
    /// `scaffold::Created`: a new folder named after `name` in `~/pinned`, a git repository with
    /// a first commit, opened as a project.
    CreateProject { name: String },
    /// The URL of the private GitHub repository the project at `path` is now pushed to.
    PublishProject { path: String },
    /// The folder of a clone of `repo`, found among the machine's projects or cloned into `~/pinned`,
    /// or cloned into `dest` when given, which must not hold files yet.
    /// The hub adds the account's GitHub token, for a private repository.
    Clone {
        repo: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dest: Option<String>,
    },
    /// The folder `path`, canonical, made first with its parents when it doesn't exist.
    MakeFolder { path: String },
    /// `Vec<ThreadMatch>`: the tabs whose conversations hold `query`, prompts' matches first.
    SearchThreads { query: String },
    /// The new tab's id, with claude already running. Refused when the folder can't be found or cloned.
    Run { run: RunSpec },
    /// The new tab's id, as `Run` makes it, but not started: the hub reopens an archived conversation
    /// for an agent this way, which then starts it with `start_claude`, under the limits.
    OpenRun { run: RunSpec },
    /// The new tab's id, with claude already running, for a workflow's Run now: a task held to the
    /// limits of the project it runs in, whose first finished turn ends it and goes to the
    /// delegation metrics. Refused past the limits. Only the hub sends it, for `run_workflow`;
    /// inside `ActFor` the asking session's own task narrows the limits and hears when it ends.
    RunNow { run: RunSpec, title: String },
    /// `true` when a live tab whose environment holds every pair took `text` as a prompt.
    Deliver { env: Vec<(String, String)>, text: String },
    /// The absolute path the file was saved under, in a private folder of the machine, for a
    /// session to read. Refused past `uploads::LIMIT`.
    Upload { name: String, data: Bytes },
    /// The `data:` URL of an image upload at `path`, so a chat draws it from any machine. Refused
    /// outside the uploads folder, for a file that isn't a picture, and past 8 MB.
    ReadUpload { path: String },
    /// The `data:` URL of the picture file at `path`, so a chat draws a picture a reply names by
    /// path. `path` is absolute or starts with `~/`. Refused otherwise, for a file that isn't a
    /// picture, and past 8 MB.
    ReadImage { path: String },
    /// `Vec<accounts::View>`: the built-in accounts, then the added ones, each as its CLI reports it.
    ListAccounts,
    /// `Vec<claude_models::ClaudeModel>`: Claude's models after keeping the rows of claude's
    /// /model menu that `lines`, a tab's screen, shows. Refused when the menu isn't on it.
    LearnModelMenu { lines: Vec<String> },
    /// The new account's `accounts::View`. Its id is a slug of `name`. `auth` is `opencode-go`
    /// when `key` is an OpenCode Go API key.
    AddAccount {
        provider: Provider,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<Auth>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// The account's `accounts::View`. Refused for an account that signs in with a CLI login.
    SetAccountKey { account: AccountId, key: String },
    /// `null`. Projects that ran under it fall back to the default account.
    RemoveAccount { account: AccountId },
    /// The project's `ProjectView`. `account: null` runs it under the default account again.
    SetProjectAccount { path: String, account: Option<AccountId> },
    /// The id of the project's orchestrator tab: the one already open, else a new one, not yet started.
    OpenOrchestrator { path: String },
    /// `global_orchestrator::Handoff`. Hands `text` to the Capitão of the project at `path`,
    /// opening it when missing, as `[<sender>] <text>`: typed now when it is idle, its first prompt
    /// when it is stopped, else queued until it is idle, or refused as busy when `busy` (else this
    /// machine's setting) says so. A repeat of `request_id` answers the first outcome and types nothing.
    Enqueue {
        path: String,
        text: String,
        request_id: String,
        /// The orchestrator's name; blank takes this machine's.
        #[serde(default)]
        sender: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        busy: Option<BusyPolicy>,
    },
    /// `global_orchestrator::Handoff`. Hands a Capitão's report to this machine's global
    /// orchestrator, opening it when missing, as `CaptainReport::line`. It never refuses as busy. A
    /// repeat of `request_id` answers the first outcome and types nothing.
    OrchestratorReport { request_id: String, report: CaptainReport },
    /// The id of this machine's global orchestrator tab: the one already open, else a new one, not
    /// yet started, in `~/.pinned/orquestrador`, made when missing and never listed as a project.
    OpenGlobalOrchestrator,
    /// The id of a new tab, not yet started, whose session adjusts `scope`'s way of working with
    /// the taste tools and whose first prompt says so. It opens in the project folder `path`, or
    /// in the folder for threads without a project.
    OpenTasteChat {
        scope: crate::taste::TasteScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// The started task's JSON, as `delegate_task` answers it. From the hub: starts a worker
    /// thread here for a tab on another machine of the account. `origin.machine` is the hub's
    /// stamp of the machine that asked; the thread nests under `origin.tab` there. `repo` names
    /// the checkout it works in, cloned with `token` when this machine has none. A repeat of
    /// `origin.task` answers the task it started.
    Delegate {
        origin: crate::delegate::Origin,
        request: Box<crate::delegate::Request>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// The task's status JSON, as `task_status` answers it, or its state once cancelled when
    /// `cancel` gives a reason. From the hub: only for the task `Delegate` started for `tab` and
    /// `task` of `origin_machine`, the hub's stamp of the machine that asks.
    RemoteTask {
        origin_machine: MachineId,
        tab: TabId,
        task: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel: Option<String>,
    },
    /// `null`. From the hub: the task this machine's tab `tab` sent to `from` with `delegate_task`
    /// ended there. `from` is the hub's stamp of the machine that reports it.
    RemoteEnded { from: MachineId, tab: TabId, task: String, end: crate::delegate::RemoteEnd },
    /// `null`. From the hub: the worker of the task this machine's tab `tab` sent to `from` waits
    /// on the user for `ask`, or, with none, waits on nothing now.
    RemoteAsked {
        from: MachineId,
        tab: TabId,
        task: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ask: Option<crate::delegate::RemoteAsk>,
    },
    /// `global_orchestrator::Settings` of this machine.
    GlobalOrchestratorSettings,
    /// The `global_orchestrator::Settings` saved, with a blank name back to the default one.
    SetGlobalOrchestratorSettings { settings: global::Settings },
    /// `auto_archive::Settings` of this machine: when it archives a finished session.
    AutoArchiveSettings,
    /// The `auto_archive::Settings` saved, answered as saved.
    SetAutoArchiveSettings { settings: crate::auto_archive::Settings },
    /// `on`, as saved. Whether the desktop app on this machine runs in the background, with no
    /// window, which it applies at once: on closes the window, off opens it. A machine without the
    /// app refuses it.
    SetBackground { on: bool },
    /// `reading::read` of the tab's session: its state and the last `items` items of its
    /// conversation, 20 when not given, read from the end of its transcript however long it is.
    ReadSession {
        tab: TabId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items: Option<usize>,
    },
    /// `repo_look::look` of the checkout at `path`, a project folder of this machine: read-only,
    /// with secrets left out. The global orchestrator's repo tools, sent by the hub.
    RepoLook { path: String, look: crate::repo_look::RepoLook },
    /// `Vec<memory::Memory>`: the project's memories that best match `query`, best first, or the
    /// latest ones for an empty query.
    MemorySearch {
        path: String,
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// The `memory::Memory` saved. With `id`, replaces that memory. Refused when the project
    /// already has a memory with the same title or nearly the same meaning.
    MemorySave {
        path: String,
        title: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<i64>,
    },
    /// The `memory::Memory` removed.
    MemoryForget { path: String, id: i64 },
    /// `memory::Status`: how many memories the project has and whether search also matches by meaning.
    MemoryStatus { path: String },
    /// The new tab's id, running the CLI's login for the account in the account's home.
    SignIn { account: AccountId },
    /// The long-lived token `claude setup-token` prints for the Claude account `account`, which the
    /// user approves in the browser it opens on this machine. It runs in no tab, and the machine
    /// answers it only to the hub, which keeps it for that account's boxes.
    SetupToken { account: AccountId },
    /// Whether a `SetupToken` for `account` was waiting, which this stops.
    CancelSetupToken { account: AccountId },
    /// The tab's folder, as `start_claude`. Moves the tab's conversation to another Claude Code
    /// account signed in here and resumes it there, continuing a turn a usage limit stopped.
    SwitchAccount { tab: TabId, account: AccountId, cols: u16, rows: u16 },
    /// `ThreadPage`: the conversation's items from byte `from` of its transcript on. With `limit`,
    /// the newest whole turns holding at least `limit` items before byte `before` instead, or
    /// before the end, read back from there; `from` is then unused and the page's `start` is the
    /// `before` of the page older than it.
    Thread {
        tab: TabId,
        from: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<u64>,
    },
    /// `null`. Types `text` into the session as one prompt and submits it.
    Prompt { tab: TabId, text: String },
    /// `workspace::Workspace` of the tab's folder.
    Workspace { tab: TabId },
    /// What `gh pr view` answers for `pr`, a number or URL, run in the tab's folder, plus `diff`.
    PullRequest { tab: TabId, pr: String },
    /// `workspace::PrStatus`: whether `pr` is open, a draft, merged or closed, without its diff.
    PrStatus { tab: TabId, pr: String },
    /// `Option<String>`: the icon of the tab's project, where T3 Code looks for one, as a data URL.
    Favicon { tab: TabId },
    /// `String`: a new title for the tab's conversation, which `claude -p` writes from `thread`, its
    /// messages, and the `previous` title. An empty `thread` reads the tab's own conversation.
    RegenerateTitle { tab: TabId, previous: String, thread: String },
    /// What `gh pr list` answers in the tab's folder: number, title, state, isDraft, url, headRefName.
    PrList { tab: TabId },
    /// `null`. Merges `pr`, or marks it ready, through gh in the tab's folder.
    PrAction { tab: TabId, pr: String, action: PrAction },
    /// `workspace::Diff`: the tab's working tree against HEAD or the default branch.
    Diff {
        tab: TabId,
        scope: DiffScope,
        #[serde(default)]
        ignore_whitespace: bool,
        /// The branch `DiffScope::Base` compares with, instead of the default one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<String>,
    },
    /// `Vec<workspace::Listener>`: the machine's web servers, the tab's session's own first.
    Ports { tab: TabId },
    /// `Vec<workspace::Editor>` installed on the machine.
    Editors,
    /// `null`. Opens the tab's folder, or `path` inside its repository, in the editor `editor`;
    /// `default` opens it in the app the system picks. `folder` takes `path` from the tab's folder
    /// instead, and `reveal` shows it in the file manager.
    OpenIn {
        tab: TabId,
        editor: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        folder: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reveal: bool,
    },
    /// `Vec<commands::Command>`, the slash commands claude offers in the tab's folder.
    Commands { tab: TabId },
    /// `Vec<String>`: the tab's folder's files and folders, relative, for `@` mentions.
    Files { tab: TabId },
    /// `skills::Listing`: every skill folder on the machine.
    Skills,
    /// `skills::SkillText` of `file` in the listed skill folder at `dir`.
    ReadSkill {
        dir: String,
        #[serde(default = "skill_md")]
        file: String,
    },
    /// `skills::SaveReport`. Writes `text` to `file` in each listed, writable copy that is
    /// unchanged since its `since`, then reloads the sessions that load them.
    SaveSkill {
        #[serde(default = "skill_md")]
        file: String,
        text: String,
        copies: Vec<crate::skills::Target>,
    },
    /// `branches::Branches` of the tab's folder.
    Branches { tab: TabId },
    /// `null`. Switches the tab's folder to `branch`, a new one when `create`. Refused while claude works there.
    Checkout {
        tab: TabId,
        branch: String,
        #[serde(default)]
        create: bool,
        /// Makes or resets `branch` at this ref instead, fetched first when it is `origin/…`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    /// `pulls::PullRequests` of the GitHub repositories of the machine's folders `paths`, through gh.
    PullRequests {
        paths: Vec<String>,
        state: pulls::ListState,
        involvement: pulls::Involvement,
        #[serde(default)]
        query: String,
        #[serde(default)]
        filters: pulls::Filters,
        /// Rows per search at most; the page asks for more 100 at a time.
        #[serde(default = "pulls_limit")]
        limit: usize,
    },
    /// `pulls::Stack` the pull request `number` of github.com's `repo` (`owner/name`) sits in, or `null`.
    PullRequestStack { repo: String, number: u64 },
    /// `pulls::FilterOptions` of `kind` matching `query` in the GitHub repositories of the folders `paths`.
    PullRequestFilterOptions {
        paths: Vec<String>,
        kind: pulls::OptionKind,
        #[serde(default)]
        query: String,
    },
    /// `Vec<pulls::FileStat>` of pull request `number` of `repo` on `host`, or `null` when it changes too many files.
    PullRequestFiles { host: String, repo: String, number: u64 },
    /// `Vec<usage::Row>`: the tokens claude's transcripts on the machine record from unix time `since`
    /// on, priced with the machine's `usage::PriceBook` over LiteLLM's list.
    TokenUsage { since: u64 },
    /// `usage::PriceBook`: the prices and model mappings the user set on the machine.
    UsagePrices,
    /// The `usage::PriceBook` after setting each model's price and mapping, or clearing it with `null`.
    SetUsagePrices {
        #[serde(default)]
        prices: HashMap<String, Option<usage::Price>>,
        #[serde(default)]
        aliases: HashMap<String, Option<String>>,
    },
    /// `usage::Limits`: the account and plan limits claude's own usage check reports.
    UsageLimits,
    /// `panel::PageMeta` of the web page at `url`, fetched from this machine.
    PageMeta {
        url: String,
        /// The origin of the page that would frame it, when a web app asks. A policy that names that
        /// origin lets the page through.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame_by: Option<String>,
    },
    /// `panel::Refs`: the tab's folder's local and remote branches, for picking what to compare with.
    Refs { tab: TabId },
    /// What a step of the header's Git action answers: a commit message, a pull request URL, or
    /// `null`. Refused while claude works in the tab's folder.
    Git { tab: TabId, step: GitStep },
    /// `scripts::ProjectScripts` of the tab's project.
    ProjectScripts { tab: TabId },
    /// `null`. Replaces the scripts of the tab's project.
    SaveProjectScripts { tab: TabId, scripts: Vec<Script> },
    /// `null`. Checks out the pull request `pr`, a number or URL, in the tab's folder with gh. Refused while claude works there.
    CheckoutPr { tab: TabId, pr: String },
    /// The tab's new folder. Moves a tab whose conversation hasn't begun to a worktree of its
    /// project, `"new"` for one made from what it has checked out, or a path, or back to the
    /// project's own checkout with `null`, and restarts claude there. Refused while claude works.
    SetWorkspace {
        tab: TabId,
        #[serde(default)]
        worktree: Option<String>,
        cols: u16,
        rows: u16,
    },
    /// `panel::FileText` of `path` in the tab's folder, or in its repository when `root`, as of `rev` when given.
    ReadFile {
        tab: TabId,
        path: String,
        #[serde(default)]
        root: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
    },
    /// The tab's watched pull request URLs, after starting or stopping the watch on `pr` when given.
    /// A watched pull request wakes the tab's claude when its checks finish, someone comments, or
    /// its branch starts to conflict.
    WatchPr {
        tab: TabId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pr: Option<String>,
        #[serde(default)]
        watching: bool,
    },
    /// A loopback URL that serves `path` with the folder around it, for a page that loads its own
    /// styles and scripts.
    FileUrl {
        tab: TabId,
        path: String,
        #[serde(default)]
        root: bool,
    },
    /// The saved file's new hash. Refused when the file changed since the read that answered `since`.
    WriteFile {
        tab: TabId,
        path: String,
        #[serde(default)]
        root: bool,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<String>,
    },
    /// `Vec<panel::Hit>`: the lines of the tab's folder that hold `query`.
    Search {
        tab: TabId,
        query: String,
        #[serde(default)]
        case: bool,
        #[serde(default)]
        regex: bool,
    },
    /// `null`. Retitles `pr` or rewrites its description.
    PrEdit {
        tab: TabId,
        pr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
    },
    /// `null`. Comments on `pr`, or answers its review thread `reply_to`.
    PrComment {
        tab: TabId,
        pr: String,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
    },
    /// `null`. Submits a review of `pr` with its line comments.
    PrReview {
        tab: TabId,
        pr: String,
        event: ReviewEvent,
        #[serde(default)]
        body: String,
        #[serde(default)]
        comments: Vec<ReviewComment>,
    },
    /// `null`. Runs the check at `check`, a GitHub Actions link, again.
    PrRerun { tab: TabId, check: String },
    /// `{guide, run}`: the review guide kept for `pr` and the run writing one. `write` starts a run.
    PrGuide {
        tab: TabId,
        pr: String,
        #[serde(default)]
        write: bool,
    },
    /// `null`. Reacts on `pr` or a remark on it, rewrites a comment, asks for or takes back a
    /// review, puts a label on or off, or resolves a review thread.
    PrChange { tab: TabId, pr: String, change: PrChange },
    /// `{candidates, truncated}`: who `pr` may ask for a review, or the labels it may carry.
    PrCandidates { tab: TabId, pr: String, of: Candidates },
    /// `null`. Merges `pr` and the stack layers under it, or rebases its whole GitHub stack, if every
    /// unmerged layer still has the head in `heads`.
    PrStack {
        tab: TabId,
        pr: String,
        action: StackAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        method: Option<MergeMethod>,
        heads: Vec<StackHead>,
    },
    /// The id of a new login shell in the tab's folder, which `attach`, `write_input`, `resize_pty`
    /// and `close_shell` take as a tab id.
    OpenShell { tab: TabId, cols: u16, rows: u16 },
    /// `null`. Ends the shell `shell`.
    CloseShell { shell: String },
    /// `Vec<String>`: those of `shells` running a command, as T3 Code's terminal-running pulse reads them.
    BusyShells { shells: Vec<String> },
    /// The address of the machine's device hub, which mirrors its simulators and emulators. It
    /// starts on the first call; `embed` is the origin of a page that frames it.
    DeviceHub {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        embed: Option<String>,
    },
    /// Whether new sessions may drive the machine's simulators and emulators, after setting it to
    /// `allow` when given.
    DeviceAgents {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allow: Option<bool>,
    },
    /// What `op` answers, run against the machine's device hub, which starts first when it isn't
    /// running, and its simulator and emulator tools.
    Device {
        #[serde(flatten)]
        op: crate::device::DeviceOp,
    },
    /// `Vec<thread::Item>`: the conversation of the subagent the tab's `Agent` tool call `tool` started.
    Subagent { tab: TabId, tool: String },
    /// The folder claude runs in. Restarts the tab's claude on a copy of its conversation that stops before
    /// the prompt `before`, after putting the folder's files back as they were then when `files`.
    Rewind { tab: TabId, before: String, files: bool, cols: u16, rows: u16 },
    /// The new tab's id, not yet started: its claude resumes a copy of the tab's conversation through
    /// the message `through`.
    Fork { tab: TabId, through: String },
    /// The path saved, relative to the tab's folder. Writes `text` to `path` inside that folder.
    SaveFile { tab: TabId, path: String, text: String },
    /// The new tab's id, not yet started: it resumes the delegated task's conversation in the child's folder.
    OpenTask { tab: TabId, task: String },
    /// The task_status body. Stops the delegated task when this tab owns it.
    CancelTask { tab: TabId, task: String },
    /// `null`. Types that finished task's notice into the tab now. The tab must own the task.
    DeliverTask { tab: TabId, task: String },
    /// `null`. The tabs whose transcript changes this caller hears of as `MachineEvent::Thread`. Empty asks for none.
    WatchThreads { tabs: Vec<TabId> },
    /// The machine's `UpdateStatus`.
    UpdateSelf,
    /// Why the machine stopped syncing projects and plugins, or `null` while it syncs.
    SyncStopped,
    /// `load::MachineState`: the machine's load and accounts now, which the hub asks for before it
    /// refuses a delegation on what the machine reported earlier.
    State,
    /// `null`. Sets an unreadable sync state aside, so the next sync starts over as on a new
    /// machine. A readable one is left alone.
    ResetSync,
    /// What `call`, a `Call` as JSON, answers when run for the agent session in tab `caller` on this
    /// machine, as the hub passes on that session's `act`. A call that starts a session holds it to
    /// the caller's delegation limits (`surface::Action::starts_work`).
    ActFor { caller: TabId, call: Value },
    /// `true` once `text` is queued for the tab, as worker reports are: typed once its session is
    /// idle. A `tab` gone since, as after a restart, is found by its Claude `session`. `false` when
    /// no tab has it. A repeat of `id` queues nothing.
    Notice {
        tab: TabId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        text: String,
        id: String,
    },
    /// `Vec<mcp::Server>`: the MCP servers Claude Code sessions here load under `account`, else the
    /// account of the project at `path`, else the default one. User and local scope come from that
    /// account's `.claude.json`, project scope from `path`'s `.mcp.json`. Each names its program or
    /// its URL's origin only, never its arguments, environment or headers.
    McpServers {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<AccountId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// `Vec<mcp::Found>`: the servers `McpServers` lists, with their command, arguments and URL and
    /// the names of their environment and headers, never their values, for the app to import.
    McpImportable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<AccountId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn skill_md() -> String {
    "SKILL.md".into()
}

fn pulls_limit() -> usize {
    100

}

impl Call {
    /// Runs off the machine's call queue, which carries keystrokes in order: a run may clone a
    /// repository for minutes, gh waits on GitHub, and the others read files or run git.
    pub fn may_wait(&self) -> bool {
        if let Call::ActFor { call, .. } = self {
            return serde_json::from_value::<Call>(call.clone()).is_ok_and(|call| call.may_wait());
        }
        matches!(
            self,
            Call::Run { .. }
                | Call::State
                | Call::RunNow { .. }
                | Call::PullRequest { .. }
                | Call::PrStatus { .. }
                | Call::Favicon { .. }
                | Call::RegenerateTitle { .. }
                | Call::StopTurn { .. }
                | Call::PrList { .. }
                | Call::PrAction { .. }
                | Call::Thread { .. }
                | Call::ReadSession { .. }
                | Call::RepoLook { .. }
                | Call::Workspace { .. }
                | Call::ProjectHistory { .. }
                | Call::Diff { .. }
                | Call::Ports { .. }
                | Call::Editors
                | Call::OpenIn { .. }
                | Call::Commands { .. }
                | Call::Files { .. }
                | Call::Skills
                | Call::ReadSkill { .. }
                | Call::SaveSkill { .. }
                | Call::Branches { .. }
                | Call::Checkout { .. }
                | Call::ListAccounts
                | Call::AddAccount { .. }
                | Call::SetAccountKey { .. }
                | Call::SignIn { .. }
                | Call::SwitchAccount { .. }
                | Call::SetModel { .. }
                | Call::TokenUsage { .. }
                | Call::UsageLimits
                | Call::PullRequests { .. }
                | Call::PullRequestStack { .. }
                | Call::PullRequestFilterOptions { .. }
                | Call::PullRequestFiles { .. }
                | Call::PageMeta { .. }
                | Call::Refs { .. }
                | Call::CheckoutPr { .. }
                | Call::Git { .. }
                | Call::SetWorkspace { .. }
                | Call::ReadFile { .. }
                | Call::ReadUpload { .. }
                | Call::ReadImage { .. }
                | Call::FileUrl { .. }
                | Call::WatchPr { .. }
                | Call::WriteFile { .. }
                | Call::Search { .. }
                | Call::PrEdit { .. }
                | Call::PrComment { .. }
                | Call::PrReview { .. }
                | Call::PrRerun { .. }
                | Call::PrGuide { .. }
                | Call::PrChange { .. }
                | Call::PrCandidates { .. }
                | Call::PrStack { .. }
                | Call::DeviceHub { .. }
                | Call::Device { .. }
                | Call::DeviceAgents { .. }
                | Call::Subagent { .. }
                | Call::Rewind { .. }
                | Call::Fork { .. }
                | Call::SaveFile { .. }
                | Call::CreateProject { .. }
                | Call::PublishProject { .. }
                | Call::BusyShells { .. }
                | Call::SearchThreads { .. }
                | Call::MemorySearch { .. }
                | Call::MemorySave { .. }
                | Call::MemoryForget { .. }
                | Call::MemoryStatus { .. }
                | Call::UpdateSelf
                | Call::McpServers { .. }
                | Call::McpImportable { .. }
        )
    }
}

/// What a machine reports about replacing itself. `to` is the release it is moving to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    UpToDate { version: String },
    Updating {
        #[serde(default)]
        to: String,
    },
    Restarting { to: String },
    Failed {
        #[serde(default)]
        to: String,
        reason: String,
    },
    Cannot { reason: String },
}

/// A tab whose conversation holds what a `SearchThreads` asked for.
#[derive(Serialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct ThreadMatch {
    pub tab: TabId,
    #[serde(flatten)]
    pub found: thread::Found,
}

/// A step of a tab's conversation. A client holding items of another `session`, or getting a
/// `reset` page, starts over.
#[derive(Serialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct ThreadPage {
    pub session: Option<String>,
    pub items: Vec<thread::Item>,
    pub next: u64,
    pub reset: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<thread::Context>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<thread::Todos>,
    /// Every agent the session spawned, whole on each page. A machine on an older pinned leaves it out.
    pub agents: Vec<subagents::Subagent>,
    /// For a read with `limit`: where its items start in the transcript, the `before` of the page
    /// older than it. 0: nothing is older. A machine on an older pinned leaves it out and answers
    /// from `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
    /// For a read with `limit` that does not reach the start: the conversation's first prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening: Option<String>,
}

/// A connector's session, the same on any machine.
///
/// `{"folder":"/repo","repo":"git@github.com:o/r.git","args":["--name","ENG-7"],"env":[["PINNED_CONNECTOR","c1"]]}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct RunSpec {
    /// The folder on the machine the connector was set up on.
    pub folder: String,
    /// The folder's git origin, which finds or clones it on a machine without that folder.
    #[serde(default)]
    pub repo: Option<String>,
    /// `claude` arguments, with `EXE_PLACEHOLDER` where the pinned binary goes.
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// The account's GitHub token, added by the hub, which clones a private repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The agent account the connector names; none runs the session under its project's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountId>,
    /// What may be spent on the box running this, in USD; none leaves the hub's lifetime cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
}

/// The session a workflow's Run now started.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct RunStarted {
    pub workflow: String,
    pub machine: MachineId,
    pub machine_name: String,
    pub tab: TabId,
}

/// An interactive Upstash Box session: its phase, age and budget, for its cost display.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct BoxStatus {
    pub id: String,
    /// `creating`, `booting` or `running`.
    pub phase: String,
    pub machine: String,
    pub tab: Option<String>,
    /// Seconds since the box row was written.
    pub age_secs: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    pub rate_usd_per_hour: f64,
}

/// What a machine reports on its own.
///
/// `{"kind":"exit","tab":"tab-1"}`, `{"kind":"output","tab":"tab-1","data":"aGk="}`
#[derive(Serialize, Deserialize, Clone, Debug, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MachineEvent {
    /// Every tab, after any of them opened, closed, started, exited or changed status, title or folder.
    Tabs { tabs: Vec<TabView> },
    Agent { event: AgentEvent },
    /// Output of a tab a client attached to, as the hub passes it on.
    Output { tab: TabId, data: Bytes },
    /// The tab's claude exited by itself.
    Exit { tab: TabId },
    /// The account's usage, after its session reported it.
    Usage {
        usage: Usage,
        #[serde(default = "accounts::default_account")]
        account: AccountId,
    },
    /// The transcript of a tab this client watches changed. The client reads it with `Call::Thread`.
    Thread { tab: TabId },
    /// Some while this process is installing a release, None when it connects with nothing in progress.
    Update { status: Option<UpdateStatus> },
    /// Skills in these scopes changed on disk, by a save or a sync pull. An open Skills page lists again.
    SkillsChanged { scopes: Vec<crate::skills::Scope> },
    /// A taste chat's session changed the account's tastes. An open Way of working page reads them again.
    TastesChanged,
    /// The machine's load and accounts, every 20 seconds. The hub keeps the latest as `MachineView::state`
    /// and passes it to no client.
    State { state: crate::load::MachineState },
    /// Whether the desktop app runs in the background, with no window: when it connects and after
    /// each change. A machine without the app never sends it. The hub keeps it as
    /// `MachineView::background` and passes the event to no client.
    Background { on: bool },
}

/// Account-level calls, answered by the hub. Names and arguments match the app's Tauri
/// commands, so the settings page sends either. The last three come only from machines,
/// forwarding a connector session's callbacks to the hub, which holds the keys.
///
/// `{"cmd":"connect_linear","key":"lin_api_…"}`, `{"cmd":"stage","connector":"c1","issue":"uuid","stage":"coding"}`
#[derive(Serialize, Deserialize, Clone, schemars::JsonSchema)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum HubCall {
    /// `connectors::Settings`.
    GetSettings,
    /// `linear::Account`.
    ConnectLinear { key: String },
    DisconnectLinear,
    /// `connectors::Suggestions`.
    LinearSuggestions,
    /// `slack::Connected`: `slack::Account` with the bot and whether the token added it.
    ConnectSlack { token: String },
    DisconnectSlack {
        /// The bot to remove; empty for the default bot, which is what an app before several bots means.
        #[serde(default)]
        bot: String,
    },
    /// Channel names.
    SlackSuggestions {
        /// The bot whose channels to list; empty for the default bot.
        #[serde(default)]
        bot: String,
    },
    /// `github::Account`.
    ConnectGithub { token: String },
    DisconnectGithub,
    /// `Vec<github::Repo>`, most recently pushed first.
    GithubRepos,
    /// `vercel::Account`, once the gateway took the key.
    ConnectVercel { key: String },
    DisconnectVercel,
    /// `Vec<vercel::Model>`: chat models tagged `structured-output`, and every decision model. The list needs no key.
    VercelModels,
    /// `upstash::Account`, once Upstash took the key. Only a signed-in app runs boxes, so this has no Tauri command.
    ConnectUpstash { key: String, claude_token: String },
    /// Deletes the account's boxes before it forgets the keys.
    DisconnectUpstash,
    /// `connectors::BoxAccess`.
    SetBoxAccess { access: BoxAccess },
    /// `connectors::RepoInsight`: whether the global orchestrator may read the account's repos.
    SetRepoInsight { settings: crate::connectors::RepoInsight },
    /// Allows or refuses the pending box question `id`.
    AnswerBox { id: String, allow: bool },
    /// The folder of `repo` on `machine`, cloned there with the account's GitHub token when it has
    /// none, or into `dest` when given.
    Clone {
        machine: MachineId,
        repo: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dest: Option<String>,
    },
    /// `Vec<sync::Project>`: the account's projects, most recently shared first.
    SharedProjects,
    /// From a machine: a project it syncs, which other machines then offer to clone.
    ShareProject { project: sync::Project },
    /// From a machine: `Vec<sync::Entry>`, every file the hub holds for the project.
    SyncList { project: String },
    /// From a machine: `null`, once the hub keeps the files.
    SyncPush { project: String, files: Vec<sync::File> },
    /// From a machine: `Vec<sync::File>`.
    SyncPull { project: String, paths: Vec<String> },
    /// From a machine: `null`. Sets the executable bit when the hub still stores each sha. Sends no bytes.
    SyncMarkExecutable { project: String, repairs: Vec<sync::ModeRepair> },
    /// The connector as stored.
    SaveConnector { connector: Connector },
    DeleteConnector { id: String },
    /// `Vec<schedule::Task>`: the account's scheduled tasks, soonest first, each with its next run
    /// and its last run.
    ListSchedules,
    /// `schedule::SaveAnswer`. A task with no id is new; the hub gives it one. The hub validates it,
    /// computes `next_at`, and keeps who created it. With `dry_run` it only answers the next three run
    /// times in the task's zone, and stores nothing.
    SaveSchedule {
        task: schedule::Task,
        #[serde(default)]
        dry_run: bool,
    },
    /// The task as stored: paused (`enabled` false) or running again.
    PauseSchedule { id: String, paused: bool },
    DeleteSchedule { id: String },
    /// `Vec<schedule::TaskRun>`: the task's runs, newest first. No `limit` answers 20.
    ScheduleRuns {
        id: String,
        #[serde(default)]
        limit: Option<u32>,
    },
    /// The account's release-protection switch: tag and main protection exist, so a task may be
    /// pre-approved for a release. Only the user's app sets it; no agent action does. Answers `on`.
    SetReleaseProtection { on: bool },
    /// `RunStarted`: a session of the saved workflow `id`, started now with no trigger behind it,
    /// on the machine the workflow names and under the limits of its project there.
    RunWorkflow { id: String },
    /// Kept for older apps, whose Generate with AI button still sends it. The hub answers an error: an agent creates a workflow with `save_connector`.
    GenerateWorkflow { model: String, description: String },
    /// Starts an interactive session on a new Upstash Box cloning `repo`. Returns `BoxStatus`.
    OpenBox {
        repo: String,
        /// None follows the project's and the account's default box size (`Tastes::box_size`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<BoxSize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget_usd: Option<f64>,
    },
    /// `BoxStatus` for the box behind `machine`, or an error when it is no box.
    BoxStatus { machine: MachineId },
    /// Adds `extra_usd` to the box behind `machine`'s budget. Returns `BoxStatus`.
    ExtendBox { machine: MachineId, extra_usd: f64 },
    /// `Vec<MachineView>`.
    Machines,
    /// From a machine: `Vec<MachineView>`, each online machine with the summary it last reported, for
    /// `orchestrator_capabilities`.
    MachineStates,
    /// From a machine: `placement::Placed`, the machine of the account that runs a heavy worker it
    /// would start, by the account's placement policy. `repo` is the project's origin, `need` the
    /// account and model the worker runs on, and `exclude` the machines that refused it already.
    Place {
        #[serde(default)]
        repo: Option<String>,
        #[serde(default)]
        need: crate::placement::Need,
        #[serde(default)]
        exclude: Vec<MachineId>,
    },
    RenameMachine { id: MachineId, name: String },
    RemoveMachine { id: MachineId },
    /// `PairCode`.
    PairCode,
    /// `NewInvite`: a single-use code that lets one person create their own account on this hub.
    CreateInvite,
    /// `Invites`: the hub's signup mode and the account's codes nobody has used yet.
    Invites,
    /// `null`. The code stops working; an error when the account has no such unused code.
    RevokeInvite { id: String },
    /// A signed-out app's connectors, moved to the account on its first sign-in.
    Import {
        store: Value,
        linear_key: Option<String>,
        slack_key: Option<String>,
        /// Each Slack bot's token by bot id. `slack_key` stays the default bot's.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        slack_keys: BTreeMap<String, String>,
        #[serde(default)]
        github_key: Option<String>,
        #[serde(default)]
        vercel_key: Option<String>,
    },
    Stage {
        connector: String,
        #[serde(flatten)]
        subject: Subject,
        stage: Stage,
    },
    SlackReply { connector: String, channel: String, thread: String, text: String },
    LinearCreate {
        connector: String,
        channel: String,
        thread: String,
        title: String,
        description: String,
        #[serde(default)]
        assignee: String,
    },
    LinearTake { connector: String, channel: String, thread: String, issue: String },
    /// From a workflow session's `pinned run set`: stores `value` as `{{vars.<name>}}` of its live graph run.
    RunSet {
        connector: String,
        #[serde(flatten)]
        subject: Subject,
        name: String,
        value: String,
    },
    /// `ExternalAgents`: the hub's MCP endpoint and the external agents the account let in.
    ExternalAgents,
    /// `AgentRequest`: what the pending OAuth authorization `request` asks for, while it waits for the user.
    AgentRequest { request: String },
    /// `AgentRedirect`: lets the agent of the pending `request` in with `scopes` on `projects`, and
    /// answers where the browser goes next, the agent's redirect with a one-use code.
    ApproveAgent { request: String, scopes: Vec<String>, projects: Vec<AgentProject> },
    /// `AgentRedirect`: refuses the pending `request`, and answers the agent's redirect with `access_denied`.
    DenyAgent { request: String },
    /// `null`. Ends the agent's access at once: its tokens stop working.
    RevokeAgent { id: String },
    /// From a machine: `global_orchestrator::Handoff` of the machine `to` names. Carries a message
    /// from the global orchestrator to a Capitão on another machine, or a Capitão's report up to the
    /// account's global orchestrator. `request_id` makes a repeat answer the first outcome.
    Relay { request_id: String, to: RelayTo },
    /// `global_orchestrator::Overview`: the account's projects on every machine, and what waits on
    /// the user.
    Overview,
    /// From a machine: the latest replies of the Capitão of the project at `path` on `machine`, as
    /// `reading::read` answers them, with `project` set to `path`.
    ReadCaptain { machine: MachineId, path: String },
    /// `taste::Tastes`: the account's, or the defaults while it has none. A machine asks too, when it
    /// starts a Capitão or the global orchestrator.
    GetTastes,
    /// `taste::Tastes`, once `patch` is applied to `scope` as `Tastes::set` does.
    SetTaste { scope: TasteScope, patch: TastePatch },
    /// `taste::Tastes`, once `field` of `scope` is back to what it inherits.
    ResetTasteField { scope: TasteScope, field: TasteField },
    /// From a machine: `true` when the account had no tastes and took these, a signed-out app's own.
    ImportTastes { tastes: Tastes },
    /// `taste::BoxSizeChoice`: the size a box cloning `repo` opens at when its call names `size`,
    /// or none, and which setting chose it. Without `repo`, a box of no project: the account's.
    GetBoxSize {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<BoxSize>,
    },
    /// `taste::Tastes`, once the default box size of `repo`'s project, or the account's without
    /// `repo`, is `size`. No `size` clears it: the project inherits the account's again, and the
    /// account's goes back to the built-in small.
    SetBoxSize {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<BoxSize>,
    },
    /// From the machine that hosts the global orchestrator, for its tab `tab`: `Call::RepoLook` of
    /// `project` (a key `projects` lists) on `machine`, an id or a name, else on the checkout a
    /// Capitão runs in, the asking machine's own, or the first online one; with `machine`,
    /// `machine_name` and `project` set. Refused while the account's `RepoInsight` is off, and past
    /// 30 calls a minute or 300 an hour. Each call writes an audit row.
    RepoLook {
        tab: TabId,
        project: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        look: crate::repo_look::RepoLook,
    },
    /// From a machine: `Call::ReadSession` of the tab `tab` on `machine`, an id or a name, else on
    /// the online machine that has that tab, with `id`, `machine` and `machine_name` set.
    ReadSession {
        tab: TabId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items: Option<usize>,
    },
    /// From a machine: the started task's JSON. Runs `request` on the machine or in the Upstash Box
    /// its target names, for the asking machine's tab `tab`, whose own id for the task is `task`.
    /// `repo` is the caller's git origin, which the worker's checkout is found or cloned by;
    /// `access` is the widest the caller may hand a child; `delegator` and `limits` are who asked
    /// and the limits it runs under, which the worker is held to and recorded with.
    Delegate {
        tab: TabId,
        task: String,
        request: Box<crate::delegate::Request>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        access: crate::delegate::Access,
        #[serde(default)]
        delegator: crate::delegate::Delegator,
        #[serde(default)]
        limits: crate::taste::Limits,
    },
    /// From a machine: the status JSON of the task its tab `tab` delegated to `machine` as `task`,
    /// or its state once cancelled when `cancel` gives a reason.
    RemoteTask {
        machine: MachineId,
        tab: TabId,
        task: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel: Option<String>,
    },
    /// From a machine: `null` once machine `to` took the end of the task its tab `tab` delegated
    /// here as `task`.
    RemoteEnded { to: MachineId, tab: TabId, task: String, end: crate::delegate::RemoteEnd },
    /// From a machine: `true` once machine `to` took what the worker of the task its tab `tab`
    /// delegated here as `task` waits on the user for. No `ask` says it waits on nothing now.
    RemoteAsked {
        to: MachineId,
        tab: TabId,
        task: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ask: Option<crate::delegate::RemoteAsk>,
    },
    /// `connectors::Settings::box_claude_tokens`. Keeps `token`, a `claude setup-token` token, for
    /// boxes that run as the pinned account `name`, which `delegate_task`'s `providerInstanceId` picks.
    SetBoxClaudeToken { name: String, token: String },
    /// `connectors::Settings::box_claude_tokens`, without the token kept for `name`.
    RemoveBoxClaudeToken { name: String },
    /// `connectors::Settings::box_claude_tokens`, once `machine` ran `claude setup-token` for its
    /// Claude account `account` and the hub kept the token it printed under that name. The token
    /// reaches no client.
    MintBoxClaudeToken { machine: MachineId, account: AccountId },
    /// Whether `machine` was still waiting on a `MintBoxClaudeToken` for `account`, which this stops.
    CancelBoxClaudeToken { machine: MachineId, account: AccountId },
    /// From a machine: `null`, once the hub keeps these records of its delegated tasks, as they
    /// are now. The hub stamps the machine, and box time for a machine that is a box.
    RecordTasks { records: Vec<crate::delegate::TaskRecord> },
    /// `delegate::DelegationStats` of the account's delegated tasks that ended in the last `days`
    /// (1, 7 or 30), on every machine.
    DelegationStats { days: u32 },
    /// `upstash::BoxUsage`: the account's Upstash Box usage, and pinned's count of the box time its
    /// delegated tasks used this UTC month. The Upstash key stays on the hub.
    BoxUsage,
    /// `Vec<marks::SessionMarks>`: how the user filed each of the account's conversations.
    SessionMarks,
    /// `Vec<marks::SessionMarks>` of the sessions `changes` named, as they are once each change
    /// landed in order. One left with only its id has no mark. The account's clients hear the same
    /// as `ToClient::SessionMarks`.
    MarkSessions { changes: Vec<crate::marks::MarkChange> },
    /// `marks::SessionMarks` of the conversation in `tab` on `machine`, once it is archived: its tab
    /// closed, and Settings › Archive lists it to bring back with its name and pin. Refused while
    /// its turn runs.
    ArchiveSession { machine: MachineId, tab: TabId },
    /// `marks::SessionMarks` of the conversation in `tab`, once it is archived as `ArchiveSession`
    /// does it. Only the machine that runs the tab sends it, from its auto-archive check, and the
    /// hub keeps a pinned thread, an orchestrator and a working one open.
    AutoArchive { tab: TabId },
    /// `Target`: the new tab, on the archived conversation's machine, that resumes it with its name
    /// and pin back.
    UnarchiveSession { session: String },
    /// `null`, once the conversation's tab closed and its marks are gone, so pinned forgets it.
    /// Claude Code keeps its transcript.
    DeleteSession { machine: MachineId, tab: TabId },
    /// From a machine: `surface::ActAnswer` of an agent session's `act`, run as the account under
    /// `surface::ACTIONS`, audited, and for a `confirm` action held for the user's approval.
    Act { act: crate::surface::AgentAct },
    /// `Vec<surface::Confirmation>`: the account's confirmations waiting on the user, oldest first,
    /// then the last 20 decided, newest first.
    Confirmations,
    /// From a signed-in client: `surface::Decided`. Approving runs the action for the session that
    /// asked; either way the session gets a notice. `entered` holds what the user typed in the app
    /// for a credential action's asked fields (`surface::Action::asks`); a credential action whose
    /// run fails waits again, so the user can type another.
    DecideConfirmation {
        id: String,
        approve: bool,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        entered: BTreeMap<String, String>,
    },
    /// From a signed-in client: `surface::Unlocked`, once `password` is the account's. Until its
    /// `until`, this connection runs the calls only the owner makes.
    Unlock { password: String },
    /// `Vec<surface::AuditEntry>`: the account's audit trail of agents' acts, newest first, `limit`
    /// rows (50, at most 200) from unix second `before` and earlier, so a page repeats the rows of
    /// its boundary second, which the client drops by id.
    AgentAudit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// `Vec<mcp::Entry>`: the account's own MCP servers, which every session pinned starts loads
    /// while enabled, each with the names of its secrets and never their values.
    ListMcpServers,
    /// `Vec<ideas::Idea>`: the account's ideas, the most recently changed first; with `project` (a repo key),
    /// only that project's.
    ListIdeas {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<String>,
    },
    /// `ideas::Idea`: adds an idea, or with `id` changes that one (its title, text and project as sent, and its
    /// status when sent). The hub sets who made it and the times.
    SaveIdea {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        title: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<crate::ideas::IdeaStatus>,
    },
    /// Removes the account's idea `id`.
    DeleteIdea { id: String },
    /// `mcp::Entry`: adds a server, enabled, or with `id` changes that one, keeping its enabled
    /// state. `secrets` holds what the user typed in the app; a name left out or blank keeps the
    /// value the server has, and a new name needs one.
    SaveMcpServer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        server: crate::mcp::Spec,
        #[serde(default, skip_serializing_if = "crate::mcp::Secrets::is_empty")]
        secrets: crate::mcp::Secrets,
    },
    /// `mcp::Entry`: turns a server of the account's list on or off for the sessions started next.
    EnableMcpServer { id: String, enabled: bool },
    /// Removes a server and its secrets from the account's list.
    RemoveMcpServer { id: String },
    /// `mcp::Entry`: adds the server `name` that the Claude Code account `account` on `machine`
    /// configures (`Call::McpImportable`), for every project or for `project` (a repository),
    /// with `secrets` typed in the app for each of its names.
    ImportMcpServer {
        machine: MachineId,
        account: AccountId,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<String>,
        #[serde(default, skip_serializing_if = "crate::mcp::Secrets::is_empty")]
        secrets: crate::mcp::Secrets,
    },
    /// From a machine only: `Vec<mcp::Load>`, the account's enabled servers with their secrets, to
    /// load into the sessions it starts.
    LoadMcpServers,
    /// `convex::Connection`, once Convex took the deploy key: connects its deployment to `repos`
    /// (repo keys, as `github.com/owner/repo`) beside the ones it had. A repo moves off any other
    /// deployment. Only the hub keeps Convex keys, so this has no Tauri command.
    ConnectConvex {
        key: String,
        #[serde(default)]
        repos: Vec<String>,
    },
    /// `convex::Connection`: sets the repos whose sessions read the deployment `id`, moving each
    /// off any other deployment.
    SetConvexRepos { id: String, repos: Vec<String> },
    /// Forgets the deployment `id` and its key. Succeeds when it is already gone.
    DisconnectConvex { id: String },
    /// The table names of the deployment `id`, sorted, for the Read Convex data step.
    ConvexTables { id: String },
    /// From a machine only: what a Convex tool of the session in `tab` asks, in the project whose
    /// repo key is `repo`. A read answers its text; `offer` answers what the session may read.
    Convex {
        tab: TabId,
        #[serde(default)]
        repo: Option<String>,
        ask: crate::convex::Ask,
    },
}

/// A project an external agent may work in: one folder on one machine.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct AgentProject {
    pub machine: MachineId,
    pub path: String,
    #[serde(default)]
    pub name: String,
}

/// An external agent the account let in through OAuth, as the Connectors page lists it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct AgentView {
    pub id: String,
    pub name: String,
    /// Where the agent's OAuth redirect points, which names who runs it.
    pub redirect: String,
    pub scopes: Vec<String>,
    pub projects: Vec<AgentProject>,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds of its last call, absent until it made one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct ExternalAgents {
    /// The URL an MCP client adds as a server.
    pub endpoint: String,
    pub agents: Vec<AgentView>,
}

/// A pending authorization, for the consent card.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct AgentRequest {
    pub request: String,
    /// The name the client registered with, which it chose itself.
    pub client: String,
    pub redirect_uri: String,
    /// What the client asked for. The user picks what it gets.
    pub scopes: Vec<String>,
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct AgentRedirect {
    pub redirect: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct MachineView {
    pub id: MachineId,
    pub name: String,
    pub os: String,
    pub online: bool,
    /// Unix seconds.
    pub last_seen: i64,
    /// The Upstash Box the machine runs in, which pinned deletes with it once its session ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstash_box: Option<String>,
    /// The build the machine last signed in with. Empty until it has.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// The phase the machine last reported. Absent until it reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateStatus>,
    /// How busy the machine is and which accounts it has, as it last told the hub. Absent while it
    /// is offline, and until a machine that reports it has connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::load::MachineState>,
    /// Whether the desktop app on the machine runs in the background, as it last reported. Absent
    /// for a machine without the app, an app older than the setting, and an offline machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct AccountView {
    pub id: String,
    pub email: String,
}

/// A one-time code that pairs a headless machine, and the command that does it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct PairCode {
    pub code: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub command: String,
}

/// An invite code, shown this once: the hub keeps only its hash.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct NewInvite {
    pub id: String,
    pub code: String,
    /// Unix seconds. A code no longer expires, so this is always `NEVER_EXPIRES`, which an older
    /// app shows as a date far away.
    pub expires_at: i64,
}

/// An invite code nobody has used yet, named by its first characters.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct InviteView {
    pub id: String,
    pub hint: String,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds. A code no longer expires, so this is always `NEVER_EXPIRES`, which an older
    /// app shows as a date far away.
    pub expires_at: i64,
}

/// The `expires_at` of a code that never expires: the last second of the year 9999. Apps older than
/// 0.17.22 still read `expires_at` and show it as a date, so the hub keeps sending this instead of
/// nothing.
pub const NEVER_EXPIRES: i64 = 253402300799;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Invites {
    pub signup: crate::account::Signup,
    pub pending: Vec<InviteView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, schemars::JsonSchema)]
pub struct Target {
    pub machine: MachineId,
    pub tab: TabId,
}

/// The answer to the call that carried `id`: `{"id":4,"ok":…}` or `{"id":4,"err":"…"}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Reply {
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok(Value),
    Err(String),
}

impl Reply {
    pub fn new(id: u64, result: Result<Value, String>) -> Self {
        Reply { id, outcome: result.map_or_else(Outcome::Err, Outcome::Ok) }
    }

    pub fn result(self) -> Result<Value, String> {
        match self.outcome {
            Outcome::Ok(v) => Ok(v),
            Outcome::Err(e) => Err(e),
        }
    }
}

/// Client to hub. The first frame must be `Auth` with a login token.
///
/// `{"type":"auth","token":"…","lock":true}`, `{"type":"call","id":1,"machine":"m1","call":{"cmd":"tabs"}}`,
/// `{"type":"hub","id":2,"call":{"cmd":"get_settings"}}`, `{"type":"attach","machine":"m1","tab":"tab-1"}`
#[derive(Serialize, Deserialize, Clone, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Auth {
        token: String,
        /// The app asks for the password before a call only the account's owner makes
        /// (`surface::owner_only`), so from now on the login alone, which the app keeps on disk,
        /// runs none. An older app leaves it out and keeps every call it had.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        lock: bool,
        /// The token `HubCall::Unlock` answered this app, sent again after a reconnect.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unlock: Option<String>,
    },
    /// Forwarded to the machine; the reply comes back as `ToClient::Reply` with this id.
    Call { id: u64, machine: MachineId, call: Call },
    /// Answered by the hub itself.
    Hub { id: u64, call: HubCall },
    /// Streams the tab's output as `MachineEvent::Output`, starting with what its screen holds.
    Attach(Target),
    Detach(Target),
}

/// Hub to client.
///
/// `{"type":"reply","id":1,"ok":[]}`, `{"type":"event","machine":"m1","event":{"kind":"exit","tab":"tab-1"}}`
#[derive(Serialize, Deserialize, Clone, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToClient {
    /// The answer to `Auth`: the account, its machines and every online machine's tabs.
    Hello { account: AccountView, machines: Vec<MachineView>, tabs: BTreeMap<MachineId, Vec<TabView>> },
    Reply(Reply),
    /// Every machine of the account, after one came online, went offline or changed.
    /// `tabs` lists the online machines' tabs, so a client can drop ids from a process that exited. An older hub leaves it out.
    Machines {
        machines: Vec<MachineView>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tabs: Option<BTreeMap<MachineId, Vec<TabView>>>,
    },
    Event { machine: MachineId, event: MachineEvent },
    /// The account's connectors, after any change or check.
    Settings { settings: Settings },
    /// Conversations whose marks changed, as they are now. One with only its id has no mark left.
    SessionMarks { marks: Vec<crate::marks::SessionMarks> },
    /// The agents' actions waiting on the user's approval, after `Hello` and after any change.
    Confirmations { items: Vec<crate::surface::Confirmation> },
}

/// Machine to hub. The first frame must be `Auth` with the machine's token.
///
/// `{"type":"auth","token":"…","name":"mac-mini","os":"macos","version":"0.6.0"}`,
/// `{"type":"call","id":7,"call":{"cmd":"slack_reply","connector":"c2","channel":"C1","thread":"1.2","text":"done"}}`
#[derive(Serialize, Deserialize, Clone, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MachineFrame {
    Auth { token: String, name: String, os: String, version: String },
    /// The answer to `ToMachine::Call` with this id.
    Reply(Reply),
    Event { event: MachineEvent },
    /// A tab's output for the viewer that subscribed to it, its screen first.
    Output { viewer: String, tab: TabId, data: Bytes },
    /// A connector session's callback; the hub answers with `ToMachine::Reply`.
    Call { id: u64, call: HubCall },
}

/// Hub to machine.
///
/// `{"type":"call","id":3,"call":{"cmd":"close_tab","tab":"tab-2"}}`, `{"type":"subscribe","tab":"tab-2","viewer":"client-4"}`
#[derive(Serialize, Deserialize, Clone, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToMachine {
    /// The answer to `Auth`.
    Hello { machine: MachineView },
    Call { id: u64, call: Call },
    Reply(Reply),
    /// Start sending the tab's output to `viewer`, its screen first, as `MachineFrame::Output`.
    /// Each client is its own viewer, so one that attaches late gets the screen without the others
    /// seeing it twice.
    Subscribe { tab: TabId, viewer: String },
    Unsubscribe { tab: TabId, viewer: String },
    /// Replace `viewer`'s set of tabs whose transcript changes it hears of. Empty removes that viewer.
    WatchThreads { viewer: String, tabs: Vec<TabId> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::BoxPermission;
    use serde_json::json;

    fn round_trip<T: Serialize + serde::de::DeserializeOwned>(frame: T, expected: Value) {
        assert_eq!(serde_json::to_value(&frame).unwrap(), expected);
        let parsed: T = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), expected);
    }

    #[test]
    fn skill_calls_default_to_skill_md_and_name_their_copies() {
        let read: Call = serde_json::from_value(json!({ "cmd": "read_skill", "dir": "/h/.claude/skills/unslop" })).unwrap();
        assert_eq!(read, Call::ReadSkill { dir: "/h/.claude/skills/unslop".into(), file: "SKILL.md".into() });
        round_trip(
            Call::SaveSkill { file: "SKILL.md".into(), text: "body".into(), copies: vec![crate::skills::Target { dir: "/h/.agents/skills/x".into(), since: "abc".into() }] },
            json!({ "cmd": "save_skill", "file": "SKILL.md", "text": "body", "copies": [{ "dir": "/h/.agents/skills/x", "since": "abc" }] }),
        );
        round_trip(Call::Skills, json!({ "cmd": "skills" }));
        assert!(Call::Skills.may_wait() && read.may_wait());
        round_trip(
            MachineEvent::SkillsChanged { scopes: vec![crate::skills::Scope::Personal, crate::skills::Scope::Plugin { plugin: "pstack".into() }] },
            json!({ "kind": "skills-changed", "scopes": [{ "kind": "personal" }, { "kind": "plugin", "plugin": "pstack" }] }),
        );
    }

    fn tab_view() -> TabView {
        TabView {
            id: "tab-3".into(),
            cwd: "/repo".into(),
            account: "work".into(),
            program: Program::Agent,
            bypass: false,
            running: true,
            status: Some(AgentStatus::NeedsInput),
            title: Some("Fix the build".into()),
            message: Some("Claude needs your permission to use Bash".into()),
            resume_at: None,
            held: false,
            connector: Some("c1".into()),
            subject: Some(Subject::LinearIssue { issue: "uuid".into() }),
            opened: 3,
            updated: Some(1792400000),
            session: Some("s1".into()),
            interrupted: false,
            cut_by_sleep: None,
            stats: Some(ThreadStats { active_ms: Some(4_320_000), cost_usd: Some(3.42) }),
            branch: Some("felipe/forms".into()),
            pr: Some(PrLink { number: 92, url: "https://github.com/o/site/pull/92".into() }),
            last_message_at: Some(1_794_226_200),
            prompt_title: Some("Fix the build".into()),
            role: None,
            spawned_by: None,
            spawned_on: None,
            orchestrator_global: false,
            taste_chat: false,
            orchestrator_turn: false,
            queued: 0,
            work: None,
            question: None,
            ask_detail: None,
            alive: true,
            asleep: false,
            headless: false,
            stopped: None,
            questions: None,
        }
    }

    #[test]
    fn a_tab_names_the_tab_that_started_it_only_when_one_did() {
        let worker = TabView { role: Some(Role::Worker), spawned_by: Some("tab-1".into()), ..tab_view() };
        let sent = serde_json::to_value(&worker).unwrap();
        assert_eq!((&sent["role"], &sent["spawned_by"]), (&json!("worker"), &json!("tab-1")));
        assert_eq!(serde_json::from_value::<TabView>(sent).unwrap(), worker);
        let plain = serde_json::to_value(tab_view()).unwrap();
        assert!(plain.get("spawned_by").is_none(), "a tab nobody started sends no spawned_by: {plain}");
        assert_eq!(serde_json::from_value::<TabView>(plain).unwrap().spawned_by, None, "an older machine's tab reads as started by nobody");
    }

    #[test]
    fn only_the_global_orchestrators_tab_says_so_and_an_older_tab_reads_as_not_it() {
        let global = TabView { orchestrator_global: true, ..tab_view() };
        let sent = serde_json::to_value(&global).unwrap();
        assert_eq!((&sent["orchestrator_global"], &sent["role"]), (&json!(true), &Value::Null));
        assert_eq!(serde_json::from_value::<TabView>(sent).unwrap(), global);
        let plain = serde_json::to_value(tab_view()).unwrap();
        assert!(plain.get("orchestrator_global").is_none(), "{plain}");
        assert!(!serde_json::from_value::<TabView>(plain).unwrap().orchestrator_global);
    }

    #[test]
    fn a_tab_carries_its_open_questions_and_an_older_tab_reads_without_them() {
        use crate::questions::{QOption, Question, Questionnaire};
        let asked = Questionnaire {
            id: "q-7".into(),
            title: Some("Checkout redesign".into()),
            questions: vec![Question {
                header: Some("Branch".into()),
                question: "Base the fix on main or on release/0.18?".into(),
                changes: "which branch the PR targets".into(),
                options: vec![QOption { label: "main".into(), description: Some("ships in the next release".into()), recommended: true }, QOption { label: "release/0.18".into(), description: None, recommended: false }],
                multi_select: false,
            }],
            projects: vec![],
            at: 1_794_240_000,
        };
        let tab = TabView { questions: Some(asked), ..tab_view() };
        let sent = serde_json::to_value(&tab).unwrap();
        assert_eq!(
            sent["questions"],
            json!({ "id": "q-7", "title": "Checkout redesign", "at": 1_794_240_000, "questions": [{ "header": "Branch", "question": "Base the fix on main or on release/0.18?", "changes": "which branch the PR targets", "options": [{ "label": "main", "description": "ships in the next release", "recommended": true }, { "label": "release/0.18" }] }] })
        );
        assert_eq!(serde_json::from_value::<TabView>(sent).unwrap(), tab);
        let older = serde_json::to_value(tab_view()).unwrap();
        assert!(older.get("questions").is_none(), "a tab with no questions sends none: {older}");
        assert_eq!(serde_json::from_value::<TabView>(older).unwrap().questions, None, "an older machine's tab reads with no questions");
    }

    #[test]
    fn answer_questions_round_trips_answered_and_skipped() {
        use crate::questions::{Answer, QuestionsReply};
        round_trip(
            Call::AnswerQuestions {
                tab: "tab-1".into(),
                id: "q-7".into(),
                reply: QuestionsReply::Answered { answers: vec![Answer { chosen: vec!["main".into()], text: None }, Answer { chosen: vec![], text: Some("Only the desktop one".into()) }] },
            },
            json!({ "cmd": "answer_questions", "tab": "tab-1", "id": "q-7", "reply": { "kind": "answered", "answers": [{ "chosen": ["main"] }, { "text": "Only the desktop one" }] } }),
        );
        round_trip(Call::AnswerQuestions { tab: "tab-1".into(), id: "q-7".into(), reply: QuestionsReply::Skipped }, json!({ "cmd": "answer_questions", "tab": "tab-1", "id": "q-7", "reply": { "kind": "skipped" } }));
    }

    #[test]
    fn a_tab_carries_its_transcripts_branch_pr_and_last_message_and_an_older_tab_reads_without_them() {
        let sent = serde_json::to_value(tab_view()).unwrap();
        assert_eq!(
            (&sent["branch"], &sent["pr"], &sent["last_message_at"]),
            (&json!("felipe/forms"), &json!({ "number": 92, "url": "https://github.com/o/site/pull/92" }), &json!(1_794_226_200))
        );
        let mut older = sent;
        for key in ["branch", "pr", "last_message_at", "prompt_title"] {
            older.as_object_mut().unwrap().remove(key);
        }
        let read: TabView = serde_json::from_value(older).unwrap();
        assert_eq!((read.branch, read.pr, read.last_message_at, read.prompt_title), (None, None, None, None));
    }

    #[test]
    fn global_orchestrator_calls_keep_their_wire_shape() {
        round_trip(
            Call::Enqueue { path: "/w/site".into(), text: "Ship the form.".into(), request_id: "r-1".into(), sender: "Orchestrator".into(), busy: Some(BusyPolicy::Refuse) },
            json!({ "cmd": "enqueue", "path": "/w/site", "text": "Ship the form.", "request_id": "r-1", "sender": "Orchestrator", "busy": "refuse" }),
        );
        let bare: Call = serde_json::from_value(json!({ "cmd": "enqueue", "path": "/w/site", "text": "x", "request_id": "r-2" })).unwrap();
        assert_eq!(bare, Call::Enqueue { path: "/w/site".into(), text: "x".into(), request_id: "r-2".into(), sender: String::new(), busy: None });
        round_trip(Call::OpenGlobalOrchestrator, json!({ "cmd": "open_global_orchestrator" }));
        round_trip(Call::GlobalOrchestratorSettings, json!({ "cmd": "global_orchestrator_settings" }));
        let partial: Call = serde_json::from_value(json!({ "cmd": "set_global_orchestrator_settings", "settings": { "name": "Almirante", "host": true } })).unwrap();
        assert_eq!(partial, Call::SetGlobalOrchestratorSettings { settings: global::Settings { name: "Almirante".into(), host: true, ..global::Settings::default() } });
        let report = CaptainReport { machine: "m1".into(), machine_name: "MacBook".into(), project: "site".into(), path: "/w/site".into(), tab: "tab-1a-3".into(), text: "Done.".into(), answers: vec!["r-1".into()] };
        round_trip(
            Call::OrchestratorReport { request_id: "report-1".into(), report: report.clone() },
            json!({ "cmd": "orchestrator_report", "request_id": "report-1", "report": { "machine": "m1", "machine_name": "MacBook", "project": "site", "path": "/w/site", "tab": "tab-1a-3", "text": "Done.", "answers": ["r-1"] } }),
        );
        round_trip(
            MachineFrame::Call { id: 9, call: HubCall::Relay { request_id: "r-1".into(), to: RelayTo::Captain { machine: "m2".into(), path: "/w/site".into(), text: "Ship it.".into(), sender: "Orchestrator".into(), busy: None } } },
            json!({ "type": "call", "id": 9, "call": { "cmd": "relay", "request_id": "r-1", "to": { "kind": "captain", "machine": "m2", "path": "/w/site", "text": "Ship it.", "sender": "Orchestrator" } } }),
        );
        round_trip(
            MachineFrame::Call { id: 10, call: HubCall::Relay { request_id: "report-1".into(), to: RelayTo::Orchestrator { report } } },
            json!({ "type": "call", "id": 10, "call": { "cmd": "relay", "request_id": "report-1", "to": { "kind": "orchestrator", "report": { "machine": "m1", "machine_name": "MacBook", "project": "site", "path": "/w/site", "tab": "tab-1a-3", "text": "Done.", "answers": ["r-1"] } } } }),
        );
        round_trip(ClientFrame::Hub { id: 3, call: HubCall::Overview }, json!({ "type": "hub", "id": 3, "call": { "cmd": "overview" } }));
        round_trip(
            MachineFrame::Call { id: 11, call: HubCall::ReadCaptain { machine: "m2".into(), path: "/w/site".into() } },
            json!({ "type": "call", "id": 11, "call": { "cmd": "read_captain", "machine": "m2", "path": "/w/site" } }),
        );
        round_trip(
            MachineFrame::Call { id: 12, call: HubCall::ReadSession { tab: "tab-2b-7".into(), machine: None, items: Some(5) } },
            json!({ "type": "call", "id": 12, "call": { "cmd": "read_session", "tab": "tab-2b-7", "items": 5 } }),
        );
        let bare: Call = serde_json::from_value(json!({ "cmd": "read_session", "tab": "tab-2b-7" })).unwrap();
        assert_eq!(bare, Call::ReadSession { tab: "tab-2b-7".into(), items: None });
    }

    #[test]
    fn calls_to_a_machine_are_tagged_by_cmd() {
        round_trip(Call::Tabs, json!({ "cmd": "tabs" }));
        round_trip(
            Call::StartClaude { tab: "tab-1".into(), cols: 120, rows: 32 },
            json!({ "cmd": "start_claude", "tab": "tab-1", "cols": 120, "rows": 32 }),
        );
        round_trip(
            Call::SetBypass { tab: "tab-1".into(), on: true, cols: 120, rows: 32 },
            json!({ "cmd": "set_bypass", "tab": "tab-1", "on": true, "cols": 120, "rows": 32 }),
        );
        round_trip(Call::PullRequestStack { repo: "o/r".into(), number: 7 }, json!({ "cmd": "pull_request_stack", "repo": "o/r", "number": 7 }));
        round_trip(
            Call::PullRequestFilterOptions { paths: vec!["/w/a".into()], kind: pulls::OptionKind::Authors, query: "oc".into() },
            json!({ "cmd": "pull_request_filter_options", "paths": ["/w/a"], "kind": "authors", "query": "oc" }),
        );
        round_trip(Call::PullRequestFiles { host: "github.com".into(), repo: "o/r".into(), number: 7 }, json!({ "cmd": "pull_request_files", "host": "github.com", "repo": "o/r", "number": 7 }));
        round_trip(Call::CreateProject { name: "My App".into() }, json!({ "cmd": "create_project", "name": "My App" }));
        round_trip(Call::PublishProject { path: "/p".into() }, json!({ "cmd": "publish_project", "path": "/p" }));
        round_trip(Call::SearchThreads { query: "flaky".into() }, json!({ "cmd": "search_threads", "query": "flaky" }));
        round_trip(Call::MakeFolder { path: "~/new".into() }, json!({ "cmd": "make_folder", "path": "~/new" }));
        round_trip(Call::Clone { repo: "https://h/o/r.git".into(), token: None, dest: Some("~/src/r".into()) }, json!({ "cmd": "clone", "repo": "https://h/o/r.git", "dest": "~/src/r" }));
        assert_eq!(serde_json::from_value::<Call>(json!({ "cmd": "clone", "repo": "o/r" })).unwrap(), Call::Clone { repo: "o/r".into(), token: None, dest: None });
        round_trip(
            Call::RestartClaude { tab: "tab-1".into(), cols: 120, rows: 32 },
            json!({ "cmd": "restart_claude", "tab": "tab-1", "cols": 120, "rows": 32 }),
        );
        round_trip(
            Call::Run {
                run: RunSpec {
                    folder: "/Users/ana/acme".into(),
                    repo: Some("git@github.com:acme/app.git".into()),
                    args: vec!["--name".into(), "ENG-7 Broken button".into(), "Run `'{{pinned}}' stage coding`".into()],
                    env: vec![("PINNED_CONNECTOR".into(), "c1".into()), ("PINNED_ISSUE".into(), "uuid".into())],
                    token: None,
                    account: Some("work".into()),
                    budget_usd: None,
                },
            },
            json!({
                "cmd": "run",
                "run": {
                    "folder": "/Users/ana/acme",
                    "repo": "git@github.com:acme/app.git",
                    "args": ["--name", "ENG-7 Broken button", "Run `'{{pinned}}' stage coding`"],
                    "env": [["PINNED_CONNECTOR", "c1"], ["PINNED_ISSUE", "uuid"]],
                    "account": "work",
                },
            }),
        );
        round_trip(Call::ListAccounts, json!({ "cmd": "list_accounts" }));
        round_trip(Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }, json!({ "cmd": "add_account", "provider": "codex", "name": "Side" }));
        round_trip(
            Call::AddAccount { provider: Provider::Claude, name: "Go".into(), auth: Some(Auth::OpenCodeGo), key: Some("oc_sk_test".into()) },
            json!({ "cmd": "add_account", "provider": "claude", "name": "Go", "auth": "opencode-go", "key": "oc_sk_test" }),
        );
        assert_eq!(
            serde_json::from_value::<Call>(json!({ "cmd": "add_account", "provider": "codex", "name": "Side" })).unwrap(),
            Call::AddAccount { provider: Provider::Codex, name: "Side".into(), auth: None, key: None }
        );
        round_trip(Call::SetAccountKey { account: "go".into(), key: "oc_sk_next".into() }, json!({ "cmd": "set_account_key", "account": "go", "key": "oc_sk_next" }));
        let old_account: crate::accounts::Account = serde_json::from_value(json!({ "id": "work", "provider": "claude", "name": "Work" })).unwrap();
        assert_eq!(old_account.auth, Auth::Login);
        round_trip(Call::SignIn { account: "work".into() }, json!({ "cmd": "sign_in", "account": "work" }));
        round_trip(
            Call::SwitchAccount { tab: "tab-1".into(), account: "spare".into(), cols: 120, rows: 32 },
            json!({ "cmd": "switch_account", "tab": "tab-1", "account": "spare", "cols": 120, "rows": 32 }),
        );
        round_trip(Call::RemoveAccount { account: "side".into() }, json!({ "cmd": "remove_account", "account": "side" }));
        round_trip(
            Call::SetProjectAccount { path: "/repo".into(), account: None },
            json!({ "cmd": "set_project_account", "path": "/repo", "account": null }),
        );
        round_trip(
            Call::Deliver { env: vec![("PINNED_SLACK_THREAD".into(), "1.2".into())], text: "yes".into() },
            json!({ "cmd": "deliver", "env": [["PINNED_SLACK_THREAD", "1.2"]], "text": "yes" }),
        );
        round_trip(
            Call::Upload { name: "photo.jpg".into(), data: Bytes(b"\xff\xd8".to_vec()) },
            json!({ "cmd": "upload", "name": "photo.jpg", "data": "/9g=" }),
        );
        round_trip(
            Call::ReadUpload { path: "/home/ana/.config/pinned/uploads/shot.png".into() },
            json!({ "cmd": "read_upload", "path": "/home/ana/.config/pinned/uploads/shot.png" }),
        );
        round_trip(
            Call::ReadImage { path: "/tmp/shots/app3.png".into() },
            json!({ "cmd": "read_image", "path": "/tmp/shots/app3.png" }),
        );
        round_trip(
            Call::Diff { tab: "tab-1".into(), scope: DiffScope::Base, ignore_whitespace: true, base: None },
            json!({ "cmd": "diff", "tab": "tab-1", "scope": "base", "ignore_whitespace": true }),
        );
        round_trip(
            Call::Diff { tab: "tab-1".into(), scope: DiffScope::Base, ignore_whitespace: false, base: Some("origin/dev".into()) },
            json!({ "cmd": "diff", "tab": "tab-1", "scope": "base", "ignore_whitespace": false, "base": "origin/dev" }),
        );
        round_trip(
            Call::ReadFile { tab: "tab-1".into(), path: "src/a.rs".into(), root: true, rev: Some("HEAD".into()) },
            json!({ "cmd": "read_file", "tab": "tab-1", "path": "src/a.rs", "root": true, "rev": "HEAD" }),
        );
        round_trip(
            Call::PrChange { tab: "tab-1".into(), pr: "https://github.com/o/r/pull/7".into(), change: PrChange::React { subject: "IC_1".into(), content: crate::panel::Reaction::ThumbsUp, on: true } },
            json!({ "cmd": "pr_change", "tab": "tab-1", "pr": "https://github.com/o/r/pull/7", "change": { "kind": "react", "subject": "IC_1", "content": "THUMBS_UP", "on": true } }),
        );
        round_trip(
            Call::PrChange { tab: "tab-1".into(), pr: "7".into(), change: PrChange::Reviewer { login: "acme/core".into(), team: true, on: false } },
            json!({ "cmd": "pr_change", "tab": "tab-1", "pr": "7", "change": { "kind": "reviewer", "login": "acme/core", "team": true, "on": false } }),
        );
        round_trip(
            Call::PrChange { tab: "tab-1".into(), pr: "7".into(), change: PrChange::Comment { id: "PRRC_1".into(), review: true, body: "Fixed.".into() } },
            json!({ "cmd": "pr_change", "tab": "tab-1", "pr": "7", "change": { "kind": "comment", "id": "PRRC_1", "review": true, "body": "Fixed." } }),
        );
        round_trip(Call::PrCandidates { tab: "tab-1".into(), pr: "7".into(), of: Candidates::Labels }, json!({ "cmd": "pr_candidates", "tab": "tab-1", "pr": "7", "of": "labels" }));
        round_trip(
            Call::PrStack { tab: "tab-1".into(), pr: "7".into(), action: StackAction::Merge, method: Some(MergeMethod::Squash), heads: vec![StackHead { number: 7, sha: "abc".into() }] },
            json!({ "cmd": "pr_stack", "tab": "tab-1", "pr": "7", "action": "merge", "method": "squash", "heads": [{ "number": 7, "sha": "abc" }] }),
        );
        round_trip(
            Call::PrReview {
                tab: "tab-1".into(),
                pr: "https://github.com/o/r/pull/7".into(),
                event: ReviewEvent::RequestChanges,
                body: "".into(),
                comments: vec![ReviewComment { path: "a.rs".into(), line: 3, side: "RIGHT".into(), start_line: None, body: "why".into() }],
            },
            json!({ "cmd": "pr_review", "tab": "tab-1", "pr": "https://github.com/o/r/pull/7", "event": "REQUEST_CHANGES", "body": "", "comments": [{ "path": "a.rs", "line": 3, "side": "RIGHT", "body": "why" }] }),
        );
        round_trip(
            Call::PrAction { tab: "tab-1".into(), pr: "7".into(), action: PrAction::AutoSquash },
            json!({ "cmd": "pr_action", "tab": "tab-1", "pr": "7", "action": "auto_squash" }),
        );
        round_trip(
            Call::PrAction { tab: "tab-1".into(), pr: "7".into(), action: PrAction::Ready },
            json!({ "cmd": "pr_action", "tab": "tab-1", "pr": "7", "action": "ready" }),
        );
        round_trip(Call::OpenIn { tab: "tab-1".into(), editor: "zed".into(), path: None, folder: false, reveal: false }, json!({ "cmd": "open_in", "tab": "tab-1", "editor": "zed" }));
        round_trip(
            Call::Device { op: crate::device::DeviceOp::Screenshot { platform: crate::device::Platform::Android, id: "emulator-5554".into() } },
            json!({ "cmd": "device", "op": "screenshot", "platform": "android", "id": "emulator-5554" }),
        );
        round_trip(Call::Device { op: crate::device::DeviceOp::List }, json!({ "cmd": "device", "op": "list" }));
        round_trip(
            Call::OpenIn { tab: "tab-1".into(), editor: "default".into(), path: Some("src/a.rs".into()), folder: true, reveal: true },
            json!({ "cmd": "open_in", "tab": "tab-1", "editor": "default", "path": "src/a.rs", "folder": true, "reveal": true }),
        );
        round_trip(Call::OpenIn { tab: "tab-1".into(), editor: "finder".into(), path: Some("src/a.rs".into()), folder: false, reveal: true }, json!({ "cmd": "open_in", "tab": "tab-1", "editor": "finder", "path": "src/a.rs", "reveal": true }));
    }

    #[test]
    fn unknown_fields_are_ignored_and_a_run_needs_only_folder_and_args() {
        let call: Call = serde_json::from_value(json!({ "cmd": "run", "run": { "folder": "/repo", "args": [], "later": 1 }, "extra": true })).unwrap();
        assert_eq!(call, Call::Run { run: RunSpec { folder: "/repo".into(), repo: None, args: vec![], env: vec![], token: None, account: None, budget_usd: None } });
        assert!(serde_json::from_value::<Call>(json!({ "cmd": "format_disk" })).is_err());
    }

    #[test]
    fn machine_events_are_tagged_by_kind_and_output_is_base64() {
        round_trip(
            MachineEvent::Output { tab: "tab-1".into(), data: Bytes(b"\x1b[2Jhi\xff".to_vec()) },
            json!({ "kind": "output", "tab": "tab-1", "data": "G1sySmhp/w==" }),
        );
        round_trip(MachineEvent::Exit { tab: "tab-1".into() }, json!({ "kind": "exit", "tab": "tab-1" }));
        round_trip(
            MachineEvent::Tabs { tabs: vec![tab_view()] },
            json!({ "kind": "tabs", "tabs": [{
                "id": "tab-3", "cwd": "/repo", "account": "work", "program": "agent", "bypass": false, "running": true, "alive": true, "status": "needs-input",
                "title": "Fix the build", "message": "Claude needs your permission to use Bash",
                "resume_at": null, "connector": "c1", "subject": { "issue": "uuid" }, "opened": 3, "updated": 1792400000, "session": "s1",
                "stats": { "active_ms": 4320000, "cost_usd": 3.42 },
                "branch": "felipe/forms", "pr": { "number": 92, "url": "https://github.com/o/site/pull/92" }, "last_message_at": 1794226200,
                "prompt_title": "Fix the build",
            }] }),
        );
        round_trip(
            MachineEvent::Usage {
                usage: Usage { five_hour: Some(crate::agent::LimitWindow { used_percentage: 23.5, resets_at: 1792400000 }), seven_day: None, monthly: None },
                account: "work".into(),
            },
            json!({ "kind": "usage", "usage": { "five_hour": { "used_percentage": 23.5, "resets_at": 1792400000 } }, "account": "work" }),
        );
        round_trip(
            MachineEvent::Usage {
                usage: Usage { five_hour: None, seven_day: None, monthly: Some(crate::agent::LimitWindow { used_percentage: 100.0, resets_at: 1795996800 }) },
                account: "go".into(),
            },
            json!({ "kind": "usage", "usage": { "monthly": { "used_percentage": 100.0, "resets_at": 1795996800 } }, "account": "go" }),
        );
        let old: MachineEvent = serde_json::from_value(json!({ "kind": "usage", "usage": {} })).unwrap();
        assert!(matches!(old, MachineEvent::Usage { account, .. } if account == "claude"), "a machine before accounts reports the default account's usage");
        assert!(serde_json::from_value::<MachineEvent>(json!({ "kind": "output", "tab": "t", "data": "not base64!" })).is_err());
        round_trip(MachineEvent::Thread { tab: "tab-1".into() }, json!({ "kind": "thread", "tab": "tab-1" }));
    }

    #[test]
    fn convex_calls_round_trip_and_older_values_read_without_the_new_fields() {
        use crate::convex::{Ask, Connection, Kind};
        let site = "github.com/acme/site";
        round_trip(
            HubCall::ConnectConvex { key: "dev:quick-lemur-789|token".into(), repos: vec![site.into()] },
            json!({ "cmd": "connect_convex", "key": "dev:quick-lemur-789|token", "repos": [site] }),
        );
        let bare: HubCall = serde_json::from_value(json!({ "cmd": "connect_convex", "key": "dev:quick-lemur-789|token" })).unwrap();
        assert!(matches!(bare, HubCall::ConnectConvex { repos, .. } if repos.is_empty()), "repos may be left out");
        round_trip(HubCall::SetConvexRepos { id: "quick-lemur-789".into(), repos: vec![] }, json!({ "cmd": "set_convex_repos", "id": "quick-lemur-789", "repos": [] }));
        round_trip(HubCall::DisconnectConvex { id: "quick-lemur-789".into() }, json!({ "cmd": "disconnect_convex", "id": "quick-lemur-789" }));
        round_trip(HubCall::ConvexTables { id: "quick-lemur-789".into() }, json!({ "cmd": "convex_tables", "id": "quick-lemur-789" }));
        round_trip(
            HubCall::Convex { tab: "tab-1-2".into(), repo: Some(site.into()), ask: Ask::Data { table: "invoices".into(), limit: Some(5), cursor: None, oldest_first: true } },
            json!({ "cmd": "convex", "tab": "tab-1-2", "repo": site, "ask": { "op": "data", "table": "invoices", "limit": 5, "cursor": null, "oldest_first": true } }),
        );
        round_trip(HubCall::Convex { tab: "tab-1-2".into(), repo: None, ask: Ask::Offer }, json!({ "cmd": "convex", "tab": "tab-1-2", "repo": null, "ask": { "op": "offer" } }));
        let sent: HubCall = serde_json::from_value(json!({ "cmd": "convex", "tab": "tab-1-2", "ask": { "op": "tables" } })).unwrap();
        assert!(matches!(sent, HubCall::Convex { repo: None, ask: Ask::Tables { table: None }, .. }));

        round_trip(
            ProjectView { path: "/Users/f/site".into(), name: "site".into(), repo: Some(site.into()), account: "default".into(), convex: Some("dev:quick-lemur-789".into()) },
            json!({ "path": "/Users/f/site", "name": "site", "repo": site, "account": "default", "convex": "dev:quick-lemur-789" }),
        );
        let older: ProjectView = serde_json::from_value(json!({ "path": "/Users/f/site", "name": "site", "account": "default" })).unwrap();
        assert_eq!(older.convex, None);
        assert_eq!(serde_json::to_value(&older).unwrap(), json!({ "path": "/Users/f/site", "name": "site", "account": "default" }), "a project without one sends nothing new");

        let connection = Connection { id: "quick-lemur-789".into(), kind: Kind::Dev, url: "https://quick-lemur-789.convex.cloud".into(), team: "acme".into(), project: "site".into(), key_hint: "…a1b2".into(), repos: vec![site.into()] };
        round_trip(
            connection.clone(),
            json!({ "id": "quick-lemur-789", "kind": "dev", "url": "https://quick-lemur-789.convex.cloud", "team": "acme", "project": "site", "key_hint": "…a1b2", "repos": [site] }),
        );
        let settings: Settings = serde_json::from_value(json!({ "linear": null, "slack": null, "connectors": [], "checked": null })).unwrap();
        assert!(settings.convex.is_empty(), "an older hub's settings read with no connections");
        let shown = serde_json::to_value(Settings { convex: vec![connection], ..settings }).unwrap();
        assert_eq!(shown["convex"][0]["key_hint"], "…a1b2");

        let node = crate::graph::Node { id: crate::graph::NodeId("c".into()), name: "Read Convex data".into(), position: [0, 0], group_name: None, kind: crate::graph::NodeKind::ConvexData { connection: "quick-lemur-789".into(), tables: None } };
        round_trip(node, json!({ "id": "c", "name": "Read Convex data", "position": [0, 0], "group_name": null, "type": "convex-data", "connection": "quick-lemur-789" }));
        let limited: crate::graph::Node = serde_json::from_value(json!({ "id": "c", "name": "Read", "position": [0, 0], "type": "convex-data", "tables": ["invoices"] })).unwrap();
        assert_eq!(limited.kind, crate::graph::NodeKind::ConvexData { connection: String::new(), tables: Some(vec!["invoices".into()]) });
    }

    #[test]
    fn hub_calls_match_the_tauri_commands() {
        round_trip(HubCall::GetSettings, json!({ "cmd": "get_settings" }));
        round_trip(HubCall::ConnectSlack { token: "xoxb-1".into() }, json!({ "cmd": "connect_slack", "token": "xoxb-1" }));
        round_trip(
            HubCall::ConnectUpstash { key: "box_1".into(), claude_token: "sk-ant-oat01-1".into() },
            json!({ "cmd": "connect_upstash", "key": "box_1", "claude_token": "sk-ant-oat01-1" }),
        );
        round_trip(HubCall::DisconnectUpstash, json!({ "cmd": "disconnect_upstash" }));
        round_trip(
            HubCall::SetBoxAccess { access: BoxAccess { workflow: BoxPermission::Ask, ..BoxAccess::default() } },
            json!({ "cmd": "set_box_access", "access": { "workflow": { "mode": "ask" }, "session": { "mode": "off" }, "worker": { "mode": "off" }, "orchestrator": { "mode": "off" } } }),
        );
        round_trip(HubCall::AnswerBox { id: "a1".into(), allow: true }, json!({ "cmd": "answer_box", "id": "a1", "allow": true }));
        round_trip(
            HubCall::Stage { connector: "c1".into(), subject: Subject::LinearIssue { issue: "uuid".into() }, stage: Stage::Coding },
            json!({ "cmd": "stage", "connector": "c1", "issue": "uuid", "stage": "coding" }),
        );
        round_trip(
            HubCall::LinearTake { connector: "c2".into(), channel: "C1".into(), thread: "1.2".into(), issue: "ENG-7".into() },
            json!({ "cmd": "linear_take", "connector": "c2", "channel": "C1", "thread": "1.2", "issue": "ENG-7" }),
        );
        round_trip(
            HubCall::RunSet { connector: "c2".into(), subject: Subject::SlackThread { channel: "C1".into(), thread: "1.2".into() }, name: "title".into(), value: "Export 500s".into() },
            json!({ "cmd": "run_set", "connector": "c2", "channel": "C1", "thread": "1.2", "name": "title", "value": "Export 500s" }),
        );
        round_trip(
            HubCall::Import { store: json!({ "connectors": [] }), linear_key: Some("lin_api_1".into()), slack_key: None, slack_keys: BTreeMap::new(), github_key: None, vercel_key: Some("vck_1".into()) },
            json!({ "cmd": "import", "store": { "connectors": [] }, "linear_key": "lin_api_1", "slack_key": null, "github_key": null, "vercel_key": "vck_1" }),
        );
        round_trip(HubCall::ConnectVercel { key: "vck_1".into() }, json!({ "cmd": "connect_vercel", "key": "vck_1" }));
        round_trip(HubCall::DisconnectVercel, json!({ "cmd": "disconnect_vercel" }));
        round_trip(HubCall::VercelModels, json!({ "cmd": "vercel_models" }));
        let before_vercel = json!({ "cmd": "import", "store": { "connectors": [] }, "linear_key": null, "slack_key": "xoxb-1" });
        match serde_json::from_value::<HubCall>(before_vercel).unwrap() {
            HubCall::Import { slack_key, github_key, vercel_key, .. } => assert_eq!((slack_key.as_deref(), github_key, vercel_key), (Some("xoxb-1"), None, None)),
            _ => panic!("not an import"),
        }
        round_trip(HubCall::SetBoxClaudeToken { name: "bloom".into(), token: "sk-ant-oat01-1".into() }, json!({ "cmd": "set_box_claude_token", "name": "bloom", "token": "sk-ant-oat01-1" }));
        round_trip(HubCall::RemoveBoxClaudeToken { name: "bloom".into() }, json!({ "cmd": "remove_box_claude_token", "name": "bloom" }));
        round_trip(HubCall::MintBoxClaudeToken { machine: "m1".into(), account: "bloom".into() }, json!({ "cmd": "mint_box_claude_token", "machine": "m1", "account": "bloom" }));
        round_trip(HubCall::CancelBoxClaudeToken { machine: "m1".into(), account: "bloom".into() }, json!({ "cmd": "cancel_box_claude_token", "machine": "m1", "account": "bloom" }));
        let end = crate::delegate::RemoteEnd { state: "result_available".into(), summary: Some("Done.".into()), error: None, usage: None };
        round_trip(
            HubCall::RemoteEnded { to: "m1".into(), tab: "tab-1".into(), task: "task-1".into(), end: end.clone() },
            json!({ "cmd": "remote_ended", "to": "m1", "tab": "tab-1", "task": "task-1", "end": { "state": "result_available", "summary": "Done." } }),
        );
        round_trip(
            HubCall::RemoteTask { machine: "m2".into(), tab: "tab-1".into(), task: "task-1".into(), cancel: Some("not now".into()) },
            json!({ "cmd": "remote_task", "machine": "m2", "tab": "tab-1", "task": "task-1", "cancel": "not now" }),
        );
        round_trip(
            Call::RemoteEnded { from: "m2".into(), tab: "tab-1".into(), task: "task-1".into(), end },
            json!({ "cmd": "remote_ended", "from": "m2", "tab": "tab-1", "task": "task-1", "end": { "state": "result_available", "summary": "Done." } }),
        );
        let request = crate::delegate::parse_request(&json!({ "task": "Fix it.", "target": { "box": { "size": "large" } } })).unwrap();
        let delegate = serde_json::to_value(HubCall::Delegate { tab: "tab-1".into(), task: "task-1".into(), request: Box::new(request), repo: Some("git@github.com:o/r.git".into()), access: crate::delegate::Access::ReadOnly, delegator: Default::default(), limits: Default::default() }).unwrap();
        assert_eq!((delegate["cmd"].as_str(), delegate["access"].as_str()), (Some("delegate"), Some("read_only")));
        assert_eq!(delegate["request"]["target"]["box"], json!({ "size": "large" }), "the box target keeps its own name on the wire");
        round_trip(HubCall::GetTastes, json!({ "cmd": "get_tastes" }));
        round_trip(HubCall::BoxUsage, json!({ "cmd": "box_usage" }));
        round_trip(HubCall::DelegationStats { days: 7 }, json!({ "cmd": "delegation_stats", "days": 7 }));
        round_trip(
            HubCall::SetTaste { scope: TasteScope::Project { repo: "github.com/acme/site".into() }, patch: TastePatch { parallel: Some(5), ..TastePatch::default() } },
            json!({ "cmd": "set_taste", "scope": { "kind": "project", "repo": "github.com/acme/site" }, "patch": { "parallel": 5 } }),
        );
        round_trip(
            HubCall::ResetTasteField { scope: TasteScope::Captains, field: TasteField::HardModel },
            json!({ "cmd": "reset_taste_field", "scope": { "kind": "captains" }, "field": "hard_model" }),
        );
        match serde_json::from_value::<HubCall>(json!({ "cmd": "import_tastes", "tastes": { "projects": { "github.com/acme/site": { "parallel": 5 } } } })).unwrap() {
            HubCall::ImportTastes { tastes } => assert_eq!(serde_json::to_value(tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 5 } })),
            _ => panic!("not an import of tastes"),
        }
    }

    #[test]
    fn client_frames_are_tagged_by_type() {
        round_trip(ClientFrame::Auth { token: "t0k".into(), lock: false, unlock: None }, json!({ "type": "auth", "token": "t0k" }));
        round_trip(ClientFrame::Auth { token: "t0k".into(), lock: true, unlock: Some("u-1".into()) }, json!({ "type": "auth", "token": "t0k", "lock": true, "unlock": "u-1" }));
        round_trip(
            ClientFrame::Call { id: 1, machine: "m1".into(), call: Call::CloseTab { tab: "tab-2".into() } },
            json!({ "type": "call", "id": 1, "machine": "m1", "call": { "cmd": "close_tab", "tab": "tab-2" } }),
        );
        round_trip(ClientFrame::Hub { id: 2, call: HubCall::Machines }, json!({ "type": "hub", "id": 2, "call": { "cmd": "machines" } }));
        round_trip(
            ClientFrame::Attach(Target { machine: "m1".into(), tab: "tab-2".into() }),
            json!({ "type": "attach", "machine": "m1", "tab": "tab-2" }),
        );
    }

    #[test]
    fn frames_to_a_client_carry_replies_and_machine_events() {
        round_trip(ToClient::Reply(Reply::new(1, Ok(json!(["tab-1"])))), json!({ "type": "reply", "id": 1, "ok": ["tab-1"] }));
        round_trip(ToClient::Reply(Reply::new(2, Ok(Value::Null))), json!({ "type": "reply", "id": 2, "ok": null }));
        round_trip(
            ToClient::Reply(Reply::new(3, Err("mac-mini is offline".into()))),
            json!({ "type": "reply", "id": 3, "err": "mac-mini is offline" }),
        );
        let reply: ToClient = serde_json::from_value(json!({ "type": "reply", "id": 3, "err": "gone" })).unwrap();
        assert!(matches!(reply, ToClient::Reply(r) if r.clone().result() == Err("gone".into())));
        round_trip(
            ToClient::Event { machine: "m1".into(), event: MachineEvent::Exit { tab: "tab-1".into() } },
            json!({ "type": "event", "machine": "m1", "event": { "kind": "exit", "tab": "tab-1" } }),
        );
        let machine = MachineView { id: "m1".into(), name: "mac-mini".into(), os: "macos".into(), online: true, last_seen: 1792400000, upstash_box: None, version: String::new(), update: None, state: None, background: None };
        round_trip(
            ToClient::Hello {
                account: AccountView { id: "a1".into(), email: "ana@acme.dev".into() },
                machines: vec![machine.clone()],
                tabs: BTreeMap::from([("m1".into(), vec![])]),
            },
            json!({
                "type": "hello",
                "account": { "id": "a1", "email": "ana@acme.dev" },
                "machines": [{ "id": "m1", "name": "mac-mini", "os": "macos", "online": true, "last_seen": 1792400000 }],
                "tabs": { "m1": [] },
            }),
        );
        round_trip(
            ToClient::Settings { settings: Settings { linear: None, slack: None, slack_bots: vec![], github: None, vercel: None, upstash: None, box_claude_tokens: BTreeMap::new(), connectors: vec![], checked: None, boxes: BoxAccess::default(), box_asks: vec![], convex: vec![], ideas: vec![], repo_insight: Default::default(), schedules: vec![] } },
            json!({ "type": "settings", "settings": { "linear": null, "slack": null, "github": null, "vercel": null, "upstash": null, "connectors": [], "checked": null } }),
        );
        let old: ToClient = serde_json::from_value(json!({ "type": "machines", "machines": [] })).unwrap();
        assert!(matches!(old, ToClient::Machines { tabs: None, .. }));
        round_trip(
            ToClient::Machines { machines: vec![machine.clone()], tabs: Some(BTreeMap::from([("m1".into(), vec![])])) },
            json!({
                "type": "machines",
                "machines": [{ "id": "m1", "name": "mac-mini", "os": "macos", "online": true, "last_seen": 1792400000 }],
                "tabs": { "m1": [] },
            }),
        );
        let bare = serde_json::to_value(ToClient::Machines { machines: vec![], tabs: None }).unwrap();
        assert!(bare.get("tabs").is_none());
    }

    #[test]
    fn frames_between_hub_and_machine() {
        round_trip(
            MachineFrame::Auth { token: "m-t0k".into(), name: "vps".into(), os: "linux".into(), version: "0.6.0".into() },
            json!({ "type": "auth", "token": "m-t0k", "name": "vps", "os": "linux", "version": "0.6.0" }),
        );
        round_trip(
            MachineFrame::Event { event: MachineEvent::Exit { tab: "tab-1".into() } },
            json!({ "type": "event", "event": { "kind": "exit", "tab": "tab-1" } }),
        );
        round_trip(
            MachineFrame::Call { id: 7, call: HubCall::SlackReply { connector: "c2".into(), channel: "C1".into(), thread: "1.2".into(), text: "done".into() } },
            json!({ "type": "call", "id": 7, "call": { "cmd": "slack_reply", "connector": "c2", "channel": "C1", "thread": "1.2", "text": "done" } }),
        );
        round_trip(MachineFrame::Reply(Reply::new(3, Ok(json!("tab-4")))), json!({ "type": "reply", "id": 3, "ok": "tab-4" }));
        round_trip(
            ToMachine::Subscribe { tab: "tab-2".into(), viewer: "client-4".into() },
            json!({ "type": "subscribe", "tab": "tab-2", "viewer": "client-4" }),
        );
        round_trip(
            MachineFrame::Output { viewer: "client-4".into(), tab: "tab-2".into(), data: Bytes(b"hi".to_vec()) },
            json!({ "type": "output", "viewer": "client-4", "tab": "tab-2", "data": "aGk=" }),
        );
        round_trip(
            ToMachine::Call { id: 3, call: Call::ResizePty { tab: "tab-2".into(), cols: 80, rows: 24 } },
            json!({ "type": "call", "id": 3, "call": { "cmd": "resize_pty", "tab": "tab-2", "cols": 80, "rows": 24 } }),
        );
        round_trip(
            ToMachine::Hello { machine: MachineView { id: "m2".into(), name: "vps".into(), os: "linux".into(), online: true, last_seen: 0, upstash_box: None, version: String::new(), update: None, state: None, background: None } },
            json!({ "type": "hello", "machine": { "id": "m2", "name": "vps", "os": "linux", "online": true, "last_seen": 0 } }),
        );
        round_trip(
            ToMachine::WatchThreads { viewer: "client-4".into(), tabs: vec!["tab-1".into()] },
            json!({ "type": "watch_threads", "viewer": "client-4", "tabs": ["tab-1"] }),
        );
        round_trip(Call::WatchThreads { tabs: vec!["tab-1".into()] }, json!({ "cmd": "watch_threads", "tabs": ["tab-1"] }));
    }

    #[test]
    fn a_decision_without_typed_fields_keeps_its_old_shape_and_mcp_servers_takes_both_fields_as_optional() {
        round_trip(HubCall::DecideConfirmation { id: "c1".into(), approve: true, entered: BTreeMap::new() }, json!({ "cmd": "decide_confirmation", "id": "c1", "approve": true }));
        let typed = BTreeMap::from([("key".to_owned(), "lin_api_x".to_owned())]);
        round_trip(HubCall::DecideConfirmation { id: "c1".into(), approve: true, entered: typed }, json!({ "cmd": "decide_confirmation", "id": "c1", "approve": true, "entered": { "key": "lin_api_x" } }));
        round_trip(Call::McpServers { account: None, path: None }, json!({ "cmd": "mcp_servers" }));
        round_trip(Call::McpServers { account: Some("work".into()), path: Some("/w/site".into()) }, json!({ "cmd": "mcp_servers", "account": "work", "path": "/w/site" }));
        assert!(Call::McpServers { account: None, path: None }.may_wait());
        round_trip(Call::McpImportable { account: Some("work".into()), path: None }, json!({ "cmd": "mcp_importable", "account": "work" }));
        assert!(Call::McpImportable { account: None, path: None }.may_wait());
        use crate::mcp::{Reach, Secrets, Spec};
        let server = Spec { name: "ctx".into(), project: None, transport: Reach::Stdio { command: "npx".into(), args: vec![], env: vec!["API_KEY".into()] } };
        let wire_server = json!({ "name": "ctx", "transport": { "type": "stdio", "command": "npx", "env": ["API_KEY"] } });
        round_trip(HubCall::ListMcpServers, json!({ "cmd": "list_mcp_servers" }));
        round_trip(HubCall::SaveMcpServer { id: None, server: server.clone(), secrets: Secrets::default() }, json!({ "cmd": "save_mcp_server", "server": wire_server }));
        let typed = Secrets { env: [("API_KEY".to_owned(), "k".to_owned())].into(), headers: BTreeMap::new() };
        round_trip(HubCall::SaveMcpServer { id: Some("s1".into()), server, secrets: typed.clone() }, json!({ "cmd": "save_mcp_server", "id": "s1", "server": wire_server, "secrets": { "env": { "API_KEY": "k" } } }));
        round_trip(HubCall::EnableMcpServer { id: "s1".into(), enabled: false }, json!({ "cmd": "enable_mcp_server", "id": "s1", "enabled": false }));
        round_trip(HubCall::RemoveMcpServer { id: "s1".into() }, json!({ "cmd": "remove_mcp_server", "id": "s1" }));
        round_trip(
            HubCall::ImportMcpServer { machine: "m1".into(), account: "claude".into(), name: "linear".into(), path: None, project: Some("github.com/a/b".into()), secrets: Secrets::default() },
            json!({ "cmd": "import_mcp_server", "machine": "m1", "account": "claude", "name": "linear", "project": "github.com/a/b" }),
        );
        round_trip(HubCall::LoadMcpServers, json!({ "cmd": "load_mcp_servers" }));
        round_trip(HubCall::ListIdeas { project: None }, json!({ "cmd": "list_ideas" }));
        round_trip(HubCall::ListIdeas { project: Some("github.com/a/b".into()) }, json!({ "cmd": "list_ideas", "project": "github.com/a/b" }));
        round_trip(HubCall::SaveIdea { id: None, title: "An iPad app".into(), text: String::new(), project: None, status: None }, json!({ "cmd": "save_idea", "title": "An iPad app" }));
        round_trip(
            HubCall::SaveIdea { id: Some("i1".into()), title: "An iPad app".into(), text: "Maybe.".into(), project: Some("github.com/a/b".into()), status: Some(crate::ideas::IdeaStatus::Dropped) },
            json!({ "cmd": "save_idea", "id": "i1", "title": "An iPad app", "text": "Maybe.", "project": "github.com/a/b", "status": "dropped" }),
        );
        round_trip(HubCall::DeleteIdea { id: "i1".into() }, json!({ "cmd": "delete_idea", "id": "i1" }));
    }

    #[test]
    fn run_in_the_background_round_trips_and_a_view_without_it_reads_as_none() {
        round_trip(Call::SetBackground { on: true }, json!({ "cmd": "set_background", "on": true }));
        round_trip(MachineEvent::Background { on: false }, json!({ "kind": "background", "on": false }));
        let older: MachineView = serde_json::from_value(json!({ "id": "m1", "name": "vps", "os": "linux", "online": true, "last_seen": 0 })).unwrap();
        assert_eq!(older.background, None);
        let reported = MachineView { background: Some(true), ..older };
        assert_eq!(serde_json::to_value(&reported).unwrap()["background"], json!(true));
    }

    #[test]
    fn an_update_status_round_trips_and_an_old_machine_view_still_reads() {
        round_trip(Call::UpdateSelf, json!({ "cmd": "update_self" }));
        assert!(Call::UpdateSelf.may_wait());
        round_trip(UpdateStatus::UpToDate { version: "0.16.0".into() }, json!({ "state": "up_to_date", "version": "0.16.0" }));
        round_trip(UpdateStatus::Updating { to: String::new() }, json!({ "state": "updating", "to": "" }));
        round_trip(UpdateStatus::Updating { to: "0.16.0".into() }, json!({ "state": "updating", "to": "0.16.0" }));
        round_trip(UpdateStatus::Restarting { to: "0.16.0".into() }, json!({ "state": "restarting", "to": "0.16.0" }));
        round_trip(UpdateStatus::Failed { to: String::new(), reason: "checksum mismatch".into() }, json!({ "state": "failed", "to": "", "reason": "checksum mismatch" }));
        round_trip(
            UpdateStatus::Failed { to: "0.16.0".into(), reason: "checksum mismatch".into() },
            json!({ "state": "failed", "to": "0.16.0", "reason": "checksum mismatch" }),
        );
        let omitted: UpdateStatus = serde_json::from_value(json!({ "state": "updating" })).unwrap();
        assert_eq!(omitted, UpdateStatus::Updating { to: String::new() });
        let omitted_failure: UpdateStatus = serde_json::from_value(json!({ "state": "failed", "reason": "checksum mismatch" })).unwrap();
        assert_eq!(omitted_failure, UpdateStatus::Failed { to: String::new(), reason: "checksum mismatch".into() });
        round_trip(UpdateStatus::Cannot { reason: "this build cannot install a release".into() }, json!({ "state": "cannot", "reason": "this build cannot install a release" }));
        round_trip(MachineEvent::Update { status: None }, json!({ "kind": "update", "status": null }));
        round_trip(
            MachineEvent::Update { status: Some(UpdateStatus::Restarting { to: "0.16.0".into() }) },
            json!({ "kind": "update", "status": { "state": "restarting", "to": "0.16.0" } }),
        );
        let old: MachineView = serde_json::from_value(json!({ "id": "m1", "name": "mac-mini", "os": "macos", "online": true, "last_seen": 1 })).unwrap();
        assert_eq!(old.version, "");
        assert_eq!(old.update, None);
        round_trip(
            MachineView { id: "m1".into(), name: "mac-mini".into(), os: "macos".into(), online: true, last_seen: 1, upstash_box: None, version: "0.16.0".into(), update: Some(UpdateStatus::UpToDate { version: "0.16.0".into() }), state: None, background: None },
            json!({ "id": "m1", "name": "mac-mini", "os": "macos", "online": true, "last_seen": 1, "version": "0.16.0", "update": { "state": "up_to_date", "version": "0.16.0" } }),
        );
    }

    #[test]
    fn an_open_box_from_an_older_app_keeps_its_size_and_a_newer_one_may_leave_it_out() {
        let older = serde_json::from_value::<HubCall>(json!({ "cmd": "open_box", "repo": "acme/site", "size": "medium" })).unwrap();
        assert!(matches!(older, HubCall::OpenBox { size: Some(BoxSize::Medium), budget_usd: None, .. }));
        let sizeless = serde_json::from_value::<HubCall>(json!({ "cmd": "open_box", "repo": "acme/site" })).unwrap();
        assert!(matches!(sizeless, HubCall::OpenBox { size: None, .. }));
        assert_eq!(serde_json::to_value(sizeless).unwrap(), json!({ "cmd": "open_box", "repo": "acme/site" }));
        let asked = serde_json::from_value::<HubCall>(json!({ "cmd": "get_box_size", "repo": "acme/site" })).unwrap();
        assert!(matches!(asked, HubCall::GetBoxSize { repo: Some(ref repo), size: None } if repo == "acme/site"));
        let cleared = serde_json::from_value::<HubCall>(json!({ "cmd": "set_box_size" })).unwrap();
        assert!(matches!(cleared, HubCall::SetBoxSize { repo: None, size: None }));
    }
}
