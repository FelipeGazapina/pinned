//! The protocol between machines, the hub and clients: one JSON object per WebSocket text frame.
//! Every enum is internally tagged and unknown fields are ignored, so versions can drift.

use crate::agent::{AgentEvent, AgentStatus, Stage, Subject, Usage};
use crate::connectors::{Connector, Settings};
use crate::branches::GitStep;
use crate::scripts::Script;
use crate::panel::{Candidates, MergeMethod, PrChange, ReviewComment, ReviewEvent, StackAction, StackHead};
use crate::workspace::{DiffScope, PrAction};
use crate::stats::ThreadStats;
use crate::{pulls, subagents, sync, thread, usage};
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// A tab's id, unique within its machine, such as `tab-3`.
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TabView {
    pub id: TabId,
    pub cwd: String,
    pub bypass: bool,
    pub running: bool,
    pub status: Option<AgentStatus>,
    pub title: Option<String>,
    /// What the session asks, while it needs input.
    pub message: Option<String>,
    /// Unix time a usage limit lifts, while one stopped the turn.
    pub resume_at: Option<u64>,
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
    /// What the conversation's work came to so far, once its transcript holds any of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<ThreadStats>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectView {
    pub path: String,
    pub name: String,
    /// The origin as `host/owner/name`, which names the project on every machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// Everything a client may ask a machine to do, local or through the hub. The comment on each
/// variant names what `Machine::call` answers.
///
/// `{"cmd":"start_claude","tab":"tab-1","cols":120,"rows":32}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Call {
    /// `Vec<TabView>`.
    Tabs,
    /// The new tab's id. `path: null` opens the machine's default folder.
    OpenTab { path: Option<String>, bypass: bool },
    /// The folder claude runs in. Restarts claude when it is already running.
    StartClaude { tab: TabId, cols: u16, rows: u16 },
    /// The folder claude runs in. Restarts claude with `bypass` turned `on` or off, resuming the
    /// tab's conversation; a session already running that way is left alone.
    SetBypass { tab: TabId, on: bool, cols: u16, rows: u16 },
    /// The folder claude runs in. Restarts claude on the tab's conversation with `--model model`,
    /// which picks it for that conversation alone.
    SetModel { tab: TabId, model: String, cols: u16, rows: u16 },
    /// The folder claude runs in. Restarts claude on the tab's conversation, so its skills,
    /// plugins and MCP servers load again.
    RestartClaude { tab: TabId, cols: u16, rows: u16 },
    /// `null`. Keystrokes as xterm reports them.
    WriteInput { tab: TabId, data: String },
    /// `null`.
    ResizePty { tab: TabId, cols: u16, rows: u16 },
    /// `null`.
    CloseTab { tab: TabId },
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
    /// The folder of a clone of `repo`, found among the machine's projects or cloned into `~/pinned`.
    /// The hub adds the account's GitHub token, for a private repository.
    Clone {
        repo: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// The new tab's id, with claude already running. Refused when the folder can't be found or cloned.
    Run { run: RunSpec },
    /// `true` when a live tab whose environment holds every pair took `text` as a prompt.
    Deliver { env: Vec<(String, String)>, text: String },
    /// The absolute path the file was saved under, in a private folder of the machine, for a
    /// session to read. Refused past `uploads::LIMIT`.
    Upload { name: String, data: Bytes },
    /// `ThreadPage`: the conversation's items from byte `from` of its transcript on.
    Thread { tab: TabId, from: u64 },
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
    /// messages, and the `previous` title.
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
    /// `null`. Opens the tab's folder, or `path` inside its repository, in the editor `editor`.
    OpenIn {
        tab: TabId,
        editor: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// `Vec<commands::Command>`, the slash commands claude offers in the tab's folder.
    Commands { tab: TabId },
    /// `Vec<String>`: the tab's folder's files and folders, relative, for `@` mentions.
    Files { tab: TabId },
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
        /// Rows per search at most; the page asks for more 100 at a time.
        #[serde(default = "pulls_limit")]
        limit: usize,
    },
    /// `pulls::Stack` the pull request `number` of github.com's `repo` (`owner/name`) sits in, or `null`.
    PullRequestStack { repo: String, number: u64 },
    /// `Vec<usage::Row>`: the tokens claude's transcripts on the machine record from unix time `since` on.
    /// `prices` are the user's, per model, which win over LiteLLM's list.
    TokenUsage {
        since: u64,
        #[serde(default)]
        prices: HashMap<String, usage::Price>,
    },
    /// `panel::PageMeta` of the web page at `url`, fetched from this machine.
    PageMeta { url: String },
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
    /// The address of the machine's device hub, which mirrors its simulators and emulators. It
    /// starts on the first call; `embed` is the origin of a page that frames it.
    DeviceHub {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        embed: Option<String>,
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
}

fn pulls_limit() -> usize {
    100

}

impl Call {
    /// Runs off the machine's call queue, which carries keystrokes in order: a run may clone a
    /// repository for minutes, gh waits on GitHub, and the others read files or run git.
    pub fn may_wait(&self) -> bool {
        matches!(
            self,
            Call::Run { .. }
                | Call::PullRequest { .. }
                | Call::PrStatus { .. }
                | Call::Favicon { .. }
                | Call::RegenerateTitle { .. }
                | Call::PrList { .. }
                | Call::PrAction { .. }
                | Call::Thread { .. }
                | Call::Workspace { .. }
                | Call::ProjectHistory { .. }
                | Call::Diff { .. }
                | Call::Ports { .. }
                | Call::Editors
                | Call::OpenIn { .. }
                | Call::Commands { .. }
                | Call::Files { .. }
                | Call::Branches { .. }
                | Call::Checkout { .. }
                | Call::TokenUsage { .. }
                | Call::PullRequests { .. }
                | Call::PullRequestStack { .. }
                | Call::PageMeta { .. }
                | Call::Refs { .. }
                | Call::CheckoutPr { .. }
                | Call::Git { .. }
                | Call::SetWorkspace { .. }
                | Call::ReadFile { .. }
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
                | Call::Subagent { .. }
                | Call::Rewind { .. }
                | Call::Fork { .. }
                | Call::SaveFile { .. }
                | Call::CreateProject { .. }
                | Call::PublishProject { .. }
        )
    }
}

/// A step of a tab's conversation. A client holding items of another `session`, or getting a
/// `reset` page, starts over.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ThreadPage {
    pub session: Option<String>,
    pub items: Vec<thread::Item>,
    pub next: u64,
    pub reset: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<thread::Context>,
    /// Every agent the session spawned, whole on each page. A machine on an older pinned leaves it out.
    pub agents: Vec<subagents::Subagent>,
}

/// A connector's session, the same on any machine.
///
/// `{"folder":"/repo","repo":"git@github.com:o/r.git","args":["--name","ENG-7"],"env":[["PINNED_CONNECTOR","c1"]]}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
}

