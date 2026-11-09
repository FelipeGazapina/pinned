//! The global orchestrator's bridge between machines. Its tab lists every project and session of
//! the account, hands a message to a Capitão on another machine and reads a Capitão or any session
//! there, and a Capitão reports up to it. A machine holds a lesser credential than a login, so the hub checks where each call comes
//! from: the overview, a message for a Capitão and a read only from the machine that hosts the
//! global orchestrator, and a report only from a machine whose tab `report.tab` is the Capitão of
//! `report.path`.

use crate::hub::{Account, Hub, Unanswered};
use futures_util::future::join_all;
use pinned_core::agent::Role;
use pinned_core::global_orchestrator::{self as global, MachineSnapshot, RelayTo};
use pinned_core::link::is_unknown_command;
use pinned_core::reading;
use pinned_core::wire::{Call, HubCall, MachineId, ProjectView, TabView};
use serde_json::{json, Value};
use std::time::Duration;

/// Each machine answers whether it hosts from its in-order call queue, behind whatever it was doing.
const HOST_WAIT: Duration = Duration::from_secs(10);
const RELAY_WAIT: Duration = Duration::from_secs(30);
/// The overview lists what each machine answered by then, and the rest as it last listed them.
const LOOK_WAIT: Duration = Duration::from_secs(5);
const TOO_OLD: &str = "precisa atualizar o pinned";

pub enum Source<'a> {
    /// A signed-in client, which may call any machine of the account anyway.
    Client,
    /// The machine whose socket sent the call.
    Machine(&'a str),
}

/// Answers the global orchestrator's hub calls: `Relay`, `Overview`, `ReadCaptain` and `ReadSession`.
pub async fn serve(hub: &Hub, account: &Account, from: Source<'_>, call: HubCall) -> Result<Value, String> {
    match call {
        HubCall::Relay { request_id, to } => relay(hub, account, from, request_id, to).await,
        HubCall::Overview => overview(hub, account, from).await,
        HubCall::ReadCaptain { machine, path } => read_captain(hub, account, from, &machine, path).await,
        HubCall::ReadSession { tab, machine, items } => read_session(hub, account, from, tab, machine, items).await,
        _ => Err("that is not a global orchestrator call".into()),
    }
}

async fn relay(hub: &Hub, account: &Account, from: Source<'_>, request_id: String, to: RelayTo) -> Result<Value, String> {
    match to {
        RelayTo::Captain { machine, path, text, sender, busy } => {
            only_host(hub, account, &from, "send to a Capitão").await?;
            let _held = hub.hold_captain(account, &machine, &path).await;
            call(hub, account, &machine, Call::Enqueue { path, text, request_id, sender, busy }).await
        }
        RelayTo::Orchestrator { mut report } => {
            let from = match from {
                Source::Machine(from) => from.to_owned(),
                Source::Client => report.machine.clone(),
            };
            let name = account.name_of(&from).ok_or_else(|| format!("machine {from} is offline"))?;
            let captain = account.tab(&from, &report.tab).is_some_and(|t| t.role == Some(Role::Orchestrator) && t.cwd == report.path);
            if !captain {
                return Err(format!("tab {} on {name} is not the Capitão of {}, so it can't report to the global orchestrator", report.tab, report.path));
            }
            (report.machine, report.machine_name) = (from, name);
            let (host, _) = host(hub, account).await?;
            call(hub, account, &host, Call::OrchestratorReport { request_id, report }).await
        }
    }
}

