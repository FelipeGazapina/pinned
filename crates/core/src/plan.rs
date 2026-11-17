//! The Captain's plans: lists of steps, the limits on them, and the states that follow the tasks they link to.
//!
//! Pure code. Nothing here reads or writes files, talks to the hub, or looks up a task. Callers pass the task
//! facts they know. A call changes the plans only when it succeeds: each write works on a copy and keeps it
//! only when nothing was refused.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;

pub const MAX_PLANS_UNFINISHED: usize = 6;
pub const MAX_PLANS_FINISHED: usize = 10;
pub const MAX_LISTS: usize = 8;
pub const MAX_STEPS_PER_LIST: usize = 20;
pub const MAX_STEPS_PER_FIX: usize = 6;
pub const MAX_FIXES_PER_STEP: usize = 3;
pub const MAX_STEPS_IN_ALL: usize = 120;
pub const MAX_NAME: usize = 60;
pub const MAX_WHAT: usize = 300;
pub const MAX_TITLE: usize = 80;
pub const MAX_NOTE: usize = 200;
pub const MAX_REASON: usize = 200;
pub const GLANCE_TEXT: usize = 32;

/// The reason a step shows when its list's task ended with a result and the Captain never reported it.
const DONE_WITH_LIST: &str = "Done with the list's task";

/// The state a step has, as the Captain reported it or as a task derives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Pending,
    Running,
    Waiting,
    Done,
    Failed,
}

/// The state of one Captain's plan as a whole, which the Orchestrator panel sorts by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptainState {
    NeedsYou,
    Fixing,
    Working,
    #[default]
    Idle,
    Finished,
    Closed,
}

/// Every plan of one Captain session. Ids are handed out from `next` across all plans, so an id is never reused.
/// Unfinished plans come first, oldest first; finished and closed plans follow, newest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CaptainPlans {
    pub next: Counters,
    pub plans: Vec<Plan>,
}

/// The last number handed out for each kind of id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub plan: u64,
    pub list: u64,
    pub step: u64,
    pub fix: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    /// Rises on every write to this plan and on every change the facts cause.
    pub rev: u64,
    /// Empty when the plan has no title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Unix milliseconds of the prompt that made the plan.
    pub asked_at: u64,
    /// Unix milliseconds of the write that found every main step done, or of the close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// The reason given when the plan was closed. Set only by `close`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    pub lists: Vec<List>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct List {
    pub id: String,
    pub name: String,
    /// The task that drives every step of this list that has no task of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub what: String,
    /// The state the Captain last reported.
    pub reported: State,
    /// Unix milliseconds of the last report. Zero when the Captain never reported.
    #[serde(default)]
    pub reported_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The task that runs this step on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// The list task's rollup cost when this step turned running. The step's cost is the rise since then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_at_start: Option<f64>,
    #[serde(default)]
    pub fixes: Vec<Fix>,
}

/// A branch opened under a failed step. The latest one is the open one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    pub id: String,
    pub steps: Vec<Step>,
}

/// The `set` arguments. Ids are optional and match existing lists and steps of the plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SetCall {
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub new: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub lists: Vec<ListInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub steps: Vec<StepInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StepInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub what: Option<String>,
}

/// The `step` arguments: a report on one step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StepCall {
    pub step: String,
    pub state: State,
    #[serde(default)]
    pub note: Option<String>,
}

/// The `fix` arguments: a new branch under a failed step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FixCall {
    pub step: String,
    pub steps: Vec<NewStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NewStep {
    pub name: String,
    #[serde(default)]
    pub what: Option<String>,
}

/// The `close` arguments. The plan is named by its id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CloseCall {
    pub plan: String,
    pub reason: String,
}

/// What a linked task looks like now. Built by the caller from the task registry; this module never looks one up.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskFacts {
    pub id: String,
    pub work: Work,
    /// The worker is waiting on the user: a question is open or it sits in a remote wait.
    pub waits_on_user: bool,
    pub review: Review,
    /// The limit or error that failed the task, shown as the step's reason.
    pub failure: Option<String>,
    /// The task's rollup cost in dollars.
    pub cost: Option<f64>,
    /// Unix milliseconds when the task ended. None while it has not ended.
    pub ended_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Working,
    WaitingForChildren,
    ResultAvailable,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Review {
    Unreviewed,
    Accepted,
    Redone,
}

