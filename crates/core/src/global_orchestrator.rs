//! The global orchestrator: one chat above every project of the account. It hands each request to
//! the project's Capitão, on whatever machine has it, and sums up what the Capitães report. Its tab
//! runs in `~/.pinned/orquestrador`, a folder no project list holds, so that folder's memory is the
//! account's global memory.

use crate::agent::{AgentStatus, Role, ROLE_ENV};
use crate::history;
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
    let mut projects: Vec<OverviewProject> = vec![];
    let mut waiting = vec![];
    for machine in machines {
        let tabs = if machine.online { machine.tabs } else { &[] };
        for project in machine.projects {
            let captain = captain_tab(tabs, &project.path);
            let workers = tabs.iter().filter(|t| (t.running || t.status == Some(AgentStatus::Working)) && t.role == Some(Role::Worker) && history::project_of(Path::new(&t.cwd)) == Path::new(&project.path)).count();
            let checkout = OverviewCheckout {
                machine: machine.id.to_owned(),
                machine_name: machine.name.to_owned(),
                online: machine.online,
                version: machine.version.to_owned(),
                path: project.path.clone(),
                captain: CaptainState::of(captain),
                captain_tab: captain.map(|t| t.id.clone()),
                workers: workers as u32,
            };
            let key = project.repo.clone().unwrap_or_else(|| format!("{}:{}", machine.id, project.path));
            match projects.iter_mut().find(|p| p.key == key) {
                Some(listed) => listed.checkouts.push(checkout),
                None => projects.push(OverviewProject { key, name: project.name.clone(), repo: project.repo.clone(), checkouts: vec![checkout] }),
            }
        }
        for tab in tabs.iter().filter(|t| !t.orchestrator_global) {
            let kind = match tab.status {
                Some(AgentStatus::NeedsInput) if tab.running && tab.interrupted => WaitingKind::NeedsInput,
                Some(AgentStatus::Failed) => WaitingKind::Failed,
                _ => continue,
            };
            let folder = history::project_of(Path::new(&tab.cwd));
            waiting.push(Waiting {
                machine: machine.id.to_owned(),
                machine_name: machine.name.to_owned(),
                tab: tab.id.clone(),
                kind,
                project: machine.projects.iter().find(|p| Path::new(&p.path) == folder).map(|p| p.path.clone()),
                title: tab.title.clone(),
                message: tab.message.clone(),
                since: tab.updated,
            });
        }
    }
    let machines = machines
        .iter()
        .map(|m| OverviewMachine { id: m.id.to_owned(), name: m.name.to_owned(), online: m.online, version: m.version.to_owned(), problem: m.problem.map(str::to_owned) })
        .collect();
    Overview { projects, waiting, machines }
}

