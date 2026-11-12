//! The global orchestrator: one chat above every project of the account. It hands each request to
//! the project's Capitão, on whatever machine has it, and sums up what the Capitães report. Its tab
//! runs in `~/.pinned/orquestrador`, a folder no project list holds, so that folder's memory is the
//! account's global memory.

use crate::accounts::AccountId;
use crate::agent::{AgentStatus, Role, ROLE_ENV};
use crate::history;
use crate::stats::PrLink;
use crate::wire::{MachineId, ProjectView, TabId, TabView};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// `PINNED_ROLE` of the global orchestrator's tab. It is not a `Role`, so `TabView.role` never
/// carries it to an older app.
pub const ROLE: &str = "global-orchestrator";
/// The tab's folder, under the home folder.
pub const FOLDER: [&str; 2] = [".pinned", "orquestrador"];
pub const DEFAULT_NAME: &str = "Orchestrator";
const NAME_MAX: usize = 40;
const SETTINGS_FILE: &str = "global-orchestrator.json";
const LEDGER_FILE: &str = "captain-requests.json";
/// Enough to answer any retry of a recent request; older ones fall off first.
const LEDGER_KEEP: usize = 500;

pub fn env() -> Vec<(String, String)> {
    vec![(ROLE_ENV.to_owned(), ROLE.to_owned())]
}

pub fn is_global(env: &[(String, String)]) -> bool {
    env.iter().any(|(k, v)| k == ROLE_ENV && v == ROLE)
}

/// How this machine's global orchestrator works. One file in the machine's config folder.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(rename = "GlobalOrchestratorSettings")]
pub struct Settings {
    /// The chat's name, and the prefix of every message it hands a Capitão, as in `[Orchestrator] `.
    #[serde(default = "default_name")]
    pub name: String,
    /// This machine hosts the account's global orchestrator. One machine of the account does.
    #[serde(default)]
    pub host: bool,
    #[serde(default)]
    pub busy: BusyPolicy,
    #[serde(default)]
    pub reports: ReportPolicy,
    /// The chat folds consecutive reports into one row.
    #[serde(default = "yes")]
    pub group_reports: bool,
    /// The phone reads the orchestrator's summaries aloud.
    #[serde(default)]
    pub read_aloud: bool,
}

/// What happens to a message for a Capitão that is busy.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BusyPolicy {
    /// It waits in the Capitão's queue and is typed once the Capitão is idle.
    #[default]
    Queue,
    /// It is refused with `ocupado`, and the orchestrator tries again later.
    Refuse,
}

/// When a Capitão reports to the global orchestrator.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportPolicy {
    /// At the end of every turn that a message from the orchestrator started, and with `tell_orchestrator`.
    #[default]
    Auto,
    /// Only with `tell_orchestrator`.
    ToolOnly,
}

fn default_name() -> String {
    DEFAULT_NAME.to_owned()
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings { name: default_name(), host: false, busy: BusyPolicy::Queue, reports: ReportPolicy::Auto, group_reports: true, read_aloud: false }
    }
}

impl Settings {
    pub fn load(dir: Option<&Path>) -> Self {
        let saved = dir.and_then(|d| std::fs::read(d.join(SETTINGS_FILE)).ok()).and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok());
        saved.unwrap_or_default().cleaned()
    }

    pub fn save(&self, dir: Option<&Path>) -> Result<(), String> {
        let Some(dir) = dir else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(SETTINGS_FILE), serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    /// The name on one line and at most 40 characters, the default one when blank. It sits inside
    /// `[` `]` in every prefix and report, so brackets are dropped.
    pub fn cleaned(mut self) -> Self {
        let name: String = self.name.chars().filter(|c| !matches!(c, '[' | ']')).map(|c| if c.is_control() { ' ' } else { c }).collect();
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        self.name = if name.is_empty() { default_name() } else { name.chars().take(NAME_MAX).collect() };
        self
    }
}

/// What became of a message handed to a Capitão, or of a report handed to the orchestrator.
///
/// `{"outcome":"na_fila","tab":"tab-1a-3","position":2}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "outcome")]
pub enum Handoff {
    /// Typed into the idle session now.
    #[serde(rename = "entregue")]
    Delivered { tab: TabId },
    /// Waits in the tab's queue and is typed once the session is idle. It survives a restart.
    /// `position` counts this message among those waiting for the tab.
    #[serde(rename = "na_fila")]
    Queued { tab: TabId, position: usize },
    /// The session was not running, or had no conversation yet: it started with the message as its
    /// first prompt.
    #[serde(rename = "iniciado")]
    Started { tab: TabId },
    /// The Capitão is working and the busy policy refuses. Nothing was typed or queued, and a retry
    /// with the same `request_id` may deliver it.
    #[serde(rename = "ocupado")]
    Busy { tab: TabId },
}

impl Handoff {
    pub fn tab(&self) -> &str {
        match self {
            Handoff::Delivered { tab } | Handoff::Queued { tab, .. } | Handoff::Started { tab } | Handoff::Busy { tab } => tab,
        }
    }
}

/// How far a handed message got.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Progress {
    /// In the tab's queue.
    #[default]
    Queued,
    /// Typed, or given as the first prompt: the Capitão's turn in flight started from it.
    InFlight,
    /// The turn it started ended.
    Answered,
}

/// One message handed on this machine, kept so a repeat of its `request_id` answers the same
/// without typing twice, and so the Capitão's reply can go back up when its turn ends.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sent {
    pub request_id: String,
    #[serde(default)]
    pub sender: String,
    /// The Capitão's project, or the global orchestrator's folder for a report it took.
    pub project: PathBuf,
    pub handoff: Handoff,
    #[serde(default)]
    pub progress: Progress,
    /// Unix seconds.
    pub at: u64,
}

/// The projects whose Capitão runs a turn that a message from the orchestrator started.
pub type Turns = Arc<Mutex<HashSet<PathBuf>>>;

/// Every message handed on this machine, newest last, saved with each change.
pub struct Ledger {
    file: Option<PathBuf>,
    sent: Vec<Sent>,
    /// What `sent` holds in flight, shared so a tab's view reads it without the ledger's lock,
    /// which a hand-off holds while it starts the tab.
    turns: Turns,
}

impl Ledger {
    pub fn load(dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join(LEDGER_FILE));
        let sent = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        let ledger = Ledger { file, sent, turns: Turns::default() };
        ledger.count_turns();
        ledger
    }

    pub fn turns(&self) -> Turns {
        self.turns.clone()
    }

    pub fn find(&self, request_id: &str) -> Option<&Sent> {
        self.sent.iter().find(|s| s.request_id == request_id)
    }

    pub fn record(&mut self, sent: Sent) {
        self.sent.push(sent);
        if self.sent.len() > LEDGER_KEEP {
            self.sent.drain(..self.sent.len() - LEDGER_KEEP);
        }
        self.changed();
    }

    /// The queued messages with these request ids were typed: the turn they start is in flight.
    /// Whether any was.
    pub fn typed(&mut self, request_ids: &[String]) -> bool {
        let mut changed = false;
        for sent in self.sent.iter_mut().filter(|s| s.progress == Progress::Queued && request_ids.contains(&s.request_id)) {
            sent.progress = Progress::InFlight;
            changed = true;
        }
        if changed {
            self.changed();
        }
        changed
    }

    pub fn queued_on(&self, tab: &str) -> usize {
        self.sent.iter().filter(|s| s.progress == Progress::Queued && matches!(&s.handoff, Handoff::Queued { tab: t, .. } if t == tab)).count()
    }

    /// Whether a message started `tab` and its turn is still in flight.
    pub fn started(&self, tab: &str) -> bool {
        self.sent.iter().any(|s| s.progress == Progress::InFlight && matches!(&s.handoff, Handoff::Started { tab: t } if t == tab))
    }

    /// The messages whose turn is in flight in `project`'s Capitão, oldest first.
    pub fn in_flight(&self, project: &Path) -> Vec<Sent> {
        self.sent.iter().filter(|s| s.progress == Progress::InFlight && s.project == project).cloned().collect()
    }

    /// Marks the in-flight messages of `project`'s Capitão answered and returns them, oldest first.
    pub fn answer(&mut self, project: &Path) -> Vec<Sent> {
        let mut answered = vec![];
        for sent in self.sent.iter_mut().filter(|s| s.progress == Progress::InFlight && s.project == project) {
            sent.progress = Progress::Answered;
            answered.push(sent.clone());
        }
        if !answered.is_empty() {
            self.changed();
        }
        answered
    }

    /// Makes the message look sent `secs` earlier, to test what age changes.
    #[cfg(test)]
    pub fn backdate(&mut self, request_id: &str, secs: u64) {
        if let Some(sent) = self.sent.iter_mut().find(|s| s.request_id == request_id) {
            sent.at = sent.at.saturating_sub(secs);
        }
    }

    fn count_turns(&self) {
        *self.turns.lock().unwrap() = self.sent.iter().filter(|s| s.progress == Progress::InFlight).map(|s| s.project.clone()).collect();
    }

    fn changed(&self) {
        self.count_turns();
        self.save();
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(bytes) = serde_json::to_vec_pretty(&self.sent) else { return };
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

/// What a Capitão tells the global orchestrator, at the end of a turn one of its messages started
/// or through `tell_orchestrator`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct CaptainReport {
    /// The machine the Capitão runs on. Empty when that machine does not know its id.
    #[serde(default)]
    pub machine: MachineId,
    #[serde(default)]
    pub machine_name: String,
    /// The project's name, its folder's.
    pub project: String,
    /// The project's folder on that machine.
    pub path: String,
    /// The Capitão's tab.
    pub tab: TabId,
    pub text: String,
    /// The `request_id`s of the orchestrator's messages whose turn this report ends. Empty for
    /// `tell_orchestrator`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<String>,
}

