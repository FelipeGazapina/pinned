//! `delegate_task` on another machine or in an Upstash Box. A thread's machine asks the hub to run
//! a worker elsewhere, asks how it goes or cancels it, and the machine that runs it reports its end.
//! The hub stamps every call with the machine whose socket sent it and finds the other machine
//! only among that account's own, so a machine reaches no machine, box or task of another account.

use crate::hub::{Account, Hub, Unanswered, RUN_WAIT};
use pinned_core::delegate::{Origin, RemoteAsk, RemoteEnd, Request};
use pinned_core::github;
use pinned_core::link::is_unknown_command;
use pinned_core::placement::{Need, Seen};
use pinned_core::wire::{Call, HubCall, TabId};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

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
        HubCall::Place { repo, need, exclude } => place(hub, account, from, repo, &need, &exclude).await,
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
    // What the machine reported up to 20 s ago can miss an account it just signed in to, so a refusal
    // is checked against what it says now.
    if pinned_core::load::refusal(&name, &request.target, account.state_of(&machine).as_ref()).is_some() {
        if let Ok(fresh) = hub.call_machine(account, &machine, Call::State, LOOK_WAIT).await {
            if let Ok(state) = serde_json::from_value::<pinned_core::load::MachineState>(fresh) {
                account.note_state(&machine, state);
            }
        }
        if let Some(reason) = pinned_core::load::refusal(&name, &request.target, account.state_of(&machine).as_ref()) {
            return Err(reason);
        }
    }
    let token = hub.connectors(account).await?.github_token().filter(|_| repo.as_deref().is_some_and(github::hosts));
    let mut started = call(hub, account, &machine, Call::Delegate { origin, request: Box::new(request), repo, token }, RUN_WAIT).await?;
    started["machine"] = json!(machine);
    started["machineName"] = json!(name);
    Ok(started)
}