/// What a machine reports on its own.
///
/// `{"kind":"exit","tab":"tab-1"}`, `{"kind":"output","tab":"tab-1","data":"aGk="}`
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MachineEvent {
    /// Every tab, after any of them opened, closed, started, exited or changed status, title or folder.
    Tabs { tabs: Vec<TabView> },
    Agent { event: AgentEvent },
    /// Output of a tab a client attached to, as the hub passes it on.
    Output { tab: TabId, data: Bytes },
    /// The tab's claude exited by itself.
    Exit { tab: TabId },
    Usage { usage: Usage },
}

/// Account-level calls, answered by the hub. Names and arguments match the app's Tauri
/// commands, so the settings page sends either. The last three come only from machines,
/// forwarding a connector session's callbacks to the hub, which holds the keys.
///
/// `{"cmd":"connect_linear","key":"lin_api_…"}`, `{"cmd":"stage","connector":"c1","issue":"uuid","stage":"coding"}`
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum HubCall {
    /// `connectors::Settings`.
    GetSettings,
    /// `linear::Account`.
    ConnectLinear { key: String },
    DisconnectLinear,
    /// `connectors::Suggestions`.
    LinearSuggestions,
    /// `slack::Account`.
    ConnectSlack { token: String },
    DisconnectSlack,
    /// Channel names.
    SlackSuggestions,
    /// `github::Account`.
    ConnectGithub { token: String },
    DisconnectGithub,
    /// `Vec<github::Repo>`, most recently pushed first.
    GithubRepos,
    /// The folder of `repo` on `machine`, cloned there with the account's GitHub token when it has none.
    Clone { machine: MachineId, repo: String },
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
    /// The connector as stored.
    SaveConnector { connector: Connector },
    DeleteConnector { id: String },
    /// `Vec<MachineView>`.
    Machines,
    RenameMachine { id: MachineId, name: String },
    RemoveMachine { id: MachineId },
    /// `PairCode`.
    PairCode,
    /// A signed-out app's connectors, moved to the account on its first sign-in.
    Import {
        store: Value,
        linear_key: Option<String>,
        slack_key: Option<String>,
        #[serde(default)]
        github_key: Option<String>,
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MachineView {
    pub id: MachineId,
    pub name: String,
    pub os: String,
    pub online: bool,
    /// Unix seconds.
    pub last_seen: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AccountView {
    pub id: String,
    pub email: String,
}

/// A one-time code that pairs a headless machine, and the command that does it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PairCode {
    pub code: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub command: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Target {
    pub machine: MachineId,
    pub tab: TabId,
}

/// The answer to the call that carried `id`: `{"id":4,"ok":…}` or `{"id":4,"err":"…"}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reply {
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
/// `{"type":"auth","token":"…"}`, `{"type":"call","id":1,"machine":"m1","call":{"cmd":"tabs"}}`,
/// `{"type":"hub","id":2,"call":{"cmd":"get_settings"}}`, `{"type":"attach","machine":"m1","tab":"tab-1"}`
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Auth { token: String },
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
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToClient {
    /// The answer to `Auth`: the account, its machines and every online machine's tabs.
    Hello { account: AccountView, machines: Vec<MachineView>, tabs: BTreeMap<MachineId, Vec<TabView>> },
    Reply(Reply),
    /// Every machine of the account, after one came online, went offline or changed.
    Machines { machines: Vec<MachineView> },
    Event { machine: MachineId, event: MachineEvent },
    /// The account's connectors, after any change or check.
    Settings { settings: Settings },
}

/// Machine to hub. The first frame must be `Auth` with the machine's token.
///
/// `{"type":"auth","token":"…","name":"mac-mini","os":"macos","version":"0.6.0"}`,
/// `{"type":"call","id":7,"call":{"cmd":"slack_reply","connector":"c2","channel":"C1","thread":"1.2","text":"done"}}`
#[derive(Serialize, Deserialize, Clone)]
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
#[derive(Serialize, Deserialize, Clone)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T: Serialize + serde::de::DeserializeOwned>(frame: T, expected: Value) {
        assert_eq!(serde_json::to_value(&frame).unwrap(), expected);
        let parsed: T = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), expected);
    }

    fn tab_view() -> TabView {
        TabView {
            id: "tab-3".into(),
            cwd: "/repo".into(),
            bypass: false,
            running: true,
            status: Some(AgentStatus::NeedsInput),
            title: Some("Fix the build".into()),
            message: Some("Claude needs your permission to use Bash".into()),
            resume_at: None,
            connector: Some("c1".into()),
            subject: Some(Subject::LinearIssue { issue: "uuid".into() }),
            opened: 3,
            updated: Some(1792400000),
            session: Some("s1".into()),
            stats: Some(ThreadStats { active_ms: Some(4_320_000), cost_usd: Some(3.42) }),
        }
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
        round_trip(Call::CreateProject { name: "My App".into() }, json!({ "cmd": "create_project", "name": "My App" }));
        round_trip(Call::PublishProject { path: "/p".into() }, json!({ "cmd": "publish_project", "path": "/p" }));
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
                },
            },
            json!({
                "cmd": "run",
                "run": {
                    "folder": "/Users/ana/acme",
                    "repo": "git@github.com:acme/app.git",
                    "args": ["--name", "ENG-7 Broken button", "Run `'{{pinned}}' stage coding`"],
                    "env": [["PINNED_CONNECTOR", "c1"], ["PINNED_ISSUE", "uuid"]],
                },
            }),
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
        round_trip(Call::OpenIn { tab: "tab-1".into(), editor: "zed".into(), path: None }, json!({ "cmd": "open_in", "tab": "tab-1", "editor": "zed" }));
    }

    #[test]
    fn unknown_fields_are_ignored_and_a_run_needs_only_folder_and_args() {
        let call: Call = serde_json::from_value(json!({ "cmd": "run", "run": { "folder": "/repo", "args": [], "later": 1 }, "extra": true })).unwrap();
        assert_eq!(call, Call::Run { run: RunSpec { folder: "/repo".into(), repo: None, args: vec![], env: vec![], token: None } });
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
                "id": "tab-3", "cwd": "/repo", "bypass": false, "running": true, "status": "needs-input",
                "title": "Fix the build", "message": "Claude needs your permission to use Bash",
                "resume_at": null, "connector": "c1", "subject": { "issue": "uuid" }, "opened": 3, "updated": 1792400000, "session": "s1",
                "stats": { "active_ms": 4320000, "cost_usd": 3.42 },
            }] }),
        );
        round_trip(
            MachineEvent::Usage { usage: Usage { five_hour: Some(crate::agent::LimitWindow { used_percentage: 23.5, resets_at: 1792400000 }), seven_day: None } },
            json!({ "kind": "usage", "usage": { "five_hour": { "used_percentage": 23.5, "resets_at": 1792400000 } } }),
        );
        assert!(serde_json::from_value::<MachineEvent>(json!({ "kind": "output", "tab": "t", "data": "not base64!" })).is_err());
    }

    #[test]
    fn hub_calls_match_the_tauri_commands() {
        round_trip(HubCall::GetSettings, json!({ "cmd": "get_settings" }));
        round_trip(HubCall::ConnectSlack { token: "xoxb-1".into() }, json!({ "cmd": "connect_slack", "token": "xoxb-1" }));
        round_trip(
            HubCall::Stage { connector: "c1".into(), subject: Subject::LinearIssue { issue: "uuid".into() }, stage: Stage::Coding },
            json!({ "cmd": "stage", "connector": "c1", "issue": "uuid", "stage": "coding" }),
        );
        round_trip(
            HubCall::LinearTake { connector: "c2".into(), channel: "C1".into(), thread: "1.2".into(), issue: "ENG-7".into() },
            json!({ "cmd": "linear_take", "connector": "c2", "channel": "C1", "thread": "1.2", "issue": "ENG-7" }),
        );
        round_trip(
            HubCall::Import { store: json!({ "connectors": [] }), linear_key: Some("lin_api_1".into()), slack_key: None, github_key: None },
            json!({ "cmd": "import", "store": { "connectors": [] }, "linear_key": "lin_api_1", "slack_key": null, "github_key": null }),
        );
    }

    #[test]
    fn client_frames_are_tagged_by_type() {
        round_trip(ClientFrame::Auth { token: "t0k".into() }, json!({ "type": "auth", "token": "t0k" }));
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
        let machine = MachineView { id: "m1".into(), name: "mac-mini".into(), os: "macos".into(), online: true, last_seen: 1792400000 };
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
            ToClient::Settings { settings: Settings { linear: None, slack: None, github: None, connectors: vec![], checked: None } },
            json!({ "type": "settings", "settings": { "linear": null, "slack": null, "github": null, "connectors": [], "checked": null } }),
        );
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
            ToMachine::Hello { machine: MachineView { id: "m2".into(), name: "vps".into(), os: "linux".into(), online: true, last_seen: 0 } },
            json!({ "type": "hello", "machine": { "id": "m2", "name": "vps", "os": "linux", "online": true, "last_seen": 0 } }),
        );
    }
}
