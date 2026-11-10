//! The hub's side of what agents do as the user (`pinned_core::surface`): each act is checked
//! against the manifest, run as the account that owns the session's machine, written to the audit
//! trail, and a `confirm` action waits for the user's approval in the app.

use crate::db::{Audit, Held};
use crate::hub::{Account, Hub};
use pinned_core::surface::{self, ActAnswer, Action, ActorRole, AgentAct, AuditOutcome, Class, Confirmation, Decided, Decision, Parsed, Side};
use pinned_core::wire::{Call, HubCall, ToClient};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Under the minute a machine waits on the hub, so the session hears why a slow machine failed.
const ACT_WAIT: Duration = Duration::from_secs(50);
const NOTICE_WAIT: Duration = Duration::from_secs(20);
/// After a machine connects, when to try the notices it missed again: its tabs come back after
/// the link, once the app restores them, and a notice finds its session only then.
const NOTICE_TRIES: [Duration; 4] = [Duration::from_secs(3), Duration::from_secs(10), Duration::from_secs(30), Duration::from_secs(90)];
/// Why an approval a hub stopped in the middle of reads as failed: it may have run.
const INTERRUPTED: &str = "the hub restarted before it could tell whether this ran; check before asking again";
/// Longer than any decision takes to run. Younger ones without an outcome may still be running on
/// the hub a deploy is replacing.
pub const DECIDING_AT_MOST: Duration = Duration::from_secs(5 * 60);

/// The session that acts, as its machine last reported its tab.
struct Caller {
    machine: Uuid,
    tab: String,
    known: bool,
    role: ActorRole,
    session: Option<String>,
    title: String,
}

impl Caller {
    fn of(account: &Account, machine: &str, tab: &str) -> Result<Caller, String> {
        let id = Uuid::parse_str(machine).map_err(|_| format!("this account has no machine {machine}"))?;
        let view = account.tab(machine, tab);
        Ok(Caller {
            machine: id,
            tab: tab.to_owned(),
            known: view.is_some(),
            role: view.as_ref().map_or(ActorRole::Session, ActorRole::of),
            session: view.as_ref().and_then(|v| v.session.clone()),
            title: view.and_then(|v| v.title.or(v.prompt_title)).unwrap_or_default(),
        })
    }
}

/// One act's audit rows: who asked what, with the arguments as the trail keeps them.
struct Trail<'a> {
    hub: &'a Hub,
    account: Uuid,
    machine: Uuid,
    tab: &'a str,
    role: ActorRole,
    action: &'a str,
    args: String,
    request_id: Option<&'a str>,
}

impl Trail<'_> {
    async fn write(&self, outcome: AuditOutcome, detail: &str) -> Result<(), String> {
        let row = Audit { account: self.account, machine: self.machine, tab: self.tab, role: self.role, action: self.action, args: &self.args, outcome, detail, request_id: self.request_id };
        self.hub.db.audit(&row).await
    }

    /// The row of something that already ran. Failing to write it is logged, not answered, so the
    /// session doesn't run the action again.
    async fn ran(&self, outcome: AuditOutcome, detail: &str) {
        if let Err(e) = self.write(outcome, detail).await {
            eprintln!("pinned-server: account {}'s {} ran, and its audit row was not written: {e}", self.account, self.action);
        }
    }
}