impl CaptainReport {
    /// The prompt typed into the global orchestrator. Its first line is a header the chat parses:
    ///
    /// `[pinned] Capitão site (MacBook) [<machine id> <tab id>]: <text>`
    ///
    /// The project and machine names lose the characters that would make it ambiguous, and an
    /// unknown machine id is `-`.
    pub fn line(&self) -> String {
        let clean = |text: &str| text.chars().map(|c| if matches!(c, '(' | ')' | '[' | ']' | ':') || c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
        let id = |text: &str| Some(text.trim()).filter(|t| !t.is_empty() && !t.contains(char::is_whitespace) && !t.contains(']')).unwrap_or("-").to_owned();
        let project = Some(clean(&self.project)).filter(|p| !p.is_empty()).unwrap_or_else(|| "?".into());
        format!("[pinned] Capitão {project} ({}) [{} {}]: {}", clean(&self.machine_name), id(&self.machine), id(&self.tab), self.text)
    }
}

/// Every project of the account, for the global orchestrator's `projects` and `waiting` tools.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct Overview {
    pub projects: Vec<OverviewProject>,
    /// What waits on the user across the account's machines.
    pub waiting: Vec<Waiting>,
    /// Every machine of the account, with or without projects.
    #[serde(default)]
    pub machines: Vec<OverviewMachine>,
    /// Every session on the account's online machines, newest activity first; `waiting` is those
    /// that wait on the user. Absent from a hub that predates the `sessions` tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<Session>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct OverviewMachine {
    pub id: MachineId,
    pub name: String,
    pub online: bool,
    #[serde(default)]
    pub version: String,
    /// Why its projects may be missing or old, such as `precisa atualizar o pinned`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// Its load, accounts and Workers at once, as it last reported them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::load::MachineState>,
}

/// One project, with every machine that has a checkout of it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct OverviewProject {
    /// What `send_to_captain` and `read_captain` take: the `repo`, or `<machine id>:<path>` for a
    /// folder with no origin.
    pub key: String,
    pub name: String,
    /// The origin as `host/owner/name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub checkouts: Vec<OverviewCheckout>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct OverviewCheckout {
    pub machine: MachineId,
    pub machine_name: String,
    pub online: bool,
    /// The pinned build the machine last signed in with.
    #[serde(default)]
    pub version: String,
    pub path: String,
    pub captain: CaptainState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captain_tab: Option<TabId>,
    /// Worker tabs of the project running on that machine.
    #[serde(default)]
    pub workers: u32,
    /// How busy that machine is, as it last reported. Absent while it is offline or too old to report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<crate::load::Load>,
}

/// Whether a message to the project's Capitão would be typed now.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptainState {
    /// Idle, finished, failed or stopped: a message is typed now or starts it. Claude Code's
    /// idle notification after a finished turn leaves it idle.
    #[serde(rename = "livre")]
    Idle,
    /// Running a turn, which a message from the orchestrator may have started before its first
    /// hook, or asking the user mid-turn: a message queues.
    #[serde(rename = "trabalhando")]
    Working,
    /// No Capitão tab, or the machine is offline: a message opens one where the machine is online.
    #[serde(rename = "nenhum")]
    Missing,
}

impl CaptainState {
    pub fn of(captain: Option<&TabView>) -> CaptainState {
        match captain {
            None => CaptainState::Missing,
            Some(tab) if tab.running && (tab.orchestrator_turn || tab.status == Some(AgentStatus::Working) || tab.status == Some(AgentStatus::NeedsInput) && tab.interrupted) => {
                CaptainState::Working
            }
            Some(_) => CaptainState::Idle,
        }
    }
}

/// A tab that waits on the user.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Waiting {
    pub machine: MachineId,
    pub machine_name: String,
    pub tab: TabId,
    pub kind: WaitingKind,
    /// Whether a `needs_input` tab waits on an approval or on a question. Absent from a hub that
    /// predates it, and from any other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<AskKind>,
    /// The tool an `approval` is for, such as `Bash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// What the tool call or the question is, such as the command a permission prompt shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The project folder the tab works in, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// What the session asks, or the error that stopped it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Unix seconds of the tab's last status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaitingKind {
    NeedsInput,
    Failed,
    /// An agent's action waits on the user's approval in the app. Only a node's `waiting` tool
    /// lists it, from `HubCall::Confirmations`; the hub's `Overview` never does.
    Confirm,
}

/// What a session that cannot go on without the user asks of them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    /// A tool waits on a permission prompt.
    Approval,
    /// The session asked a question (`AskUserQuestion`).
    Question,
}

impl AskKind {
    /// How a row or a notice names it.
    pub fn word(self) -> &'static str {
        match self {
            AskKind::Approval => "approval",
            AskKind::Question => "question",
        }
    }
}

/// What the notice `message` of a session blocked on the user asks: an approval, with the tool it
/// is for, when Claude Code said it needs permission to use one, else a question. The sidebar's
/// `askOf` in `ui/global-chat.js` follows the same cases, which `waiting_cases.json` lists.
pub fn ask_of(message: Option<&str>) -> (AskKind, Option<String>) {
    let message = message.unwrap_or_default();
    if !message.to_ascii_lowercase().contains("permission") {
        return (AskKind::Question, None);
    }
    let tool = message.split_once(" to use ").map(|(_, tool)| tool.trim().trim_end_matches('.').to_owned()).filter(|tool| !tool.is_empty());
    (AskKind::Approval, tool)
}

/// Why `tab` waits on the user, or None when it does not. The sidebar's `waitingKindOf` in
/// `ui/global-chat.js` follows the same cases, which `waiting_cases.json` lists for both tests.
///
/// Only a session that cannot go on without the user waits: a question (`AskUserQuestion`) or an
/// approval pending mid-turn, or a failure. A finished turn that ended asking something is an
/// `offer_of`, which waits on nobody. The global orchestrator's own tab never waits: the user talks
/// to it directly.
pub fn waiting_kind(tab: &TabView) -> Option<WaitingKind> {
    if tab.orchestrator_global {
        return None;
    }
    match tab.status {
        Some(AgentStatus::Failed) => Some(WaitingKind::Failed),
        Some(AgentStatus::NeedsInput) if tab.running && tab.interrupted => Some(WaitingKind::NeedsInput),
        _ => None,
    }
}

/// How long a finished turn's question stays an offer.
pub const OFFER_SECS: u64 = 24 * 60 * 60;

/// What the finished turn of `tab` offered or asked in its last paragraph, while that still stands
/// at `now` (Unix seconds): the tab's session runs, its turn is over, and it is not a worker (whose
/// result already went up to the thread that started it, the delivery its task ends on). An offer
/// drops after `OFFER_SECS`. `ui/global-chat.js`'s `offerOf` follows the same cases.
pub fn offer_of(tab: &TabView, now: u64) -> Option<&str> {
    let finished = tab.running && !matches!(tab.status, Some(AgentStatus::Working | AgentStatus::Failed)) && !(tab.status == Some(AgentStatus::NeedsInput) && tab.interrupted);
    let fresh = tab.updated.is_none_or(|at| now.saturating_sub(at) < OFFER_SECS);
    (!tab.orchestrator_global && finished && fresh && tab.role != Some(Role::Worker)).then_some(tab.question.as_deref()).flatten()
}

