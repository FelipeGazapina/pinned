//! The hub's side of what agents do as the user (`pinned_core::surface`): each act is checked
//! against the manifest, run as the account that owns the session's machine, written to the audit
//! trail, and a `confirm` action waits for the user's approval in the app.

use crate::boxes::Opener;
use crate::db::{Audit, Held};
use crate::hub::{Account, Hub};
use pinned_core::surface::{self, ActAnswer, Action, ActorRole, AgentAct, AuditOutcome, Class, Confirmation, Decided, Decision, Parsed, Side};
use pinned_core::connectors::{Placement, RunsOn};
use pinned_core::schedule::{self, Actor};
use pinned_core::wire::{Call, HubCall, RunStarted, ToClient};
use serde_json::Value;
use std::collections::BTreeMap;
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
    /// Its session skips its own permission prompts.
    bypass: bool,
}

impl Caller {
    fn actor(&self) -> Actor {
        Actor::Agent { role: self.role, machine: self.machine.to_string(), tab: self.tab.clone(), session_title: self.title.clone() }
    }

    fn of(account: &Account, machine: &str, tab: &str) -> Result<Caller, String> {
        let id = Uuid::parse_str(machine).map_err(|_| format!("this account has no machine {machine}"))?;
        let view = account.tab(machine, tab);
        Ok(Caller {
            machine: id,
            tab: tab.to_owned(),
            known: view.is_some(),
            role: view.as_ref().map_or(ActorRole::Session, ActorRole::of),
            session: view.as_ref().and_then(|v| v.session.clone()),
            bypass: view.as_ref().is_some_and(|v| v.bypass),
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
    /// The scheduled task the act is about, known once it has run.
    task: std::sync::Mutex<Option<String>>,
}

impl Trail<'_> {
    async fn write(&self, outcome: AuditOutcome, detail: &str) -> Result<(), String> {
        let task = self.task.lock().unwrap().clone();
        let row = Audit { account: self.account, machine: self.machine, tab: self.tab, role: self.role, action: self.action, args: &self.args, outcome, detail, request_id: self.request_id, task: task.as_deref() };
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
    let trail = Trail { hub, account: account.id, machine: caller.machine, tab: &caller.tab, role: caller.role, action: &act.action, args: surface::summary(&act.args), request_id, task: Default::default() };
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
    // A schedule act whose task cannot be read is refused here, before anything runs.
    let why = match schedule_needs_ok(hub, account, &admitted.call).await {
        Ok(why) => why,
        Err(refusal) => {
            let refusal = surface::mask_secrets(&refusal);
            trail.write(AuditOutcome::Refused, &refusal).await?;
            return Err(refusal);
        }
    };
    if surface::held_for_owner(admitted.action) || why.is_some() {
        let asked = Confirmation {
            id: String::new(),
            at: 0,
            machine: origin.to_owned(),
            machine_name: String::new(),
            tab: caller.tab.clone(),
            title: caller.title.clone(),
            role: caller.role,
            action: admitted.action.name.to_owned(),
            about: why.as_deref().map(surface::mask_secrets).unwrap_or_default(),
            args: match surface::without_asked(admitted.action, &act.args) {
                Value::Null => Value::Object(Default::default()),
                // The held copy is listed to every app of the account, and an approval runs it, so a
                // schedule act holds what the user sees: the stored task is masked the same way.
                args if matches!(&admitted.call, Parsed::Hub(call) if schedule::is_schedule_call(call)) => surface::mask_values(args),
                args => args,
            },
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
        note_task(&trail, &act.action, &act.args, &Value::Null, String::new());
        trail.write(AuditOutcome::Pending, "").await?;
        push_confirmations(hub, account).await;
        return json(ActAnswer::Pending { id: id.to_string(), next: surface::PENDING_NEXT.into() });
    }
    let widened = surface::widening(&admitted.call).unwrap_or_default();
    let ran = if surface::HUB_LIMITS.contains(&admitted.action.name) {
        limited(hub, account, &caller, admitted.action, admitted.call).await
    } else {
        run(hub, account, &caller.tab, caller.actor(), admitted.call, &admitted.target, admitted.on_origin).await
    };
    match ran {
        Ok(result) => {
            let widened = note_task(&trail, act.action.as_str(), &act.args, &result, widened);
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
    if let Some(field) = surface::secret_arg(&act.args, action.sends) {
        return Err(format!("{}: {field}. The user enters it in the app.", surface::SECRET_REFUSAL));
    }
    let call = surface::parse(action, &surface::with_blank_asks(action, &act.args))?;
    if surface::skips_prompts(&call) && !caller.bypass {
        return Err(format!("{name} would run a session that skips its permission prompts, and your own session asks for them"));
    }
    if let Parsed::Hub(HubCall::SaveConnector { connector }) = &call {
        if hub.connectors(account).await?.widens_convex(connector) {
            return Err(format!("{name} would let this workflow read more of Convex than it does now. Only the user sets what a workflow reads from Convex, on its Read Convex data step in the app."));
        }
    }
    let asked = act.machine.as_deref().map(str::trim).filter(|m| !m.is_empty() && action.side == Side::Node);
    let target = match asked {
        Some(asked) => crate::remote::resolve(hub, account, asked).await?.0,
        None => origin.to_owned(),
    };
    let on_origin = target == origin;
    if action.elsewhere && on_origin {
        return Err(format!("{name} reads files on another machine of the account: on your own machine use your own shell and files"));
    }
    if action.starts_work && !on_origin {
        return Err(format!("{name} starts work only on this session's own machine for now"));
    }
    Ok(Admitted { action, call, target, on_origin })
}

/// Runs the call as the app runs it: a `HubCall` through the hub's own path, a `Call` on its
/// machine, where the origin machine hears which session asked. Another machine hears no session
/// for a call that checks one, which then refuses what needs the asking session's own.
async fn run(hub: &Arc<Hub>, account: &Arc<Account>, caller_tab: &str, actor: Actor, call: Parsed, target: &str, on_origin: bool) -> Result<Value, String> {
    match call {
        Parsed::Hub(call) if schedule::is_schedule_call(&call) => Box::pin(hub.schedule_call(account, call, actor)).await,
        // An agent's Unarchive only reopens the tab: starting a session is `start_claude`'s, under its limits.
        Parsed::Hub(HubCall::UnarchiveSession { session }) => json(hub.unarchive_session(account, &session, false).await?),
        // An idea made by an agent says which one.
        Parsed::Hub(HubCall::SaveIdea { id, title, text, project, status }) => json(hub.save_idea(account, actor, id.as_deref(), &title, &text, project.as_deref(), status).await?),
        Parsed::Hub(call) => Box::pin(hub.hub_call(account, call)).await,
        Parsed::Node(call) => {
            let guarded = matches!(call, Call::Prompt { .. } | Call::OpenGlobalOrchestrator | Call::RestartClaude { .. } | Call::SetModel { .. } | Call::SetBypass { .. } | Call::Rewind { .. } | Call::SwitchAccount { .. } | Call::SetWorkspace { .. });
            let call = match (on_origin, guarded) {
                (true, _) => Call::ActFor { caller: caller_tab.to_owned(), call: json(&call)? },
                (false, true) => Call::ActFor { caller: String::new(), call: json(&call)? },
                (false, false) => call,
            };
            hub.call_machine(account, target, call, ACT_WAIT).await.map_err(String::from)
        }
    }
}

/// Runs a hub action that starts work (`surface::HUB_LIMITS`) through the limits the hub holds it
/// to, for the session that asked. One with no path here is refused, never run unheld.
async fn limited(hub: &Arc<Hub>, account: &Arc<Account>, caller: &Caller, action: &Action, call: Parsed) -> Result<Value, String> {
    match call {
        Parsed::Hub(HubCall::OpenBox { repo, size, budget_usd }) => {
            let opener = Opener { machine: caller.machine, tab: caller.tab.clone(), role: caller.role };
            json(hub.boxes.open_session(hub, account, repo, size, budget_usd, Some(opener)).await?)
        }
        Parsed::Hub(HubCall::RunWorkflow { id }) => json(run_workflow(hub, account, caller, &id).await?),
        _ => Err(format!("{} starts work on the hub, and nothing holds it to the limits there", action.name)),
    }
}

/// An agent's Run now of workflow `id`: on its own machine, as its own task, held to the limits it
/// runs under (`Call::RunNow` inside `ActFor`), and never with wider permissions than its own.
async fn run_workflow(hub: &Arc<Hub>, account: &Arc<Account>, caller: &Caller, id: &str) -> Result<RunStarted, String> {
    let title = if caller.title.is_empty() { caller.tab.clone() } else { caller.title.clone() };
    let by = format!("{} {title:?} through run_workflow", caller.role.label());
    let run = hub.connectors(account).await?.run_now(id, &by)?;
    let origin = caller.machine.to_string();
    let elsewhere = match &run.runs_on {
        RunsOn::FreshBox(_) => Some("an Upstash Box".to_owned()),
        RunsOn::Machine(machine) if machine != &origin => Some(account.name_of(machine).unwrap_or_else(|| machine.clone())),
        RunsOn::Machine(_) | RunsOn::Any => None,
    };
    if let Some(elsewhere) = elsewhere {
        return Err(format!("run_workflow starts work only on this session's own machine for now, and this workflow runs on {elsewhere}"));
    }
    let spec = hub.run_spec(account, &run).await;
    if !caller.bypass && spec.args.windows(2).any(|pair| pair[0] == "--permission-mode" && pair[1] == "bypassPermissions") {
        return Err("run_workflow would run a session that skips its permission prompts, and your own session asks for them".into());
    }
    let call = Call::ActFor { caller: caller.tab.clone(), call: json(crate::hub::run_call(spec, &run.subject))? };
    let tab = hub.call_machine(account, &origin, call, ACT_WAIT).await.map_err(String::from)?;
    let tab = tab.as_str().ok_or("the machine started the session without a tab id")?.to_owned();
    Ok(account.started(id, Placement { machine: origin, tab }))
}

/// The user's decision on confirmation `id`, from a signed-in client of `account`. `entered` is
/// what they typed in the app for a credential action. It never reaches the trail, the stored
/// confirmation, the notice or an error, and a credential action whose run fails waits again.
pub async fn decide(hub: &Arc<Hub>, account: &Arc<Account>, id: &str, approve: bool, entered: BTreeMap<String, String>) -> Result<Value, String> {
    let unknown = || format!("no confirmation {id} waits");
    let hide = |error: &str| surface::hide_entered(error, &entered);
    let key = Uuid::parse_str(id).map_err(|_| unknown())?;
    let typed = if approve { typed_args(hub, account, key, &entered).await.map_err(|e| hide(&e))? } else { None };
    let held = hub.db.decide_confirmation(account.id, key, approve).await?.ok_or_else(unknown)?;
    let asked = &held.confirmation;
    let machine = Uuid::parse_str(&asked.machine).map_err(|e| e.to_string())?;
    let trail = Trail { hub, account: account.id, machine, tab: &asked.tab, role: asked.role, action: &asked.action, args: surface::summary(&asked.args), request_id: asked.request_id.as_deref(), task: Default::default() };
    trail.write(if approve { AuditOutcome::Approved } else { AuditOutcome::Denied }, "").await?;
    let action = surface::find(&asked.action);
    let (decision, outcome, result) = match (approve, action) {
        (false, _) => (Decision::Denied, "denied".to_owned(), Value::Null),
        (true, None) => (Decision::Approved, format!("failed: no action {} any more", asked.action), Value::Null),
        (true, Some(action)) => match rerun(hub, account, action, asked, typed.as_ref().unwrap_or(&asked.args)).await {
            Ok(result) => (Decision::Approved, "done".to_owned(), result),
            // A key the service refused: the user types another, so it waits again, with no notice.
            Err(e) if !action.asks.is_empty() => {
                let error = hide(&e);
                trail.ran(AuditOutcome::Failed, &error).await;
                hub.db.reopen_confirmation(account.id, key).await?;
                push_confirmations(hub, account).await;
                return Err(error);
            }
            Err(e) => (Decision::Approved, format!("failed: {}", hide(&e)), Value::Null),
        },
    };
    note_task(&trail, &asked.action, &asked.args, &result, String::new());
    if decision == Decision::Approved {
        match outcome.strip_prefix("failed: ") {
            Some(error) => trail.ran(AuditOutcome::Failed, error).await,
            None => trail.ran(AuditOutcome::Done, "").await,
        }
    }
    let shown = surface::hide_entered_in(surface::redact(result.clone(), action.map_or(&[][..], |a| a.hides)), &entered);
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
            let trail = Trail { hub, account, machine, tab: &asked.tab, role: asked.role, action: &asked.action, args: surface::summary(&asked.args), request_id: asked.request_id.as_deref(), task: Default::default() };
            trail.ran(AuditOutcome::Failed, INTERRUPTED).await;
            eprintln!("pinned-server: confirmation {} was approved when a hub stopped; it reads as failed", asked.id);
        }
        if !matches!(notify(hub, &*hub.account(account).await?, &held).await, Ok(true)) {
            eprintln!("pinned-server: the session that asked for confirmation {} gets its notice once its machine connects", asked.id);
        }
    }
    Ok(())
}

/// The arguments an approval runs with when the user typed some in the app: the stored ones with
/// `entered`, checked before the confirmation is taken, so one that lacks a field still waits.
/// None runs the stored ones, for an action that asks nothing.
async fn typed_args(hub: &Hub, account: &Account, key: Uuid, entered: &BTreeMap<String, String>) -> Result<Option<Value>, String> {
    let waiting = hub.db.confirmation(account.id, key).await?.filter(|held| held.confirmation.decision.is_none());
    let Some(asked) = waiting.map(|held| held.confirmation) else { return Ok(None) };
    let Some(action) = surface::find(&asked.action) else { return Ok(None) };
    if action.asks.is_empty() && entered.is_empty() {
        return Ok(None);
    }
    let args = surface::with_entered(action, &asked.args, entered)?;
    surface::parse(action, &args)?;
    Ok(Some(args))
}

/// Runs an approved confirmation with `args` for the session that asked, on the machine it named.
async fn rerun(hub: &Arc<Hub>, account: &Arc<Account>, action: &'static Action, asked: &Confirmation, args: &Value) -> Result<Value, String> {
    let call = surface::parse(action, args)?;
    let target = match asked.target.as_deref() {
        Some(named) => crate::remote::resolve(hub, account, named).await?.0,
        None => asked.machine.clone(),
    };
    let on_origin = target == asked.machine;
    let actor = Actor::Agent { role: asked.role, machine: asked.machine.clone(), tab: asked.tab.clone(), session_title: asked.title.clone() };
    run(hub, account, &asked.tab, actor, call, &target, on_origin).await
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

/// Why an agent's schedule act waits for the user although its action is a Write, or None when it
/// runs at once. Every rule of when an agent's schedule act waits lives here:
/// - a save that carries approvals (`schedule::save_needs_ok`), as a dry run never does;
/// - an edit, a pause or a start again, or a delete of a task the user created;
/// - starting a task that carries approvals again, whoever created it.
///
/// It fails closed: a task id it cannot read, or a task the account does not hold, is an error that
/// refuses the act, never a "no". It reads ids with `hub::task_key`, as `Hub::schedule_call` does.
async fn schedule_needs_ok(hub: &Hub, account: &Account, call: &Parsed) -> Result<Option<String>, String> {
    let Parsed::Hub(call) = call else { return Ok(None) };
    let stored = |id: &str| {
        let id = id.to_owned();
        async move { hub.db.task(account.id, crate::hub::task_key(&id)?).await?.ok_or_else(|| format!("this account has no task {id}")) }
    };
    let users = |task: &schedule::Task, what: &str| (task.created_by == Actor::User).then(|| format!("an agent wants to {what} \"{}\", a task the user created", task.title));
    let why = match call {
        HubCall::ListSchedules => Vec::new(),
        HubCall::ScheduleRuns { id, .. } => stored(id).await.map(|_| Vec::new())?,
        HubCall::DeleteSchedule { id } => users(&stored(id).await?, "delete, with its run history,").into_iter().collect(),
        HubCall::PauseSchedule { id, paused } => {
            let task = stored(id).await?;
            let mut why: Vec<String> = users(&task, if *paused { "pause" } else { "start again" }).into_iter().collect();
            if !paused {
                why.extend(schedule::save_needs_ok(&task));
            }
            why
        }
        HubCall::SaveSchedule { task, dry_run } => {
            let mut why = Vec::new();
            if !task.id.trim().is_empty() {
                why.extend(users(&stored(&task.id).await?, "edit"));
            }
            if *dry_run {
                return Ok(None);
            }
            why.extend(schedule::save_needs_ok(task));
            why
        }
        _ => Vec::new(),
    };
    Ok((!why.is_empty()).then(|| why.join("; ")))
}

/// The task a schedule act was about, from the act's arguments and what it answered.
fn task_of(action: &str, args: &Value, result: &Value) -> Option<String> {
    match action {
        "save_schedule" => result["task"]["id"].as_str().map(str::to_owned),
        "pause_schedule" | "delete_schedule" | "schedule_runs" => args["id"].as_str().map(str::to_owned),
        _ => None,
    }
}

/// Records the task on the act's audit rows and words what a save came to for its detail.
fn note_task(trail: &Trail<'_>, action: &str, args: &Value, result: &Value, detail: String) -> String {
    let Some(task) = task_of(action, args, result) else { return detail };
    *trail.task.lock().unwrap() = Some(task);
    match (action, result["task"]["title"].as_str()) {
        ("save_schedule", Some(title)) => {
            let next = result["next"][0]["local"].as_str().map_or_else(|| "no run".to_owned(), str::to_owned);
            surface::mask_secrets(&format!("saved \"{title}\"; next: {next}"))
        }
        _ => detail,
    }
}