fn json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// Runs `act` for the session in tab `act.tab` of `origin`, a machine of `account`, or holds it
/// for the user's approval. Every outcome but a repeat writes one audit row.
pub async fn act(hub: &Arc<Hub>, account: &Arc<Account>, origin: &str, act: AgentAct) -> Result<Value, String> {
    let caller = Caller::of(account, origin, &act.tab)?;
    let request_id = act.request_id.as_deref().filter(|id| !id.is_empty());
    let trail = Trail { hub, account: account.id, machine: caller.machine, tab: &caller.tab, role: caller.role, action: &act.action, args: surface::summary(&act.args), request_id };
    if let Some(id) = request_id {
        match hub.db.audited(account.id, caller.machine, &caller.tab, &act.action, id).await? {
            Some((AuditOutcome::Pending, _)) | None => {}
            Some((earlier, at)) => return json(ActAnswer::Repeat { earlier, at }),
        }
    }
    let admitted = match admit(hub, account, origin, &caller, &act).await {
        Ok(admitted) => admitted,
        Err(refusal) => {
            let refusal = surface::mask_secrets(&refusal);
            trail.write(AuditOutcome::Refused, &refusal).await?;
            return Err(refusal);
        }
    };
    if admitted.action.class == Class::Confirm {
        let asked = Confirmation {
            id: String::new(),
            at: 0,
            machine: origin.to_owned(),
            machine_name: String::new(),
            tab: caller.tab.clone(),
            title: caller.title.clone(),
            role: caller.role,
            action: admitted.action.name.to_owned(),
            about: String::new(),
            args: if act.args.is_null() { Value::Object(Default::default()) } else { act.args.clone() },
            target: (!admitted.on_origin).then(|| admitted.target.clone()),
            request_id: request_id.map(str::to_owned),
            decision: None,
            decided_at: None,
            outcome: None,
        };
        let (id, held) = hub.db.hold_confirmation(account.id, caller.machine, caller.session.as_deref(), &asked).await?;
        if !held {
            let earlier = hub.db.confirmation(account.id, id).await?.map(|held| held.confirmation);
            if let Some(Confirmation { decision: Some(decision), decided_at, .. }) = earlier {
                let earlier = if decision == Decision::Approved { AuditOutcome::Approved } else { AuditOutcome::Denied };
                return json(ActAnswer::Repeat { earlier, at: decided_at.unwrap_or_default() });
            }
            return json(ActAnswer::Pending { id: id.to_string(), next: surface::PENDING_NEXT.into() });
        }
        trail.write(AuditOutcome::Pending, "").await?;
        push_confirmations(hub, account).await;
        return json(ActAnswer::Pending { id: id.to_string(), next: surface::PENDING_NEXT.into() });
    }
    let widened = surface::widening(&admitted.call).unwrap_or_default();
    match run(hub, account, &caller.tab, admitted.call, &admitted.target, admitted.on_origin).await {
        Ok(result) => {
            trail.ran(AuditOutcome::Done, &widened).await;
            json(ActAnswer::Done { result: surface::redact(result, admitted.action.hides) })
        }
        Err(error) => {
            let error = surface::mask_secrets(&error);
            trail.ran(AuditOutcome::Failed, &error).await;
            Err(error)
        }
    }
}

/// An act the manifest lets run: its action, its call, and the machine a `Call` runs on.
struct Admitted {
    action: &'static Action,
    call: Parsed,
    target: String,
    on_origin: bool,
}

/// The act as it may run, or why it may not, checked in the order the contract lists.
async fn admit(hub: &Hub, account: &Account, origin: &str, caller: &Caller, act: &AgentAct) -> Result<Admitted, String> {
    if !caller.known {
        let name = account.name_of(origin).unwrap_or_else(|| origin.to_owned());
        return Err(format!("{name} runs no session {}", caller.tab));
    }
    let name = act.action.as_str();
    let action = surface::find(name).ok_or_else(|| format!("no action {name}; call actions to list them"))?;
    if action.class == Class::Excluded {
        return Err(format!("{name} is not open to agents: {}", action.about));
    }
    if !action.wired {
        let class = serde_json::to_value(action.class).ok().and_then(|c| c.as_str().map(str::to_owned)).unwrap_or_default();
        return Err(format!("{name} is a {class} action that is not wired yet"));
    }
    if let Some(field) = surface::secret_arg(&act.args) {
        return Err(format!("{}: {field}. The user enters it in the app.", surface::SECRET_REFUSAL));
    }
    let call = surface::parse(action, &act.args)?;
    let asked = act.machine.as_deref().map(str::trim).filter(|m| !m.is_empty() && action.side == Side::Node);
    let target = match asked {
        Some(asked) => crate::remote::resolve(hub, account, asked).await?.0,
        None => origin.to_owned(),
    };
    let on_origin = target == origin;
    if action.starts_work && !on_origin {
        return Err(format!("{name} starts work only on this session's own machine for now"));
    }
    Ok(Admitted { action, call, target, on_origin })
}