/// Every project of the account and what waits on the user, for the global orchestrator's
/// `projects` and `waiting`. Tabs are what each machine last reported. Each online machine is asked
/// at once for its projects and for its global orchestrator settings: a machine too old to answer
/// those needs an update, and one that does not answer in time shows the projects it listed last.
/// An offline machine shows those too, with no Capitão.
async fn overview(hub: &Hub, account: &Account, from: Source<'_>) -> Result<Value, String> {
    let machines = hub.machines(account).await?;
    let asker = match from {
        Source::Machine(from) => Some(from),
        Source::Client => None,
    };
    let looks = join_all(machines.iter().map(|m| async move {
        // The asking machine's own settings decide whether it may ask, so they get the host's wait.
        let settings_wait = if asker == Some(m.id.as_str()) { HOST_WAIT } else { LOOK_WAIT };
        match m.online {
            true => Some(look(hub, account, &m.id, settings_wait).await),
            false => None,
        }
    }))
    .await;
    if let Source::Machine(from) = from {
        let settings = machines.iter().zip(&looks).filter_map(|(m, look)| Some((m.id.as_str(), m.name.as_str(), &look.as_ref()?.settings)));
        let (host, host_name) = the_host(settings)?;
        if host != from {
            return Err(not_host(account, from, &host_name, "list the account's projects"));
        }
    }
    let seen: Vec<(Vec<ProjectView>, Vec<TabView>, Option<String>)> = machines
        .iter()
        .zip(looks)
        .map(|(m, look)| {
            let Some(look) = look else { return (account.last_listed(&m.id), vec![], None) };
            let problem = look.problem();
            let projects = match look.projects {
                Ok(projects) => {
                    account.listed(&m.id, projects.clone());
                    projects
                }
                Err(_) => account.last_listed(&m.id),
            };
            (projects, look.tabs.unwrap_or_default(), problem)
        })
        .collect();
    let snapshots: Vec<MachineSnapshot> = machines
        .iter()
        .zip(&seen)
        .map(|(m, (projects, tabs, problem))| MachineSnapshot { id: &m.id, name: &m.name, online: m.online, version: &m.version, projects, tabs, problem: problem.as_deref() })
        .collect();
    let overview = global::Overview { sessions: Some(global::sessions(&snapshots)), ..global::overview(&snapshots) };
    serde_json::to_value(overview).map_err(|e| e.to_string())
}

/// What one online machine told the overview.
struct Look {
    projects: Result<Vec<ProjectView>, Unanswered>,
    settings: Result<Value, Unanswered>,
    tabs: Result<Vec<TabView>, Unanswered>,
}

impl Look {
    /// What the overview says about a machine whose answers fell short. A machine whose settings
    /// alone did not come in lists fine: those only decide who hosts.
    fn problem(&self) -> Option<String> {
        let errors = [self.projects.as_ref().err(), self.settings.as_ref().err(), self.tabs.as_ref().err()];
        if errors.iter().flatten().any(|e| matches!(e, Unanswered::Refused(e) if is_unknown_command(e))) {
            return Some(TOO_OLD.into());
        }
        match self.projects.as_ref().err().or(self.tabs.as_ref().err())? {
            Unanswered::Lost(_) => Some("não respondeu".into()),
            Unanswered::Refused(e) => Some(e.clone()),
        }
    }
}

async fn look(hub: &Hub, account: &Account, machine: &str, settings_wait: Duration) -> Look {
    let (projects, settings, tabs) = tokio::join!(
        hub.call_machine(account, machine, Call::ListProjects, LOOK_WAIT),
        hub.call_machine(account, machine, Call::GlobalOrchestratorSettings, settings_wait),
        tabs(hub, account, machine, LOOK_WAIT)
    );
    let projects = projects.and_then(|listed| serde_json::from_value(listed).map_err(|e| Unanswered::Refused(e.to_string())));
    Look { projects, settings, tabs }
}

/// The latest replies of the Capitão of the project at `path` on `machine`, as the node's own
/// `read_captain` answers them.
async fn read_captain(hub: &Hub, account: &Account, from: Source<'_>, machine: &str, path: String) -> Result<Value, String> {
    only_host(hub, account, &from, "read a Capitão").await?;
    let mut out = if !account.is_online(machine) {
        if !hub.machines(account).await?.iter().any(|m| m.id == machine) {
            return Err(format!("this account has no machine {machine}"));
        }
        json!({ "state": "offline", "next": reading::next_for("offline") })
    } else {
        let tabs = tabs(hub, account, machine, RELAY_WAIT).await?;
        match global::captain_tab(&tabs, &path) {
            Some(tab) if tab.running || tab.session.is_some() => {
                let page = hub.call_machine(account, machine, Call::Thread { tab: tab.id.clone(), from: 0 }, RELAY_WAIT).await;
                reading::read(tab, page.map_err(String::from), reading::DEFAULT_ITEMS)
            }
            _ => json!({ "state": "not_started", "next": reading::next_for("not_started") }),
        }
    };
    out["project"] = json!(path);
    Ok(out)
}

