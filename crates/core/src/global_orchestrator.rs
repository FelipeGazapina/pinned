//! The global orchestrator: one chat above every project of the account. It hands each request to
//! the project's Capitão, on whatever machine has it, and sums up what the Capitães report. Its tab
//! runs in `~/.pinned/orquestrador`, a folder no project list holds, so that folder's memory is the
//! account's global memory.

use crate::agent::{AgentStatus, ROLE_ENV};
use crate::wire::{MachineId, TabId, TabView};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `PINNED_ROLE` of the global orchestrator's tab. It is not a `Role`, so `TabView.role` never
/// carries it to an older app.
pub const ROLE: &str = "global-orchestrator";
/// The tab's folder, under the home folder.
pub const FOLDER: [&str; 2] = [".pinned", "orquestrador"];
pub const DEFAULT_NAME: &str = "Orquestrador";
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
    /// The chat's name, and the prefix of every message it hands a Capitão, as in `[Orquestrador] `.
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

/// Every message handed on this machine, newest last, saved with each change.
pub struct Ledger {
    file: Option<PathBuf>,
    sent: Vec<Sent>,
}

impl Ledger {
    pub fn load(dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join(LEDGER_FILE));
        let sent = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Ledger { file, sent }
    }

    pub fn find(&self, request_id: &str) -> Option<&Sent> {
        self.sent.iter().find(|s| s.request_id == request_id)
    }

    pub fn record(&mut self, sent: Sent) {
        self.sent.push(sent);
        if self.sent.len() > LEDGER_KEEP {
            self.sent.drain(..self.sent.len() - LEDGER_KEEP);
        }
        self.save();
    }

    /// The queued messages with these request ids were typed: the turn they start is in flight.
    pub fn typed(&mut self, request_ids: &[String]) {
        let mut changed = false;
        for sent in self.sent.iter_mut().filter(|s| s.progress == Progress::Queued && request_ids.contains(&s.request_id)) {
            sent.progress = Progress::InFlight;
            changed = true;
        }
        if changed {
            self.save();
        }
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
            self.save();
        }
        answered
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
}

/// Whether a message to the project's Capitão would be typed now.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptainState {
    /// Idle, finished, failed or stopped: a message is typed now or starts it.
    #[serde(rename = "livre")]
    Idle,
    /// Working or waiting on the user: a message queues.
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
            Some(tab) if tab.running && matches!(tab.status, Some(AgentStatus::Working | AgentStatus::NeedsInput)) => CaptainState::Working,
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
        assert_eq!(serde_json::to_value(Settings::default()).unwrap(), serde_json::json!({ "name": "Orquestrador", "host": false, "busy": "queue", "reports": "auto", "group_reports": true, "read_aloud": false }));
        let read: Settings = serde_json::from_str(r#"{"name":"Almirante","busy":"refuse","reports":"tool_only"}"#).unwrap();
        assert_eq!(read, Settings { name: "Almirante".into(), busy: BusyPolicy::Refuse, reports: ReportPolicy::ToolOnly, ..Settings::default() });
        assert_eq!(Settings { name: "  [Comandante]\n ".into(), ..Settings::default() }.cleaned().name, "Comandante");
        assert_eq!(Settings { name: " ".into(), ..Settings::default() }.cleaned().name, "Orquestrador");
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
    fn a_captain_is_working_only_while_its_session_runs_a_turn_or_asks() {
        let tab = |running, status| TabView { running, status, ..serde_json::from_value(serde_json::json!({ "id": "t", "cwd": "/p", "bypass": false, "running": false, "status": null, "title": null, "message": null, "resume_at": null, "connector": null, "opened": 1 })).unwrap() };
        assert_eq!(CaptainState::of(None), CaptainState::Missing);
        assert_eq!(CaptainState::of(Some(&tab(true, Some(AgentStatus::Working)))), CaptainState::Working);
        assert_eq!(CaptainState::of(Some(&tab(true, Some(AgentStatus::NeedsInput)))), CaptainState::Working);
        assert_eq!(CaptainState::of(Some(&tab(true, Some(AgentStatus::Done)))), CaptainState::Idle);
        assert_eq!(CaptainState::of(Some(&tab(false, Some(AgentStatus::Working)))), CaptainState::Idle, "a stopped Capitão starts on the next message");
        assert_eq!(serde_json::to_value([CaptainState::Idle, CaptainState::Working, CaptainState::Missing]).unwrap(), serde_json::json!(["livre", "trabalhando", "nenhum"]));
    }

    #[test]
    fn the_ledger_moves_a_queued_message_in_flight_then_answered_and_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(Some(dir.path()));
        let sent = |id: &str, handoff, progress| Sent { request_id: id.into(), sender: "Orquestrador".into(), project: "/p".into(), handoff, progress, at: 1 };
        ledger.record(sent("r1", Handoff::Queued { tab: "t".into(), position: 1 }, Progress::Queued));
        ledger.record(sent("r2", Handoff::Delivered { tab: "t".into() }, Progress::InFlight));
        assert_eq!(ledger.queued_on("t"), 1);
        ledger.typed(&["r1".into()]);
        let ledger = Ledger::load(Some(dir.path()));
        assert_eq!(ledger.in_flight(Path::new("/p")).iter().map(|s| s.request_id.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);
        let mut ledger = ledger;
        assert_eq!(ledger.answer(Path::new("/p")).len(), 2);
        assert_eq!(ledger.answer(Path::new("/p")).len(), 0, "an answered message reports once");
        assert_eq!(ledger.find("r1").map(|s| s.progress), Some(Progress::Answered));
    }
}
