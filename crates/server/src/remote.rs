//! `delegate_task` on another machine or in an Upstash Box. A thread's machine asks the hub to run
//! a worker elsewhere, asks how it goes or cancels it, and the machine that runs it reports its end.
//! The hub stamps every call with the machine whose socket sent it and finds the other machine
//! only among that account's own, so a machine reaches no machine, box or task of another account.

use crate::hub::{Account, Hub, Unanswered, RUN_WAIT};
use pinned_core::delegate::{Origin, RemoteAsk, RemoteEnd, Request};
use pinned_core::github;
use pinned_core::link::is_unknown_command;
use pinned_core::wire::{Call, HubCall, TabId};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const LOOK_WAIT: Duration = Duration::from_secs(30);

/// Answers a machine's `Delegate`, `RemoteTask`, `RemoteEnded` and `RemoteAsked`. `from` is the machine whose
/// socket sent the call.
pub async fn serve(hub: &Arc<Hub>, account: &Arc<Account>, from: &str, call: HubCall) -> Result<Value, String> {
    match call {
        HubCall::Delegate { tab, task, request, repo, access, delegator, limits } => {
            let machine_name = account.name_of(from).unwrap_or_else(|| from.to_owned());
            let origin = Origin { machine: from.to_owned(), machine_name, tab, task, access, delegator, limits };
            delegate(hub, account, from, origin, *request, repo).await
        }
        HubCall::RemoteTask { machine, tab, task, cancel } => remote_task(hub, account, from, &machine, tab, task, cancel).await,
        HubCall::RemoteEnded { to, tab, task, end } => ended(hub, account, from, &to, tab, task, end).await,
        HubCall::RemoteAsked { to, tab, task, ask } => asked(hub, account, from, &to, tab, task, ask).await,
        _ => Err("that is not a delegate call".into()),
    }
}

async fn delegate(hub: &Arc<Hub>, account: &Arc<Account>, from: &str, origin: Origin, request: Request, repo: Option<String>) -> Result<Value, String> {
    // A start the caller gave up waiting on is asked again: the repeat waits for the first, then
    // gets the worker or box that start made, since a worker's machine and a box's row both know
    // the origin task.
    let _starting = hub.hold_start(account, from, &origin.tab, &origin.task).await;
    if let Some(boxed) = request.target.boxed.clone() {
        return hub.boxes.open_delegate(hub, account, origin, request, repo, boxed).await;
    }
    let asked = request.target.machine.clone().ok_or("name target.machine or target.box")?;
    let (machine, name) = resolve(hub, account, &asked).await?;
    if machine == from {
        return Err(format!("{name} is this machine: leave target.machine out to run the worker here"));
    }
    if !account.is_online(&machine) {
        return Err(format!("{name} is offline, so it can't take the worker; nothing ran elsewhere"));
    }
    let token = hub.connectors(account).await?.github_token().filter(|_| repo.as_deref().is_some_and(github::hosts));
    let mut started = call(hub, account, &machine, Call::Delegate { origin, request: Box::new(request), repo, token }, RUN_WAIT).await?;
    started["machine"] = json!(machine);
    started["machineName"] = json!(name);
    Ok(started)
}

/// A machine of the account by its hub id or its name, with or without `@`.
pub(crate) async fn resolve(hub: &Hub, account: &Account, asked: &str) -> Result<(String, String), String> {
    let asked = asked.trim().trim_start_matches('@');
    let machines = hub.machines(account).await?;
    if let Some(m) = machines.iter().find(|m| m.id == asked) {
        return Ok((m.id.clone(), m.name.clone()));
    }
    let named: Vec<_> = machines.iter().filter(|m| m.name.eq_ignore_ascii_case(asked)).collect();
    match named.as_slice() {
        [m] => Ok((m.id.clone(), m.name.clone())),
        [] => Err(format!("this account has no machine {asked}")),
        _ => Err(format!("{} machines are named {asked}: pass the id projects() lists", named.len())),
    }
}