/// Runs the call as the app runs it: a `HubCall` through the hub's own path, a `Call` on its
/// machine, where the origin machine hears which session asked.
async fn run(hub: &Arc<Hub>, account: &Arc<Account>, caller_tab: &str, call: Parsed, target: &str, on_origin: bool) -> Result<Value, String> {
    match call {
        // An agent's Unarchive only reopens the tab: starting a session is `start_claude`'s, under its limits.
        Parsed::Hub(HubCall::UnarchiveSession { session }) => json(hub.unarchive_session(account, &session, false).await?),
        Parsed::Hub(call) => Box::pin(hub.hub_call(account, call)).await,
        Parsed::Node(call) => {
            let call = if on_origin { Call::ActFor { caller: caller_tab.to_owned(), call: json(&call)? } } else { call };
            hub.call_machine(account, target, call, ACT_WAIT).await.map_err(String::from)
        }
    }
}

/// The user's decision on confirmation `id`, from a signed-in client of `account`.
pub async fn decide(hub: &Arc<Hub>, account: &Arc<Account>, id: &str, approve: bool) -> Result<Value, String> {
    let unknown = || format!("no confirmation {id} waits");
    let key = Uuid::parse_str(id).map_err(|_| unknown())?;
    let held = hub.db.decide_confirmation(account.id, key, approve).await?.ok_or_else(unknown)?;
    let asked = &held.confirmation;
    let machine = Uuid::parse_str(&asked.machine).map_err(|e| e.to_string())?;
    let trail = Trail { hub, account: account.id, machine, tab: &asked.tab, role: asked.role, action: &asked.action, args: surface::summary(&asked.args), request_id: asked.request_id.as_deref() };
    trail.write(if approve { AuditOutcome::Approved } else { AuditOutcome::Denied }, "").await?;
    let action = surface::find(&asked.action);
    let (decision, outcome, result) = match (approve, action) {
        (false, _) => (Decision::Denied, "denied".to_owned(), Value::Null),
        (true, None) => (Decision::Approved, format!("failed: no action {} any more", asked.action), Value::Null),
        (true, Some(action)) => match rerun(hub, account, action, asked).await {
            Ok(result) => (Decision::Approved, "done".to_owned(), result),
            Err(e) => (Decision::Approved, format!("failed: {}", surface::mask_secrets(&e)), Value::Null),
        },
    };
    if decision == Decision::Approved {
        match outcome.strip_prefix("failed: ") {
            Some(error) => trail.ran(AuditOutcome::Failed, error).await,
            None => trail.ran(AuditOutcome::Done, "").await,
        }
    }
    let shown = surface::redact(result.clone(), action.map_or(&[][..], |a| a.hides));
    hub.db.finish_confirmation(key, &outcome, &shown).await?;
    push_confirmations(hub, account).await;
    let decided = Held { confirmation: Confirmation { decision: Some(decision), outcome: Some(outcome.clone()), ..asked.clone() }, session: held.session.clone(), result: shown };
    let (hub_for_notice, account_for_notice) = (hub.clone(), account.clone());
    tokio::spawn(async move {
        match notify(&hub_for_notice, &account_for_notice, &decided).await {
            Ok(true) => {}
            Ok(false) => eprintln!("pinned-server: no tab runs the session that asked for confirmation {}; it gets its notice once its machine connects with it", decided.confirmation.id),
            Err(e) => eprintln!("pinned-server: the session that asked for confirmation {} gets its notice later: {e}", decided.confirmation.id),
        }
    });
    json(Decided { decision, outcome, result })
}

