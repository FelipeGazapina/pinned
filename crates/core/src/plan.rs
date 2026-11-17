//! The Captain's plan: lists of steps, the limits on them, and the states that follow the tasks they link to.
//!
//! Pure code. Nothing here reads or writes files, talks to the hub, or looks up a task. Callers pass the task
//! facts they know and the plan changes in memory only when a call succeeds.

use serde::{Deserialize, Serialize};

pub const MAX_LISTS: usize = 8;
pub const MAX_STEPS_PER_LIST: usize = 20;
pub const MAX_STEPS_PER_FIX: usize = 6;
pub const MAX_FIXES_PER_STEP: usize = 3;
pub const MAX_STEPS_IN_ALL: usize = 120;
pub const MAX_NAME: usize = 60;
pub const MAX_WHAT: usize = 300;
pub const MAX_TITLE: usize = 80;
pub const MAX_NOTE: usize = 200;

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

/// The state of the Captain as a whole, which the Orchestrator panel sorts by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptainState {
    NeedsYou,
    Fixing,
    Working,
    #[default]
    Idle,
    Finished,
}

/// A Captain's plan as stored. `rev` rises on every write, and the id counters never go back, so an id is
/// never reused for another step, even after `new: true` replaces the plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Unix milliseconds of the prompt that made the plan.
    pub asked_at: u64,
    /// Unix milliseconds of the write that found every main step done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    pub lists: Vec<List>,
    #[serde(default)]
    pub ids: Counters,
}

/// The last number handed out for each kind of id.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Counters {
    pub list: u64,
    pub step: u64,
    pub fix: u64,
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
    /// The state the Captain last reported. Zero in `reported_at` means never reported.
    pub reported: State,
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

/// A branch opened under a failed step. Only the latest one is open.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    pub id: String,
    pub steps: Vec<Step>,
}

/// The `set` arguments. Ids are optional and match existing lists and steps.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct SetCall {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub new: bool,
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

/// The full plan as a client reads it, with states and costs derived from the linked tasks.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlanView {
    pub rev: u64,
    pub title: Option<String>,
    pub asked_at: u64,
    pub finished_at: Option<u64>,
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

/// The small shape synced to the hub: one dot per main step.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PlanGlance {
    pub rev: u64,
    pub state: CaptainState,
    pub done: usize,
    pub total: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    pub lists: Vec<GlanceList>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GlanceList {
    pub name: String,
    pub dots: String,
}

impl PlanGlance {
    pub fn is_empty(&self) -> bool {
        self.lists.is_empty()
    }
}

// Scaffold for the red commit: every entry point compiles and does nothing yet.

/// Applies a `set` call. Returns the answer text.
pub fn set(
    _slot: &mut Option<Plan>,
    _call: SetCall,
    _asked_at: u64,
    _facts: &[TaskFacts],
    _now: u64,
) -> Result<String, String> {
    Ok(String::new())
}

/// Applies a report on one step. Returns the answer text.
pub fn step(
    _slot: &mut Option<Plan>,
    _call: StepCall,
    _facts: &[TaskFacts],
    _now: u64,
) -> Result<String, String> {
    Ok(String::new())
}

/// Opens a fix branch under a failed step. Returns the answer text.
pub fn fix(
    _slot: &mut Option<Plan>,
    _call: FixCall,
    _facts: &[TaskFacts],
    _now: u64,
) -> Result<String, String> {
    Ok(String::new())
}

/// Records what the facts show (start costs, finish time). Returns true when the plan changed.
pub fn settle(_plan: &mut Plan, _facts: &[TaskFacts], _now: u64) -> bool {
    false
}

/// The plan with every state and cost derived from the facts.
pub fn derive(plan: &Plan, _facts: &[TaskFacts]) -> PlanView {
    PlanView {
        rev: plan.rev,
        title: None,
        asked_at: plan.asked_at,
        finished_at: None,
        lists: Vec::new(),
    }
}