/// The machine of `from`'s account that runs a heavy worker `from` would start, by the account's
/// placement policy and what each machine last reported. Only this account's machines are seen,
/// and never a box.
async fn place(hub: &Arc<Hub>, account: &Arc<Account>, from: &str, repo: Option<String>, need: &Need, exclude: &[String]) -> Result<Value, String> {
    let policy = hub.db.tastes(account.id).await?.unwrap_or_default().placement;
    let cloneable = match repo.as_deref() {
        Some(repo) if github::hosts(repo) => hub.connectors(account).await?.github_token().is_some(),
        _ => false,
    };
    let key = repo.as_deref().and_then(pinned_core::machine::repo_key);
    let lists = |machine: &str| key.as_deref().is_some_and(|key| account.last_listed(machine).iter().any(|project| project.repo.as_deref().is_some_and(|repo| repo.eq_ignore_ascii_case(key))));
    let mut machines = hub.machines(account).await?;
    machines.retain(|machine| machine.upstash_box.is_none());
    // What a machine reported up to 20 s ago can miss an account it just signed in to, so a
    // machine is asked again before it is skipped for that.
    for machine in machines.iter_mut().filter(|machine| machine.online) {
        if machine.state.as_ref().is_some_and(|state| pinned_core::placement::lacks_account(&machine.name, need, state).is_some()) {
            if let Ok(fresh) = hub.call_machine(account, &machine.id, Call::State, LOOK_WAIT).await {
                if let Ok(state) = serde_json::from_value::<pinned_core::load::MachineState>(fresh) {
                    account.note_state(&machine.id, state);
                    machine.state = account.state_of(&machine.id);
                }
            }
        }
    }
    let seen: Vec<Seen> = machines
        .into_iter()
        .map(|machine| Seen { has_repo: machine.id == from || cloneable || lists(&machine.id), id: machine.id, name: machine.name, online: machine.online, state: machine.state })
        .collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    serde_json::to_value(pinned_core::placement::place(&policy, from, need, &seen, exclude, now)).map_err(|e| e.to_string())
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
/// worker box whose node is offline has no node to ask: a cancel deletes it, and a look says it
/// still boots, or that it is offline once it had connected. A task whose end the hub holds for
/// `from` answers from it first, as its box and machine may be gone.
async fn remote_task(hub: &Arc<Hub>, account: &Arc<Account>, from: &str, machine: &str, tab: TabId, task: String, cancel: Option<String>) -> Result<Value, String> {
    // After a delivery of that end that runs now, as on the reconnect of `from`: a look never
    // overtakes it, so while the hub holds an end, `from` reads it, never a gone worker.
    let delivering = match Uuid::parse_str(from) {
        Ok(origin) => Some(hub.hold_delivery(account, origin, &task).await),
        Err(_) => None,
    };
    if let Some(end) = held_end(hub, account, from, &task).await? {
        if cancel.is_none() {
            if let Ok(origin) = Uuid::parse_str(from) {
                let _ = hub.db.read_held_end(account.id, origin, &task).await;
            }
        }
        return Ok(json!({ "workState": end.state, "summary": end.summary, "error": end.error, "usage": end.usage }));
    }
    drop(delivering);
    let worker_box = hub.boxes.delegate_box(hub, account, machine).await?;
    let asked = Call::RemoteTask { origin_machine: from.to_owned(), tab, task, cancel: cancel.clone() };
    let offline = match worker_box.as_ref().filter(|_| !account.is_online(machine)) {
        Some(row) => {
            let removed = matches!(row.phase, crate::boxes::Phase::Ending(_)) && hub.db.machine_name(account.id, row.machine).await?.is_none();
            Some(crate::boxes::offline_answer(&row.phase, removed, machine))
        }
        None => None,
    };
    let answer = match &offline {
        Some(e) => Err(e.clone()),
        None => call(hub, account, machine, asked, LOOK_WAIT).await,
    };
    match (worker_box, cancel) {
        (Some(row), Some(reason)) => {
            let mut answer = answer.unwrap_or_else(|_| json!({ "workState": "cancelled", "error": reason }));
            hub.boxes.add_box_usage(hub, account, &row, &mut answer).await;
            // A box cancelled while its link is down ends for that, not as done.
            let why = if offline.as_deref() == Some(pinned_core::delegate::BOX_OFFLINE) { crate::boxes::End::Offline } else { crate::boxes::End::Done };
            hub.boxes.end_delegate(hub, account, row, why).await?;
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
/// that machine took it. With `to` offline the hub holds the end for it and answers taken once the
/// row is written; only an end that is neither passed on nor held is an error, which `from` sends
/// again. One `to` answered it did not take, as a machine that restarted and forgot the task does,
/// the hub keeps unclaimed, so `unclaimed_ends` lists it. A worker box goes once its end got through
/// or is held.
async fn ended(hub: &Arc<Hub>, account: &Arc<Account>, from: &str, to: &str, tab: TabId, task: String, end: RemoteEnd) -> Result<Value, String> {
    let worker_box = hub.boxes.delegate_box(hub, account, from).await?;
    let mut end = end;
    if let Some(row) = &worker_box {
        hub.boxes.add_end_box_usage(hub, account, row, &mut end).await;
    }
    let passed = Call::RemoteEnded { from: from.to_owned(), tab: tab.clone(), task: task.clone(), end: end.clone() };
    let taken = match hub.call_machine(account, to, passed, LOOK_WAIT).await {
        Ok(taken) if taken == Value::Bool(true) => {
            // A copy held while `to` was offline has nothing left to tell.
            if let Ok(origin) = Uuid::parse_str(to) {
                let _ = hub.db.drop_held_end(account.id, origin, &task).await;
            }
            taken
        }
        Ok(taken) => {
            // Not taken: kept, so neither its task's status nor the user loses it.
            if let Err(e) = hold_end(hub, account, to, from, &tab, &task, end, true).await {
                eprintln!("pinned-server: the end of task {task} that machine {to} did not take is not kept: {e}");
            }
            taken
        }
        Err(Unanswered::Refused(e)) if !account.is_online(to) => {
            if !hold_end(hub, account, to, from, &tab, &task, end, false).await? {
                return Err(e);
            }
            // The machine may have come back between the refusal and the write, after its own flush.
            if account.is_online(to) {
                tokio::spawn(deliver_held(hub.clone(), account.clone(), to.to_owned()));
            }
            Value::Bool(true)
        }
        Err(Unanswered::Refused(_)) => Value::Bool(false),
        Err(Unanswered::Lost(e)) => return Err(e),
    };
    if let Some(row) = worker_box {
        hub.boxes.end_delegate(hub, account, row, crate::boxes::End::Done).await?;
    }
    Ok(taken)
}

/// The most of a held end's summary or error the hub keeps: 256 KiB. A worker's summary is its last
/// reply, a few KiB at most in practice; this keeps one runaway reply from parking megabytes per task
/// in Postgres for `HELD_FOR`, and keeps the delivery one small socket frame.
pub const HELD_TEXT_MAX: usize = 256 * 1024;

/// `text` cut to `HELD_TEXT_MAX` bytes on a character boundary, with a marker that says how much went.
pub fn clamp_held(text: String) -> String {
    if text.len() <= HELD_TEXT_MAX {
        return text;
    }
    let mut cut = HELD_TEXT_MAX;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…[truncated by pinned: {} bytes]", &text[..cut], text.len() - cut)
}

/// Holds `end` for the machine `origin` of the account, which is offline, until it comes back, or,
/// `unclaimed`, as one it answered it did not take. False when `origin` is no machine of the
/// account, which leaves the end to its sender. An error when the row could not be written: the
/// sender then keeps it and sends it again. Logs ids and sizes, never the end's text.
#[allow(clippy::too_many_arguments)]
pub async fn hold_end(hub: &Hub, account: &Account, origin: &str, from: &str, tab: &str, task: &str, mut end: RemoteEnd, unclaimed: bool) -> Result<bool, String> {
    let (Ok(origin_id), Ok(from_id)) = (Uuid::parse_str(origin), Uuid::parse_str(from)) else { return Ok(false) };
    if hub.db.machine_name(account.id, origin_id).await?.is_none() {
        return Ok(false);
    }
    let size = end.summary.as_ref().map_or(0, String::len) + end.error.as_ref().map_or(0, String::len);
    end.summary = end.summary.map(clamp_held);
    end.error = end.error.map(clamp_held);
    let report = serde_json::to_value(&end).map_err(|e| e.to_string())?;
    match hub.db.hold_end(account.id, origin_id, task, tab, from_id, &report, unclaimed).await {
        Ok(fresh) => {
            let kept = if fresh { "holds" } else { "already held" };
            let why = if unclaimed { "unclaimed by" } else { "for offline" };
            eprintln!("pinned-server: the hub {kept} the end of task {task} from {from} {why} machine {origin} of account {} ({size} bytes)", account.id);
        }
        Err(e) => {
            eprintln!("pinned-server: could not hold the end of task {task} from {from} for machine {origin}: {e}");
            return Err(format!("the hub could not keep the end of task {task} for its machine, which is offline; it is sent again"));
        }
    }
    Ok(true)
}

/// The end the hub holds for `origin`'s task `task`, when it holds one.
async fn held_end(hub: &Hub, account: &Account, origin: &str, task: &str) -> Result<Option<RemoteEnd>, String> {
    let Ok(origin) = Uuid::parse_str(origin) else { return Ok(None) };
    let Some(held) = hub.db.held_ends(account.id, origin, Some(task)).await?.into_iter().next() else { return Ok(None) };
    Ok(serde_json::from_value(held.end).ok())
}

/// Passes every end the hub holds for `machine`, which just connected, as the `Call::RemoteEnded` it
/// missed. A row goes only once the machine took it; one it did not answer stays for its next
/// connection, and its status answers from the row meanwhile. One it answered it did not take, as
/// a machine that restarted and forgot the task does, stays unclaimed for `HELD_FOR` and is not
/// offered again. One delivery of a row runs at a time and sends only a row still waiting, and a
/// machine takes one end per task, so a repeat is a no-op there.
pub async fn deliver_held(hub: Arc<Hub>, account: Arc<Account>, machine: String) {
    let Ok(origin) = Uuid::parse_str(&machine) else { return };
    match hub.db.expire_held_ends(account.id).await {
        Ok(0) => {}
        Ok(n) => eprintln!("pinned-server: {n} held ends of account {} expired unread", account.id),
        Err(e) => eprintln!("pinned-server: could not expire account {}'s held ends: {e}", account.id),
    }
    let held = match hub.db.held_ends(account.id, origin, None).await {
        Ok(held) => held,
        Err(e) => return eprintln!("pinned-server: could not read the ends held for machine {machine}: {e}"),
    };
    for held in held.into_iter().filter(|held| held.unclaimed_at.is_none()) {
        // A delivery that ran meanwhile, from another connection of the machine, dropped or unclaimed
        // the row, and a status look meanwhile may have read it.
        let _delivering = hub.hold_delivery(&account, origin, &held.task).await;
        let read = match hub.db.held_ends(account.id, origin, Some(&held.task)).await {
            Ok(rows) => match rows.iter().find(|row| row.unclaimed_at.is_none()) {
                Some(row) => row.read,
                None => continue,
            },
            Err(_) => continue,
        };
        let Ok(end) = serde_json::from_value::<RemoteEnd>(held.end) else {
            eprintln!("pinned-server: the held end of task {} for machine {machine} is unreadable; it is dropped", held.task);
            let _ = hub.db.drop_held_end(account.id, origin, &held.task).await;
            continue;
        };
        let ended = Call::RemoteEnded { from: held.from.to_string(), tab: held.tab, task: held.task.clone(), end };
        match hub.call_machine(&account, &machine, ended, LOOK_WAIT).await {
            Ok(taken) if taken == Value::Bool(true) => {
                if let Err(e) = hub.db.drop_held_end(account.id, origin, &held.task).await {
                    eprintln!("pinned-server: could not drop the delivered end of task {}: {e}", held.task);
                }
                eprintln!("pinned-server: passed the held end of task {} to machine {machine} (taken: true)", held.task);
            }
            Ok(_) if read => {
                // It settled the task from the status it read, so its not-taken is a repeat.
                if let Err(e) = hub.db.drop_held_end(account.id, origin, &held.task).await {
                    eprintln!("pinned-server: could not drop the read end of task {}: {e}", held.task);
                }
                eprintln!("pinned-server: machine {machine} had the held end of task {} from its status already", held.task);
            }
            Ok(taken) => {
                if let Err(e) = hub.db.unclaim_held_end(account.id, origin, &held.task).await {
                    eprintln!("pinned-server: could not mark the held end of task {} unclaimed: {e}", held.task);
                }
                eprintln!("pinned-server: machine {machine} did not take the held end of task {} (answered {taken}); it stays unclaimed", held.task);
            }
            Err(e) => eprintln!("pinned-server: machine {machine} did not take the held end of task {}: {}", held.task, String::from(e)),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_text_past_the_cap_is_cut_with_a_marker_and_never_dropped() {
        assert_eq!(clamp_held("Footer fixed.".into()), "Footer fixed.");
        let exact = "a".repeat(HELD_TEXT_MAX);
        assert_eq!(clamp_held(exact.clone()), exact);
        let long = "a".repeat(HELD_TEXT_MAX + 10);
        assert_eq!(clamp_held(long), format!("{}…[truncated by pinned: 10 bytes]", "a".repeat(HELD_TEXT_MAX)));
        // A cut inside a character backs off to its start.
        let wide = format!("{}é{}", "a".repeat(HELD_TEXT_MAX - 1), "b".repeat(5));
        assert_eq!(clamp_held(wide), format!("{}…[truncated by pinned: 7 bytes]", "a".repeat(HELD_TEXT_MAX - 1)));
    }
}