/// Gives each confirmation a hub that stopped between the decision and its outcome left one: a
/// denial is denied, an approval failed with `INTERRUPTED` and a `failed` audit row. Then tells its
/// session, or leaves the notice to `deliver_notices` once its machine connects.
pub async fn finish_interrupted(hub: &Arc<Hub>) -> Result<(), String> {
    for (account, held) in hub.db.finish_interrupted(&format!("failed: {INTERRUPTED}"), DECIDING_AT_MOST).await? {
        let asked = &held.confirmation;
        if asked.decision == Some(Decision::Approved) {
            let machine = Uuid::parse_str(&asked.machine).map_err(|e| e.to_string())?;
            let trail = Trail { hub, account, machine, tab: &asked.tab, role: asked.role, action: &asked.action, args: surface::summary(&asked.args), request_id: asked.request_id.as_deref() };
            trail.ran(AuditOutcome::Failed, INTERRUPTED).await;
            eprintln!("pinned-server: confirmation {} was approved when a hub stopped; it reads as failed", asked.id);
        }
        if !matches!(notify(hub, &*hub.account(account).await?, &held).await, Ok(true)) {
            eprintln!("pinned-server: the session that asked for confirmation {} gets its notice once its machine connects", asked.id);
        }
    }
    Ok(())
}

/// Runs an approved confirmation for the session that asked, on the machine it named.
async fn rerun(hub: &Arc<Hub>, account: &Arc<Account>, action: &'static Action, asked: &Confirmation) -> Result<Value, String> {
    let call = surface::parse(action, &asked.args)?;
    let target = match asked.target.as_deref() {
        Some(named) => crate::remote::resolve(hub, account, named).await?.0,
        None => asked.machine.clone(),
    };
    let on_origin = target == asked.machine;
    run(hub, account, &asked.tab, call, &target, on_origin).await
}

/// Tells the session that asked what the user decided, through the queue its worker reports use.
/// False when no tab on its machine runs that session, so the notice waits.
async fn notify(hub: &Hub, account: &Account, held: &Held) -> Result<bool, String> {
    let asked = &held.confirmation;
    let call = Call::Notice { tab: asked.tab.clone(), session: held.session.clone(), text: surface::notice_text(asked, &held.result), id: asked.id.clone() };
    let queued = hub.call_machine(account, &asked.machine, call, NOTICE_WAIT).await.map_err(String::from)?;
    if queued != Value::Bool(true) {
        return Ok(false);
    }
    let id = Uuid::parse_str(&asked.id).map_err(|e| e.to_string())?;
    hub.db.notified(id).await.map(|()| true)
}

/// Sends the notices a machine missed while it was offline, once it is back, and again while its
/// tabs come back.
pub async fn deliver_notices(hub: Arc<Hub>, account: Arc<Account>, machine: String) {
    let Ok(id) = Uuid::parse_str(&machine) else { return };
    for wait in NOTICE_TRIES {
        tokio::time::sleep(wait).await;
        let missed = match hub.db.unnotified(account.id, id).await {
            Ok(missed) => missed,
            Err(e) => return eprintln!("pinned-server: could not read the notices machine {machine} missed: {e}"),
        };
        let mut waiting = false;
        for held in missed {
            match notify(&hub, &account, &held).await {
                Ok(queued) => waiting |= !queued,
                Err(e) => return eprintln!("pinned-server: machine {machine} did not take the notice of confirmation {}: {e}", held.confirmation.id),
            }
        }
        if !waiting {
            return;
        }
    }
}

/// The confirmations that wait, to every client of the account.
pub async fn push_confirmations(hub: &Hub, account: &Account) {
    match waiting(hub, account).await {
        Ok(items) => account.broadcast(&ToClient::Confirmations { items }),
        Err(e) => eprintln!("pinned-server: could not read account {}'s confirmations: {e}", account.id),
    }
}

/// The confirmations that wait, to one client that just signed in.
pub async fn send_confirmations(hub: &Hub, account: &Account, client: u64) {
    if let Ok(items) = waiting(hub, account).await {
        account.to_client(client, &ToClient::Confirmations { items });
    }
}

async fn waiting(hub: &Hub, account: &Account) -> Result<Vec<Confirmation>, String> {
    Ok(hub.db.confirmations(account.id).await?.into_iter().filter(|c| c.decision.is_none()).collect())
}
