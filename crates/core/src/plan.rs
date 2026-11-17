//! The Captain's plans: lists of steps, the limits on them, and the states that follow the tasks they link to.
//!
//! Pure code. Nothing here reads or writes files, talks to the hub, or looks up a task. Callers pass the task
//! facts they know. A call changes the plans only when it succeeds.

use serde::{Deserialize, Serialize};

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

/// The state a step has, as the Captain reported it or as a task derives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Pending,
    Running,
    Waiting,
    Done,
    Failed,
}

/// The state of one Captain's plan as a whole, which the Orchestrator panel sorts by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ListInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub steps: Vec<StepInput>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct StepInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub what: Option<String>,
}

/// The `step` arguments: a report on one step.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct StepCall {
    pub step: String,
    pub state: State,
    #[serde(default)]
    pub note: Option<String>,
}

/// The `fix` arguments: a new branch under a failed step.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FixCall {
    pub step: String,
    pub steps: Vec<NewStep>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct NewStep {
    pub name: String,
    #[serde(default)]
    pub what: Option<String>,
}

/// The `close` arguments. The plan is named by its id.
#[derive(Clone, Debug, PartialEq, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlanView {
    pub id: String,
    pub rev: u64,
    pub title: String,
    pub asked_at: u64,
    pub finished_at: Option<u64>,
    pub closed: Option<String>,
    pub lists: Vec<ListView>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ListView {
    pub id: String,
    pub name: String,
    pub task: Option<String>,
    pub steps: Vec<StepView>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FixView {
    pub id: String,
    pub steps: Vec<StepView>,
}

/// The small shape the Orchestrator panel shows for one plan: one dots string per list.
#[derive(Clone, Debug, PartialEq, Serialize)]
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

// Scaffold for the red commit: every entry point compiles and does nothing yet.

/// Applies a `set` call. Returns the answer text.
pub fn set(
    _plans: &mut CaptainPlans,
    _call: SetCall,
    _asked_at: u64,
    _facts: &[TaskFacts],
    _now: u64,
) -> Result<String, String> {
    Ok(String::new())
}

/// Applies a report on one step. Returns the answer text.
pub fn step(
    _plans: &mut CaptainPlans,
    _call: StepCall,
    _facts: &[TaskFacts],
    _now: u64,
) -> Result<String, String> {
    Ok(String::new())
}

/// Opens a fix branch under a failed step. Returns the answer text.
pub fn fix(
    _plans: &mut CaptainPlans,
    _call: FixCall,
    _facts: &[TaskFacts],
    _now: u64,
) -> Result<String, String> {
    Ok(String::new())
}

/// Closes a plan. Returns the answer text.
pub fn close(
    _plans: &mut CaptainPlans,
    _call: CloseCall,
    _facts: &[TaskFacts],
    _now: u64,
) -> Result<String, String> {
    Ok(String::new())
}

/// Records what the facts show (start costs, finish times) in every plan. Returns true when a plan changed.
pub fn settle(_plans: &mut CaptainPlans, _facts: &[TaskFacts], _now: u64) -> bool {
    false
}

/// The plan with every state and cost derived from the facts.
pub fn derive(plan: &Plan, _facts: &[TaskFacts]) -> PlanView {
    PlanView {
        id: plan.id.clone(),
        rev: plan.rev,
        title: plan.title.clone(),
        asked_at: plan.asked_at,
        finished_at: plan.finished_at,
        closed: plan.closed.clone(),
        lists: Vec::new(),
    }
}

/// The glance of one plan.
pub fn glance(plan: &Plan, _facts: &[TaskFacts]) -> PlanGlance {
    PlanGlance {
        id: plan.id.clone(),
        title: String::new(),
        state: CaptainState::Idle,
        done: 0,
        total: 0,
        now: None,
        finished_at: None,
        strips: Vec::new(),
    }
}

/// The glance list the Orchestrator panel shows: every unfinished plan, plus the latest finished or closed one.
pub fn glance_list(_plans: &CaptainPlans, _facts: &[TaskFacts]) -> Vec<PlanGlance> {
    Vec::new()
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
        report(&mut p, "s1", "done", &[], 8).unwrap();
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