/// One plan as a client reads it, with states and costs derived from the linked tasks.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct PlanView {
    pub id: String,
    pub rev: u64,
    pub title: String,
    pub asked_at: u64,
    pub finished_at: Option<u64>,
    pub closed: Option<String>,
    pub lists: Vec<ListView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct ListView {
    pub id: String,
    pub name: String,
    pub task: Option<String>,
    pub steps: Vec<StepView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct StepView {
    pub id: String,
    pub name: String,
    pub what: String,
    pub state: State,
    /// The reason shown with the state: a failure, "Redone", "Done with the list's task", "after one fix".
    pub why: Option<String>,
    pub cost: Option<f64>,
    pub task: Option<String>,
    pub fixes: Vec<FixView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct FixView {
    pub id: String,
    pub steps: Vec<StepView>,
}

/// The small shape the Orchestrator panel shows for one plan: one dots string per list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlanGlance {
    pub id: String,
    pub title: String,
    pub state: CaptainState,
    pub done: usize,
    pub total: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    pub strips: Vec<String>,
}

/// Applies a `set` call. Returns the answer text.
pub fn set(
    plans: &mut CaptainPlans,
    call: SetCall,
    asked_at: u64,
    facts: &[TaskFacts],
    now: u64,
) -> Result<String, String> {
    commit(plans, |plans| set_on(plans, call, asked_at, facts, now))
}

/// Applies a report on one step. Returns the answer text.
pub fn step(
    plans: &mut CaptainPlans,
    call: StepCall,
    facts: &[TaskFacts],
    now: u64,
) -> Result<String, String> {
    commit(plans, |plans| step_on(plans, call, facts, now))
}

/// Opens a fix branch under a failed step. Returns the answer text.
pub fn fix(
    plans: &mut CaptainPlans,
    call: FixCall,
    facts: &[TaskFacts],
    now: u64,
) -> Result<String, String> {
    commit(plans, |plans| fix_on(plans, call, facts, now))
}

/// Closes a plan. Returns the answer text.
pub fn close(
    plans: &mut CaptainPlans,
    call: CloseCall,
    facts: &[TaskFacts],
    now: u64,
) -> Result<String, String> {
    commit(plans, |plans| close_on(plans, call, facts, now))
}

/// Records what the facts show (start costs, finish times) in every plan. Returns true when a plan changed.
pub fn settle(plans: &mut CaptainPlans, facts: &[TaskFacts], now: u64) -> bool {
    let mut changed = false;
    for plan in plans.plans.iter_mut() {
        if settle_plan(plan, facts, now) {
            plan.rev += 1;
            changed = true;
        }
    }
    if changed {
        order(plans);
    }
    changed
}

/// The plan with every state and cost derived from the facts.
pub fn derive(plan: &Plan, facts: &[TaskFacts]) -> PlanView {
    PlanView {
        id: plan.id.clone(),
        rev: plan.rev,
        title: plan.title.clone(),
        asked_at: plan.asked_at,
        finished_at: plan.finished_at,
        closed: plan.closed.clone(),
        lists: plan
            .lists
            .iter()
            .map(|l| {
                let shown = list_shown(l, facts);
                ListView {
                    id: l.id.clone(),
                    name: l.name.clone(),
                    task: l.task.clone(),
                    steps: l
                        .steps
                        .iter()
                        .zip(shown)
                        .map(|(s, sh)| step_view(s, sh, facts))
                        .collect(),
                }
            })
            .collect(),
    }
}

/// The glance of one plan.
pub fn glance(plan: &Plan, facts: &[TaskFacts]) -> PlanGlance {
    let view = derive(plan, facts);
    let mains: Vec<&StepView> = view.lists.iter().flat_map(|l| l.steps.iter()).collect();
    PlanGlance {
        id: plan.id.clone(),
        title: clip(&plan.title),
        state: captain_state(plan, &view),
        done: mains.iter().filter(|s| s.state == State::Done).count(),
        total: mains.len(),
        now: view
            .lists
            .iter()
            .find_map(|l| first_active(&l.steps))
            .map(|s| clip(&s.name)),
        finished_at: plan.finished_at,
        strips: view
            .lists
            .iter()
            .map(|l| l.steps.iter().map(dot).collect::<String>())
            .collect(),
    }
}

/// The glance list the Orchestrator panel shows: every unfinished plan, plus the latest finished or closed one
/// with no strips.
pub fn glance_list(plans: &CaptainPlans, facts: &[TaskFacts]) -> Vec<PlanGlance> {
    let mut out = Vec::new();
    let mut latest_finished = true;
    for plan in &plans.plans {
        let g = glance(plan, facts);
        if plan.finished_at.is_none() {
            out.push(g);
        } else if latest_finished {
            out.push(PlanGlance {
                strips: Vec::new(),
                ..g
            });
            latest_finished = false;
        }
    }
    out
}

// ---- writes: each works on a copy, kept only when nothing was refused ----

fn commit<F>(plans: &mut CaptainPlans, write: F) -> Result<String, String>
where
    F: FnOnce(&mut CaptainPlans) -> Result<String, String>,
{
    let mut next = plans.clone();
    let answer = write(&mut next)?;
    *plans = next;
    Ok(answer)
}

enum Target {
    New,
    Existing(usize),
}

fn set_on(
    plans: &mut CaptainPlans,
    call: SetCall,
    asked_at: u64,
    facts: &[TaskFacts],
    now: u64,
) -> Result<String, String> {
    let title = call
        .title
        .as_deref()
        .map(|t| one_line("title", t, MAX_TITLE, false))
        .transpose()?;
    let inputs = call
        .lists
        .iter()
        .map(list_in)
        .collect::<Result<Vec<_>, _>>()?;
    let pi = match pick(plans, call.plan.as_deref(), call.new)? {
        Target::New => {
            plans.next.plan += 1;
            let id = format!("p{}", plans.next.plan);
            plans.plans.push(Plan {
                id,
                rev: 0,
                title: String::new(),
                asked_at,
                finished_at: None,
                closed: None,
                lists: Vec::new(),
            });
            plans.plans.len() - 1
        }
        Target::Existing(i) => i,
    };
    let plan_id = plans.plans[pi].id.clone();
    let old = plans.plans[pi].lists.clone();
    let foreign = foreign_ids(plans, &plan_id);
    let (lists, kept) = merge_lists(&old, &inputs, &mut plans.next, &plan_id, &foreign)?;
    check_limits(&lists)?;

    let plan = &mut plans.plans[pi];
    plan.lists = lists;
    if let Some(t) = title {
        plan.title = t;
    }
    plan.rev += 1;
    settle_plan(plan, facts, now);
    tidy(plans)?;

    let mut text = answer_of(plans, &plan_id, facts);
    if !kept.is_empty() {
        let word = if kept.len() == 1 { "step" } else { "steps" };
        text.push_str(&format!(
            "\nkept {} started {} you left out: {}.",
            kept.len(),
            word,
            kept.join(", ")
        ));
    }
    Ok(text)
}

fn step_on(
    plans: &mut CaptainPlans,
    call: StepCall,
    facts: &[TaskFacts],
    now: u64,
) -> Result<String, String> {
    let note = note_field(call.note.as_deref())?;
    let pi = plan_with_step(plans, &call.step)
        .ok_or_else(|| format!("no step {}. Nothing changed.", call.step))?;
    let plan_id = plans.plans[pi].id.clone();
    if plans.plans[pi].closed.is_some() {
        return Err(format!(
            "{plan_id} is closed, so its steps are frozen. Nothing changed."
        ));
    }
    if let Some(task) = holding_task(&plans.plans[pi], &call.step, facts) {
        return Err(format!(
            "{} follows task {task}, which has not ended. Wait for it, or cancel {task} first. Nothing changed.",
            call.step
        ));
    }
    let plan = &mut plans.plans[pi];
    // The same report again changes nothing, unless being newer lets it win over a task that ended.
    if find_step(&plan.lists, &call.step).is_some_and(|s| s.reported == call.state && s.note == note) {
        let mut again = plan.clone();
        if let Some(s) = find_step_mut(&mut again.lists, &call.step) {
            s.reported_at = now;
        }
        if derive(&again, facts) == derive(plan, facts) {
            return Ok(answer_of(plans, &plan_id, facts));
        }
    }
    let Some(s) = find_step_mut(&mut plan.lists, &call.step) else {
        return Err(format!("no step {}. Nothing changed.", call.step));
    };
    s.reported = call.state;
    s.reported_at = now;
    s.note = note;
    plan.rev += 1;
    settle_plan(plan, facts, now);
    tidy(plans)?;
    Ok(answer_of(plans, &plan_id, facts))
}

fn fix_on(
    plans: &mut CaptainPlans,
    call: FixCall,
    facts: &[TaskFacts],
    now: u64,
) -> Result<String, String> {
    let pi = plan_with_step(plans, &call.step)
        .ok_or_else(|| format!("no step {}. Nothing changed.", call.step))?;
    let plan_id = plans.plans[pi].id.clone();
    if plans.plans[pi].closed.is_some() {
        return Err(format!(
            "{plan_id} is closed, so its steps are frozen. Nothing changed."
        ));
    }
    let view = derive(&plans.plans[pi], facts);
    let state = every_step(&view)
        .into_iter()
        .find(|s| s.id == call.step)
        .map_or(State::Pending, |s| s.state);
    if state != State::Failed {
        return Err(format!(
            "{} is not failed (it reads {}), so it cannot get a fix. Nothing changed.",
            call.step,
            state_name(state)
        ));
    }
    if call.steps.is_empty() {
        return Err("a fix needs at least one step. Nothing changed.".into());
    }
    if call.steps.len() > MAX_STEPS_PER_FIX {
        return Err(format!(
            "limit: {MAX_STEPS_PER_FIX} steps per fix. Nothing changed."
        ));
    }
    let mut news = Vec::new();
    for s in &call.steps {
        news.push(StepIn {
            id: None,
            name: one_line("name", &s.name, MAX_NAME, true)?,
            what: text_field("what", s.what.as_deref().unwrap_or(""), MAX_WHAT)?,
        });
    }

    let plan = &mut plans.plans[pi];
    if count_all(&plan.lists) + news.len() > MAX_STEPS_IN_ALL {
        return Err(format!(
            "limit: {MAX_STEPS_IN_ALL} steps in all, fixes included. Nothing changed."
        ));
    }
    let Some(s) = find_step_mut(&mut plan.lists, &call.step) else {
        return Err(format!("no step {}. Nothing changed.", call.step));
    };
    if s.fixes.len() >= MAX_FIXES_PER_STEP {
        return Err(format!(
            "limit: {MAX_FIXES_PER_STEP} fixes per step. Nothing changed."
        ));
    }
    plans.next.fix += 1;
    let fix_id = format!("f{}", plans.next.fix);
    let mut steps = Vec::new();
    for n in news {
        plans.next.step += 1;
        steps.push(new_step(format!("s{}", plans.next.step), n.name, n.what));
    }
    let ids: Vec<String> = steps.iter().map(|s| s.id.clone()).collect();
    s.fixes.push(Fix {
        id: fix_id.clone(),
        steps,
    });
    plan.rev += 1;
    settle_plan(plan, facts, now);
    tidy(plans)?;
    Ok(format!(
        "Fix {fix_id} on {} has steps {}.\n{}",
        call.step,
        ids.join(", "),
        answer_of(plans, &plan_id, facts)
    ))
}

fn close_on(
    plans: &mut CaptainPlans,
    call: CloseCall,
    facts: &[TaskFacts],
    now: u64,
) -> Result<String, String> {
    let reason = text_field("reason", &call.reason, MAX_REASON)?;
    if reason.is_empty() {
        return Err("reason is empty. Nothing changed.".into());
    }
    let pi = plans
        .plans
        .iter()
        .position(|p| p.id == call.plan)
        .ok_or_else(|| format!("no plan {}. Nothing changed.", call.plan))?;
    let plan = &plans.plans[pi];
    if plan.closed.is_some() {
        return Err(format!("{} is already closed. Nothing changed.", plan.id));
    }
    for task in linked_tasks(plan) {
        if unfinished(&task, facts).is_some() {
            return Err(format!(
                "cannot close {} yet: cancel {task} first, or wait for it. Nothing changed.",
                plan.id
            ));
        }
    }
    let plan_id = plan.id.clone();
    let plan = &mut plans.plans[pi];
    plan.closed = Some(reason);
    plan.finished_at = Some(now);
    plan.rev += 1;
    tidy(plans)?;
    Ok(format!(
        "Closed {plan_id}.\n{}",
        answer_of(plans, &plan_id, facts)
    ))
}

/// Which plan a `set` writes to. Refused when the call names two plans or a closed one.
fn pick(plans: &CaptainPlans, plan: Option<&str>, new: bool) -> Result<Target, String> {
    match (plan, new) {
        (Some(_), true) => Err("give plan or new, not both. Nothing changed.".into()),
        (Some(id), false) => {
            let i = plans
                .plans
                .iter()
                .position(|p| p.id == id)
                .ok_or_else(|| format!("no plan {id}. Nothing changed."))?;
            if plans.plans[i].closed.is_some() {
                return Err(format!("{id} is closed. Nothing changed."));
            }
            Ok(Target::Existing(i))
        }
        (None, true) => Ok(Target::New),
        (None, false) => {
            let open: Vec<&Plan> = plans
                .plans
                .iter()
                .filter(|p| p.finished_at.is_none())
                .collect();
            match open.as_slice() {
                [] => Ok(Target::New),
                [only] => Ok(Target::Existing(
                    plans
                        .plans
                        .iter()
                        .position(|p| p.id == only.id)
                        .unwrap_or(0),
                )),
                many => {
                    let names: Vec<String> = many
                        .iter()
                        .map(|p| {
                            format!(
                                "{} ({})",
                                p.id,
                                if p.title.is_empty() {
                                    "no title"
                                } else {
                                    p.title.as_str()
                                }
                            )
                        })
                        .collect();
                    Err(format!(
                        "say which plan: {}. Nothing changed.",
                        names.join(", ")
                    ))
                }
            }
        }
    }
}

/// Merges the call's lists into the plan's lists. Lists match by id, else by exact name. Started lists and steps
/// the call leaves out are kept. Returns the new lists and the ids of the kept steps.
fn merge_lists(
    old: &[List],
    inputs: &[ListIn],
    next: &mut Counters,
    plan_id: &str,
    foreign: &HashMap<String, String>,
) -> Result<(Vec<List>, Vec<String>), String> {
    let own = own_steps(old);
    let mut taken = vec![false; old.len()];
    let mut matched: Vec<Option<usize>> = vec![None; inputs.len()];
    for (i, inp) in inputs.iter().enumerate() {
        let Some(id) = &inp.id else { continue };
        let Some(j) = old.iter().position(|l| &l.id == id) else {
            return Err(foreign_refusal(foreign, id)
                .unwrap_or_else(|| format!("no list {id} in plan {plan_id}. Nothing changed.")));
        };
        if taken[j] {
            return Err(format!("{id} is listed twice. Nothing changed."));
        }
        taken[j] = true;
        matched[i] = Some(j);
    }
    for (i, inp) in inputs.iter().enumerate() {
        if inp.id.is_none() {
            if let Some(j) = (0..old.len()).find(|&j| !taken[j] && old[j].name == inp.name) {
                taken[j] = true;
                matched[i] = Some(j);
            }
        }
    }

    let mut lists: Vec<(String, List)> = Vec::new();
    let mut order_ids: Vec<String> = Vec::new();
    let mut kept_steps: Vec<String> = Vec::new();
    for (i, inp) in inputs.iter().enumerate() {
        let base = matched[i].map(|j| &old[j]);
        let id = match base {
            Some(l) => l.id.clone(),
            None => {
                next.list += 1;
                format!("l{}", next.list)
            }
        };
        let (steps, kept) = merge_steps(
            base.map_or(&[][..], |l| l.steps.as_slice()),
            &inp.steps,
            &id,
            next,
            &own,
            foreign,
        )?;
        kept_steps.extend(kept);
        order_ids.push(id.clone());
        let task = base.and_then(|l| l.task.clone());
        lists.push((
            id.clone(),
            List {
                id,
                name: inp.name.clone(),
                task,
                steps,
            },
        ));
    }

    let old_ids: Vec<String> = old.iter().map(|l| l.id.clone()).collect();
    let mut kept_lists: Vec<String> = Vec::new();
    for (j, l) in old.iter().enumerate() {
        if taken[j] {
            continue;
        }
        let (steps, kept) = merge_steps(&l.steps, &[], &l.id, next, &own, foreign)?;
        if l.task.is_none() && steps.is_empty() {
            continue;
        }
        kept_steps.extend(kept);
        kept_lists.push(l.id.clone());
        lists.push((
            l.id.clone(),
            List {
                id: l.id.clone(),
                name: l.name.clone(),
                task: l.task.clone(),
                steps,
            },
        ));
    }
    place_kept(&mut order_ids, &old_ids, &kept_lists);

    let mut pool: HashMap<String, List> = lists.into_iter().collect();
    let merged: Vec<List> = order_ids.iter().filter_map(|id| pool.remove(id)).collect();
    kept_steps.sort_by_key(|id| num(id));
    Ok((merged, kept_steps))
}

/// Merges one list's steps. Steps match by id within this list, else by exact name. A started step the call
/// leaves out is kept; a pending one goes. Returns the steps and the ids of the kept ones.
fn merge_steps(
    old: &[Step],
    inputs: &[StepIn],
    list_id: &str,
    next: &mut Counters,
    own: &HashMap<String, Option<String>>,
    foreign: &HashMap<String, String>,
) -> Result<(Vec<Step>, Vec<String>), String> {
    let mut taken = vec![false; old.len()];
    let mut matched: Vec<Option<usize>> = vec![None; inputs.len()];
    for (i, inp) in inputs.iter().enumerate() {
        let Some(id) = &inp.id else { continue };
        match own.get(id) {
            Some(None) => {
                return Err(format!(
                    "{id} is a fix step, not a list step. Nothing changed."
                ))
            }
            Some(Some(owner)) if owner.as_str() != list_id => {
                return Err(format!(
                    "{id} is in list {owner}, not in this list. Nothing changed."
                ));
            }
            Some(Some(_)) => {}
            None => {
                return Err(foreign_refusal(foreign, id).unwrap_or_else(|| {
                    format!("no step {id} in list {list_id}. Nothing changed.")
                }));
            }
        }
        let Some(j) = old.iter().position(|s| &s.id == id) else {
            return Err(format!("no step {id} in list {list_id}. Nothing changed."));
        };
        if taken[j] {
            return Err(format!("{id} is listed twice. Nothing changed."));
        }
        taken[j] = true;
        matched[i] = Some(j);
    }
    for (i, inp) in inputs.iter().enumerate() {
        if inp.id.is_none() {
            if let Some(j) = (0..old.len()).find(|&j| !taken[j] && old[j].name == inp.name) {
                taken[j] = true;
                matched[i] = Some(j);
            }
        }
    }

    let mut ids: Vec<String> = Vec::new();
    let mut pool: HashMap<String, Step> = HashMap::new();
    for (i, inp) in inputs.iter().enumerate() {
        let step = match matched[i] {
            Some(j) => {
                let o = &old[j];
                Step {
                    id: o.id.clone(),
                    name: inp.name.clone(),
                    what: inp.what.clone(),
                    reported: o.reported,
                    reported_at: o.reported_at,
                    note: o.note.clone(),
                    task: o.task.clone(),
                    cost_at_start: o.cost_at_start,
                    fixes: o.fixes.clone(),
                }
            }
            None => {
                next.step += 1;
                new_step(
                    format!("s{}", next.step),
                    inp.name.clone(),
                    inp.what.clone(),
                )
            }
        };
        ids.push(step.id.clone());
        pool.insert(step.id.clone(), step);
    }
    let mut kept: Vec<String> = Vec::new();
    for (j, o) in old.iter().enumerate() {
        if !taken[j] && started(o) {
            kept.push(o.id.clone());
            pool.insert(o.id.clone(), o.clone());
        }
    }
    let old_ids: Vec<String> = old.iter().map(|s| s.id.clone()).collect();
    place_kept(&mut ids, &old_ids, &kept);
    kept.sort_by_key(|id| num(id));
    let steps: Vec<Step> = ids.iter().filter_map(|id| pool.remove(id)).collect();
    Ok((steps, kept))
}

/// Puts each kept id back into `order` after the nearest earlier old id that is still in `order`.
fn place_kept(order: &mut Vec<String>, old_ids: &[String], kept: &[String]) {
    for (i, id) in old_ids.iter().enumerate() {
        if !kept.contains(id) {
            continue;
        }
        let mut pos = 0;
        for j in (0..i).rev() {
            if let Some(p) = order.iter().position(|x| x == &old_ids[j]) {
                pos = p + 1;
                break;
            }
        }
        while pos < order.len() {
            match old_ids.iter().position(|x| x == &order[pos]) {
                Some(k) if k < i && kept.contains(&order[pos]) => pos += 1,
                _ => break,
            }
        }
        order.insert(pos, id.clone());
    }
}

/// A step the Captain has started or that a task drives: it is kept when a call leaves it out.
fn started(s: &Step) -> bool {
    s.reported != State::Pending || s.task.is_some() || !s.fixes.is_empty()
}

fn new_step(id: String, name: String, what: String) -> Step {
    Step {
        id,
        name,
        what,
        reported: State::Pending,
        reported_at: 0,
        note: None,
        task: None,
        cost_at_start: None,
        fixes: Vec::new(),
    }
}

/// The step ids that belong to this plan, with the list each main step is in. Fix steps map to None.
fn own_steps(lists: &[List]) -> HashMap<String, Option<String>> {
    fn add(s: &Step, list: Option<&str>, out: &mut HashMap<String, Option<String>>) {
        out.insert(s.id.clone(), list.map(String::from));
        for f in &s.fixes {
            for x in &f.steps {
                add(x, None, out);
            }
        }
    }
    let mut out = HashMap::new();
    for l in lists {
        for s in &l.steps {
            add(s, Some(&l.id), &mut out);
        }
    }
    out
}

/// The ids of every list and step (fix steps included) of the other plans, with the plan that holds each.
fn foreign_ids(plans: &CaptainPlans, plan_id: &str) -> HashMap<String, String> {
    fn add(steps: &[Step], plan: &str, out: &mut HashMap<String, String>) {
        for s in steps {
            out.insert(s.id.clone(), plan.to_string());
            for f in &s.fixes {
                add(&f.steps, plan, out);
            }
        }
    }
    let mut out = HashMap::new();
    for p in plans.plans.iter().filter(|p| p.id != plan_id) {
        for l in &p.lists {
            out.insert(l.id.clone(), p.id.clone());
            add(&l.steps, &p.id, &mut out);
        }
    }
    out
}

fn foreign_refusal(foreign: &HashMap<String, String>, id: &str) -> Option<String> {
    foreign
        .get(id)
        .map(|p| format!("{id} is in plan {p}. Nothing changed."))
}

fn check_limits(lists: &[List]) -> Result<(), String> {
    if lists.len() > MAX_LISTS {
        return Err(format!(
            "limit: {MAX_LISTS} lists per plan. Nothing changed."
        ));
    }
    if lists.iter().any(|l| l.steps.len() > MAX_STEPS_PER_LIST) {
        return Err(format!(
            "limit: {MAX_STEPS_PER_LIST} steps per list. Nothing changed."
        ));
    }
    if count_all(lists) > MAX_STEPS_IN_ALL {
        return Err(format!(
            "limit: {MAX_STEPS_IN_ALL} steps in all, fixes included. Nothing changed."
        ));
    }
    Ok(())
}

/// Counts main steps and fix steps, at any depth.
fn count_all(lists: &[List]) -> usize {
    fn count(steps: &[Step]) -> usize {
        steps
            .iter()
            .map(|s| 1 + s.fixes.iter().map(|f| count(&f.steps)).sum::<usize>())
            .sum()
    }
    lists.iter().map(|l| count(&l.steps)).sum()
}

/// Keeps the store within its bounds: finished plans newest first, at most ten; unfinished at most six.
fn tidy(plans: &mut CaptainPlans) -> Result<(), String> {
    order(plans);
    let mut finished_seen = 0;
    plans.plans.retain(|p| {
        if p.finished_at.is_none() {
            return true;
        }
        finished_seen += 1;
        finished_seen <= MAX_PLANS_FINISHED
    });
    if plans
        .plans
        .iter()
        .filter(|p| p.finished_at.is_none())
        .count()
        > MAX_PLANS_UNFINISHED
    {
        return Err(format!("limit: {MAX_PLANS_UNFINISHED} unfinished plans. Finish or close one first. Nothing changed."));
    }
    Ok(())
}

/// Unfinished plans first, oldest first; then finished and closed plans, newest first.
fn order(plans: &mut CaptainPlans) {
    plans
        .plans
        .sort_by(|a, b| match (a.finished_at, b.finished_at) {
            (None, None) => num(&a.id).cmp(&num(&b.id)),
            (Some(x), Some(y)) => y.cmp(&x).then(num(&b.id).cmp(&num(&a.id))),
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
        });
}

fn num(id: &str) -> u64 {
    id.get(1..).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Records the start cost of each step that a list task has started, and sets or clears the finish time.
fn settle_plan(plan: &mut Plan, facts: &[TaskFacts], now: u64) -> bool {
    if plan.closed.is_some() {
        return false;
    }
    let mut changed = false;
    for list in plan.lists.iter_mut() {
        let Some(f) = list.task.as_deref().and_then(|id| fact(facts, id)) else {
            continue;
        };
        if is_terminal(f) {
            continue;
        }
        let (Some(k), Some(cost)) = (first_open(list), f.cost) else {
            continue;
        };
        let s = &mut list.steps[k];
        if s.cost_at_start.is_none() {
            s.cost_at_start = Some(cost);
            changed = true;
        }
    }
    let done = all_main_done(plan, facts);
    match (done, plan.finished_at) {
        (true, None) => {
            plan.finished_at = Some(now);
            changed = true;
        }
        (false, Some(_)) => {
            plan.finished_at = None;
            changed = true;
        }
        _ => {}
    }
    changed
}

fn all_main_done(plan: &Plan, facts: &[TaskFacts]) -> bool {
    let view = derive(plan, facts);
    let mains: Vec<&StepView> = view.lists.iter().flat_map(|l| l.steps.iter()).collect();
    !mains.is_empty() && mains.iter().all(|s| s.state == State::Done)
}

// ---- derive ----

/// A step's state, reason and cost before fixes are applied.
#[derive(Clone, Debug)]
struct Shown {
    state: State,
    why: Option<String>,
    cost: Option<f64>,
}

fn fact<'a>(facts: &'a [TaskFacts], id: &str) -> Option<&'a TaskFacts> {
    facts.iter().find(|f| f.id == id)
}