/// The status of the task `from`'s tab delegated to `machine`, or its state once cancelled. A
/// worker box still booting has no node to ask: a cancel deletes it, and a look says it boots.
async fn remote_task(hub: &Arc<Hub>, account: &Arc<Account>, from: &str, machine: &str, tab: TabId, task: String, cancel: Option<String>) -> Result<Value, String> {
    let worker_box = hub.boxes.delegate_box(hub, account, machine).await?;
    let asked = Call::RemoteTask { origin_machine: from.to_owned(), tab, task, cancel: cancel.clone() };
    let answer = match (&worker_box, account.is_online(machine)) {
        (Some(_), false) => Err(pinned_core::delegate::BOX_BOOTING.to_owned()),
        _ => call(hub, account, machine, asked, LOOK_WAIT).await,
    };
    match (worker_box, cancel) {
        (Some(row), Some(reason)) => {
            let mut answer = answer.unwrap_or_else(|_| json!({ "workState": "cancelled", "error": reason }));
            hub.boxes.add_box_usage(hub, account, &row, &mut answer).await;
            hub.boxes.end_delegate(hub, account, row).await?;
            Ok(answer)
        }
        (Some(row), None) => {
            let mut answer = answer?;
            hub.boxes.add_box_usage(hub, account, &row, &mut answer).await;
            Ok(answer)
        }
        _ => answer,
    }
}

/// Passes the end of a task `from` ran to the machine `to` whose tab delegated it. Answers whether
/// that machine took it; only a machine the hub could not reach is an error, which `from` sends
/// again. A worker box goes once its end got through.
async fn ended(hub: &Arc<Hub>, account: &Arc<Account>, from: &str, to: &str, tab: TabId, task: String, end: RemoteEnd) -> Result<Value, String> {
    let worker_box = hub.boxes.delegate_box(hub, account, from).await?;
    let mut end = end;
    if let Some(row) = &worker_box {
        hub.boxes.add_end_box_usage(hub, account, row, &mut end).await;
    }
    let taken = match hub.call_machine(account, to, Call::RemoteEnded { from: from.to_owned(), tab, task, end }, LOOK_WAIT).await {
        Ok(taken) => taken,
        Err(Unanswered::Refused(e)) if !account.is_online(to) => return Err(e),
        Err(Unanswered::Refused(_)) => Value::Bool(false),
        Err(Unanswered::Lost(e)) => return Err(e),
    };
    if let Some(row) = worker_box {
        hub.boxes.end_delegate(hub, account, row).await?;
    }
    Ok(taken)
}

/// Passes what a task `from` runs waits on the user for, or that it waits on nothing, to the
/// machine `to` whose tab delegated it. Answers whether that machine took it, as `ended` does; a
/// machine that predates this call, or has no such task, does not, and `from` leaves it.
async fn asked(hub: &Arc<Hub>, account: &Arc<Account>, from: &str, to: &str, tab: TabId, task: String, ask: Option<RemoteAsk>) -> Result<Value, String> {
    match hub.call_machine(account, to, Call::RemoteAsked { from: from.to_owned(), tab, task, ask }, LOOK_WAIT).await {
        Ok(taken) => Ok(taken),
        Err(Unanswered::Refused(e)) if !account.is_online(to) => Err(e),
        Err(Unanswered::Refused(_)) => Ok(Value::Bool(false)),
        Err(Unanswered::Lost(e)) => Err(e),
    }
}

async fn call(hub: &Hub, account: &Account, machine: &str, call: Call, wait: Duration) -> Result<Value, String> {
    let name = || account.name_of(machine).unwrap_or_else(|| machine.to_owned());
    match hub.call_machine(account, machine, call, wait).await {
        Ok(answer) => Ok(answer),
        Err(Unanswered::Refused(e)) if is_unknown_command(&e) => Err(format!("{} needs to update pinned to take a worker from another machine", name())),
        Err(Unanswered::Refused(e)) => Err(e),
        Err(Unanswered::Lost(e)) => Err(format!("{e}, so the worker may have started there: look at {}'s threads before starting it again", name())),
    }
}