/// The global orchestrator's system prompt. `name` is its configured name, `machine` the machine it
/// runs on.
pub fn prompt(name: &str, machine: &str) -> String {
    format!(
        "Você é o {name}, o orquestrador global do Felipe no pinned: o chat acima de todos os projetos dele, em todas as máquinas. Você roda na máquina {machine}, numa pasta que não é projeto.

Você nunca faz trabalho de projeto: não lê nem edita arquivos de projeto, não roda comandos neles e não inicia workers. Todo pedido de projeto vai para o Capitão daquele projeto com send_to_captain. O Capitão é a sessão que lidera o projeto; ele faz o trabalho e te reporta.

Para cada pedido:
1. Chame projects() primeiro e escolha o projeto pelo texto do Felipe. Use a key que projects() lista como project.
2. #projeto no texto força o projeto, e @máquina força a máquina. Cada marca vale só para o parágrafo em que aparece: um texto com parágrafos para projetos diferentes vira uma mensagem para cada Capitão.
3. Quando o mesmo repo está em várias máquinas, prefira a máquina com um Capitão rodando (livre ou trabalhando), depois a sua própria máquina ({machine}), depois a primeira online, e passe machine em send_to_captain. Se ainda houver dúvida, pergunte ao Felipe antes de enviar qualquer coisa.
4. Use um request_id novo em cada mensagem. Repita um request_id só para reenviar a mesma mensagem depois de um erro.
5. send_to_captain responde entregue (digitado agora), na_fila (o Capitão está ocupado e lê quando terminar), iniciado (o Capitão começou com a mensagem) ou ocupado (tente de novo mais tarde). Conte ao Felipe em uma frase.

Os relatórios dos Capitães chegam como mensagens que começam com \"[pinned] Capitão <projeto> (<máquina>)\". Resuma cada um para o Felipe em poucas linhas: o que foi feito, o que falta e o que precisa dele. read_captain(project) lê as últimas respostas de um Capitão, e waiting() lista o que espera o Felipe em todas as máquinas.

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
        ProjectView { path: path.into(), name: path.rsplit('/').next().unwrap().into(), repo: repo.map(Into::into), account: "default".into() }
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
            serde_json::to_value(overview(&machines)).unwrap(),
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
    fn a_folder_without_an_origin_is_keyed_by_its_machine_and_an_offline_machine_has_no_capitao_or_waiting() {
        let notes = [project("/w/notes", None), project("/w/site", Some("github.com/o/site"))];
        let tabs = [view("tab-b-1", "/w/notes", Some(Role::Orchestrator), true, Some(AgentStatus::Done)), view("tab-b-2", "/w/site", None, true, Some(AgentStatus::Failed))];
        let machines = [MachineSnapshot { id: "m-1", name: "Linux box", online: false, version: "0.17.10", projects: &notes, tabs: &tabs, problem: None }];
        assert_eq!(
            serde_json::to_value(overview(&machines)).unwrap(),
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
        let question = TabView { interrupted: true, title: Some("Fix the form".into()), message: Some("Claude needs your permission to use Bash".into()), updated: Some(1794234244), ..view("tab-c-2", "/w/site/.claude/worktrees/pinned-1", Some(Role::Worker), true, Some(AgentStatus::NeedsInput)) };
        let idle_notice = TabView { interrupted: false, message: Some("Claude is waiting for your input".into()), ..view("tab-c-1", "/w/site", Some(Role::Orchestrator), true, Some(AgentStatus::NeedsInput)) };
        let failed = TabView { message: Some("API Error: 500".into()), updated: Some(1794234300), ..view("tab-c-3", "/tmp/scratch", None, false, Some(AgentStatus::Failed)) };
        let own = TabView { orchestrator_global: true, interrupted: true, ..view("tab-c-4", "/home/f/.pinned/orquestrador", None, true, Some(AgentStatus::NeedsInput)) };
        let tabs = [idle_notice, question, failed, own];
        let machines = [MachineSnapshot { id: "m-1", name: "MacBook", online: true, version: "0.17.19", projects: &site, tabs: &tabs, problem: None }];
        assert_eq!(
            serde_json::to_value(overview(&machines)).unwrap(),
            serde_json::json!({
                "projects": [{ "key": "github.com/o/site", "name": "site", "repo": "github.com/o/site", "checkouts": [{ "machine": "m-1", "machine_name": "MacBook", "online": true, "version": "0.17.19", "path": "/w/site", "captain": "livre", "captain_tab": "tab-c-1", "workers": 1 }] }],
                "waiting": [
                    { "machine": "m-1", "machine_name": "MacBook", "tab": "tab-c-2", "kind": "needs_input", "project": "/w/site", "title": "Fix the form", "message": "Claude needs your permission to use Bash", "since": 1794234244 },
                    { "machine": "m-1", "machine_name": "MacBook", "tab": "tab-c-3", "kind": "failed", "message": "API Error: 500", "since": 1794234300 },
                ],
                "machines": [{ "id": "m-1", "name": "MacBook", "online": true, "version": "0.17.19" }],
            })
        );
    }

    #[test]
    fn the_capitao_hears_how_its_turn_goes_back_to_the_orchestrator_under_each_policy() {
        assert_eq!(captain_prompt("Almirante", ReportPolicy::Auto), "A message that starts with \"[Almirante] \" comes from the global orchestrator, the user's chat above every project. Do that work as you would the user's. Mid-work, use tell_orchestrator for a question it must answer or news it must hear now. Your last reply of such a turn goes back to it by itself, so end that turn with what you did, what is left and what needs the user.");
        assert!(captain_prompt("Almirante", ReportPolicy::ToolOnly).ends_with("Nothing goes back to it by itself: when you finish its request, report with tell_orchestrator."));
        assert!(prompt("Comandante", "mini").starts_with("Você é o Comandante, o orquestrador global do Felipe no pinned"));
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