fn is_terminal(f: &TaskFacts) -> bool {
    matches!(
        f.work,
        Work::ResultAvailable | Work::Failed | Work::Cancelled
    )
}

/// The task id, if the facts say it has not ended.
fn unfinished(id: &str, facts: &[TaskFacts]) -> Option<String> {
    fact(facts, id)
        .filter(|f| !is_terminal(f))
        .map(|_| id.to_string())
}

fn ends_at(f: &TaskFacts) -> u64 {
    f.ended_at.unwrap_or(0)
}

/// A step that runs on its own task, or a step with no task, which reads what the Captain reported.
fn own_shown(step: &Step, facts: &[TaskFacts]) -> Shown {
    let plain = Shown {
        state: step.reported,
        why: None,
        cost: None,
    };
    let Some(f) = step.task.as_deref().and_then(|id| fact(facts, id)) else {
        return plain;
    };
    let cost = f.cost;
    if !is_terminal(f) {
        let state = if f.waits_on_user {
            State::Waiting
        } else {
            State::Running
        };
        return Shown {
            state,
            why: None,
            cost,
        };
    }
    if step.reported_at > ends_at(f) {
        return Shown { cost, ..plain };
    }
    match f.work {
        Work::ResultAvailable if f.review == Review::Redone => Shown {
            state: State::Failed,
            why: Some("Redone".into()),
            cost,
        },
        Work::ResultAvailable => Shown {
            state: State::Done,
            why: None,
            cost,
        },
        Work::Failed => Shown {
            state: State::Failed,
            why: f.failure.clone(),
            cost,
        },
        // The only terminal work left is a cancelled task.
        _ => Shown {
            state: State::Failed,
            why: Some("Cancelled".into()),
            cost,
        },
    }
}

/// The first step of a list with no task of its own that is not done: the one a list task drives.
fn first_open(list: &List) -> Option<usize> {
    list.steps
        .iter()
        .position(|s| s.task.is_none() && s.reported != State::Done)
}

/// Every step of a list, as the list's task drives it. A step with its own task follows that task instead.
fn list_shown(list: &List, facts: &[TaskFacts]) -> Vec<Shown> {
    let lf = list.task.as_deref().and_then(|id| fact(facts, id));
    let open = first_open(list);
    list.steps
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let Some(f) = lf.filter(|_| s.task.is_none()) else {
                return own_shown(s, facts);
            };
            let running = Some(k) == open;
            let cost = match (is_terminal(f) || running, s.cost_at_start, f.cost) {
                (true, Some(a), Some(b)) => Some(b - a),
                _ => None,
            };
            let plain = Shown {
                state: s.reported,
                why: None,
                cost,
            };
            if !is_terminal(f) {
                if !running {
                    return plain;
                }
                let state = if f.waits_on_user {
                    State::Waiting
                } else {
                    State::Running
                };
                return Shown {
                    state,
                    why: None,
                    cost,
                };
            }
            if s.reported_at > ends_at(f) {
                return plain;
            }
            match f.work {
                Work::ResultAvailable if s.reported == State::Done => Shown {
                    state: State::Done,
                    why: None,
                    cost,
                },
                Work::ResultAvailable if f.review == Review::Redone => Shown {
                    state: State::Failed,
                    why: Some("Redone".into()),
                    cost,
                },
                Work::ResultAvailable => Shown {
                    state: State::Done,
                    why: Some(DONE_WITH_LIST.into()),
                    cost,
                },
                _ if running => {
                    let why = match f.work {
                        Work::Failed => f.failure.clone(),
                        _ => Some("Cancelled".into()),
                    };
                    Shown {
                        state: State::Failed,
                        why,
                        cost,
                    }
                }
                _ => plain,
            }
        })
        .collect()
}

fn step_view(step: &Step, shown: Shown, facts: &[TaskFacts]) -> StepView {
    let fixes: Vec<FixView> = step
        .fixes
        .iter()
        .map(|f| FixView {
            id: f.id.clone(),
            steps: f
                .steps
                .iter()
                .map(|s| step_view(s, own_shown(s, facts), facts))
                .collect(),
        })
        .collect();
    let mut state = shown.state;
    let mut why = shown.why;
    if state == State::Failed {
        if let Some(last) = fixes.last() {
            if !last.steps.is_empty() && last.steps.iter().all(|s| s.state == State::Done) {
                state = State::Done;
                why = Some(after_fixes(fixes.len()));
            }
        }
    }
    StepView {
        id: step.id.clone(),
        name: step.name.clone(),
        what: step.what.clone(),
        state,
        why,
        cost: shown.cost,
        task: step.task.clone(),
        fixes,
    }
}

fn after_fixes(n: usize) -> String {
    if n == 1 {
        "after one fix".into()
    } else {
        format!("after {n} fixes")
    }
}

/// The task that holds a step back from a report: its own link, or its list's task while the step is the one
/// the list's task drives. None when the step is free to be reported.
fn holding_task(plan: &Plan, step_id: &str, facts: &[TaskFacts]) -> Option<String> {
    for list in &plan.lists {
        if let Some(k) = list.steps.iter().position(|s| s.id == step_id) {
            let s = &list.steps[k];
            if let Some(t) = s.task.as_deref() {
                return unfinished(t, facts);
            }
            if first_open(list) == Some(k) {
                return list.task.as_deref().and_then(|t| unfinished(t, facts));
            }
            return None;
        }
    }
    find_step(&plan.lists, step_id)?
        .task
        .as_deref()
        .and_then(|t| unfinished(t, facts))
}

fn captain_state(plan: &Plan, view: &PlanView) -> CaptainState {
    if plan.closed.is_some() {
        return CaptainState::Closed;
    }
    if plan.finished_at.is_some() {
        return CaptainState::Finished;
    }
    let all = every_step(view);
    if all.iter().any(|s| s.state == State::Waiting) {
        CaptainState::NeedsYou
    } else if view
        .lists
        .iter()
        .any(|l| l.steps.iter().any(|s| s.state == State::Failed))
    {
        CaptainState::Fixing
    } else if all.iter().any(|s| s.state == State::Running) {
        CaptainState::Working
    } else {
        CaptainState::Idle
    }
}

/// The first step, in plan order, that is running or waiting. A fix comes right after the step it fixes.
fn first_active(steps: &[StepView]) -> Option<&StepView> {
    for s in steps {
        if matches!(s.state, State::Running | State::Waiting) {
            return Some(s);
        }
        for f in &s.fixes {
            if let Some(x) = first_active(&f.steps) {
                return Some(x);
            }
        }
    }
    None
}

fn every_step(view: &PlanView) -> Vec<&StepView> {
    fn walk<'a>(steps: &'a [StepView], out: &mut Vec<&'a StepView>) {
        for s in steps {
            out.push(s);
            for f in &s.fixes {
                walk(&f.steps, out);
            }
        }
    }
    let mut out = Vec::new();
    for l in &view.lists {
        walk(&l.steps, &mut out);
    }
    out
}

/// One dot per main step: d done, x done after a fix, r running, f failed, w waiting, p pending.
fn dot(s: &StepView) -> char {
    match s.state {
        State::Done if s.why.as_deref().is_some_and(|w| w.starts_with("after ")) => 'x',
        State::Done => 'd',
        State::Running => 'r',
        State::Failed => 'f',
        State::Waiting => 'w',
        State::Pending => 'p',
    }
}

fn clip(text: &str) -> String {
    text.chars().take(GLANCE_TEXT).collect()
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Pending => "pending",
        State::Running => "running",
        State::Waiting => "waiting",
        State::Done => "done",
        State::Failed => "failed",
    }
}