/// The glance of the plan, or an empty glance when there is none.
pub fn glance(_plan: Option<&Plan>, _facts: &[TaskFacts]) -> PlanGlance {
    PlanGlance::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set_with(slot: &mut Option<Plan>, v: serde_json::Value, now: u64) -> Result<String, String> {
        let call: SetCall = serde_json::from_value(v).expect("set call JSON");
        set(slot, call, 1000, &[], now)
    }

    fn report(slot: &mut Option<Plan>, step_id: &str, state: &str, facts: &[TaskFacts], now: u64) -> Result<String, String> {
        let state: State = serde_json::from_value(json!(state)).expect("state name");
        step(slot, StepCall { step: step_id.into(), state, note: None }, facts, now)
    }

    fn fix_on(slot: &mut Option<Plan>, step_id: &str, names: &[&str], now: u64) -> Result<String, String> {
        let steps = names.iter().map(|n| NewStep { name: (*n).into(), what: None }).collect();
        fix(slot, FixCall { step: step_id.into(), steps }, &[], now)
    }

    fn bytes(slot: &Option<Plan>) -> String {
        serde_json::to_string(slot).unwrap()
    }

    /// One line per list: "l1 Build: s1 Schema, s2 API".
    fn outline(slot: &Option<Plan>) -> String {
        let Some(plan) = slot else { return "no plan".into() };
        plan.lists
            .iter()
            .map(|l| {
                let steps: Vec<String> = l.steps.iter().map(|s| format!("{} {}", s.id, s.name)).collect();
                format!("{} {}: {}", l.id, l.name, steps.join(", "))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn three_steps() -> Option<Plan> {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A"}, {"name": "B"}, {"name": "C"}]}]}), 1).unwrap();
        slot
    }

    fn task(id: &str, work: Work) -> TaskFacts {
        TaskFacts { id: id.into(), work, waits_on_user: false, review: Review::Unreviewed, failure: None, cost: None, ended_at: None }
    }

    fn ended(id: &str, work: Work, at: u64) -> TaskFacts {
        TaskFacts { ended_at: Some(at), ..task(id, work) }
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
        all_steps(view).into_iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no step {id} in the view"))
    }

    /// A plan with one step, s1, that links to the given task.
    fn linked(task_id: &str) -> Option<Plan> {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "Work"}]}]}), 1).unwrap();
        slot.as_mut().unwrap().lists[0].steps[0].task = Some(task_id.into());
        slot
    }

    fn s1_under(slot: &Option<Plan>, facts: &[TaskFacts]) -> StepView {
        let view = derive(slot.as_ref().unwrap(), facts);
        view_step(&view, "s1").clone()
    }

    fn assert_set_refused(slot: &mut Option<Plan>, v: serde_json::Value, limit: &str) {
        let before = bytes(slot);
        let err = set_with(slot, v, 9).expect_err("the call should be refused");
        assert!(err.contains(limit), "error should name {limit:?}, got {err:?}");
        assert_eq!(bytes(slot), before, "a refused call must leave the plan unchanged");
    }

    fn assert_fix_refused(slot: &mut Option<Plan>, step_id: &str, names: &[&str], limit: &str) {
        let before = bytes(slot);
        let err = fix_on(slot, step_id, names, 9).expect_err("the fix should be refused");
        assert!(err.contains(limit), "error should name {limit:?}, got {err:?}");
        assert_eq!(bytes(slot), before, "a refused fix must leave the plan unchanged");
    }

    fn failed_s1() -> Option<Plan> {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "Keep"}]}]}), 1).unwrap();
        report(&mut slot, "s1", "failed", &[], 2).unwrap();
        slot
    }

    // ---- ids and set ----

    #[test]
    fn ids_are_short_unique_and_never_reused() {
        let mut slot = None;
        set_with(
            &mut slot,
            json!({"lists": [
                {"name": "Build", "steps": [{"name": "Schema"}, {"name": "API"}]},
                {"name": "Ship", "steps": [{"name": "Release"}]}
            ]}),
            1,
        )
        .unwrap();
        assert_eq!(outline(&slot), "l1 Build: s1 Schema, s2 API\nl2 Ship: s3 Release");

        set_with(
            &mut slot,
            json!({"lists": [
                {"name": "Build", "steps": [{"name": "Schema"}, {"name": "Docs"}]},
                {"name": "Ship", "steps": [{"name": "Release"}]}
            ]}),
            2,
        )
        .unwrap();
        assert_eq!(outline(&slot), "l1 Build: s1 Schema, s4 Docs\nl2 Ship: s3 Release");

        set_with(&mut slot, json!({"new": true, "lists": [{"name": "Next", "steps": [{"name": "Plan"}]}]}), 3).unwrap();
        assert_eq!(outline(&slot), "l3 Next: s5 Plan");
    }

    #[test]
    fn the_answer_lists_each_list_with_its_step_ids_and_states() {
        let mut slot = None;
        let answer = set_with(
            &mut slot,
            json!({"lists": [
                {"name": "Build", "steps": [{"name": "Schema"}, {"name": "API"}]},
                {"name": "Ship", "steps": [{"name": "Release"}]}
            ]}),
            1,
        )
        .unwrap();
        assert_eq!(answer, "Plan rev 1.\nl1 Build: s1 pending, s2 pending\nl2 Ship: s3 pending");
    }

    #[test]
    fn merge_keeps_a_started_step_after_the_nearest_earlier_step() {
        let mut slot = three_steps();
        report(&mut slot, "s2", "done", &[], 2).unwrap();
        let answer = set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "C"}, {"name": "A"}]}]}), 3).unwrap();
        assert_eq!(outline(&slot), "l1 Build: s3 C, s1 A, s2 B");
        assert_eq!(answer, "Plan rev 3.\nl1 Build: s3 pending, s1 pending, s2 done\nkept 1 started step you left out: s2.");
    }

    #[test]
    fn merge_removes_an_omitted_pending_step() {
        let mut slot = three_steps();
        let answer = set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A"}, {"name": "C"}]}]}), 2).unwrap();
        assert_eq!(outline(&slot), "l1 Build: s1 A, s3 C");
        assert_eq!(answer, "Plan rev 2.\nl1 Build: s1 pending, s3 pending");
    }

    #[test]
    fn merge_matched_steps_keep_id_state_link_and_fixes() {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "Schema"}, {"name": "API"}]}]}), 1).unwrap();
        report(&mut slot, "s2", "failed", &[], 2).unwrap();
        fix_on(&mut slot, "s2", &["Patch"], 3).unwrap();
        report(&mut slot, "s1", "running", &[], 4).unwrap();
        slot.as_mut().unwrap().lists[0].steps[0].task = Some("t1".into());

        set_with(
            &mut slot,
            json!({"lists": [{"name": "Build", "steps": [
                {"id": "s2", "name": "API v2", "what": "the new one"},
                {"name": "Schema"}
            ]}]}),
            5,
        )
        .unwrap();

        assert_eq!(outline(&slot), "l1 Build: s2 API v2, s1 Schema");
        let plan = slot.as_ref().unwrap();
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
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "Deploy"}]}]}), 1).unwrap();
        report(&mut slot, "s1", "done", &[], 2).unwrap();

        let answer = set_with(&mut slot, json!({"lists": [{"name": "Test", "steps": [{"name": "Deploy"}]}]}), 3).unwrap();

        assert_eq!(outline(&slot), "l1 Build: s1 Deploy\nl2 Test: s2 Deploy");
        assert_eq!(answer, "Plan rev 3.\nl1 Build: s1 done\nl2 Test: s2 pending\nkept 1 started step you left out: s1.");
    }

    #[test]
    fn every_write_bumps_rev() {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A"}]}]}), 1).unwrap();
        assert_eq!(slot.as_ref().unwrap().rev, 1);
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A"}]}]}), 2).unwrap();
        assert_eq!(slot.as_ref().unwrap().rev, 2);
        report(&mut slot, "s1", "failed", &[], 3).unwrap();
        assert_eq!(slot.as_ref().unwrap().rev, 3);
        fix_on(&mut slot, "s1", &["Patch"], 4).unwrap();
        assert_eq!(slot.as_ref().unwrap().rev, 4);
        set_with(&mut slot, json!({"new": true, "lists": [{"name": "Next", "steps": [{"name": "X"}]}]}), 5).unwrap();
        assert_eq!(slot.as_ref().unwrap().rev, 5);
    }

    #[test]
    fn new_replaces_the_plan_and_its_title() {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A"}]}]}), 1).unwrap();
        set_with(&mut slot, json!({"new": true, "title": "Second", "lists": [{"name": "Next", "steps": [{"name": "X"}]}]}), 2)
            .unwrap();
        assert_eq!(outline(&slot), "l2 Next: s2 X");
        let plan = slot.as_ref().unwrap();
        assert_eq!(plan.title.as_deref(), Some("Second"));
        assert_eq!(plan.rev, 2);
    }

    // ---- limits: each row refused one over, plan unchanged ----

    #[test]
    fn lists_per_plan_refuses_nine_lists() {
        let mut slot = None;
        let lists: Vec<_> = (1..=9).map(|i| json!({"name": format!("L{i}"), "steps": [{"name": "x"}]})).collect();
        assert_set_refused(&mut slot, json!({"lists": lists}), "lists per plan");
    }

    #[test]
    fn steps_per_list_refuses_twenty_one_steps() {
        let mut slot = three_steps();
        let steps: Vec<_> = (1..=21).map(|i| json!({"name": format!("S{i}")})).collect();
        assert_set_refused(&mut slot, json!({"lists": [{"name": "Build", "steps": steps}]}), "steps per list");
    }

    #[test]
    fn steps_in_all_refuses_one_hundred_twenty_one_steps() {
        let mut slot = three_steps();
        let mut lists: Vec<_> = (1..=6)
            .map(|i| {
                let steps: Vec<_> = (1..=20).map(|j| json!({"name": format!("S{i}-{j}")})).collect();
                json!({"name": format!("L{i}"), "steps": steps})
            })
            .collect();
        lists.push(json!({"name": "L7", "steps": [{"name": "one more"}]}));
        assert_set_refused(&mut slot, json!({"lists": lists}), "steps in all");
    }

    fn six_lists_of_nineteen() -> serde_json::Value {
        let lists: Vec<_> = (1..=6)
            .map(|i| {
                let steps: Vec<_> = (1..=19).map(|j| json!({"name": format!("S{i}-{j}")})).collect();
                json!({"name": format!("L{i}"), "steps": steps})
            })
            .collect();
        json!({"lists": lists})
    }

    #[test]
    fn steps_in_all_counts_the_steps_of_fixes() {
        let mut slot = None;
        set_with(&mut slot, six_lists_of_nineteen(), 1).unwrap();
        report(&mut slot, "s1", "failed", &[], 2).unwrap();
        // 114 steps, plus six fix steps, is exactly 120.
        fix_on(&mut slot, "s1", &["a", "b", "c", "d", "e", "f"], 3).unwrap();
        assert_eq!(slot.as_ref().unwrap().ids.step, 120, "six fix steps take ids s115..s120");
        // Re-send the same six lists with one more list: 120 + 1 is over the limit.
        let mut again = six_lists_of_nineteen();
        again["lists"].as_array_mut().unwrap().push(json!({"name": "Extra", "steps": [{"name": "one more"}]}));
        assert_set_refused(&mut slot, again, "steps in all");
    }

    #[test]
    fn name_refuses_sixty_one_characters_and_takes_sixty() {
        let mut slot = None;
        assert_set_refused(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "n".repeat(61)}]}]}), "name");
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "n".repeat(60)}]}]}), 2).unwrap();
        assert_eq!(slot.as_ref().unwrap().lists[0].steps[0].name.chars().count(), 60);
    }

    #[test]
    fn list_name_is_held_to_the_same_limit() {
        let mut slot = None;
        assert_set_refused(&mut slot, json!({"lists": [{"name": "b".repeat(61), "steps": []}]}), "name");
    }

    #[test]
    fn what_refuses_301_characters_and_takes_300() {
        let mut slot = None;
        assert_set_refused(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A", "what": "w".repeat(301)}]}]}), "what");
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A", "what": "w".repeat(300)}]}]}), 2).unwrap();
        assert_eq!(slot.as_ref().unwrap().lists[0].steps[0].what.chars().count(), 300);
    }

    #[test]
    fn title_refuses_81_characters_and_takes_80() {
        let mut slot = None;
        assert_set_refused(&mut slot, json!({"title": "t".repeat(81), "lists": [{"name": "Build", "steps": []}]}), "title");
        set_with(&mut slot, json!({"title": "t".repeat(80), "lists": [{"name": "Build", "steps": []}]}), 2).unwrap();
        assert_eq!(slot.as_ref().unwrap().title.as_ref().unwrap().chars().count(), 80);
    }

    #[test]
    fn note_refuses_201_characters_and_takes_200() {
        let mut slot = failed_s1();
        let before = bytes(&slot);
        let long = StepCall { step: "s1".into(), state: State::Failed, note: Some("n".repeat(201)) };
        let err = step(&mut slot, long, &[], 3).expect_err("a 201-character note is refused");
        assert!(err.contains("note"), "error should name note, got {err:?}");
        assert_eq!(bytes(&slot), before);
        let ok = StepCall { step: "s1".into(), state: State::Failed, note: Some("n".repeat(200)) };
        step(&mut slot, ok, &[], 4).unwrap();
        assert_eq!(slot.as_ref().unwrap().lists[0].steps[0].note.as_ref().unwrap().chars().count(), 200);
    }

    #[test]
    fn limits_count_characters_after_trimming() {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": format!("   {}   ", "n".repeat(60))}]}]}), 1)
            .unwrap();
        assert_eq!(slot.as_ref().unwrap().lists[0].steps[0].name, "n".repeat(60));
    }

    #[test]
    fn name_and_title_refuse_a_newline_and_what_accepts_one() {
        let mut slot = None;
        assert_set_refused(&mut slot, json!({"lists": [{"name": "Build\nNow", "steps": []}]}), "name");
        assert_set_refused(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A\nB"}]}]}), "name");
        assert_set_refused(&mut slot, json!({"title": "Two\nLines", "lists": [{"name": "Build", "steps": []}]}), "title");
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "A", "what": "line one\nline two"}]}]}), 2)
            .unwrap();
        assert_eq!(slot.as_ref().unwrap().lists[0].steps[0].what, "line one\nline two");
    }

    #[test]
    fn note_refuses_a_control_character_other_than_a_newline() {
        let mut slot = failed_s1();
        let before = bytes(&slot);
        let bell = StepCall { step: "s1".into(), state: State::Failed, note: Some("ok\u{7}".into()) };
        let err = step(&mut slot, bell, &[], 3).expect_err("a bell in a note is refused");
        assert!(err.contains("note"), "error should name note, got {err:?}");
        assert_eq!(bytes(&slot), before);
    }

    #[test]
    fn fix_refuses_a_seventh_step() {
        let mut slot = failed_s1();
        assert_fix_refused(&mut slot, "s1", &["a", "b", "c", "d", "e", "f", "g"], "steps per fix");
    }

    #[test]
    fn fix_refuses_a_fourth_fix_on_one_step() {
        let mut slot = failed_s1();
        fix_on(&mut slot, "s1", &["one"], 3).unwrap();
        fix_on(&mut slot, "s1", &["two"], 4).unwrap();
        fix_on(&mut slot, "s1", &["three"], 5).unwrap();
        assert_fix_refused(&mut slot, "s1", &["four"], "fixes per step");
    }

    // ---- derive: one row per state in the table ----

    #[test]
    fn table_working_task_reads_running_unless_the_worker_waits_on_the_user() {
        let plan = linked("t1");
        let mut f = task("t1", Work::Working);
        assert_eq!(s1_under(&plan, &[f.clone()]).state, State::Running);
        f.waits_on_user = true;
        assert_eq!(s1_under(&plan, &[f]).state, State::Waiting);
    }

    #[test]
    fn table_waiting_for_children_reads_running_unless_the_worker_waits_on_the_user() {
        let plan = linked("t1");
        let mut f = task("t1", Work::WaitingForChildren);
        assert_eq!(s1_under(&plan, &[f.clone()]).state, State::Running);
        f.waits_on_user = true;
        assert_eq!(s1_under(&plan, &[f]).state, State::Waiting);
    }

    #[test]
    fn table_result_accepted_reads_done() {
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
        assert_eq!((s.state, s.why), (State::Failed, Some("usage limit reached".into())));
    }

    #[test]
    fn table_cancelled_task_reads_failed_cancelled() {
        let plan = linked("t1");
        let s = s1_under(&plan, &[ended("t1", Work::Cancelled, 2000)]);
        assert_eq!((s.state, s.why), (State::Failed, Some("Cancelled".into())));
    }

    #[test]
    fn table_no_task_reads_the_reported_state() {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "Work"}]}]}), 1).unwrap();
        report(&mut slot, "s1", "running", &[], 2).unwrap();
        let s = s1_under(&slot, &[]);
        assert_eq!((s.state, s.why), (State::Running, None));
    }

    #[test]
    fn report_is_refused_while_the_linked_task_is_working_and_names_it() {
        let plan = linked("t1");
        let mut slot = plan;
        let before = bytes(&slot);
        let err = report(&mut slot, "s1", "done", &[task("t1", Work::Working)], 1500).expect_err("refused");
        assert!(err.contains("t1"), "error should name the task, got {err:?}");
        assert_eq!(bytes(&slot), before);
    }

    #[test]
    fn a_report_after_the_task_ends_wins_over_the_task() {
        let plan = linked("t1");
        let mut slot = plan;
        report(&mut slot, "s1", "running", &[ended("t1", Work::ResultAvailable, 2000)], 3000).unwrap();
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Accepted;
        assert_eq!(s1_under(&slot, &[f]).state, State::Running);
    }

    #[test]
    fn a_report_before_the_task_ends_loses_to_the_task() {
        let mut slot = None;
        set_with(&mut slot, json!({"lists": [{"name": "Build", "steps": [{"name": "Work"}]}]}), 1).unwrap();
        report(&mut slot, "s1", "running", &[], 1500).unwrap();
        slot.as_mut().unwrap().lists[0].steps[0].task = Some("t1".into());
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Accepted;
        assert_eq!(s1_under(&slot, &[f]).state, State::Done);
    }

    // ---- derive: list-level links ----

    #[test]
    fn list_task_working_runs_the_first_step_not_done() {
        let mut slot = three_steps();
        report(&mut slot, "s1", "done", &[], 1100).unwrap();
        slot.as_mut().unwrap().lists[0].task = Some("t1".into());
        let view = derive(slot.as_ref().unwrap(), &[task("t1", Work::Working)]);
        let (s1, s2, s3) = (view_step(&view, "s1"), view_step(&view, "s2"), view_step(&view, "s3"));
        assert_eq!((s1.state, s1.why.clone()), (State::Done, None));
        assert_eq!((s2.state, s2.why.clone()), (State::Running, None));
        assert_eq!((s3.state, s3.why.clone()), (State::Pending, None));
    }

    #[test]
    fn list_task_result_finishes_the_steps_the_captain_never_reported() {
        let mut slot = three_steps();
        report(&mut slot, "s1", "done", &[], 1100).unwrap();
        slot.as_mut().unwrap().lists[0].task = Some("t1".into());
        let view = derive(slot.as_ref().unwrap(), &[ended("t1", Work::ResultAvailable, 2000)]);
        let reason = Some("Done with the list's task".to_string());
        let (s1, s2, s3) = (view_step(&view, "s1"), view_step(&view, "s2"), view_step(&view, "s3"));
        assert_eq!((s1.state, s1.why.clone()), (State::Done, None));
        assert_eq!((s2.state, s2.why.clone()), (State::Done, reason.clone()));
        assert_eq!((s3.state, s3.why.clone()), (State::Done, reason));
    }

    #[test]
    fn list_task_failure_fails_the_running_step() {
        let mut slot = three_steps();
        slot.as_mut().unwrap().lists[0].task = Some("t1".into());
        let mut f = ended("t1", Work::Failed, 2000);
        f.failure = Some("build crashed".into());
        let view = derive(slot.as_ref().unwrap(), &[f]);
        let (s1, s2) = (view_step(&view, "s1"), view_step(&view, "s2"));
        assert_eq!((s1.state, s1.why.clone()), (State::Failed, Some("build crashed".into())));
        assert_eq!((s2.state, s2.why.clone()), (State::Pending, None));
    }

    #[test]
    fn list_task_redone_fails_the_steps_it_would_finish() {
        let mut slot = three_steps();
        slot.as_mut().unwrap().lists[0].task = Some("t1".into());
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Redone;
        let view = derive(slot.as_ref().unwrap(), &[f]);
        for id in ["s1", "s2", "s3"] {
            let s = view_step(&view, id);
            assert_eq!((s.state, s.why.clone()), (State::Failed, Some("Redone".into())), "step {id}");
        }
    }

    #[test]
    fn a_step_link_beats_the_list_link() {
        let mut slot = three_steps();
        slot.as_mut().unwrap().lists[0].task = Some("t1".into());
        slot.as_mut().unwrap().lists[0].steps[0].task = Some("t2".into());
        let mut f = ended("t1", Work::ResultAvailable, 2000);
        f.review = Review::Accepted;
        let view = derive(slot.as_ref().unwrap(), &[f, task("t2", Work::Working)]);
        assert_eq!(view_step(&view, "s1").state, State::Running);
        assert_eq!(view_step(&view, "s2").state, State::Done);
    }

    #[test]
    fn a_list_link_cost_is_the_rise_since_the_step_started() {
        let mut slot = three_steps();
        slot.as_mut().unwrap().lists[0].task = Some("t1".into());
        let mut working = task("t1", Work::Working);
        working.cost = Some(1.0);
        assert!(settle(slot.as_mut().unwrap(), std::slice::from_ref(&working), 1500));
        assert_eq!(slot.as_ref().unwrap().lists[0].steps[0].cost_at_start, Some(1.0));

        let mut ended_task = ended("t1", Work::ResultAvailable, 2000);
        ended_task.review = Review::Accepted;
        ended_task.cost = Some(3.5);
        let view = derive(slot.as_ref().unwrap(), &[ended_task]);
        assert_eq!(view_step(&view, "s1").cost, Some(2.5));
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
        let mut slot = three_steps();
        assert_fix_refused(&mut slot, "s1", &["Patch"], "not failed");
    }

    #[test]
    fn fix_branch_merges_back_as_done_after_one_fix() {
        let mut slot = three_steps();
        report(&mut slot, "s1", "failed", &[], 2).unwrap();
        let answer = fix_on(&mut slot, "s1", &["Patch"], 3).unwrap();
        assert_eq!(answer, "Fix f1 on s1 has steps s4.");
        assert_eq!(view_step(&derive(slot.as_ref().unwrap(), &[]), "s1").state, State::Failed);

        report(&mut slot, "s4", "done", &[], 4).unwrap();
        let s = view_step(&derive(slot.as_ref().unwrap(), &[]), "s1").clone();
        assert_eq!((s.state, s.why), (State::Done, Some("after one fix".into())));
    }

    #[test]
    fn two_fixes_merge_back_as_done_after_2_fixes() {
        let mut slot = three_steps();
        report(&mut slot, "s1", "failed", &[], 2).unwrap();
        fix_on(&mut slot, "s1", &["First try"], 3).unwrap();
        report(&mut slot, "s4", "failed", &[], 4).unwrap();
        fix_on(&mut slot, "s1", &["Second try"], 5).unwrap();
        report(&mut slot, "s5", "done", &[], 6).unwrap();

        let s = view_step(&derive(slot.as_ref().unwrap(), &[]), "s1").clone();
        assert_eq!((s.state, s.why), (State::Done, Some("after 2 fixes".into())));
        assert_eq!(slot.as_ref().unwrap().lists[0].steps[0].fixes.len(), 2);
    }

    #[test]
    fn a_fix_step_that_fails_keeps_the_branch_red() {
        let mut slot = three_steps();
        report(&mut slot, "s1", "failed", &[], 2).unwrap();
        fix_on(&mut slot, "s1", &["Patch"], 3).unwrap();
        report(&mut slot, "s4", "failed", &[], 4).unwrap();
        assert_eq!(view_step(&derive(slot.as_ref().unwrap(), &[]), "s1").state, State::Failed);
    }

    // ---- settle and glance ----

    #[test]
    fn finished_at_is_set_once_every_main_step_is_done() {
        let mut slot = three_steps();
        for id in ["s1", "s2", "s3"] {
            report(&mut slot, id, "done", &[], 2).unwrap();
        }
        let plan = slot.as_mut().unwrap();
        assert!(settle(plan, &[], 4000));
        assert_eq!(plan.finished_at, Some(4000));
        assert!(!settle(plan, &[], 5000), "a second settle with nothing new changes nothing");
        assert_eq!(glance(Some(plan), &[]).state, CaptainState::Finished);
    }

    #[test]
    fn glance_dots_for_a_fixed_plan_are_literal() {
        let mut slot = None;
        set_with(
            &mut slot,
            json!({"lists": [
                {"name": "Build", "steps": [{"name": "A"}, {"name": "B"}, {"name": "C"}, {"name": "D"}, {"name": "E"}]},
                {"name": "Ship", "steps": [{"name": "F"}]}
            ]}),
            1,
        )
        .unwrap();
        report(&mut slot, "s1", "done", &[], 2).unwrap();
        report(&mut slot, "s2", "failed", &[], 3).unwrap();
        fix_on(&mut slot, "s2", &["Patch"], 4).unwrap();
        report(&mut slot, "s7", "done", &[], 5).unwrap();
        report(&mut slot, "s3", "running", &[], 6).unwrap();
        report(&mut slot, "s4", "waiting", &[], 7).unwrap();
        report(&mut slot, "s6", "failed", &[], 8).unwrap();

        let g = glance(slot.as_ref(), &[]);
        assert_eq!(g.lists[0].dots, "dxrwp");
        assert_eq!(g.lists[1].dots, "f");
        assert_eq!((g.done, g.total), (2, 6));
        assert_eq!(g.now.as_deref(), Some("C"));
        assert_eq!(g.state, CaptainState::NeedsYou);
    }

    #[test]
    fn glance_state_follows_the_table() {
        let waiting = linked("t1");
        let g = glance(waiting.as_ref(), &[task("t1", Work::Working)]);
        assert_eq!(g.state, CaptainState::Working, "a linked working task reads running, not waiting");
        let mut waits = task("t1", Work::Working);
        waits.waits_on_user = true;
        assert_eq!(glance(waiting.as_ref(), &[waits]).state, CaptainState::NeedsYou);

        let mut failed = three_steps();
        report(&mut failed, "s1", "failed", &[], 2).unwrap();
        assert_eq!(glance(failed.as_ref(), &[]).state, CaptainState::Fixing);

        let mut running = three_steps();
        report(&mut running, "s1", "running", &[], 2).unwrap();
        assert_eq!(glance(running.as_ref(), &[]).state, CaptainState::Working);

        let idle = three_steps();
        assert_eq!(glance(idle.as_ref(), &[]).state, CaptainState::Idle);
    }

    #[test]
    fn glance_is_empty_without_a_plan() {
        let g = glance(None, &[]);
        assert!(g.is_empty());
        assert_eq!(g.state, CaptainState::Idle);
    }

    #[test]
    fn glance_at_the_limits_stays_under_1200_bytes() {
        let mut slot = None;
        let lists: Vec<_> = (1..=8)
            .map(|i| {
                let steps: Vec<_> = (1..=15).map(|j| json!({"name": format!("{:03}-{}", i * 100 + j, "x".repeat(56))})).collect();
                json!({"name": format!("{:03}-{}", i, "n".repeat(56)), "steps": steps})
            })
            .collect();
        set_with(&mut slot, json!({"lists": lists}), 1).unwrap();
        report(&mut slot, "s1", "running", &[], 2).unwrap();
        let g = glance(slot.as_ref(), &[]);
        assert_eq!((g.lists.len(), g.total), (8, 120));
        let len = serde_json::to_string(&g).unwrap().len();
        assert!(len < 1200, "glance is {len} bytes");
    }
}