/// The last `items` items of the session in tab `tab`, as the node's own `read_session` answers
/// them: on `machine`, an id or a name, else on the online machine whose tabs hold it. A node too
/// old for `Call::ReadSession` is read from the start of the transcript, as `read_captain` reads.
async fn read_session(hub: &Hub, account: &Account, from: Source<'_>, tab: String, machine: Option<String>, items: Option<usize>) -> Result<Value, String> {
    only_host(hub, account, &from, "read a session").await?;
    let machines = hub.machines(account).await?;
    let named = match &machine {
        Some(m) => Some(machines.iter().find(|known| known.id == *m || known.name.eq_ignore_ascii_case(m)).ok_or_else(|| format!("this account has no machine {m}"))?),
        None => None,
    };
    let mut holders = vec![];
    for m in machines.iter().filter(|m| m.online && named.is_none_or(|n| n.id == m.id)) {
        if let Some(view) = tabs(hub, account, &m.id, LOOK_WAIT).await.ok().and_then(|tabs| tabs.into_iter().find(|t| t.id == tab)) {
            holders.push((m, view));
        }
    }
    let (m, view) = match (holders.len(), named) {
        (1, _) => holders.remove(0),
        (0, Some(m)) if !m.online => return Ok(json!({ "id": tab, "machine": m.id, "machine_name": m.name, "state": "offline", "next": reading::next_for("offline") })),
        (0, Some(m)) => return Err(format!("{} has no session {tab}", m.name)),
        (0, None) => return Err(format!("no online machine of this account has a session {tab}")),
        _ => return Err(format!("{} each have a session {tab}: pass machine", holders.iter().map(|(m, _)| m.name.as_str()).collect::<Vec<_>>().join(" and "))),
    };
    let mut out = match hub.call_machine(account, &m.id, Call::ReadSession { tab: tab.clone(), items }, RELAY_WAIT).await {
        Err(Unanswered::Refused(e)) if is_unknown_command(&e) => {
            let page = hub.call_machine(account, &m.id, Call::Thread { tab: tab.clone(), from: 0 }, RELAY_WAIT).await;
            reading::read(&view, page.map_err(String::from), items.unwrap_or(reading::DEFAULT_ITEMS))
        }
        answer => answer.map_err(String::from)?,
    };
    (out["id"], out["machine"], out["machine_name"]) = (json!(tab), json!(m.id), json!(m.name));
    Ok(out)
}

/// The tabs `machine` last reported, or its answer when it has not reported any yet.
async fn tabs(hub: &Hub, account: &Account, machine: &str, wait: Duration) -> Result<Vec<TabView>, Unanswered> {
    if let Some(tabs) = account.tabs_of(machine) {
        return Ok(tabs);
    }
    let tabs = hub.call_machine(account, machine, Call::Tabs, wait).await?;
    serde_json::from_value(tabs).map_err(|e| Unanswered::Refused(e.to_string()))
}

/// Refuses a machine that does not host the global orchestrator. A signed-in client may anyway.
async fn only_host(hub: &Hub, account: &Account, from: &Source<'_>, what: &str) -> Result<(), String> {
    let Source::Machine(from) = from else { return Ok(()) };
    let (host, host_name) = host(hub, account).await?;
    match host == *from {
        true => Ok(()),
        false => Err(not_host(account, from, &host_name, what)),
    }
}

fn not_host(account: &Account, from: &str, host_name: &str, what: &str) -> String {
    let name = account.name_of(from).unwrap_or_else(|| from.to_owned());
    format!("{name} does not host the global orchestrator ({host_name} does), so it can't {what}")
}