/// The question a reply puts to the user: its last paragraph's last sentence, when that paragraph
/// ends in a question mark outside code, cut at `QUESTION_CHARS`. None when the reply ends on
/// anything else, so a question quoted or answered earlier in the reply asks nothing.
pub fn question_in(reply: &str) -> Option<String> {
    let chars: Vec<char> = reply.trim_end().chars().collect();
    let paragraph = (1..chars.len()).rev().find(|&i| chars[i] == '\n' && chars[i - 1] == '\n').map_or(0, |i| i + 1);
    let closes = |c: &char| c.is_whitespace() || matches!(c, '*' | '_' | ')' | '"' | '\u{201d}' | '\u{bb}');
    let ends_sentence = |i: usize| chars.get(i + 1).is_none_or(closes);
    let mut in_code = false;
    let mut end = None;
    for (i, c) in chars.iter().enumerate().skip(paragraph) {
        match c {
            '`' => in_code = !in_code,
            '?' | '\u{ff1f}' if !in_code && ends_sentence(i) => end = Some(i),
            _ => {}
        }
    }
    let end = end.filter(|&end| chars[end + 1..].iter().all(closes))?;
    let start = (paragraph..end).rev().find(|&i| chars[i] == '\n' || (matches!(chars[i], '.' | '!' | '?' | '\u{ff1f}') && chars[i + 1].is_whitespace())).map_or(paragraph, |i| i + 1);
    let sentence: String = chars[start..=end].iter().collect();
    let sentence = sentence.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '*' | '_' | '-' | '#' | '>')).split_whitespace().collect::<Vec<_>>().join(" ");
    Some(crate::thread::clip(&sentence, QUESTION_CHARS))
}

const QUESTION_CHARS: usize = 300;

/// What one machine holds, as `overview` reads it: the node's own lists, or what the hub last heard.
pub struct MachineSnapshot<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub online: bool,
    pub version: &'a str,
    pub projects: &'a [ProjectView],
    pub tabs: &'a [TabView],
    /// Why its projects may be missing or old.
    pub problem: Option<&'a str>,
}

/// The project's Capitão among `tabs`: the lowest-numbered orchestrator tab working in `path`.
pub fn captain_tab<'a>(tabs: &'a [TabView], path: &str) -> Option<&'a TabView> {
    tabs.iter().filter(|t| t.role == Some(Role::Orchestrator) && t.cwd == path).min_by_key(|t| t.opened)
}

/// Every project on `machines`, one entry per repo however many machines have it, and what waits
/// on the user there. An offline machine's checkouts are listed with no Capitão, and nothing of it
/// waits, since its tabs are not live.
pub fn overview(machines: &[MachineSnapshot]) -> Overview {
    overview_at(machines, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

/// `overview` as it stands at `now`, in Unix seconds: the time an offer's 24 hours run against.
pub fn overview_at(machines: &[MachineSnapshot], now: u64) -> Overview {
    let mut projects: Vec<OverviewProject> = vec![];
    let mut waiting = vec![];
    let mut sessions = vec![];
    for machine in machines {
        let tabs = if machine.online { machine.tabs } else { &[] };
        for project in machine.projects {
            let captain = captain_tab(tabs, &project.path);
            let workers = tabs.iter().filter(|t| (t.running || t.status == Some(AgentStatus::Working)) && !finished(t) && t.role == Some(Role::Worker) && history::project_of(Path::new(&t.cwd)) == Path::new(&project.path)).count();
            let checkout = OverviewCheckout {
                machine: machine.id.to_owned(),
                machine_name: machine.name.to_owned(),
                online: machine.online,
                version: machine.version.to_owned(),
                path: project.path.clone(),
                captain: CaptainState::of(captain),
                captain_tab: captain.map(|t| t.id.clone()),
                workers: workers as u32,
                load: None,
            };
            let key = project.repo.clone().unwrap_or_else(|| format!("{}:{}", machine.id, project.path));
            match projects.iter_mut().find(|p| p.key == key) {
                Some(listed) => listed.checkouts.push(checkout),
                None => projects.push(OverviewProject { key, name: project.name.clone(), repo: project.repo.clone(), checkouts: vec![checkout] }),
            }
        }
        // What waits on the user is the sessions in those two states, so the two lists never differ.
        for tab in tabs.iter().filter(|t| !t.orchestrator_global) {
            let session = session(machine, tab, now);
            let kind = match session.state {
                SessionState::WaitingOnInput => Some(WaitingKind::NeedsInput),
                SessionState::Error => Some(WaitingKind::Failed),
                _ => None,
            };
            if let Some(kind) = kind {
                let (ask, tool) = if kind == WaitingKind::NeedsInput { let (ask, tool) = ask_of(tab.message.as_deref()); (Some(ask), tool) } else { (None, None) };
                waiting.push(Waiting {
                    machine: session.machine.clone(),
                    machine_name: session.machine_name.clone(),
                    tab: session.id.clone(),
                    kind,
                    ask,
                    tool,
                    detail: tab.ask_detail.clone().filter(|_| kind == WaitingKind::NeedsInput),
                    project: machine.projects.iter().any(|p| Path::new(&p.path) == Path::new(&session.project.path)).then(|| session.project.path.clone()),
                    title: session.title.clone(),
                    message: session.message.clone(),
                    since: tab.updated,
                });
            }
            sessions.push(session);
        }
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
    let machines = machines
        .iter()
        .map(|m| OverviewMachine { id: m.id.to_owned(), name: m.name.to_owned(), online: m.online, version: m.version.to_owned(), problem: m.problem.map(str::to_owned), state: None })
        .collect();
    Overview { projects, waiting, machines, sessions: Some(sessions) }
}

/// Whether a tab's turn ended and nothing waits on it: idle with a status, unlike a session that
/// started and has not reported yet. A worker in this state already reported to its parent.
fn finished(tab: &TabView) -> bool {
    tab.status.is_some() && SessionState::of(tab) == SessionState::Idle
}

/// One session of the account, as the global orchestrator's `sessions` tool lists it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Session {
    /// The tab's id, which `read_session` takes.
    pub id: TabId,
    pub machine: MachineId,
    pub machine_name: String,
    /// The account the session runs under, such as `claude` or `bloom`.
    pub account: AccountId,
    pub project: SessionProject,
    pub role: SessionRole,
    /// The Capitão a worker works for: the first Capitão up the tabs that started it, else its
    /// project's Capitão on that machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captain: Option<TabId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<PrLink>,
    pub state: SessionState,
    /// What it asks the user, or the error that stopped it. An idle session's is the offer its
    /// finished turn ended on, which waits on nobody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Unix seconds of its last message, else of its last status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<u64>,
}

/// The project a session works in: the project its folder belongs to, a worktree's being its
/// repository's.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct SessionProject {
    /// The key `projects` lists, or `<machine id>:<path>` for a folder the machine does not list.
    pub key: String,
    pub name: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionRole {
    Captain,
    Worker,
    /// A thread the user opened, which no Capitão runs.
    Thread,
}

/// What a session is doing, from what its machine knows of its tab.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Its session runs a turn: status working, or a message from the global orchestrator started
    /// it before its first hook.
    Running,
    /// Its session runs and no turn is in progress: done, Claude Code's idle notice, or no status yet.
    Idle,
    /// It waits on the user (`waiting_kind` is `needs_input`): a question or an approval pending
    /// mid-turn. A finished turn that ended asking something is `Idle`, its question in `message`.
    WaitingOnInput,
    /// It stopped on a failure, a usage limit included (`waiting_kind` is `failed`).
    Error,
    /// Its session is not running. A message starts it again.
    Settled,
}

impl SessionState {
    pub fn of(tab: &TabView) -> SessionState {
        match waiting_kind(tab) {
            Some(WaitingKind::Failed) => SessionState::Error,
            Some(WaitingKind::NeedsInput | WaitingKind::Confirm) => SessionState::WaitingOnInput,
            None if !tab.running => SessionState::Settled,
            None if tab.status == Some(AgentStatus::Working) || tab.orchestrator_turn => SessionState::Running,
            None => SessionState::Idle,
        }
    }
}

impl Session {
    /// Whether the session is in `project`, a key, name or folder, and on `machine`, an id or a
    /// name with or without `@`. Names match in any case.
    pub fn matches(&self, project: Option<&str>, machine: Option<&str>) -> bool {
        let project_is = |p: &str| self.project.key == p || self.project.path == p || self.project.name.eq_ignore_ascii_case(p);
        let machine_is = |m: &str| self.machine == m || self.machine_name.eq_ignore_ascii_case(m);
        project.map(str::trim).is_none_or(project_is) && machine.map(|m| m.trim().trim_start_matches('@')).is_none_or(machine_is)
    }
}

/// One tab as a session. `overview` lists one per tab of each online machine, the global
/// orchestrator's own tab left out.
fn session(machine: &MachineSnapshot, tab: &TabView, now: u64) -> Session {
    let folder = history::project_of(Path::new(&tab.cwd));
    let listed = machine.projects.iter().find(|p| Path::new(&p.path) == folder);
    let path = folder.display().to_string();
    let project = SessionProject {
        key: listed.and_then(|p| p.repo.clone()).unwrap_or_else(|| format!("{}:{path}", machine.id)),
        name: listed.map(|p| p.name.clone()).unwrap_or_else(|| folder.file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned())),
        path,
    };
    let role = match tab.role {
        Some(Role::Orchestrator) => SessionRole::Captain,
        Some(Role::Worker) => SessionRole::Worker,
        None => SessionRole::Thread,
    };
    let captain = (role == SessionRole::Worker).then(|| owning_captain(machine.tabs, tab).or_else(|| captain_tab(machine.tabs, &project.path)).map(|c| c.id.clone())).flatten();
    let state = SessionState::of(tab);
    Session {
        id: tab.id.clone(),
        machine: machine.id.to_owned(),
        machine_name: machine.name.to_owned(),
        account: tab.account.clone(),
        project,
        role,
        captain,
        title: tab.title.clone().or_else(|| tab.prompt_title.clone()),
        branch: tab.branch.clone(),
        pr: tab.pr.clone(),
        message: match state {
            SessionState::WaitingOnInput | SessionState::Error => tab.message.clone(),
            _ => offer_of(tab, now).map(str::to_owned),
        },
        state,
        last_activity: tab.last_message_at.or(tab.updated),
    }
}