/// The plan's text, headed with its id: one line per list, then one per fix.
fn answer_of(plans: &CaptainPlans, plan_id: &str, facts: &[TaskFacts]) -> String {
    let Some(plan) = plans.plans.iter().find(|p| p.id == plan_id) else {
        return String::new();
    };
    let view = derive(plan, facts);
    let mut lines = vec![if plan.title.is_empty() {
        format!("{} rev {}", plan.id, plan.rev)
    } else {
        format!("{} rev {}: {}", plan.id, plan.rev, plan.title)
    }];
    for l in &view.lists {
        let body: Vec<String> = l.steps.iter().map(word).collect();
        if body.is_empty() {
            lines.push(format!("{} {}:", l.id, l.name));
        } else {
            lines.push(format!("{} {}: {}", l.id, l.name, body.join(", ")));
        }
    }
    fn fixes(steps: &[StepView], lines: &mut Vec<String>) {
        for s in steps {
            for f in &s.fixes {
                let body: Vec<String> = f.steps.iter().map(word).collect();
                lines.push(format!("{} on {}: {}", f.id, s.id, body.join(", ")));
                fixes(&f.steps, lines);
            }
        }
    }
    for l in &view.lists {
        fixes(&l.steps, &mut lines);
    }
    lines.join("\n")
}

fn word(s: &StepView) -> String {
    match &s.why {
        Some(w) => format!("{} {} ({w})", s.id, state_name(s.state)),
        None => format!("{} {}", s.id, state_name(s.state)),
    }
}

fn linked_tasks(plan: &Plan) -> Vec<String> {
    fn walk(steps: &[Step], out: &mut Vec<String>) {
        for s in steps {
            out.extend(s.task.clone());
            for f in &s.fixes {
                walk(&f.steps, out);
            }
        }
    }
    let mut out = Vec::new();
    for l in &plan.lists {
        out.extend(l.task.clone());
        walk(&l.steps, &mut out);
    }
    out
}

fn plan_with_step(plans: &CaptainPlans, step_id: &str) -> Option<usize> {
    plans
        .plans
        .iter()
        .position(|p| find_step(&p.lists, step_id).is_some())
}

fn find_step<'a>(lists: &'a [List], id: &str) -> Option<&'a Step> {
    fn within<'a>(s: &'a Step, id: &str) -> Option<&'a Step> {
        if s.id == id {
            return Some(s);
        }
        s.fixes
            .iter()
            .flat_map(|f| f.steps.iter())
            .find_map(|x| within(x, id))
    }
    lists
        .iter()
        .flat_map(|l| l.steps.iter())
        .find_map(|s| within(s, id))
}

fn find_step_mut<'a>(lists: &'a mut [List], id: &str) -> Option<&'a mut Step> {
    fn within<'a>(s: &'a mut Step, id: &str) -> Option<&'a mut Step> {
        if s.id == id {
            return Some(s);
        }
        for f in s.fixes.iter_mut() {
            for x in f.steps.iter_mut() {
                if let Some(found) = within(x, id) {
                    return Some(found);
                }
            }
        }
        None
    }
    for l in lists.iter_mut() {
        for s in l.steps.iter_mut() {
            if let Some(found) = within(s, id) {
                return Some(found);
            }
        }
    }
    None
}

/// A `set` list after its text is checked. `id` is trimmed; a blank id counts as none.
struct ListIn {
    id: Option<String>,
    name: String,
    steps: Vec<StepIn>,
}

struct StepIn {
    id: Option<String>,
    name: String,
    what: String,
}

fn list_in(l: &ListInput) -> Result<ListIn, String> {
    Ok(ListIn {
        id: clean_id(&l.id),
        name: one_line("name", &l.name, MAX_NAME, true)?,
        steps: l.steps.iter().map(step_in).collect::<Result<_, _>>()?,
    })
}

fn step_in(s: &StepInput) -> Result<StepIn, String> {
    Ok(StepIn {
        id: clean_id(&s.id),
        name: one_line("name", &s.name, MAX_NAME, true)?,
        what: text_field("what", s.what.as_deref().unwrap_or(""), MAX_WHAT)?,
    })
}

fn clean_id(id: &Option<String>) -> Option<String> {
    id.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// U+2028 and U+2029 break a line where `char::is_control` sees none.
fn line_separator(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}')
}

/// Embeddings, overrides, isolates and marks that make text read in another order than it is stored.
fn direction_mark(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// Characters that show nothing, so two names that read the same can differ.
fn invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
}

/// A name or title: one line, no control, direction or invisible characters, counted in
/// characters after trimming.
fn one_line(field: &str, raw: &str, max: usize, required: bool) -> Result<String, String> {
    if raw.chars().any(|c| c.is_control() || line_separator(c)) {
        return Err(format!(
            "{field} may not contain a line break or other control character. Nothing changed."
        ));
    }
    if raw.chars().any(direction_mark) {
        return Err(format!("{field} may not contain a character that changes the text's direction. Nothing changed."));
    }
    if raw.chars().any(invisible) {
        return Err(format!("{field} may not contain an invisible character. Nothing changed."));
    }
    let text = raw.trim();
    if text.is_empty() && required {
        return Err(format!("{field} is empty. Nothing changed."));
    }
    if text.chars().count() > max {
        return Err(format!(
            "limit: {max} characters for {field}. Nothing changed."
        ));
    }
    Ok(text.to_string())
}

/// Free text (what, note, reason): `\n` line breaks allowed, other control characters, line
/// separators and direction marks not, counted after trimming.
fn text_field(field: &str, raw: &str, max: usize) -> Result<String, String> {
    if raw.chars().any(|c| c.is_control() && c != '\n') {
        return Err(format!(
            "{field} may hold line breaks but no other control character. Nothing changed."
        ));
    }
    if raw.chars().any(line_separator) {
        return Err(format!("{field} may break lines only with a plain line break. Nothing changed."));
    }
    if raw.chars().any(direction_mark) {
        return Err(format!("{field} may not contain a character that changes the text's direction. Nothing changed."));
    }
    let text = raw.trim();
    if text.chars().count() > max {
        return Err(format!(
            "limit: {max} characters for {field}. Nothing changed."
        ));
    }
    Ok(text.to_string())
}