/// The one online machine whose settings say it hosts the global orchestrator. It asks each time
/// rather than caching, because a machine's settings also change in its own app, which the hub
/// never sees.
async fn host(hub: &Hub, account: &Account) -> Result<(MachineId, String), String> {
    let online = account.online();
    let answers = join_all(online.iter().map(|(id, _)| hub.call_machine(account, id, Call::GlobalOrchestratorSettings, HOST_WAIT))).await;
    the_host(online.iter().zip(&answers).map(|((id, name), answer)| (id.as_str(), name.as_str(), answer)))
}

/// The host among each online machine's answer to `Call::GlobalOrchestratorSettings`.
fn the_host<'a>(answers: impl Iterator<Item = (&'a str, &'a str, &'a Result<Value, Unanswered>)>) -> Result<(MachineId, String), String> {
    let mut hosts = vec![];
    let mut silent = vec![];
    for (id, name, answer) in answers {
        match answer {
            Ok(settings) if settings["host"] == true => hosts.push((id.to_owned(), name.to_owned())),
            Ok(_) => {}
            // An older machine can't host it.
            Err(Unanswered::Refused(e)) if is_unknown_command(e) => {}
            Err(_) => silent.push(name),
        }
    }
    silent.sort();
    match hosts.len() {
        1 => Ok(hosts.remove(0)),
        0 if silent.is_empty() => Err("no online machine hosts the global orchestrator: choose one in its settings".into()),
        0 => Err(format!("no online machine hosts the global orchestrator, and {} did not say whether it does", silent.join(", "))),
        _ => {
            let mut names: Vec<String> = hosts.into_iter().map(|(_, name)| name).collect();
            names.sort();
            Err(format!("{} each host the global orchestrator: keep it on one machine in its settings", names.join(" and ")))
        }
    }
}

async fn call(hub: &Hub, account: &Account, machine: &str, call: Call) -> Result<Value, String> {
    match hub.call_machine(account, machine, call, RELAY_WAIT).await {
        Ok(answer) => Ok(answer),
        Err(Unanswered::Refused(e)) if is_unknown_command(&e) => Err(format!("{} {TOO_OLD}", account.name_of(machine).unwrap_or_else(|| machine.to_owned()))),
        Err(Unanswered::Refused(e)) => Err(e),
        // The machine keeps the request id, so a retry answers what it did instead of doing it twice.
        Err(Unanswered::Lost(e)) => Err(format!("{e}; try again with the same request_id")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_shows_why_the_overview_lacks_its_projects_or_its_tabs() {
        let problem = |projects: Result<Vec<ProjectView>, Unanswered>, settings: Result<Value, Unanswered>, tabs: Result<Vec<TabView>, Unanswered>| Look { projects, settings, tabs }.problem();
        let lost = || Unanswered::Lost("mini did not answer".into());
        let old = || Unanswered::Refused("this machine runs pinned 0.17.0, which can't answer that: unknown variant `tabs`".into());
        assert_eq!(problem(Ok(vec![]), Ok(json!({})), Ok(vec![])), None);
        assert_eq!(problem(Ok(vec![]), Err(lost()), Ok(vec![])), None, "settings only decide who hosts");
        assert_eq!(problem(Ok(vec![]), Ok(json!({})), Err(lost())), Some("não respondeu".into()), "tabs that never came are not an idle machine");
        assert_eq!(problem(Err(lost()), Ok(json!({})), Ok(vec![])), Some("não respondeu".into()));
        assert_eq!(problem(Err(Unanswered::Refused("no disk".into())), Ok(json!({})), Err(lost())), Some("no disk".into()));
        assert_eq!(problem(Ok(vec![]), Ok(json!({})), Err(old())), Some("precisa atualizar o pinned".into()));
        assert_eq!(problem(Err(lost()), Err(old()), Ok(vec![])), Some("precisa atualizar o pinned".into()));
    }
}
