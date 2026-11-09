//! The global orchestrator's bridge between machines. Its tab hands a message to a Capitão on
//! another machine, and a Capitão reports up to it, through `HubCall::Relay`. A machine holds a
//! lesser credential than a login, so the hub checks where each relay comes from: a message for a
//! Capitão only from the machine that hosts the global orchestrator, and a report only from a
//! machine whose tab `report.tab` is the Capitão of `report.path`.

use crate::hub::{Account, Hub, Unanswered};
use futures_util::future::join_all;
use pinned_core::agent::Role;
use pinned_core::global_orchestrator::RelayTo;
use pinned_core::link::is_unknown_command;
use pinned_core::wire::{Call, MachineId};
use serde_json::Value;
use std::time::Duration;

/// Each machine answers whether it hosts from its in-order call queue, behind whatever it was doing.
const HOST_WAIT: Duration = Duration::from_secs(10);
const RELAY_WAIT: Duration = Duration::from_secs(30);

pub enum Source<'a> {
    /// A signed-in client, which may call any machine of the account anyway.
    Client,
    /// The machine whose socket sent the relay.
    Machine(&'a str),
}

pub async fn relay(hub: &Hub, account: &Account, from: Source<'_>, request_id: String, to: RelayTo) -> Result<Value, String> {
    match to {
        RelayTo::Captain { machine, path, text, sender, busy } => {
            if let Source::Machine(from) = from {
                let (host, host_name) = host(hub, account).await?;
                if host != from {
                    let name = account.name_of(from).unwrap_or_else(|| from.to_owned());
                    return Err(format!("{name} does not host the global orchestrator ({host_name} does), so it can't send to a Capitão"));
                }
            }
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

/// The one online machine whose settings say it hosts the global orchestrator. It asks each time
/// rather than caching, because a machine's settings also change in its own app, which the hub
/// never sees.
async fn host(hub: &Hub, account: &Account) -> Result<(MachineId, String), String> {
    let online = account.online();
    let answers = join_all(online.iter().map(|(id, _)| hub.call_machine(account, id, Call::GlobalOrchestratorSettings, HOST_WAIT))).await;
    let mut hosts = vec![];
    let mut silent = vec![];
    for ((id, name), answer) in online.into_iter().zip(answers) {
        match answer {
            Ok(settings) if settings["host"] == true => hosts.push((id, name)),
            Ok(_) => {}
            // An older machine can't host it.
            Err(Unanswered::Refused(e)) if is_unknown_command(&e) => {}
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
        Err(Unanswered::Refused(e)) if is_unknown_command(&e) => {
            Err(format!("{} precisa atualizar o pinned", account.name_of(machine).unwrap_or_else(|| machine.to_owned())))
        }
        Err(Unanswered::Refused(e)) => Err(e),
        // The machine keeps the request id, so a retry answers what it did instead of doing it twice.
        Err(Unanswered::Lost(e)) => Err(format!("{e}; try again with the same request_id")),
    }
}