fn note_field(raw: Option<&str>) -> Result<Option<String>, String> {
    let text = text_field("note", raw.unwrap_or(""), MAX_NOTE)?;
    Ok(if text.is_empty() { None } else { Some(text) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// The asked_at every test passes to `set`.
    const ASKED: u64 = 1000;

    fn set_json(plans: &mut CaptainPlans, v: Value, now: u64) -> Result<String, String> {
        let call: SetCall = serde_json::from_value(v).expect("set call JSON");
        set(plans, call, ASKED, &[], now)
    }

    fn report(
        plans: &mut CaptainPlans,
        step_id: &str,
        state: &str,
        facts: &[TaskFacts],
        now: u64,
    ) -> Result<String, String> {
        let state: State = serde_json::from_value(json!(state)).expect("state name");
        step(
            plans,
            StepCall {
                step: step_id.into(),
                state,
                note: None,
            },
            facts,
            now,
        )
    }

    fn report_note(
        plans: &mut CaptainPlans,
        step_id: &str,
        state: &str,
        note: &str,
        now: u64,
    ) -> Result<String, String> {
        let state: State = serde_json::from_value(json!(state)).expect("state name");
        step(
            plans,
            StepCall {
                step: step_id.into(),
                state,
                note: Some(note.into()),
            },
            &[],
            now,
        )
    }

    fn fix_on(
        plans: &mut CaptainPlans,
        step_id: &str,
        names: &[&str],
        facts: &[TaskFacts],
        now: u64,
    ) -> Result<String, String> {
        let steps = names
            .iter()
            .map(|n| NewStep {
                name: (*n).into(),
                what: None,
            })
            .collect();
        fix(
            plans,
            FixCall {
                step: step_id.into(),
                steps,
            },
            facts,
            now,
        )
    }

    fn close_on(
        plans: &mut CaptainPlans,
        plan_id: &str,
        reason: &str,
        facts: &[TaskFacts],
        now: u64,
    ) -> Result<String, String> {
        close(
            plans,
            CloseCall {
                plan: plan_id.into(),
                reason: reason.into(),
            },
            facts,
            now,
        )
    }

    fn bytes(plans: &CaptainPlans) -> String {
        serde_json::to_string(plans).unwrap()
    }

    fn plan_of<'a>(plans: &'a CaptainPlans, id: &str) -> &'a Plan {
        plans
            .plans
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("no plan {id}"))
    }

    fn plan_bytes(plans: &CaptainPlans, id: &str) -> String {
        serde_json::to_string(plan_of(plans, id)).unwrap()
    }

    fn plan_ids(plans: &CaptainPlans) -> Vec<String> {
        plans.plans.iter().map(|p| p.id.clone()).collect()
    }

    /// One line per list: "l1 Build: s1 Schema, s2 API".
    fn outline(plans: &CaptainPlans, id: &str) -> String {
        plan_of(plans, id)
            .lists
            .iter()
            .map(|l| {
                let steps: Vec<String> = l
                    .steps
                    .iter()
                    .map(|s| format!("{} {}", s.id, s.name))
                    .collect();
                format!("{} {}: {}", l.id, l.name, steps.join(", "))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A store with one plan p1: one list "Build" with three steps, s1 A, s2 B, s3 C.
    fn three_steps() -> CaptainPlans {
        let mut p = CaptainPlans::default();
        set_json(&mut p, json!({"lists": [{"name": "Build", "steps": [{"name": "A"}, {"name": "B"}, {"name": "C"}]}]}), 1).unwrap();
        p
    }

    /// A store with one plan p1 whose only step, s1, links to the given task.
    fn linked(task_id: &str) -> CaptainPlans {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "Work"}]}]}),
            1,
        )
        .unwrap();
        p.plans[0].lists[0].steps[0].task = Some(task_id.into());
        p
    }

    /// Two plans. p1 "First": Build with s1 Schema (done) and s2 API (failed, fixed by f1 with step s3).
    /// p2 "Second": Ship with s4 Deploy (done) and s5 Check (pending).
    fn two_plans() -> CaptainPlans {
        let mut p = CaptainPlans::default();
        set_json(&mut p, json!({"title": "First", "lists": [{"name": "Build", "steps": [{"name": "Schema"}, {"name": "API"}]}]}), 1)
            .unwrap();
        report(&mut p, "s1", "done", &[], 2).unwrap();
        report(&mut p, "s2", "failed", &[], 3).unwrap();
        fix_on(&mut p, "s2", &["Patch"], &[], 4).unwrap();
        set_json(
            &mut p,
            json!({"new": true, "title": "Second", "lists": [{"name": "Ship", "steps": [{"name": "Deploy"}, {"name": "Check"}]}]}),
            5,
        )
        .unwrap();
        report(&mut p, "s4", "done", &[], 6).unwrap();
        p
    }

    fn task(id: &str, work: Work) -> TaskFacts {
        TaskFacts {
            id: id.into(),
            work,
            waits_on_user: false,
            review: Review::Unreviewed,
            failure: None,
            cost: None,
            ended_at: None,
        }
    }

    fn ended(id: &str, work: Work, at: u64) -> TaskFacts {
        TaskFacts {
            ended_at: Some(at),
            ..task(id, work)
        }
    }

    fn all_steps(view: &PlanView) -> Vec<&StepView> {
        fn walk<'a>(steps: &'a [StepView], out: &mut Vec<&'a StepView>) {
            for s in steps {
                out.push(s);
                for f in &s.fixes {
                    walk(&f.steps, out);
                }
            }
        }
        let mut out = Vec::new();
        for l in &view.lists {
            walk(&l.steps, &mut out);
        }
        out
    }

    fn view_step<'a>(view: &'a PlanView, id: &str) -> &'a StepView {
        all_steps(view)
            .into_iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no step {id} in the view"))
    }

    /// The derived view of step s1 in the only plan.
    fn s1_under(plans: &CaptainPlans, facts: &[TaskFacts]) -> StepView {
        view_step(&derive(&plans.plans[0], facts), "s1").clone()
    }

    fn assert_set_refused(plans: &mut CaptainPlans, v: Value, expected: &str) {
        let before = bytes(plans);
        let err = set_json(plans, v, 9).expect_err("the call should be refused");
        assert_eq!(err, expected);
        assert_eq!(
            bytes(plans),
            before,
            "a refused call must leave the plans unchanged"
        );
    }

    fn assert_fix_refused(plans: &mut CaptainPlans, step_id: &str, names: &[&str], expected: &str) {
        let before = bytes(plans);
        let err = fix_on(plans, step_id, names, &[], 9).expect_err("the fix should be refused");
        assert_eq!(err, expected);
        assert_eq!(
            bytes(plans),
            before,
            "a refused fix must leave the plans unchanged"
        );
    }

    /// One step s1 "Keep", reported failed, in plan p1.
    fn failed_s1() -> CaptainPlans {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "Keep"}]}]}),
            1,
        )
        .unwrap();
        report(&mut p, "s1", "failed", &[], 2).unwrap();
        p
    }

    // ---- ids and set ----

    #[test]
    fn ids_are_short_unique_across_plans_and_never_reused() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [
                {"name": "Build", "steps": [{"name": "Schema"}, {"name": "API"}]},
                {"name": "Ship", "steps": [{"name": "Release"}]}
            ]}),
            1,
        )
        .unwrap();
        assert_eq!(
            outline(&p, "p1"),
            "l1 Build: s1 Schema, s2 API\nl2 Ship: s3 Release"
        );
        assert_eq!(plan_of(&p, "p1").asked_at, 1000);

        set_json(
            &mut p,
            json!({"plan": "p1", "lists": [
                {"name": "Build", "steps": [{"name": "Schema"}, {"name": "Docs"}]},
                {"name": "Ship", "steps": [{"name": "Release"}]}
            ]}),
            2,
        )
        .unwrap();
        assert_eq!(
            outline(&p, "p1"),
            "l1 Build: s1 Schema, s4 Docs\nl2 Ship: s3 Release"
        );

        set_json(
            &mut p,
            json!({"new": true, "lists": [{"name": "Next", "steps": [{"name": "Plan"}]}]}),
            3,
        )
        .unwrap();
        assert_eq!(outline(&p, "p2"), "l3 Next: s5 Plan");
        assert_eq!(
            serde_json::to_value(&p.next).unwrap(),
            json!({"plan": 2, "list": 3, "step": 5, "fix": 0})
        );

        set_json(
            &mut p,
            json!({"plan": "p1", "lists": [
                {"name": "Build", "steps": [{"name": "Schema"}, {"name": "Docs"}, {"name": "Extra"}]},
                {"name": "Ship", "steps": [{"name": "Release"}]}
            ]}),
            4,
        )
        .unwrap();
        assert_eq!(
            outline(&p, "p1"),
            "l1 Build: s1 Schema, s4 Docs, s6 Extra\nl2 Ship: s3 Release",
            "s2 is not reused"
        );
    }

    #[test]
    fn the_answer_lists_each_list_with_its_step_ids_and_states() {
        let mut p = CaptainPlans::default();
        let answer = set_json(
            &mut p,
            json!({"lists": [
                {"name": "Build", "steps": [{"name": "Schema"}, {"name": "API"}]},
                {"name": "Ship", "steps": [{"name": "Release"}]}
            ]}),
            1,
        )
        .unwrap();
        assert_eq!(
            answer,
            "p1 rev 1\nl1 Build: s1 pending, s2 pending\nl2 Ship: s3 pending"
        );
    }

    #[test]
    fn the_answer_headers_a_titled_plan_with_its_title() {
        let mut p = CaptainPlans::default();
        let answer = set_json(
            &mut p,
            json!({"title": "Ship it", "lists": [{"name": "Build", "steps": [{"name": "A"}]}]}),
            1,
        )
        .unwrap();
        assert_eq!(answer, "p1 rev 1: Ship it\nl1 Build: s1 pending");
    }

    #[test]
    fn merge_keeps_a_started_step_after_the_nearest_earlier_step() {
        let mut p = three_steps();
        report(&mut p, "s2", "done", &[], 2).unwrap();
        let answer = set_json(&mut p, json!({"plan": "p1", "lists": [{"name": "Build", "steps": [{"name": "C"}, {"name": "A"}]}]}), 3)
            .unwrap();
        assert_eq!(outline(&p, "p1"), "l1 Build: s3 C, s1 A, s2 B");
        assert_eq!(answer, "p1 rev 3\nl1 Build: s3 pending, s1 pending, s2 done\nkept 1 started step you left out: s2.");
    }

    #[test]
    fn merge_removes_an_omitted_pending_step() {
        let mut p = three_steps();
        let answer = set_json(&mut p, json!({"plan": "p1", "lists": [{"name": "Build", "steps": [{"name": "A"}, {"name": "C"}]}]}), 2)
            .unwrap();
        assert_eq!(outline(&p, "p1"), "l1 Build: s1 A, s3 C");
        assert_eq!(answer, "p1 rev 2\nl1 Build: s1 pending, s3 pending");
    }

    #[test]
    fn merge_matched_steps_keep_id_state_link_and_fixes() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "Schema"}, {"name": "API"}]}]}),
            1,
        )
        .unwrap();
        report(&mut p, "s2", "failed", &[], 2).unwrap();
        fix_on(&mut p, "s2", &["Patch"], &[], 3).unwrap();
        report(&mut p, "s1", "running", &[], 4).unwrap();
        p.plans[0].lists[0].steps[0].task = Some("t1".into());

        set_json(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Build", "steps": [
                {"id": "s2", "name": "API v2", "what": "the new one"},
                {"name": "Schema"}
            ]}]}),
            5,
        )
        .unwrap();

        assert_eq!(outline(&p, "p1"), "l1 Build: s2 API v2, s1 Schema");
        let plan = plan_of(&p, "p1");
        let api = &plan.lists[0].steps[0];
        assert_eq!(api.what, "the new one");
        assert_eq!(api.reported, State::Failed);
        assert_eq!(api.fixes.len(), 1);
        assert_eq!(api.fixes[0].id, "f1");
        let schema = &plan.lists[0].steps[1];
        assert_eq!(schema.task.as_deref(), Some("t1"));
        assert_eq!(schema.reported, State::Running);
    }

    #[test]
    fn a_started_step_named_under_another_list_becomes_a_new_step_there() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "Deploy"}]}]}),
            1,
        )
        .unwrap();
        report(&mut p, "s1", "done", &[], 2).unwrap();

        let answer = set_json(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Test", "steps": [{"name": "Deploy"}]}]}),
            3,
        )
        .unwrap();

        assert_eq!(outline(&p, "p1"), "l1 Build: s1 Deploy\nl2 Test: s2 Deploy");
        assert_eq!(answer, "p1 rev 3\nl1 Build: s1 done\nl2 Test: s2 pending\nkept 1 started step you left out: s1.");
    }

    #[test]
    fn a_started_list_the_call_omits_stays_with_its_started_steps() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [
                {"name": "Build", "steps": [{"name": "A"}, {"name": "B"}]},
                {"name": "Ship", "steps": [{"name": "C"}]}
            ]}),
            1,
        )
        .unwrap();
        report(&mut p, "s2", "done", &[], 2).unwrap();

        let answer = set_json(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Ship", "steps": [{"name": "C"}]}]}),
            3,
        )
        .unwrap();

        assert_eq!(outline(&p, "p1"), "l1 Build: s2 B\nl2 Ship: s3 C");
        assert_eq!(answer, "p1 rev 3\nl1 Build: s2 done\nl2 Ship: s3 pending\nkept 1 started step you left out: s2.");
    }

    #[test]
    fn several_kept_steps_are_listed_together_in_the_answer() {
        let mut p = three_steps();
        report(&mut p, "s1", "done", &[], 2).unwrap();
        report(&mut p, "s3", "running", &[], 3).unwrap();
        let answer = set_json(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Build", "steps": []}]}),
            4,
        )
        .unwrap();
        assert_eq!(outline(&p, "p1"), "l1 Build: s1 A, s3 C");
        assert_eq!(
            answer,
            "p1 rev 4\nl1 Build: s1 done, s3 running\nkept 2 started steps you left out: s1, s3."
        );
    }

    #[test]
    fn title_changes_only_when_the_call_gives_one() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"title": "First", "lists": [{"name": "Build", "steps": [{"name": "A"}]}]}),
            1,
        )
        .unwrap();
        set_json(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Build", "steps": [{"name": "A"}]}]}),
            2,
        )
        .unwrap();
        assert_eq!(plan_of(&p, "p1").title, "First");
        set_json(&mut p, json!({"plan": "p1", "title": "Second", "lists": [{"name": "Build", "steps": [{"name": "A"}]}]}), 3).unwrap();
        assert_eq!(plan_of(&p, "p1").title, "Second");
    }

    #[test]
    fn every_write_bumps_the_rev_of_its_own_plan_only() {
        let mut p = two_plans();
        assert_eq!(plan_of(&p, "p1").rev, 4);
        assert_eq!(plan_of(&p, "p2").rev, 2);
        set_json(&mut p, json!({"plan": "p2", "lists": [{"name": "Ship", "steps": [{"name": "Deploy"}, {"name": "Check"}]}]}), 7)
            .unwrap();
        assert_eq!(plan_of(&p, "p2").rev, 3);
        assert_eq!(
            plan_of(&p, "p1").rev,
            4,
            "a call to p2 leaves the rev of p1 alone"
        );
        // s1 is done already, and the same report again is no write.
        report(&mut p, "s1", "running", &[], 8).unwrap();
        assert_eq!(plan_of(&p, "p1").rev, 5);
        assert_eq!(plan_of(&p, "p2").rev, 3);
    }

    #[test]
    fn a_duplicate_step_id_in_one_call_is_refused() {
        let mut p = three_steps();
        assert_set_refused(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Build", "steps": [{"id": "s1", "name": "A"}, {"id": "s1", "name": "A again"}]}]}),
            "s1 is listed twice. Nothing changed.",
        );
    }

    // ---- several plans ----

    #[test]
    fn new_true_adds_a_plan_and_never_changes_another() {
        let mut p = two_plans();
        let before = plan_bytes(&p, "p1");
        set_json(
            &mut p,
            json!({"new": true, "lists": [{"name": "Next", "steps": [{"name": "X"}]}]}),
            9,
        )
        .unwrap();
        assert_eq!(plan_ids(&p), vec!["p1", "p2", "p3"]);
        assert_eq!(plan_bytes(&p, "p1"), before);
    }

    #[test]
    fn merge_without_a_plan_goes_to_the_only_unfinished_plan() {
        let mut p = two_plans();
        // p2 is finished by closing it, so p1 is the only unfinished plan.
        close_on(&mut p, "p2", "Shipped", &[], 7).unwrap();
        let answer = set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "Schema"}]}]}),
            8,
        )
        .unwrap();
        assert_eq!(answer, "p1 rev 5: First\nl1 Build: s1 done, s2 failed\nf1 on s2: s3 pending\nkept 1 started step you left out: s2.");
        assert_eq!(outline(&p, "p1"), "l1 Build: s1 Schema, s2 API");
    }

    #[test]
    fn merge_without_a_plan_is_refused_with_two_unfinished_plans() {
        let mut p = two_plans();
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "Schema"}]}]}),
            "say which plan: p1 (First), p2 (Second). Nothing changed.",
        );
    }

    #[test]
    fn a_merge_naming_a_step_or_list_of_another_plan_is_refused() {
        let mut p = two_plans();
        assert_set_refused(
            &mut p,
            json!({"plan": "p2", "lists": [{"name": "Build", "steps": [{"id": "s1", "name": "Schema"}]}]}),
            "s1 is in plan p1. Nothing changed.",
        );
        assert_set_refused(
            &mut p,
            json!({"plan": "p2", "lists": [{"id": "l1", "name": "Build", "steps": []}]}),
            "l1 is in plan p1. Nothing changed.",
        );
        assert_set_refused(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Build", "steps": [{"id": "s9", "name": "Nope"}]}]}),
            "no step s9 in list l1. Nothing changed.",
        );
    }

    #[test]
    fn a_closed_plan_refuses_merge_step_fix_and_close() {
        let mut p = two_plans();
        close_on(&mut p, "p2", "Shipped", &[], 7).unwrap();
        let before = bytes(&p);

        assert_eq!(
            set_json(&mut p, json!({"plan": "p2", "lists": []}), 8),
            Err("p2 is closed. Nothing changed.".to_string())
        );
        assert_eq!(
            report(&mut p, "s5", "done", &[], 8),
            Err("p2 is closed, so its steps are frozen. Nothing changed.".to_string())
        );
        assert_eq!(
            fix_on(&mut p, "s5", &["Retry"], &[], 8),
            Err("p2 is closed, so its steps are frozen. Nothing changed.".to_string())
        );
        assert_eq!(
            close_on(&mut p, "p2", "Again", &[], 8),
            Err("p2 is already closed. Nothing changed.".to_string())
        );
        assert_eq!(bytes(&p), before);
    }

    #[test]
    fn set_merge_into_p2_leaves_p1_bytes_equal() {
        let mut p = two_plans();
        let before = plan_bytes(&p, "p1");
        set_json(
            &mut p,
            json!({"plan": "p2", "lists": [{"name": "Ship", "steps": [{"name": "Deploy"}, {"name": "Check"}, {"name": "Ship it"}]}]}),
            7,
        )
        .unwrap();
        assert_eq!(
            outline(&p, "p2"),
            "l2 Ship: s4 Deploy, s5 Check, s6 Ship it"
        );
        assert_eq!(plan_bytes(&p, "p1"), before);
    }

    #[test]
    fn step_on_p2_leaves_p1_bytes_equal() {
        let mut p = two_plans();
        let before = plan_bytes(&p, "p1");
        report(&mut p, "s5", "running", &[], 7).unwrap();
        assert_eq!(plan_of(&p, "p2").lists[0].steps[1].reported, State::Running);
        assert_eq!(plan_bytes(&p, "p1"), before);
    }

    #[test]
    fn fix_on_p2_leaves_p1_bytes_equal() {
        let mut p = two_plans();
        let before = plan_bytes(&p, "p1");
        report(&mut p, "s5", "failed", &[], 7).unwrap();
        fix_on(&mut p, "s5", &["Retry"], &[], 8).unwrap();
        assert_eq!(plan_of(&p, "p2").lists[0].steps[1].fixes.len(), 1);
        assert_eq!(plan_bytes(&p, "p1"), before);
    }

    #[test]
    fn close_p2_leaves_p1_bytes_equal() {
        let mut p = two_plans();
        let before = plan_bytes(&p, "p1");
        close_on(&mut p, "p2", "Shipped", &[], 7).unwrap();
        assert_eq!(plan_of(&p, "p2").closed.as_deref(), Some("Shipped"));
        assert_eq!(plan_bytes(&p, "p1"), before);
    }

    #[test]
    fn new_true_with_a_plan_id_is_refused() {
        let mut p = two_plans();
        assert_set_refused(
            &mut p,
            json!({"new": true, "plan": "p1", "lists": []}),
            "give plan or new, not both. Nothing changed.",
        );
    }

    #[test]
    fn a_merge_into_a_finished_plan_that_adds_a_pending_step_unfinishes_it() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "A"}]}]}),
            1,
        )
        .unwrap();
        report(&mut p, "s1", "done", &[], 2).unwrap();
        assert_eq!(plan_of(&p, "p1").finished_at, Some(2));

        set_json(&mut p, json!({"plan": "p1", "lists": [{"name": "Build", "steps": [{"name": "A"}, {"name": "B"}]}]}), 3).unwrap();
        assert_eq!(plan_of(&p, "p1").finished_at, None);
        assert_eq!(outline(&p, "p1"), "l1 Build: s1 A, s2 B");
    }

    #[test]
    fn a_whole_store_with_two_plans_survives_a_json_round_trip() {
        let p = two_plans();
        let json = serde_json::to_string(&p).unwrap();
        let back: CaptainPlans = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(plan_ids(&back), vec!["p1", "p2"]);
    }

    // ---- close ----

    #[test]
    fn close_is_refused_while_a_linked_task_is_not_terminal_and_names_it() {
        let mut p = linked("t1");
        assert_eq!(
            close_on(&mut p, "p1", "Stopped", &[task("t1", Work::Working)], 5),
            Err(
                "cannot close p1 yet: cancel t1 first, or wait for it. Nothing changed."
                    .to_string()
            )
        );

        let mut list_task = three_steps();
        list_task.plans[0].lists[0].task = Some("t2".into());
        assert_eq!(
            close_on(
                &mut list_task,
                "p1",
                "Stopped",
                &[task("t2", Work::WaitingForChildren)],
                5
            ),
            Err(
                "cannot close p1 yet: cancel t2 first, or wait for it. Nothing changed."
                    .to_string()
            )
        );

        let mut fixed = failed_s1();
        fix_on(&mut fixed, "s1", &["Patch"], &[], 3).unwrap();
        fixed.plans[0].lists[0].steps[0].fixes[0].steps[0].task = Some("t3".into());
        assert_eq!(
            close_on(&mut fixed, "p1", "Stopped", &[task("t3", Work::Working)], 5),
            Err(
                "cannot close p1 yet: cancel t3 first, or wait for it. Nothing changed."
                    .to_string()
            )
        );
    }

    #[test]
    fn close_is_accepted_after_the_task_ends_and_sets_finished_at_and_closed() {
        let mut p = linked("t1");
        let before_close = bytes(&p);
        assert!(close_on(&mut p, "p1", "Stopped", &[task("t1", Work::Working)], 5).is_err());
        assert_eq!(bytes(&p), before_close);

        let answer = close_on(
            &mut p,
            "p1",
            "Stopped by the owner",
            &[ended("t1", Work::Cancelled, 4)],
            5,
        )
        .unwrap();
        assert!(answer.starts_with("Closed p1.\n"), "answer was {answer:?}");
        let plan = plan_of(&p, "p1");
        assert_eq!(plan.closed.as_deref(), Some("Stopped by the owner"));
        assert_eq!(plan.finished_at, Some(5));
        assert_eq!(
            p.plans[0].lists[0].steps[0].reported,
            State::Pending,
            "a closed plan keeps its steps"
        );
    }

    // ---- bounds ----

    #[test]
    fn the_seventh_unfinished_plan_is_refused_and_nothing_changes() {
        let mut p = CaptainPlans::default();
        for i in 1..=6 {
            set_json(&mut p, json!({"new": true, "lists": [{"name": format!("Plan {i}"), "steps": [{"name": "A"}]}]}), i).unwrap();
        }
        let before = bytes(&p);
        let err = set_json(
            &mut p,
            json!({"new": true, "lists": [{"name": "Seventh", "steps": [{"name": "A"}]}]}),
            9,
        )
        .unwrap_err();
        assert_eq!(
            err,
            "limit: 6 unfinished plans. Finish or close one first. Nothing changed."
        );
        assert_eq!(bytes(&p), before);
    }

    #[test]
    fn the_eleventh_finished_plan_drops_the_oldest_finished_one() {
        let mut p = CaptainPlans::default();
        for i in 1..=11u64 {
            set_json(
                &mut p,
                json!({"new": true, "lists": [{"name": "Build", "steps": [{"name": "A"}]}]}),
                i,
            )
            .unwrap();
            close_on(&mut p, &format!("p{i}"), "Done", &[], i).unwrap();
        }
        assert_eq!(
            plan_ids(&p),
            vec!["p11", "p10", "p9", "p8", "p7", "p6", "p5", "p4", "p3", "p2"],
            "ten finished plans are kept, newest first; p1 was dropped"
        );
        assert_eq!(
            p.next.plan, 11,
            "the dropped plan's id is not handed out again"
        );
    }

    #[test]
    fn a_merge_that_unfinishes_a_plan_counts_against_the_six() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "A"}]}]}),
            1,
        )
        .unwrap();
        report(&mut p, "s1", "done", &[], 2).unwrap();
        for i in 2..=7u64 {
            set_json(
                &mut p,
                json!({"new": true, "lists": [{"name": "Build", "steps": [{"name": "A"}]}]}),
                i,
            )
            .unwrap();
        }
        assert_eq!(
            p.plans.iter().filter(|x| x.finished_at.is_none()).count(),
            6
        );
        let before = bytes(&p);
        let err = set_json(&mut p, json!({"plan": "p1", "lists": [{"name": "Build", "steps": [{"name": "A"}, {"name": "B"}]}]}), 9)
            .unwrap_err();
        assert_eq!(
            err,
            "limit: 6 unfinished plans. Finish or close one first. Nothing changed."
        );
        assert_eq!(bytes(&p), before);
    }

    // ---- limits: each row refused one over, nothing changed ----

    #[test]
    fn lists_per_plan_refuses_nine_lists_and_takes_eight() {
        let mut p = CaptainPlans::default();
        let lists: Vec<_> = (1..=9)
            .map(|i| json!({"name": format!("L{i}"), "steps": [{"name": "x"}]}))
            .collect();
        assert_set_refused(
            &mut p,
            json!({"lists": lists}),
            "limit: 8 lists per plan. Nothing changed.",
        );
        let lists: Vec<_> = (1..=8)
            .map(|i| json!({"name": format!("L{i}"), "steps": [{"name": "x"}]}))
            .collect();
        set_json(&mut p, json!({"lists": lists}), 1).unwrap();
        assert_eq!(plan_of(&p, "p1").lists.len(), 8);
    }

    #[test]
    fn steps_per_list_refuses_twenty_one_steps_and_takes_twenty() {
        let mut p = three_steps();
        let steps: Vec<_> = (1..=21).map(|i| json!({"name": format!("S{i}")})).collect();
        assert_set_refused(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Build", "steps": steps}]}),
            "limit: 20 steps per list. Nothing changed.",
        );
        let steps: Vec<_> = (1..=20).map(|i| json!({"name": format!("S{i}")})).collect();
        set_json(
            &mut p,
            json!({"plan": "p1", "lists": [{"name": "Build", "steps": steps}]}),
            2,
        )
        .unwrap();
        assert_eq!(plan_of(&p, "p1").lists[0].steps.len(), 20);
    }

    #[test]
    fn steps_in_all_refuses_one_hundred_twenty_one_steps_and_takes_one_hundred_twenty() {
        let mut p = CaptainPlans::default();
        let mut lists: Vec<_> = (1..=6)
            .map(|i| {
                let steps: Vec<_> = (1..=20)
                    .map(|j| json!({"name": format!("S{i}-{j}")}))
                    .collect();
                json!({"name": format!("L{i}"), "steps": steps})
            })
            .collect();
        lists.push(json!({"name": "L7", "steps": [{"name": "one more"}]}));
        assert_set_refused(
            &mut p,
            json!({"lists": lists}),
            "limit: 120 steps in all, fixes included. Nothing changed.",
        );
        lists.pop();
        set_json(&mut p, json!({"lists": lists}), 2).unwrap();
        assert_eq!(
            plan_of(&p, "p1")
                .lists
                .iter()
                .map(|l| l.steps.len())
                .sum::<usize>(),
            120
        );
    }

    fn six_lists_of_nineteen() -> Value {
        let lists: Vec<_> = (1..=6)
            .map(|i| {
                let steps: Vec<_> = (1..=19)
                    .map(|j| json!({"name": format!("S{i}-{j}")}))
                    .collect();
                json!({"name": format!("L{i}"), "steps": steps})
            })
            .collect();
        json!({"lists": lists})
    }

    #[test]
    fn steps_in_all_counts_the_steps_of_fixes() {
        let mut p = CaptainPlans::default();
        set_json(&mut p, six_lists_of_nineteen(), 1).unwrap();
        report(&mut p, "s1", "failed", &[], 2).unwrap();
        // 114 main steps, plus six fix steps, is exactly 120.
        fix_on(&mut p, "s1", &["a", "b", "c", "d", "e", "f"], &[], 3).unwrap();
        assert_eq!(p.next.step, 120, "the six fix steps take ids s115..s120");
        // The same six lists with one more list is 121 steps in all.
        let mut again = six_lists_of_nineteen();
        again["lists"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "Extra", "steps": [{"name": "one more"}]}));
        assert_set_refused(
            &mut p,
            again,
            "limit: 120 steps in all, fixes included. Nothing changed.",
        );
    }

    #[test]
    fn step_name_refuses_sixty_one_characters_and_takes_sixty() {
        let mut p = CaptainPlans::default();
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "n".repeat(61)}]}]}),
            "limit: 60 characters for name. Nothing changed.",
        );
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "n".repeat(60)}]}]}),
            2,
        )
        .unwrap();
        assert_eq!(plan_of(&p, "p1").lists[0].steps[0].name.chars().count(), 60);
    }

    #[test]
    fn list_name_is_held_to_the_same_limit() {
        let mut p = CaptainPlans::default();
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "b".repeat(61), "steps": []}]}),
            "limit: 60 characters for name. Nothing changed.",
        );
    }

    #[test]
    fn names_count_characters_not_bytes() {
        let mut p = CaptainPlans::default();
        // "é" is two bytes: sixty of them are 120 bytes, still sixty characters.
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "é".repeat(60)}]}]}),
            1,
        )
        .unwrap();
        assert_eq!(plan_of(&p, "p1").lists[0].steps[0].name, "é".repeat(60));
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "é".repeat(61)}]}]}),
            "limit: 60 characters for name. Nothing changed.",
        );
    }

    #[test]
    fn what_refuses_301_characters_and_takes_300() {
        let mut p = CaptainPlans::default();
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "A", "what": "w".repeat(301)}]}]}),
            "limit: 300 characters for what. Nothing changed.",
        );
        set_json(&mut p, json!({"lists": [{"name": "Build", "steps": [{"name": "A", "what": "w".repeat(300)}]}]}), 2).unwrap();
        assert_eq!(
            plan_of(&p, "p1").lists[0].steps[0].what.chars().count(),
            300
        );
    }

    #[test]
    fn title_refuses_81_characters_and_takes_80() {
        let mut p = CaptainPlans::default();
        assert_set_refused(
            &mut p,
            json!({"title": "t".repeat(81), "lists": [{"name": "Build", "steps": []}]}),
            "limit: 80 characters for title. Nothing changed.",
        );
        set_json(
            &mut p,
            json!({"title": "t".repeat(80), "lists": [{"name": "Build", "steps": []}]}),
            2,
        )
        .unwrap();
        assert_eq!(plan_of(&p, "p1").title.chars().count(), 80);
    }

    #[test]
    fn note_refuses_201_characters_and_takes_200() {
        let mut p = failed_s1();
        assert_eq!(
            report_note(&mut p, "s1", "failed", &"n".repeat(201), 3),
            Err("limit: 200 characters for note. Nothing changed.".to_string())
        );
        report_note(&mut p, "s1", "failed", &"n".repeat(200), 4).unwrap();
        assert_eq!(
            plan_of(&p, "p1").lists[0].steps[0]
                .note
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            200
        );
    }

    #[test]
    fn close_reason_refuses_201_characters() {
        let mut p = three_steps();
        let before = bytes(&p);
        assert_eq!(
            close_on(&mut p, "p1", &"r".repeat(201), &[], 4),
            Err("limit: 200 characters for reason. Nothing changed.".to_string())
        );
        assert_eq!(bytes(&p), before);
    }

    #[test]
    fn limits_count_characters_after_trimming() {
        let mut p = CaptainPlans::default();
        set_json(&mut p, json!({"lists": [{"name": "Build", "steps": [{"name": format!("   {}   ", "n".repeat(60))}]}]}), 1).unwrap();
        assert_eq!(plan_of(&p, "p1").lists[0].steps[0].name, "n".repeat(60));
    }

    #[test]
    fn an_empty_name_is_refused() {
        let mut p = CaptainPlans::default();
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "   ", "steps": []}]}),
            "name is empty. Nothing changed.",
        );
    }

    #[test]
    fn name_and_title_refuse_a_line_break_or_control_character() {
        let mut p = CaptainPlans::default();
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "Build\nNow", "steps": []}]}),
            "name may not contain a line break or other control character. Nothing changed.",
        );
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "A\nB"}]}]}),
            "name may not contain a line break or other control character. Nothing changed.",
        );
        assert_set_refused(
            &mut p,
            json!({"title": "Two\nLines", "lists": [{"name": "Build", "steps": []}]}),
            "title may not contain a line break or other control character. Nothing changed.",
        );
    }

    const SEPARATORS: [char; 2] = ['\u{2028}', '\u{2029}'];
    const DIRECTION: [char; 12] = ['\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}'];
    const INVISIBLE: [char; 5] = ['\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}'];

    #[test]
    fn name_and_title_refuse_a_line_separator_a_direction_mark_or_an_invisible_character() {
        let cases = SEPARATORS.iter().map(|c| (*c, "may not contain a line break or other control character"))
            .chain(DIRECTION.iter().map(|c| (*c, "may not contain a character that changes the text's direction")))
            .chain(INVISIBLE.iter().map(|c| (*c, "may not contain an invisible character")));
        for (c, why) in cases {
            let text = format!("Build{c}Now");
            let mut p = CaptainPlans::default();
            assert_set_refused(&mut p, json!({"lists": [{"name": text, "steps": []}]}), &format!("name {why}. Nothing changed."));
            assert_set_refused(&mut p, json!({"lists": [{"name": "Build", "steps": [{"name": text}]}]}), &format!("name {why}. Nothing changed."));
            assert_set_refused(&mut p, json!({"title": text, "lists": [{"name": "Build", "steps": []}]}), &format!("title {why}. Nothing changed."));
            let mut failed = failed_s1();
            assert_fix_refused(&mut failed, "s1", &[&text], &format!("name {why}. Nothing changed."));
        }
    }

    #[test]
    fn what_note_and_reason_refuse_a_line_separator_or_a_direction_mark_and_keep_a_newline() {
        let cases = SEPARATORS.iter().map(|c| (*c, "may break lines only with a plain line break"))
            .chain(DIRECTION.iter().map(|c| (*c, "may not contain a character that changes the text's direction")));
        for (c, why) in cases {
            let text = format!("line one\n{c}line two");
            let mut p = CaptainPlans::default();
            assert_set_refused(&mut p, json!({"lists": [{"name": "Build", "steps": [{"name": "A", "what": text}]}]}), &format!("what {why}. Nothing changed."));
            let mut failed = failed_s1();
            let before = bytes(&failed);
            assert_eq!(report_note(&mut failed, "s1", "failed", &text, 3), Err(format!("note {why}. Nothing changed.")));
            assert_eq!(close_on(&mut failed, "p1", &text, &[], 3), Err(format!("reason {why}. Nothing changed.")));
            assert_eq!(bytes(&failed), before);
        }
        let mut failed = failed_s1();
        assert!(close_on(&mut failed, "p1", "line one\nline two", &[], 3).is_ok());
    }

    #[test]
    fn what_accepts_a_newline() {
        let mut p = CaptainPlans::default();
        set_json(&mut p, json!({"lists": [{"name": "Build", "steps": [{"name": "A", "what": "line one\nline two"}]}]}), 1).unwrap();
        assert_eq!(
            plan_of(&p, "p1").lists[0].steps[0].what,
            "line one\nline two"
        );
    }

    #[test]
    fn what_and_note_refuse_a_tab_or_a_bell() {
        let mut p = CaptainPlans::default();
        assert_set_refused(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "A", "what": "a\tb"}]}]}),
            "what may hold line breaks but no other control character. Nothing changed.",
        );
        let mut failed = failed_s1();
        assert_eq!(
            report_note(&mut failed, "s1", "failed", "ok\u{7}", 3),
            Err(
                "note may hold line breaks but no other control character. Nothing changed."
                    .to_string()
            )
        );
    }

    #[test]
    fn fix_refuses_a_seventh_step() {
        let mut p = failed_s1();
        assert_fix_refused(
            &mut p,
            "s1",
            &["a", "b", "c", "d", "e", "f", "g"],
            "limit: 6 steps per fix. Nothing changed.",
        );
    }

    #[test]
    fn fix_refuses_a_fourth_fix_on_one_step() {
        let mut p = failed_s1();
        fix_on(&mut p, "s1", &["one"], &[], 3).unwrap();
        report(&mut p, "s2", "failed", &[], 4).unwrap();
        fix_on(&mut p, "s1", &["two"], &[], 5).unwrap();
        report(&mut p, "s3", "failed", &[], 6).unwrap();
        fix_on(&mut p, "s1", &["three"], &[], 7).unwrap();
        report(&mut p, "s4", "failed", &[], 8).unwrap();
        assert_fix_refused(
            &mut p,
            "s1",
            &["four"],
            "limit: 3 fixes per step. Nothing changed.",
        );
    }

    #[test]
    fn fix_refuses_an_empty_branch() {
        let mut p = failed_s1();
        assert_fix_refused(
            &mut p,
            "s1",
            &[],
            "a fix needs at least one step. Nothing changed.",
        );
    }

    #[test]
    fn fix_steps_count_toward_the_one_hundred_twenty() {
        let mut p = CaptainPlans::default();
        let lists: Vec<_> = (1..=6)
            .map(|i| {
                let steps: Vec<_> = (1..=20)
                    .map(|j| json!({"name": format!("S{i}-{j}")}))
                    .collect();
                json!({"name": format!("L{i}"), "steps": steps})
            })
            .collect();
        set_json(&mut p, json!({"lists": lists}), 1).unwrap();
        report(&mut p, "s1", "failed", &[], 2).unwrap();
        assert_fix_refused(
            &mut p,
            "s1",
            &["one more"],
            "limit: 120 steps in all, fixes included. Nothing changed.",
        );
    }

    // ---- derive: one row per state in the table ----

    #[test]
    fn table_working_task_reads_running_unless_the_worker_waits_on_the_user() {
        let plan = linked("t1");
        let mut f = task("t1", Work::Working);
        assert_eq!(
            s1_under(&plan, std::slice::from_ref(&f)).state,
            State::Running
        );
        f.waits_on_user = true;
        assert_eq!(s1_under(&plan, &[f]).state, State::Waiting);
    }

    #[test]
    fn table_waiting_for_children_reads_running_unless_the_worker_waits_on_the_user() {
        let plan = linked("t1");
        let mut f = task("t1", Work::WaitingForChildren);
        assert_eq!(
            s1_under(&plan, std::slice::from_ref(&f)).state,
            State::Running
        );
        f.waits_on_user = true;
        assert_eq!(s1_under(&plan, &[f]).state, State::Waiting);
    }

    #[test]
    fn table_result_accepted_reads_done_with_no_reason() {
        let plan = linked("t1");
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Accepted;
        let s = s1_under(&plan, &[f]);
        assert_eq!((s.state, s.why), (State::Done, None));
    }

    #[test]
    fn table_result_without_a_review_reads_done() {
        let plan = linked("t1");
        let s = s1_under(&plan, &[ended("t1", Work::ResultAvailable, 2000)]);
        assert_eq!((s.state, s.why), (State::Done, None));
    }

    #[test]
    fn table_result_redone_reads_failed_redone() {
        let plan = linked("t1");
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Redone;
        let s = s1_under(&plan, &[f]);
        assert_eq!((s.state, s.why), (State::Failed, Some("Redone".into())));
    }

    #[test]
    fn table_failed_task_reads_failed_with_its_reason() {
        let plan = linked("t1");
        let mut f = ended("t1", Work::Failed, 2000);
        f.failure = Some("usage limit reached".into());
        let s = s1_under(&plan, &[f]);
        assert_eq!(
            (s.state, s.why),
            (State::Failed, Some("usage limit reached".into()))
        );
    }

    #[test]
    fn table_cancelled_task_reads_failed_cancelled() {
        let plan = linked("t1");
        let s = s1_under(&plan, &[ended("t1", Work::Cancelled, 2000)]);
        assert_eq!((s.state, s.why), (State::Failed, Some("Cancelled".into())));
    }

    #[test]
    fn table_a_step_with_no_task_reads_its_reported_state() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "Work"}]}]}),
            1,
        )
        .unwrap();
        report(&mut p, "s1", "running", &[], 2).unwrap();
        let s = s1_under(&p, &[]);
        assert_eq!((s.state, s.why), (State::Running, None));
    }

    #[test]
    fn a_report_is_refused_while_the_linked_task_is_working_and_names_it() {
        let mut p = linked("t1");
        let before = bytes(&p);
        let err =
            report(&mut p, "s1", "done", &[task("t1", Work::Working)], 1500).expect_err("refused");
        assert_eq!(err, "s1 follows task t1, which has not ended. Wait for it, or cancel t1 first. Nothing changed.");
        assert_eq!(bytes(&p), before);
    }

    #[test]
    fn a_report_after_the_task_ends_wins_over_the_task() {
        let mut p = linked("t1");
        report(
            &mut p,
            "s1",
            "running",
            &[ended("t1", Work::ResultAvailable, 2000)],
            3000,
        )
        .unwrap();
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Accepted;
        assert_eq!(s1_under(&p, &[f]).state, State::Running);
    }

    #[test]
    fn a_step_reported_as_it_stands_changes_nothing() {
        let mut p = CaptainPlans::default();
        set_json(&mut p, json!({"lists": [{"name": "Build", "steps": [{"name": "Work"}]}]}), 1).unwrap();
        report_note(&mut p, "s1", "running", "on it", 2).unwrap();
        let before = bytes(&p);
        assert_eq!(report_note(&mut p, "s1", "running", "on it", 3), Ok("p1 rev 2\nl1 Build: s1 running".into()));
        assert_eq!(bytes(&p), before);
        assert_eq!(report(&mut p, "s1", "running", &[], 4), Ok("p1 rev 3\nl1 Build: s1 running".into()), "a note dropped is a change");
    }

    #[test]
    fn the_same_report_after_the_task_ends_still_wins_over_the_task() {
        let mut p = linked("t1");
        let reported = p.plans[0].lists[0].steps[0].reported;
        let rev = p.plans[0].rev;
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Accepted;
        report(&mut p, "s1", &serde_json::to_value(reported).unwrap().as_str().unwrap().to_owned(), std::slice::from_ref(&f), 3000).unwrap();
        assert_eq!(p.plans[0].rev, rev + 1);
        assert_eq!(s1_under(&p, &[f]).state, reported);
    }

    #[test]
    fn a_report_from_before_the_task_ends_loses_to_the_task() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"lists": [{"name": "Build", "steps": [{"name": "Work"}]}]}),
            1,
        )
        .unwrap();
        report(&mut p, "s1", "running", &[], 1500).unwrap();
        p.plans[0].lists[0].steps[0].task = Some("t1".into());
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Accepted;
        assert_eq!(s1_under(&p, &[f]).state, State::Done);
    }

    #[test]
    fn a_linked_task_missing_from_the_facts_leaves_the_reported_state() {
        let mut p = linked("t1");
        assert_eq!(s1_under(&p, &[]).state, State::Pending);
        report(&mut p, "s1", "done", &[], 2).unwrap();
        assert_eq!(s1_under(&p, &[]).state, State::Done);
    }

    // ---- derive: list-level links ----

    #[test]
    fn list_task_working_runs_the_first_step_not_done() {
        let mut p = three_steps();
        report(&mut p, "s1", "done", &[], 1100).unwrap();
        p.plans[0].lists[0].task = Some("t1".into());
        let view = derive(&p.plans[0], &[task("t1", Work::Working)]);
        let (s1, s2, s3) = (
            view_step(&view, "s1"),
            view_step(&view, "s2"),
            view_step(&view, "s3"),
        );
        assert_eq!((s1.state, s1.why.clone()), (State::Done, None));
        assert_eq!((s2.state, s2.why.clone()), (State::Running, None));
        assert_eq!((s3.state, s3.why.clone()), (State::Pending, None));
    }

    #[test]
    fn list_task_result_finishes_the_steps_the_captain_never_reported() {
        let mut p = three_steps();
        report(&mut p, "s1", "done", &[], 1100).unwrap();
        p.plans[0].lists[0].task = Some("t1".into());
        let view = derive(&p.plans[0], &[ended("t1", Work::ResultAvailable, 2000)]);
        let reason = Some("Done with the list's task".to_string());
        let (s1, s2, s3) = (
            view_step(&view, "s1"),
            view_step(&view, "s2"),
            view_step(&view, "s3"),
        );
        assert_eq!((s1.state, s1.why.clone()), (State::Done, None));
        assert_eq!((s2.state, s2.why.clone()), (State::Done, reason.clone()));
        assert_eq!((s3.state, s3.why.clone()), (State::Done, reason));
    }

    #[test]
    fn list_task_failure_fails_the_running_step() {
        let mut p = three_steps();
        p.plans[0].lists[0].task = Some("t1".into());
        let mut f = ended("t1", Work::Failed, 2000);
        f.failure = Some("build crashed".into());
        let view = derive(&p.plans[0], &[f]);
        let (s1, s2) = (view_step(&view, "s1"), view_step(&view, "s2"));
        assert_eq!(
            (s1.state, s1.why.clone()),
            (State::Failed, Some("build crashed".into()))
        );
        assert_eq!((s2.state, s2.why.clone()), (State::Pending, None));
    }

    #[test]
    fn list_task_redone_fails_the_steps_it_would_finish() {
        let mut p = three_steps();
        p.plans[0].lists[0].task = Some("t1".into());
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Redone;
        let view = derive(&p.plans[0], &[f]);
        for id in ["s1", "s2", "s3"] {
            let s = view_step(&view, id);
            assert_eq!(
                (s.state, s.why.clone()),
                (State::Failed, Some("Redone".into())),
                "step {id}"
            );
        }
    }

    #[test]
    fn a_report_on_the_running_step_of_a_working_list_task_is_refused() {
        let mut p = three_steps();
        p.plans[0].lists[0].task = Some("t1".into());
        let working = [task("t1", Work::Working)];
        assert_eq!(
            report(&mut p, "s1", "done", &working, 2),
            Err("s1 follows task t1, which has not ended. Wait for it, or cancel t1 first. Nothing changed.".to_string())
        );
        report(&mut p, "s2", "done", &working, 3).unwrap();
        assert_eq!(
            view_step(&derive(&p.plans[0], &working), "s2").state,
            State::Done
        );
    }

    #[test]
    fn a_step_link_beats_the_list_link() {
        let mut p = three_steps();
        p.plans[0].lists[0].task = Some("t1".into());
        p.plans[0].lists[0].steps[0].task = Some("t2".into());
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Accepted;
        let view = derive(&p.plans[0], &[f, task("t2", Work::Working)]);
        assert_eq!(view_step(&view, "s1").state, State::Running);
        assert_eq!(view_step(&view, "s2").state, State::Done);
    }

    #[test]
    fn a_list_link_cost_is_the_rise_since_the_step_started() {
        let mut p = three_steps();
        p.plans[0].lists[0].task = Some("t1".into());
        let mut working = task("t1", Work::Working);
        working.cost = Some(1.0);
        assert!(settle(&mut p, std::slice::from_ref(&working), 1500));
        assert_eq!(p.plans[0].lists[0].steps[0].cost_at_start, Some(1.0));
        assert!(
            !settle(&mut p, std::slice::from_ref(&working), 1600),
            "a second settle with nothing new changes nothing"
        );

        let mut ended_task = ended("t1", Work::ResultAvailable, 2000);
        ended_task.review = Review::Accepted;
        ended_task.cost = Some(3.5);
        let view = derive(&p.plans[0], &[ended_task]);
        assert_eq!(view_step(&view, "s1").cost, Some(2.5));
        assert_eq!(
            view_step(&view, "s2").cost,
            None,
            "a step that never started has no cost"
        );
    }

    #[test]
    fn a_step_with_its_own_task_costs_the_task_rollup() {
        let plan = linked("t1");
        let mut f = task("t1", Work::Working);
        f.cost = Some(4.25);
        assert_eq!(s1_under(&plan, &[f]).cost, Some(4.25));
    }

    // ---- fixes ----

    #[test]
    fn fix_is_refused_on_a_step_that_is_not_failed() {
        let mut p = three_steps();
        assert_fix_refused(
            &mut p,
            "s1",
            &["Patch"],
            "s1 is not failed (it reads pending), so it cannot get a fix. Nothing changed.",
        );
    }

    #[test]
    fn a_fix_branch_merges_back_as_done_after_one_fix() {
        let mut p = three_steps();
        report(&mut p, "s1", "failed", &[], 2).unwrap();
        let answer = fix_on(&mut p, "s1", &["Patch"], &[], 3).unwrap();
        assert!(
            answer.starts_with("Fix f1 on s1 has steps s4.\n"),
            "answer was {answer:?}"
        );
        assert_eq!(
            view_step(&derive(&p.plans[0], &[]), "s1").state,
            State::Failed
        );

        report(&mut p, "s4", "done", &[], 4).unwrap();
        let s = view_step(&derive(&p.plans[0], &[]), "s1").clone();
        assert_eq!(
            (s.state, s.why),
            (State::Done, Some("after one fix".into()))
        );
    }

    #[test]
    fn two_fixes_merge_back_as_done_after_2_fixes() {
        let mut p = three_steps();
        report(&mut p, "s1", "failed", &[], 2).unwrap();
        fix_on(&mut p, "s1", &["First try"], &[], 3).unwrap();
        report(&mut p, "s4", "failed", &[], 4).unwrap();
        fix_on(&mut p, "s1", &["Second try"], &[], 5).unwrap();
        report(&mut p, "s5", "done", &[], 6).unwrap();

        let s = view_step(&derive(&p.plans[0], &[]), "s1").clone();
        assert_eq!(
            (s.state, s.why),
            (State::Done, Some("after 2 fixes".into()))
        );
        assert_eq!(p.plans[0].lists[0].steps[0].fixes.len(), 2);
    }

    #[test]
    fn a_failing_fix_step_keeps_the_branch_red() {
        let mut p = three_steps();
        report(&mut p, "s1", "failed", &[], 2).unwrap();
        fix_on(&mut p, "s1", &["Patch"], &[], 3).unwrap();
        report(&mut p, "s4", "failed", &[], 4).unwrap();
        assert_eq!(
            view_step(&derive(&p.plans[0], &[]), "s1").state,
            State::Failed
        );
    }

    // ---- settle and finishing ----

    #[test]
    fn finished_at_is_set_once_every_main_step_is_done() {
        let mut p = three_steps();
        for id in ["s1", "s2", "s3"] {
            report(&mut p, id, "done", &[], 2).unwrap();
        }
        assert_eq!(plan_of(&p, "p1").finished_at, Some(2));
        assert!(
            !settle(&mut p, &[], 4000),
            "a settle with nothing new changes nothing"
        );
        assert_eq!(plan_of(&p, "p1").finished_at, Some(2));
        assert_eq!(glance(plan_of(&p, "p1"), &[]).state, CaptainState::Finished);
    }

    #[test]
    fn a_plan_with_no_steps_is_not_finished() {
        let mut p = CaptainPlans::default();
        set_json(&mut p, json!({"lists": []}), 1).unwrap();
        assert!(!settle(&mut p, &[], 2));
        assert_eq!(plan_of(&p, "p1").finished_at, None);
    }

    #[test]
    fn a_redone_review_unfinishes_a_finished_plan() {
        let mut p = linked("t1");
        let mut accepted = ended("t1", Work::ResultAvailable, 2000);
        accepted.review = Review::Accepted;
        assert!(settle(&mut p, std::slice::from_ref(&accepted), 2500));
        assert_eq!(plan_of(&p, "p1").finished_at, Some(2500));

        let mut redone = ended("t1", Work::ResultAvailable, 2000);
        redone.review = Review::Redone;
        assert!(settle(&mut p, &[redone], 3000));
        assert_eq!(plan_of(&p, "p1").finished_at, None);
    }

    // ---- glance ----

    #[test]
    fn glance_dots_for_a_fixed_plan_are_literal() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"title": "Fixed", "lists": [
                {"name": "Build", "steps": [{"name": "A"}, {"name": "B"}, {"name": "C"}, {"name": "D"}, {"name": "E"}]},
                {"name": "Ship", "steps": [{"name": "F"}]}
            ]}),
            1,
        )
        .unwrap();
        report(&mut p, "s1", "done", &[], 2).unwrap();
        report(&mut p, "s2", "failed", &[], 3).unwrap();
        fix_on(&mut p, "s2", &["Patch"], &[], 4).unwrap();
        report(&mut p, "s7", "done", &[], 5).unwrap();
        report(&mut p, "s3", "running", &[], 6).unwrap();
        report(&mut p, "s4", "waiting", &[], 7).unwrap();
        report(&mut p, "s6", "failed", &[], 8).unwrap();

        let g = glance(plan_of(&p, "p1"), &[]);
        assert_eq!(
            serde_json::to_value(&g).unwrap(),
            json!({
                "id": "p1",
                "title": "Fixed",
                "state": "needs_you",
                "done": 2,
                "total": 6,
                "now": "C",
                "strips": ["dxrwp", "f"]
            })
        );
    }

    #[test]
    fn glance_state_per_plan_follows_the_table() {
        let waiting = linked("t1");
        let g = glance(plan_of(&waiting, "p1"), &[task("t1", Work::Working)]);
        assert_eq!(
            g.state,
            CaptainState::Working,
            "a linked working task reads running, not waiting"
        );
        let mut waits = task("t1", Work::Working);
        waits.waits_on_user = true;
        assert_eq!(
            glance(plan_of(&waiting, "p1"), &[waits]).state,
            CaptainState::NeedsYou
        );

        let mut failed = three_steps();
        report(&mut failed, "s1", "failed", &[], 2).unwrap();
        assert_eq!(
            glance(plan_of(&failed, "p1"), &[]).state,
            CaptainState::Fixing
        );

        let mut running = three_steps();
        report(&mut running, "s1", "running", &[], 2).unwrap();
        assert_eq!(
            glance(plan_of(&running, "p1"), &[]).state,
            CaptainState::Working
        );

        let idle = three_steps();
        assert_eq!(glance(plan_of(&idle, "p1"), &[]).state, CaptainState::Idle);

        let mut closed = three_steps();
        report(&mut closed, "s1", "failed", &[], 2).unwrap();
        close_on(&mut closed, "p1", "Gave up", &[], 3).unwrap();
        assert_eq!(
            glance(plan_of(&closed, "p1"), &[]).state,
            CaptainState::Closed
        );
    }

    #[test]
    fn glance_list_holds_every_unfinished_plan_and_one_finished_plan_with_no_strips() {
        let mut p = two_plans();
        close_on(&mut p, "p2", "Shipped", &[], 7).unwrap();
        set_json(
            &mut p,
            json!({"new": true, "lists": [{"name": "Third", "steps": [{"name": "A"}]}]}),
            8,
        )
        .unwrap();
        let list = glance_list(&p, &[]);
        let ids: Vec<&str> = list.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["p1", "p3", "p2"]);
        assert_eq!(list[0].strips, vec!["df".to_string()]);
        assert!(
            list[2].strips.is_empty(),
            "the finished plan carries no strips"
        );
        assert_eq!(list[2].state, CaptainState::Closed);
    }

    #[test]
    fn glance_clips_title_and_now_to_thirty_two_characters() {
        let mut p = CaptainPlans::default();
        set_json(
            &mut p,
            json!({"title": "T".repeat(80), "lists": [{"name": "Build", "steps": [{"name": "N".repeat(60)}]}]}),
            1,
        )
        .unwrap();
        report(&mut p, "s1", "running", &[], 2).unwrap();
        let g = glance(plan_of(&p, "p1"), &[]);
        assert_eq!(g.title, "T".repeat(32));
        assert_eq!(g.now.as_deref(), Some("N".repeat(32).as_str()));
    }

    #[test]
    fn glance_at_the_limits_stays_under_4096_bytes() {
        let mut p = CaptainPlans::default();
        for n in 1..=6u64 {
            let lists: Vec<Value> = (1..=8)
                .map(|l| {
                    let steps: Vec<Value> =
                        (1..=15).map(|_| json!({"name": "😀".repeat(60)})).collect();
                    json!({"name": format!("L{l}"), "steps": steps})
                })
                .collect();
            set_json(
                &mut p,
                json!({"new": true, "title": "😀".repeat(80), "lists": lists}),
                n,
            )
            .unwrap();
            let first = plan_of(&p, &format!("p{n}")).lists[0].steps[0].id.clone();
            report(&mut p, &first, "running", &[], n + 10).unwrap();
        }
        let list = glance_list(&p, &[]);
        assert_eq!(list.len(), 6);
        let len = serde_json::to_string(&list).unwrap().len();
        assert!(len < 4096, "glance list is {len} bytes");
    }
}