/// The first Capitão up the tabs that started `tab`.
fn owning_captain<'a>(tabs: &'a [TabView], tab: &TabView) -> Option<&'a TabView> {
    let mut parent = tab.spawned_by.as_deref();
    for _ in 0..tabs.len() {
        let up = tabs.iter().find(|t| Some(t.id.as_str()) == parent)?;
        if up.role == Some(Role::Orchestrator) {
            return Some(up);
        }
        parent = up.spawned_by.as_deref();
    }
    None
}

/// The global orchestrator's system prompt. `name` is its configured name, `machine` the machine it
/// runs on.
pub fn prompt(name: &str, machine: &str) -> String {
    format!(
        "Você é o {name}, o orquestrador global do Felipe no pinned: o chat acima de todos os projetos dele, em todas as máquinas. Você roda na máquina {machine}, numa pasta que não é projeto.

Você nunca faz trabalho de projeto: não edita arquivos, não roda comandos e não inicia workers. Você pode **ler** os repositórios só com repo_overview, repo_read e repo_search, que são somente leitura. Todo trabalho de projeto vai para o Capitão daquele projeto com send_to_captain; ele faz o trabalho e te reporta. Seus próprios Read, Write, Edit, Grep e Glob alcançam só a sua pasta, o seu scratchpad, a sua pasta de memória e os uploads do Felipe (e leem o scratchpad dos Capitães); você não tem Bash.

Para cada pedido:
1. Chame projects() primeiro e escolha o projeto pelo texto do Felipe. Use a key que projects() lista como project.
2. #projeto no texto força o projeto, e @máquina força a máquina. Cada marca vale só para o parágrafo em que aparece: um texto com parágrafos para projetos diferentes vira uma mensagem para cada Capitão.
3. Quando o mesmo repo está em várias máquinas, prefira a máquina com um Capitão rodando (livre ou trabalhando), depois a sua própria máquina ({machine}), depois a primeira online, e passe machine em send_to_captain. Se ainda houver dúvida, pergunte ao Felipe antes de enviar qualquer coisa.
4. Use um request_id novo em cada mensagem. Repita um request_id só para reenviar a mesma mensagem depois de um erro.
4b. Quando o Felipe pede o Capitão numa Upstash Box, ou nenhuma máquina dele deve rodar o trabalho, passe box: true (ou {{ \"size\": \"large\" }}) em send_to_captain em vez de machine. O hub abre a box com a conexão Upstash da conta, clona o repo e inicia o Capitão lá; a resposta é iniciado com a máquina da box, que liga em alguns minutos e aparece em projects(). A box é apagada quando o Capitão fica parado. Uma nova mensagem para o mesmo projeto com box vai para a box que já roda.
5. send_to_captain responde entregue (digitado agora), na_fila (o Capitão está ocupado e lê quando terminar), iniciado (o Capitão começou com a mensagem) ou ocupado (tente de novo mais tarde). Conte ao Felipe em uma frase.

Capitão reports arrive as messages that start with \"[pinned] Capitão <project> (<machine>)\". Sum each one up for Felipe in a few lines: what was done, what is left and what needs Felipe. read_captain(project) reads a Capitão's last replies, and waiting() lists what waits on Felipe on every machine.

To answer what is running in a project or on a machine, such as \"what is running in client-apps?\", call sessions(project) or sessions(machine=...). It lists every session on every machine and account: loose threads Felipe opened, workers and Capitães. projects() counts only a Capitão's workers, so never answer that question from projects() alone. Answer with each session's title, state, branch, PR, machine and account, and call read_session(id) to read the last messages of any session whose title and state don't say enough.

Before you hand a request to a Capitão, look at the repo when the request is more than a one-liner: repo_overview(project) for its shape and its CLAUDE.md/AGENTS.md rules, then at most a few repo_search or repo_read calls to find where the work lands. Use what you find to (1) split the request into one message per independent piece, in order; (2) write the message with file paths and symbols to start from, the repo's rules that apply, and a suggested model tier (small and contained, or crosses modules). When a Capitão reports something you can check, such as a file or a change it says it made, check it with repo_search or repo_read before you tell Felipe it is done, and say so when it doesn't match. If a question needs more than about five reads, ask the Capitão to explore and report instead of reading on. Never quote a line marked [redacted]. When a repo tool answers that reading repositories is off, tell Felipe in one sentence where to turn it on and go on without it.

Quando uma ferramenta falha porque uma máquina ou o hub precisa atualizar, ou projects() lista uma máquina com problem, diga ao Felipe numa frase simples qual máquina precisa atualizar o pinned ou não respondeu, sem o texto do erro.

Quando uma resposta termina com uma pergunta fechada ao Felipe, a última linha traz só as opções, separadas por \" | \", assim:
Opções: Sim | Ainda não

Guarde fatos duráveis sobre o Felipe e os projetos dele (preferências, decisões, qual máquina tem o quê) com memory_save, procure com memory_search antes de decidir e apague os errados com memory_forget."
    )
}

/// What a project's Capitão is told about the global orchestrator `name`, on top of its own prompt.
pub fn captain_prompt(name: &str, reports: ReportPolicy) -> String {
    let end = match reports {
        ReportPolicy::Auto => "Your last reply of such a turn goes back to it by itself, so end that turn with what you did, what is left and what needs the user.",
        ReportPolicy::ToolOnly => "Nothing goes back to it by itself: when you finish its request, report with tell_orchestrator.",
    };
    format!("A message that starts with \"[{name}] \" comes from the global orchestrator, the user's chat above every project. Do that work as you would the user's. Mid-work, use tell_orchestrator for a question it must answer or news it must hear now. {end}")
}

/// Where a relayed message goes, and what it carries.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayTo {
    /// `text` for the Capitão of the project at `path` on `machine`, as `Call::Enqueue` takes it.
    /// The hub takes it only from the machine that hosts the account's global orchestrator.
    Captain {
        machine: MachineId,
        path: String,
        text: String,
        #[serde(default)]
        sender: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        busy: Option<BusyPolicy>,
    },
    /// A report for the account's global orchestrator, as `Call::OrchestratorReport` takes it. The
    /// hub takes it only from a machine whose Capitão tab `report.tab` works in `report.path`.
    Orchestrator { report: CaptainReport },
    /// `text` for the Capitão of `repo` in an Upstash Box the hub opens with the account's own
    /// connection, or the box that already runs it. Taken only from the global orchestrator's host.
    CaptainBox {
        repo: String,
        /// None follows the project's and the account's default box size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<crate::connectors::BoxSize>,
        text: String,
        #[serde(default)]
        sender: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        busy: Option<BusyPolicy>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(text: &str) -> CaptainReport {
        CaptainReport {
            machine: "6f1c2b7d-0000-4000-8000-00000000c0de".into(),
            machine_name: "MacBook".into(),
            project: "site".into(),
            path: "/Users/f/site".into(),
            tab: "tab-1a-3".into(),
            text: text.into(),
            answers: vec![],
        }
    }

    #[test]
    fn a_report_line_names_the_project_machine_and_tab_in_a_header_the_chat_parses() {
        assert_eq!(report("Done: the form validates.\nPR #12 is open.").line(), "[pinned] Capitão site (MacBook) [6f1c2b7d-0000-4000-8000-00000000c0de tab-1a-3]: Done: the form validates.\nPR #12 is open.");
        let odd = CaptainReport { machine: String::new(), machine_name: "Felipe's (mini)\n".into(), project: "a[b]:c".into(), ..report("ok") };
        assert_eq!(odd.line(), "[pinned] Capitão a b c (Felipe's mini) [- tab-1a-3]: ok");
    }

    #[test]
    fn settings_default_to_queueing_and_automatic_reports_and_keep_a_clean_name() {
        assert_eq!(serde_json::to_value(Settings::default()).unwrap(), serde_json::json!({ "name": "Orchestrator", "host": false, "busy": "queue", "reports": "auto", "group_reports": true, "read_aloud": false }));
        let read: Settings = serde_json::from_str(r#"{"name":"Almirante","busy":"refuse","reports":"tool_only"}"#).unwrap();
        assert_eq!(read, Settings { name: "Almirante".into(), busy: BusyPolicy::Refuse, reports: ReportPolicy::ToolOnly, ..Settings::default() });
        assert_eq!(Settings { name: "  [Comandante]\n ".into(), ..Settings::default() }.cleaned().name, "Comandante");
        assert_eq!(Settings { name: " ".into(), ..Settings::default() }.cleaned().name, "Orchestrator");
    }

    #[test]
    fn a_handoff_reads_as_its_portuguese_outcome() {
        let queued = Handoff::Queued { tab: "tab-1a-3".into(), position: 2 };
        assert_eq!(serde_json::to_string(&queued).unwrap(), r#"{"outcome":"na_fila","tab":"tab-1a-3","position":2}"#);
        for (handoff, outcome) in [(Handoff::Delivered { tab: "t".into() }, "entregue"), (Handoff::Started { tab: "t".into() }, "iniciado"), (Handoff::Busy { tab: "t".into() }, "ocupado")] {
            assert_eq!(serde_json::to_value(&handoff).unwrap(), serde_json::json!({ "outcome": outcome, "tab": "t" }));
        }
    }

    #[test]
    fn a_captain_is_working_only_while_its_session_runs_a_turn_or_asks_mid_turn() {
        let tab = |running, status| TabView { running, status, interrupted: true, ..serde_json::from_value(serde_json::json!({ "id": "t", "cwd": "/p", "bypass": false, "running": false, "status": null, "title": null, "message": null, "resume_at": null, "connector": null, "opened": 1 })).unwrap() };
        assert_eq!(CaptainState::of(None), CaptainState::Missing);
        assert_eq!(CaptainState::of(Some(&tab(true, Some(AgentStatus::Working)))), CaptainState::Working);
        assert_eq!(CaptainState::of(Some(&tab(true, Some(AgentStatus::NeedsInput)))), CaptainState::Working, "it asks the user mid-turn");
        let notified = TabView { interrupted: false, ..tab(true, Some(AgentStatus::NeedsInput)) };
        assert_eq!(CaptainState::of(Some(&notified)), CaptainState::Idle, "Claude Code's idle notification after a finished turn");
        assert_eq!(CaptainState::of(Some(&tab(true, Some(AgentStatus::Done)))), CaptainState::Idle);
        assert_eq!(CaptainState::of(Some(&tab(false, Some(AgentStatus::Working)))), CaptainState::Idle, "a stopped Capitão starts on the next message");
        assert_eq!(CaptainState::of(Some(&tab(true, None))), CaptainState::Idle, "a running Capitão that no message started");
        let started = TabView { orchestrator_turn: true, ..tab(true, None) };
        assert_eq!(CaptainState::of(Some(&started)), CaptainState::Working, "an orchestrator message started it, before its first hook");
        assert_eq!(CaptainState::of(Some(&TabView { running: false, ..started })), CaptainState::Idle, "it stopped mid-turn");
        assert_eq!(serde_json::to_value([CaptainState::Idle, CaptainState::Working, CaptainState::Missing]).unwrap(), serde_json::json!(["livre", "trabalhando", "nenhum"]));
    }

    fn view(id: &str, cwd: &str, role: Option<Role>, running: bool, status: Option<AgentStatus>) -> TabView {
        let mut tab: TabView = serde_json::from_value(serde_json::json!({ "id": id, "cwd": cwd, "bypass": false, "running": running, "status": status, "title": null, "message": null, "resume_at": null, "connector": null, "opened": id.rsplit('-').next().unwrap().parse::<u64>().unwrap() })).unwrap();
        tab.role = role;
        tab
    }

    fn project(path: &str, repo: Option<&str>) -> ProjectView {
        ProjectView { path: path.into(), name: path.rsplit('/').next().unwrap().into(), repo: repo.map(Into::into), account: "default".into(), convex: None }
    }

    #[test]
    fn the_same_repo_on_two_machines_is_one_project_with_a_checkout_on_each() {
        let mini = [project("/Users/f/site", Some("github.com/o/site"))];
        let mini_tabs = [
            view("tab-a-3", "/Users/f/site", Some(Role::Orchestrator), true, Some(AgentStatus::Working)),
            view("tab-a-5", "/Users/f/site", Some(Role::Orchestrator), true, Some(AgentStatus::Done)),
            view("tab-a-4", "/Users/f/site/.claude/worktrees/pinned-1", Some(Role::Worker), true, Some(AgentStatus::Working)),
            view("tab-a-6", "/Users/f/site", Some(Role::Worker), true, None),
            view("tab-a-7", "/Users/f/site/.claude/worktrees/pinned-2", Some(Role::Worker), false, Some(AgentStatus::Done)),
            view("tab-a-8", "/Users/f/site", None, true, Some(AgentStatus::Working)),
        ];
        let book = [project("/Users/felipe/code/site", Some("github.com/o/site"))];
        let machines = [
            MachineSnapshot { id: "m-mini", name: "Mac mini", online: true, version: "0.17.19", projects: &mini, tabs: &mini_tabs, problem: None },
            MachineSnapshot { id: "m-book", name: "MacBook", online: true, version: "0.17.18", projects: &book, tabs: &[], problem: Some("precisa atualizar o pinned") },
        ];
        assert_eq!(
            serde_json::to_value(Overview { sessions: None, ..overview(&machines) }).unwrap(),
            serde_json::json!({
                "projects": [{
                    "key": "github.com/o/site",
                    "name": "site",
                    "repo": "github.com/o/site",
                    "checkouts": [
                        { "machine": "m-mini", "machine_name": "Mac mini", "online": true, "version": "0.17.19", "path": "/Users/f/site", "captain": "trabalhando", "captain_tab": "tab-a-3", "workers": 2 },
                        { "machine": "m-book", "machine_name": "MacBook", "online": true, "version": "0.17.18", "path": "/Users/felipe/code/site", "captain": "nenhum", "workers": 0 },
                    ],
                }],
                "waiting": [],
                "machines": [
                    { "id": "m-mini", "name": "Mac mini", "online": true, "version": "0.17.19" },
                    { "id": "m-book", "name": "MacBook", "online": true, "version": "0.17.18", "problem": "precisa atualizar o pinned" },
                ],
            })
        );
    }

    #[test]
    fn a_worker_whose_turn_ended_leaves_the_count_and_one_at_work_waiting_failed_or_starting_stays() {
        let site = [project("/w/site", Some("github.com/o/site"))];
        let tabs = [
            view("tab-d-1", "/w/site", Some(Role::Orchestrator), true, Some(AgentStatus::Done)),
            view("tab-d-2", "/w/site/.claude/worktrees/pinned-1", Some(Role::Worker), true, Some(AgentStatus::Done)),
            TabView { message: Some("Claude is waiting for your input".into()), ..view("tab-d-3", "/w/site", Some(Role::Worker), true, Some(AgentStatus::NeedsInput)) },
            view("tab-d-4", "/w/site/.claude/worktrees/pinned-2", Some(Role::Worker), true, Some(AgentStatus::Working)),
            TabView { interrupted: true, ..view("tab-d-5", "/w/site", Some(Role::Worker), true, Some(AgentStatus::NeedsInput)) },
            view("tab-d-6", "/w/site", Some(Role::Worker), true, Some(AgentStatus::Failed)),
            view("tab-d-7", "/w/site", Some(Role::Worker), true, None),
            view("tab-d-8", "/w/site", Some(Role::Worker), false, Some(AgentStatus::Done)),
        ];
        let machines = [MachineSnapshot { id: "m-1", name: "MacBook", online: true, version: "0.18.0", projects: &site, tabs: &tabs, problem: None }];
        let overview = overview(&machines);
        assert_eq!(overview.projects[0].checkouts[0].workers, 4);
        let states: Vec<(&str, SessionState)> = overview.sessions.as_deref().unwrap().iter().filter(|s| s.role == SessionRole::Worker).map(|s| (s.id.as_str(), s.state)).collect();
        assert_eq!(
            states,
            [
                ("tab-d-2", SessionState::Idle),
                ("tab-d-3", SessionState::Idle),
                ("tab-d-4", SessionState::Running),
                ("tab-d-5", SessionState::WaitingOnInput),
                ("tab-d-6", SessionState::Error),
                ("tab-d-7", SessionState::Idle),
                ("tab-d-8", SessionState::Settled),
            ]
        );
    }

    #[test]
    fn a_folder_without_an_origin_is_keyed_by_its_machine_and_an_offline_machine_has_no_capitao_or_waiting() {
        let notes = [project("/w/notes", None), project("/w/site", Some("github.com/o/site"))];
        let tabs = [view("tab-b-1", "/w/notes", Some(Role::Orchestrator), true, Some(AgentStatus::Done)), view("tab-b-2", "/w/site", None, true, Some(AgentStatus::Failed))];
        let machines = [MachineSnapshot { id: "m-1", name: "Linux box", online: false, version: "0.17.10", projects: &notes, tabs: &tabs, problem: None }];
        assert_eq!(
            serde_json::to_value(Overview { sessions: None, ..overview(&machines) }).unwrap(),
            serde_json::json!({
                "projects": [
                    { "key": "m-1:/w/notes", "name": "notes", "checkouts": [{ "machine": "m-1", "machine_name": "Linux box", "online": false, "version": "0.17.10", "path": "/w/notes", "captain": "nenhum", "workers": 0 }] },
                    { "key": "github.com/o/site", "name": "site", "repo": "github.com/o/site", "checkouts": [{ "machine": "m-1", "machine_name": "Linux box", "online": false, "version": "0.17.10", "path": "/w/site", "captain": "nenhum", "workers": 0 }] },
                ],
                "waiting": [],
                "machines": [{ "id": "m-1", "name": "Linux box", "online": false, "version": "0.17.10" }],
            })
        );
    }

    #[test]
    fn waiting_holds_questions_mid_turn_and_failures_but_not_the_idle_notice_or_the_orchestrators_own_tab() {
        let site = [project("/w/site", Some("github.com/o/site"))];
        let question = TabView { interrupted: true, ask_detail: Some("git push origin main".into()), title: Some("Fix the form".into()), message: Some("Claude needs your permission to use Bash".into()), updated: Some(1794234244), ..view("tab-c-2", "/w/site/.claude/worktrees/pinned-1", Some(Role::Worker), true, Some(AgentStatus::NeedsInput)) };
        let idle_notice = TabView { interrupted: false, message: Some("Claude is waiting for your input".into()), ..view("tab-c-1", "/w/site", Some(Role::Orchestrator), true, Some(AgentStatus::NeedsInput)) };
        let failed = TabView { message: Some("API Error: 500".into()), updated: Some(1794234300), ..view("tab-c-3", "/tmp/scratch", None, false, Some(AgentStatus::Failed)) };
        let own = TabView { orchestrator_global: true, interrupted: true, ..view("tab-c-4", "/home/f/.pinned/orquestrador", None, true, Some(AgentStatus::NeedsInput)) };
        let tabs = [idle_notice, question, failed, own];
        let machines = [MachineSnapshot { id: "m-1", name: "MacBook", online: true, version: "0.17.19", projects: &site, tabs: &tabs, problem: None }];
        assert_eq!(
            serde_json::to_value(Overview { sessions: None, ..overview(&machines) }).unwrap(),
            serde_json::json!({
                "projects": [{ "key": "github.com/o/site", "name": "site", "repo": "github.com/o/site", "checkouts": [{ "machine": "m-1", "machine_name": "MacBook", "online": true, "version": "0.17.19", "path": "/w/site", "captain": "livre", "captain_tab": "tab-c-1", "workers": 1 }] }],
                "waiting": [
                    { "machine": "m-1", "machine_name": "MacBook", "tab": "tab-c-2", "kind": "needs_input", "ask": "approval", "tool": "Bash", "detail": "git push origin main", "project": "/w/site", "title": "Fix the form", "message": "Claude needs your permission to use Bash", "since": 1794234244 },
                    { "machine": "m-1", "machine_name": "MacBook", "tab": "tab-c-3", "kind": "failed", "message": "API Error: 500", "since": 1794234300 },
                ],
                "machines": [{ "id": "m-1", "name": "MacBook", "online": true, "version": "0.17.19" }],
            })
        );
    }

    #[test]
    fn waiting_lists_a_workers_approval_and_question_from_every_machine_with_the_machine_the_project_and_what_is_asked() {
        let site = [project("/w/site", Some("github.com/o/site"))];
        let on_mini = [project("/Users/f/site", Some("github.com/o/site"))];
        let approval = TabView { interrupted: true, ask_detail: Some("git push origin main".into()), title: Some("Push the fix".into()), message: Some("Claude needs your permission to use Bash".into()), updated: Some(1794239000), ..view("tab-m-3", "/Users/f/site/.claude/worktrees/pinned-3", Some(Role::Worker), true, Some(AgentStatus::NeedsInput)) };
        let question = TabView { interrupted: true, ask_detail: Some("Which one should open the PR?".into()), title: Some("Pick the PR opener".into()), message: Some("Claude needs your input".into()), updated: Some(1794239500), ..view("tab-b-9", "/w/site/.claude/worktrees/pinned-9", Some(Role::Worker), true, Some(AgentStatus::NeedsInput)) };
        let working = TabView { interrupted: true, ..view("tab-m-4", "/Users/f/site/.claude/worktrees/pinned-4", Some(Role::Worker), true, Some(AgentStatus::Working)) };
        let (book_tabs, mini_tabs) = ([question], [approval, working]);
        let machines = [
            MachineSnapshot { id: "m-book", name: "MacBook", online: true, version: "0.17.24", projects: &site, tabs: &book_tabs, problem: None },
            MachineSnapshot { id: "m-mini", name: "Mac mini", online: true, version: "0.17.24", projects: &on_mini, tabs: &mini_tabs, problem: None },
        ];
        let waiting: Vec<_> = overview_at(&machines, 1794240000).waiting.iter().map(|w| serde_json::to_value(w).unwrap()).collect();
        assert_eq!(
            waiting,
            vec![
                serde_json::json!({ "machine": "m-book", "machine_name": "MacBook", "tab": "tab-b-9", "kind": "needs_input", "ask": "question", "detail": "Which one should open the PR?", "project": "/w/site", "title": "Pick the PR opener", "message": "Claude needs your input", "since": 1794239500 }),
                serde_json::json!({ "machine": "m-mini", "machine_name": "Mac mini", "tab": "tab-m-3", "kind": "needs_input", "ask": "approval", "tool": "Bash", "detail": "git push origin main", "project": "/Users/f/site", "title": "Push the fix", "message": "Claude needs your permission to use Bash", "since": 1794239000 }),
            ],
            "the worker on the Mac mini waits under that machine's name, with the command it asks; the one that works does not"
        );
    }

    #[test]
    fn a_permission_notice_is_an_approval_with_its_tool_and_any_other_a_question() {
        assert_eq!(ask_of(Some("Claude needs your permission to use Bash")), (AskKind::Approval, Some("Bash".into())));
        assert_eq!(ask_of(Some("Claude needs your permission to use mcp__pinned__act")), (AskKind::Approval, Some("mcp__pinned__act".into())));
        assert_eq!(ask_of(Some("Claude needs your permission")), (AskKind::Approval, None));
        assert_eq!(ask_of(Some("Claude needs your input")), (AskKind::Question, None));
        assert_eq!(ask_of(None), (AskKind::Question, None));
    }

    #[test]
    fn a_reply_asks_the_user_when_a_sentence_outside_code_ends_in_a_question_mark() {
        assert_eq!(
            question_in("O `-11` era o último dos quatro workers da revisão da PR 254.\n\nFalta só a sua decisão: coloco na revisão para o Leandro a ideia de consultar a e-Rede antes de cobrar de novo?"),
            Some("Falta só a sua decisão: coloco na revisão para o Leandro a ideia de consultar a e-Rede antes de cobrar de novo?".into()),
            "monorepo-photoshot's PR #254"
        );
        assert_eq!(
            question_in("**A produção ainda não tem a correção.** O hub continua na v0.17.19, então o clique segue quebrado até sair a v0.17.20. Quer que eu faça a release?\n\n**O PR #89 corrige o mesmo bug.** A segunda saída é fechar o #89."),
            None,
            "a question mid-reply, with more after it, is not what the turn ended on"
        );
        assert_eq!(
            question_in("Why did the CI fail? A missing semicolon in app.ts, now fixed. All 214 checks pass and the PR is merged, nothing left for you to do."),
            None,
            "a rhetorical opener answered in the same paragraph"
        );
        assert_eq!(question_in("Merged the fix.\n\nWant me to cut the release?\n\n"), Some("Want me to cut the release?".into()), "trailing blank lines");
        assert_eq!(question_in("Done. Want me to open the PR? Say yes and I will."), None, "a question the paragraph goes on from");
        assert_eq!(question_in("Mandei ao Orquestrador o caminho de cliques; ele já recebeu. Não mexi no código."), None, "the Captain's report asks nothing");
        assert_eq!(question_in("Use `a?.b`, see https://x.dev/search?q=1, then:\n```js\nconst ok = done ? 1 : 2; // why?\n```\nDone."), None, "question marks in code and links");
        assert_eq!(question_in("1. **Ship it now?**"), Some("Ship it now?".into()));
    }

    #[test]
    fn waiting_counts_exactly_the_cases_the_sidebar_shares() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("waiting_cases.json")).unwrap();
        let now = cases["now"].as_u64().unwrap();
        let tab_of = |case: &serde_json::Value| {
            let mut view = cases["base"].clone();
            view.as_object_mut().unwrap().extend(case["view"].as_object().unwrap().clone());
            serde_json::from_value::<TabView>(view).unwrap()
        };
        let verdict = |case: &serde_json::Value| {
            let tab = tab_of(case);
            let machines = [MachineSnapshot { id: "m-1", name: "MacBook", online: true, version: "0.17.21", projects: &[], tabs: std::slice::from_ref(&tab), problem: None }];
            let overview = overview_at(&machines, now);
            let waiting = overview.waiting.first();
            (case["case"].clone(), serde_json::to_value(waiting.map(|w| w.kind)).unwrap(), serde_json::to_value(waiting.and_then(|w| w.ask)).unwrap(), offer_of(&tab, now).is_some())
        };
        let cases_list = cases["cases"].as_array().unwrap();
        assert_eq!(cases_list.iter().map(verdict).collect::<Vec<_>>(), cases_list.iter().map(|c| (c["case"].clone(), c["waits"].clone(), c["ask"].clone(), c["offer"] == true)).collect::<Vec<_>>());
    }

    #[test]
    fn a_finished_turn_ending_in_a_question_is_not_blocked_but_a_pending_prompt_and_a_stopped_thread_are_told_apart() {
        let tab = |id: &str, running: bool, status, interrupted: bool, question: Option<&str>| TabView { interrupted, question: question.map(str::to_owned), message: Some("msg".into()), updated: Some(1794239000), ..view(id, "/w/site", None, running, Some(status)) };
        let book = [
            tab("tab-1-1", true, AgentStatus::Done, false, Some("Quer que eu abra o PR?")),
            tab("tab-1-2", true, AgentStatus::NeedsInput, true, None),
            tab("tab-1-3", false, AgentStatus::Done, false, Some("Quer que eu abra o PR?")),
        ];
        let machines = [MachineSnapshot { id: "m-1", name: "MacBook", online: true, version: "0.17.24", projects: &[], tabs: &book, problem: None }];
        let overview = overview_at(&machines, 1794240000);
        assert_eq!(overview.waiting.iter().map(|w| w.tab.as_str()).collect::<Vec<_>>(), ["tab-1-2"], "only the pending prompt blocks");
        let sessions = overview.sessions.unwrap();
        let by = |id: &str| sessions.iter().find(|s| s.id == id).map(|s| (s.state, s.message.as_deref())).unwrap();
        assert_eq!(by("tab-1-1"), (SessionState::Idle, Some("Quer que eu abra o PR?")), "the offer rides on the idle session");
        assert_eq!(by("tab-1-2"), (SessionState::WaitingOnInput, Some("msg")));
        assert_eq!(by("tab-1-3"), (SessionState::Settled, None), "a stopped thread lists nothing");
        let later = overview_at(&machines, 1794240000 + OFFER_SECS);
        assert_eq!(later.sessions.unwrap().iter().find(|s| s.id == "tab-1-1").unwrap().message, None, "24 hours drop the offer");
    }

    #[test]
    fn sessions_list_loose_threads_workers_and_capitaes_on_every_online_machine_and_account() {
        let apps = [project("/Users/f/client-apps", Some("github.com/o/client-apps"))];
        let talk = |tab: TabView, branch: &str, pr: Option<u64>, at| TabView { branch: Some(branch.into()), pr: pr.map(|n| PrLink { number: n, url: format!("https://github.com/o/client-apps/pull/{n}") }), last_message_at: Some(at), ..tab };
        let on = |account: &str, tab: TabView| TabView { account: account.into(), ..tab };
        let slack = talk(TabView { title: Some("Someone mentioned you in the Slack thread".into()), ..on("bloom", view("tab-b-2", "/Users/f/client-apps", None, true, Some(AgentStatus::Done))) }, "felipega/fix-and-ops", None, 1794240000);
        let asking = TabView { interrupted: true, message: Some("Claude needs your permission to use Bash".into()), updated: Some(1794239000), ..on("bloom", view("tab-b-3", "/Users/f/client-apps", None, true, Some(AgentStatus::NeedsInput))) };
        let new_thread = TabView { title: Some("New thread".into()), ..on("bloom", view("tab-b-4", "/Users/f/client-apps", None, true, None)) };
        let captain = talk(view("tab-b-5", "/Users/f/client-apps", Some(Role::Orchestrator), true, Some(AgentStatus::Working)), "main", None, 1794230000);
        let lead = TabView { spawned_by: Some("tab-b-5".into()), ..view("tab-b-6", "/Users/f/client-apps/.claude/worktrees/pinned-1", Some(Role::Worker), true, Some(AgentStatus::Working)) };
        let sub = talk(TabView { spawned_by: Some("tab-b-6".into()), ..view("tab-b-7", "/Users/f/client-apps/.claude/worktrees/pinned-2", Some(Role::Worker), true, Some(AgentStatus::Done)) }, "pinned/sub", Some(92), 1794235000);
        let own = TabView { orchestrator_global: true, ..view("tab-b-8", "/Users/f/.pinned/orquestrador", None, true, Some(AgentStatus::Working)) };
        let asked = TabView { question: Some("Should I open the PR now?".into()), message: Some("Claude is waiting for your input".into()), ..on("bloom", talk(view("tab-b-9", "/Users/f/client-apps", None, true, Some(AgentStatus::NeedsInput)), "felipega/fix-and-ops", None, 1794241000)) };
        let book = [slack, asking, new_thread, captain, lead, sub, own, asked];
        let old = TabView { prompt_title: Some("Someone mentioned you in the Slack thread".into()), message: Some("API Error: 500".into()), updated: Some(1793800000), ..view("tab-d-1", "/home/f/client-apps", None, false, Some(AgentStatus::Failed)) };
        let stopped = view("tab-d-2", "/home/f/scratch", None, false, Some(AgentStatus::Done));
        let dev = [old, stopped];
        let machines = [
            MachineSnapshot { id: "m-book", name: "MacBook", online: true, version: "0.17.21", projects: &apps, tabs: &book, problem: None },
            MachineSnapshot { id: "m-dev", name: "dev", online: true, version: "0.17.20", projects: &[], tabs: &dev, problem: None },
            MachineSnapshot { id: "m-off", name: "Mac mini", online: false, version: "0.17.21", projects: &apps, tabs: &dev, problem: None },
        ];
        let overview = overview(&machines);
        let listed = overview.sessions.clone().unwrap();
        let apps_on_book = serde_json::json!({ "key": "github.com/o/client-apps", "name": "client-apps", "path": "/Users/f/client-apps" });
        assert_eq!(
            serde_json::to_value(&listed).unwrap(),
            serde_json::json!([
                { "id": "tab-b-9", "machine": "m-book", "machine_name": "MacBook", "account": "bloom", "project": apps_on_book, "role": "thread", "branch": "felipega/fix-and-ops", "state": "idle", "message": "Should I open the PR now?", "last_activity": 1794241000 },
                { "id": "tab-b-2", "machine": "m-book", "machine_name": "MacBook", "account": "bloom", "project": apps_on_book, "role": "thread", "title": "Someone mentioned you in the Slack thread", "branch": "felipega/fix-and-ops", "state": "idle", "last_activity": 1794240000 },
                { "id": "tab-b-3", "machine": "m-book", "machine_name": "MacBook", "account": "bloom", "project": apps_on_book, "role": "thread", "state": "waiting_on_input", "message": "Claude needs your permission to use Bash", "last_activity": 1794239000 },
                { "id": "tab-b-7", "machine": "m-book", "machine_name": "MacBook", "account": "claude", "project": apps_on_book, "role": "worker", "captain": "tab-b-5", "branch": "pinned/sub", "pr": { "number": 92, "url": "https://github.com/o/client-apps/pull/92" }, "state": "idle", "last_activity": 1794235000 },
                { "id": "tab-b-5", "machine": "m-book", "machine_name": "MacBook", "account": "claude", "project": apps_on_book, "role": "captain", "branch": "main", "state": "running", "last_activity": 1794230000 },
                { "id": "tab-d-1", "machine": "m-dev", "machine_name": "dev", "account": "claude", "project": { "key": "m-dev:/home/f/client-apps", "name": "client-apps", "path": "/home/f/client-apps" }, "role": "thread", "title": "Someone mentioned you in the Slack thread", "state": "error", "message": "API Error: 500", "last_activity": 1793800000 },
                { "id": "tab-b-4", "machine": "m-book", "machine_name": "MacBook", "account": "bloom", "project": apps_on_book, "role": "thread", "title": "New thread", "state": "idle" },
                { "id": "tab-b-6", "machine": "m-book", "machine_name": "MacBook", "account": "claude", "project": apps_on_book, "role": "worker", "captain": "tab-b-5", "state": "running" },
                { "id": "tab-d-2", "machine": "m-dev", "machine_name": "dev", "account": "claude", "project": { "key": "m-dev:/home/f/scratch", "name": "scratch", "path": "/home/f/scratch" }, "role": "thread", "state": "settled" },
            ])
        );
        let ids = |project: Option<&str>, machine: Option<&str>| listed.iter().filter(|s| s.matches(project, machine)).map(|s| s.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids(Some("Client-Apps"), None), ["tab-b-9", "tab-b-2", "tab-b-3", "tab-b-7", "tab-b-5", "tab-d-1", "tab-b-4", "tab-b-6"], "a name matches the project on every machine");
        assert_eq!(ids(Some("github.com/o/client-apps"), Some("@macbook")), ["tab-b-9", "tab-b-2", "tab-b-3", "tab-b-7", "tab-b-5", "tab-b-4", "tab-b-6"]);
        assert_eq!(ids(Some("m-dev:/home/f/client-apps"), None), ["tab-d-1"]);
        assert_eq!(ids(None, Some("m-dev")), ["tab-d-1", "tab-d-2"]);
        let waiting: Vec<(&str, WaitingKind, Option<&str>)> = overview.waiting.iter().map(|w| (w.tab.as_str(), w.kind, w.message.as_deref())).collect();
        assert_eq!(
            waiting,
            [
                ("tab-b-3", WaitingKind::NeedsInput, Some("Claude needs your permission to use Bash")),
                ("tab-d-1", WaitingKind::Failed, Some("API Error: 500")),
            ],
            "what waits is the sessions waiting on input or stopped on an error; a loose thread that asked is an offer, not a wait"
        );
        assert_eq!(overview.waiting[1].title.as_deref(), Some("Someone mentioned you in the Slack thread"), "a restored tab's first prompt names it");
    }

    #[test]
    fn a_session_waits_on_input_or_is_in_error_exactly_when_it_waits_on_the_user() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("waiting_cases.json")).unwrap();
        for case in cases["cases"].as_array().unwrap().iter().filter(|c| c["view"]["orchestrator_global"] != true) {
            let mut view = cases["base"].clone();
            view.as_object_mut().unwrap().extend(case["view"].as_object().unwrap().clone());
            let state = SessionState::of(&serde_json::from_value(view).unwrap());
            let expected = match case["waits"].as_str() {
                Some("needs_input") => Some(SessionState::WaitingOnInput),
                Some("failed") => Some(SessionState::Error),
                _ => None,
            };
            assert_eq!(Some(state).filter(|s| matches!(s, SessionState::WaitingOnInput | SessionState::Error)), expected, "{}", case["case"]);
        }
    }

    #[test]
    fn the_capitao_hears_how_its_turn_goes_back_to_the_orchestrator_under_each_policy() {
        assert_eq!(captain_prompt("Almirante", ReportPolicy::Auto), "A message that starts with \"[Almirante] \" comes from the global orchestrator, the user's chat above every project. Do that work as you would the user's. Mid-work, use tell_orchestrator for a question it must answer or news it must hear now. Your last reply of such a turn goes back to it by itself, so end that turn with what you did, what is left and what needs the user.");
        assert!(captain_prompt("Almirante", ReportPolicy::ToolOnly).ends_with("Nothing goes back to it by itself: when you finish its request, report with tell_orchestrator."));
        assert!(prompt("Comandante", "mini").starts_with("Você é o Comandante, o orquestrador global do Felipe no pinned"));
        assert!(prompt("Comandante", "mini").contains("such as \"what is running in client-apps?\", call sessions(project) or sessions(machine=...)."));
    }

    #[test]
    fn the_ledger_moves_a_queued_message_in_flight_then_answered_and_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(Some(dir.path()));
        let turns = ledger.turns();
        let in_turn = || turns.lock().unwrap().iter().cloned().collect::<Vec<PathBuf>>();
        let sent = |id: &str, project: &str, handoff, progress| Sent { request_id: id.into(), sender: "Orquestrador".into(), project: project.into(), handoff, progress, at: 1 };
        ledger.record(sent("r1", "/p", Handoff::Queued { tab: "t".into(), position: 1 }, Progress::Queued));
        ledger.record(sent("r0", "/q", Handoff::Queued { tab: "u".into(), position: 1 }, Progress::Queued));
        assert_eq!(in_turn(), Vec::<PathBuf>::new(), "a queued message starts no turn");
        ledger.record(sent("r2", "/p", Handoff::Delivered { tab: "t".into() }, Progress::InFlight));
        assert_eq!(in_turn(), [PathBuf::from("/p")]);
        assert_eq!(ledger.queued_on("t"), 1);
        assert_eq!((ledger.typed(&["r1".into()]), ledger.typed(&["r1".into()])), (true, false));
        let ledger = Ledger::load(Some(dir.path()));
        assert_eq!(ledger.in_flight(Path::new("/p")).iter().map(|s| s.request_id.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);
        let turns = ledger.turns();
        assert_eq!(turns.lock().unwrap().iter().cloned().collect::<Vec<PathBuf>>(), [PathBuf::from("/p")], "a reload knows the turn in flight");
        let mut ledger = ledger;
        assert_eq!(ledger.answer(Path::new("/p")).len(), 2);
        assert!(turns.lock().unwrap().is_empty(), "the answered turn is over");
        assert_eq!(ledger.answer(Path::new("/p")).len(), 0, "an answered message reports once");
        assert_eq!(ledger.find("r1").map(|s| s.progress), Some(Progress::Answered));
    }
}
