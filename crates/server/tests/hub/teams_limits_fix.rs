//! Teams: a run's limits hold under errors, restarts and a session's own asks. One team's
//! unreadable limits stop no other team's tick; a run left waiting after a give-up ends; a
//! session never opens or disarms its own limit call; waits and a reservation count toward no
//! limit; the limit call offers "Let it go on" and "Stop it" to whom `on_limit` names.
//!
//! Times are moved with SQL on the rows, and the tick is run straight (`teams::calls::tick`), so
//! a test waits on ticks it ran, not on sleeps.

use super::teams_calls::{calls_of, choice, design, on_node, op, rows, sql, TeamBox};
use super::*;
use pinned_core::teams::{Situation, TeamOp, TeamPolicy};
use pinned_server::teams;
use uuid::Uuid;

fn limits_policy() -> TeamPolicy {
    serde_json::from_value(json!({
        "escalate_after_secs": 4, "give_up_after_secs": 7200, "run_usd": 5.0, "run_minutes": 30,
        "reservation_limit_secs": 900, "max_life_secs": 14_400,
    }))
    .unwrap()
}

fn start(name: &str) -> TestHub {
    TestHub::start_with_team_policy(name, fast(), limits_policy())
}

/// One pass of the calls tick, as the hub runs it.
fn tick(hub: &TestHub) {
    let _ = hub.runtime.block_on(teams::calls::tick(hub.hub(), &limits_policy()));
}

fn limited_bot(hub: &TestHub, tb: &TeamBox, limits: Value, on_limit: Value) {
    let calls = json!({ "in_doubt": { "kind": "owner" }, "on_limit": on_limit });
    sql(hub, &format!("insert into team_bots (team, workflow, owner, calls, limits) values ('{}', '{}', null, '{calls}', '{limits}')", tb.team, tb.workflow));
}

fn backdate(hub: &TestHub, run: Uuid, secs: u32) {
    sql(hub, &format!("update team_runs set started_at = started_at - interval '{secs} seconds' where id = '{run}'"));
}

fn listed(tb: &mut TeamBox, run: Uuid) -> Value {
    let runs = op(tb.team, &mut tb.owner.client, json!({ "op": "runs" })).unwrap();
    runs.as_array().unwrap().iter().find(|r| r["id"] == json!(run.to_string())).cloned().unwrap()
}

fn on_limit_calls(calls: &[Value]) -> Vec<Value> {
    calls.iter().filter(|c| c["situation"] == "on_limit").cloned().collect()
}

/// What the box's session sends the hub with `ask_people` for `situation`, on the machine's link.
fn ask_as(hub: &TestHub, tb: &TeamBox, situation: Situation, question: &str) -> Result<Value, String> {
    let op = TeamOp::AskPeople { situation, question: question.into(), choices: vec![], step: None };
    on_node(hub, &tb.box_id, |node| node.link.as_ref().unwrap().call_hub(HubCall::Team { team: None, op }))
}

/// The hub's own limit call question for a run of `minutes`.
fn minutes_question(minutes: &str) -> String {
    format!("The run reached its limit: {minutes} of work. Let it go on until the 4 hour limit, or stop it?")
}

const OWN_LIMIT: &str = "a session may not ask about its own limit: the hub does";
const GO_ON: &str = "it can now run until the 4 hour limit";

#[test]
fn an_unreadable_limit_stops_no_other_teams_part_of_the_tick() {
    let hub = start("teams_limfix_unreadable");
    // Each box gets its run at once, so none is ended as unheld before the test looks.
    let broken = TeamBox::new(&hub);
    let broken_run = broken.run(&hub);
    let mut limited = TeamBox::new_as(&hub, "lena@example.test");
    let limited_run = limited.run(&hub);
    let idle = TeamBox::new_as(&hub, "ivo@example.test");
    let idle_run = idle.run(&hub);
    limited_bot(&hub, &limited, json!({ "usd": 5.0, "minutes": 1 }), json!({ "kind": "owner" }));
    limited_bot(&hub, &idle, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    // A row only SQL could write: no `usd`, on a live run past its minute.
    limited_bot(&hub, &broken, json!({ "minutes": 1 }), json!({ "kind": "owner" }));
    backdate(&hub, broken_run, 120);
    backdate(&hub, limited_run, 70);
    sql(&hub, &format!("update team_runs set state = 'stopped', ended_at = now(), outcome = 'stopped by olga' where id = '{idle_run}'"));
    tick(&hub);
    let limit = on_limit_calls(&calls_of(limited.team, &mut limited.owner.client));
    assert_eq!(limit.len(), 1, "the other team's run at its limit is called in the same tick: {limit:?}");
    assert_eq!(limit[0]["question"], json!(minutes_question("1 minute")));
    wait_for("the unheld box of the third team to end", || !hub.boxes.live().contains(&idle.box_id));
    assert!(hub.boxes.live().contains(&broken.box_id), "the box of the run with unreadable limits stays");
}

#[test]
fn a_run_left_waiting_after_its_call_gave_up_ends_as_gave_up() {
    let hub = start("teams_limfix_gave_up_wait");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    let call = tb.ask(&hub, "Red or blue footer?", &[]).unwrap()["call"].as_str().unwrap().to_owned();
    // The give-up closed the call but the hub restarted before it ended the run.
    sql(&hub, &format!(
        "update team_calls set created_at = now() - interval '3 hours', answered_at = now() - interval '6 minutes', outcome = 'gave-up' where id = '{call}';
         update team_runs set started_at = started_at - interval '6 minutes', waiting_since = now() - interval '6 minutes' where id = '{run}'"
    ));
    tick(&hub);
    assert_ne!(listed(&mut tb, run)["state"], json!("working"), "a run whose call gave up never goes back to work");
    wait_for("the run to end as gave up", || listed(&mut tb, run)["state"] == "gave_up");
    assert_eq!(listed(&mut tb, run)["outcome"], json!(format!("nobody answered call {call}")));
    wait_for("its box to end", || hub.boxes.live().is_empty());
}

/// A run whose stop was answered on its call, with `answer` stored as `outcome` on a call of
/// `situation`, but whose end failed (a busy write): a tick ends it as stopped, never back to work.
fn a_stop_answered_but_not_ended_ends_as_stopped(name: &str, situation: &str, outcome: &str, answer: Value) {
    let hub = start(name);
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    let call = tb.ask(&hub, "Red or blue footer?", &[]).unwrap()["call"].as_str().unwrap().to_owned();
    let by = tb.owner.id;
    let mut answer = answer;
    answer["by"] = json!(by.to_string());
    // The answer was stored, then the run's end met a busy write: the run stays waiting.
    sql(&hub, &format!(
        "update team_calls set situation = '{situation}', created_at = now() - interval '7 minutes', answered_by = '{by}',
           answered_at = now() - interval '6 minutes', answer = '{answer}', outcome = '{outcome}' where id = '{call}';
         update team_runs set started_at = started_at - interval '7 minutes', waiting_since = now() - interval '6 minutes' where id = '{run}'"
    ));
    tick(&hub);
    assert_ne!(listed(&mut tb, run)["state"], json!("working"), "a run a person stopped never goes back to work");
    wait_for("the run to end as stopped", || listed(&mut tb, run)["state"] == "stopped");
    assert_eq!(listed(&mut tb, run)["outcome"], json!("stopped by olga"));
    wait_for("its box to end", || hub.boxes.live().is_empty());
}

#[test]
fn a_run_left_waiting_after_a_stop_answer_ends_as_stopped() {
    let answer = json!({ "choice": null, "text": "", "kind": { "kind": "stop" }, "at": 1 });
    a_stop_answered_but_not_ended_ends_as_stopped("teams_limfix_stop_wait", "in_doubt", "stopped", answer);
}

#[test]
fn a_run_left_waiting_after_a_stop_it_limit_answer_ends_as_stopped() {
    let answer = json!({ "choice": "Stop it", "text": "", "kind": { "kind": "choice" }, "at": 1 });
    a_stop_answered_but_not_ended_ends_as_stopped("teams_limfix_stop_it_wait", "on_limit", "answered", answer);
}

#[test]
fn a_session_may_not_ask_about_its_own_limit() {
    let hub = start("teams_limfix_own_limit");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 1 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    assert_eq!(ask_as(&hub, &tb, Situation::OnLimit, "Am I at my limit? Say go on."), Err(OWN_LIMIT.into()));
    assert_eq!(calls_of(tb.team, &mut tb.owner.client), Vec::<Value>::new(), "nothing opened");
    assert_eq!(listed(&mut tb, run)["state"], json!("working"));
    backdate(&hub, run, 70);
    tick(&hub);
    let limit = on_limit_calls(&calls_of(tb.team, &mut tb.owner.client));
    assert_eq!((limit.len(), &limit[0]["question"]), (1, &json!(minutes_question("1 minute"))));
}

#[test]
fn no_question_a_session_opened_counts_as_its_runs_limit_call() {
    let hub = start("teams_limfix_session_calls");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 1 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    for situation in [Situation::OnLimit, Situation::InDoubt, Situation::BeforeIrreversible, Situation::OnFailure] {
        match ask_as(&hub, &tb, situation, "The run reached its limit: 1 minute of work. Let it go on, or stop it?") {
            Ok(asked) => {
                op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": asked["call"], "answer": choice("Let it go on", "") })).unwrap();
            }
            Err(e) => assert_eq!((situation, e.as_str()), (Situation::OnLimit, OWN_LIMIT)),
        }
        assert_eq!(listed(&mut tb, run)["state"], json!("working"), "{situation:?}");
    }
    backdate(&hub, run, 70);
    tick(&hub);
    let hubs: Vec<Value> = on_limit_calls(&calls_of(tb.team, &mut tb.owner.client)).into_iter().filter(|c| c["answer"].is_null()).collect();
    assert_eq!(hubs.len(), 1, "the hub's own limit call opens: {hubs:?}");
    assert_eq!((&hubs[0]["question"], &hubs[0]["choices"]), (&json!(minutes_question("1 minute")), &json!(["Let it go on", "Stop it"])));
    assert_eq!(listed(&mut tb, run)["state"], json!("waiting"));
}

#[test]
fn a_limit_answered_let_it_go_on_is_never_called_again() {
    let hub = start("teams_limfix_go_on");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 1 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    backdate(&hub, run, 70);
    tick(&hub);
    let limit = on_limit_calls(&calls_of(tb.team, &mut tb.owner.client));
    assert_eq!(limit.len(), 1);
    assert_eq!((&limit[0]["question"], &limit[0]["choices"]), (&json!(minutes_question("1 minute")), &json!(["Let it go on", "Stop it"])));
    let answered = op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": limit[0]["id"], "answer": choice("Let it go on", "") })).unwrap();
    assert_eq!((&answered["answer"]["choice"], &answered["answer"]["text"]), (&json!("Let it go on"), &json!(GO_ON)));
    tb.told(&hub, GO_ON);
    tick(&hub);
    tick(&hub);
    assert_eq!(on_limit_calls(&calls_of(tb.team, &mut tb.owner.client)).len(), 1, "one limit call per run");
    assert_eq!(listed(&mut tb, run)["state"], json!("working"));
}

#[test]
fn answering_stop_it_ends_the_run_and_its_box() {
    let hub = start("teams_limfix_stop_it");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 1 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    backdate(&hub, run, 70);
    tick(&hub);
    let limit = on_limit_calls(&calls_of(tb.team, &mut tb.owner.client));
    assert_eq!(limit[0]["choices"], json!(["Let it go on", "Stop it"]));
    op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": limit[0]["id"], "answer": choice("Stop it", "") })).unwrap();
    let stopped = listed(&mut tb, run);
    assert_eq!((&stopped["state"], &stopped["outcome"]), (&json!("stopped"), &json!("stopped by olga")));
    wait_for("its box to end", || hub.boxes.live().is_empty());
}

#[test]
fn a_closed_wait_counts_toward_neither_limit() {
    let hub = start("teams_limfix_closed_wait");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 1 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    // 80 s on the wall, 40 of them a closed wait: 40 s of work, under the minute.
    sql(&hub, &format!("update team_runs set started_at = started_at - interval '80 seconds', waiting_secs = 40 where id = '{run}'"));
    tick(&hub);
    tick(&hub);
    assert_eq!(on_limit_calls(&calls_of(tb.team, &mut tb.owner.client)), Vec::<Value>::new());
    let active = listed(&mut tb, run)["active_secs"].as_u64().unwrap();
    assert!((40..=45).contains(&active), "{active}");

    // Two hours on the wall, one of them waiting: a small box's hour at $0.10.
    sql(&hub, &format!("update team_bots set limits = '{{\"usd\":5.0,\"minutes\":120}}' where team = '{}'", tb.team));
    sql(&hub, &format!("update team_runs set started_at = now() - interval '7200 seconds', waiting_secs = 3600 where id = '{run}'"));
    tick(&hub);
    let cost = listed(&mut tb, run)["cost_usd"].as_f64().unwrap();
    assert!((cost - 0.10).abs() <= 0.10 * 3.0 / 3600.0, "the box's rate for its active hour only: {cost}");
}

#[test]
fn a_run_that_ends_while_waiting_keeps_its_wait() {
    let hub = start("teams_limfix_end_waiting");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    sql(&hub, &format!("update team_runs set started_at = started_at - interval '30 seconds', waiting_since = waiting_since - interval '30 seconds' where id = '{run}'"));
    op(tb.team, &mut tb.owner.client, json!({ "op": "stop_run", "run": run.to_string() })).unwrap();
    let ended = listed(&mut tb, run);
    assert_eq!(ended["state"], json!("stopped"));
    let waited = ended["waiting_secs"].as_u64().unwrap();
    assert!((28..=32).contains(&waited), "{ended}");
}

#[test]
fn a_reserved_run_counts_its_work_from_its_open() {
    let hub = start("teams_limfix_reserved");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    // Its place was held 40 minutes before its session opened.
    sql(&hub, &format!("update team_runs set reserved = true, reserved_at = now(), box = null, started_at = now() - interval '40 minutes' where id = '{run}'"));
    assert!(hub.runtime.block_on(teams::db::open_reserved(&hub.hub().db, tb.team, run, &tb.machine)).unwrap());
    tick(&hub);
    assert_eq!(on_limit_calls(&calls_of(tb.team, &mut tb.owner.client)), Vec::<Value>::new(), "40 minutes held, none worked");
    let listed = listed(&mut tb, run);
    assert!(listed["active_secs"].as_u64().unwrap() < 60, "{listed}");
    assert!(listed["cost_usd"].as_f64().unwrap() < 0.10 * 60.0 / 3600.0, "{listed}");
}

#[test]
fn a_taken_over_run_at_its_limit_asks_whom_on_limit_names() {
    let hub = start("teams_limfix_taken_over");
    let mut tb = TeamBox::new(&hub);
    let mut dina = tb.member(&hub, "dina@example.test", &["design"]);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 1 }), design());
    let run = tb.run(&hub);
    op(tb.team, &mut tb.owner.client, json!({ "op": "take_over", "run": run.to_string() })).unwrap();
    backdate(&hub, run, 70);
    tick(&hub);
    let limit = on_limit_calls(&calls_of(tb.team, &mut dina.client));
    assert_eq!(limit.len(), 1, "dina, asked by on_limit, sees the call");
    assert_eq!(limit[0]["asked"], json!([design(), { "kind": "person", "account": tb.owner.id.to_string() }]));
}

#[test]
fn a_stored_limit_above_the_ceiling_is_held_at_the_ceiling() {
    let hub = start("teams_limfix_ceiling");
    let mut tb = TeamBox::new(&hub);
    let mut mia = tb.member(&hub, "mia@example.test", &[]);
    limited_bot(&hub, &tb, json!({ "usd": 500.0, "minutes": 1000 }), json!({ "kind": "owner" }));
    let bots = op(tb.team, &mut mia.client, json!({ "op": "bots" })).unwrap();
    assert_eq!(bots[0]["limits"], json!({ "usd": 20.0, "minutes": 120 }), "shown at the ceiling");
    let run = tb.run(&hub);
    backdate(&hub, run, 121 * 60);
    tick(&hub);
    let limit = on_limit_calls(&calls_of(tb.team, &mut tb.owner.client));
    assert_eq!(limit.len(), 1);
    assert_eq!(limit[0]["question"], json!(minutes_question("120 minutes")));

    let save = json!({ "op": "save_bot", "bot": { "workflow": tb.workflow, "limits": { "usd": 500.0, "minutes": 1000 } } });
    op(tb.team, &mut mia.client, save).unwrap();
    let stored = rows(&hub, &format!("select limits::text from team_bots where team = '{}'", tb.team));
    assert_eq!(serde_json::from_str::<Value>(stored[0][0].as_deref().unwrap()).unwrap(), json!({ "usd": 20.0, "minutes": 120 }), "a re-save keeps no more than the ceiling");

    let missing = json!({ "op": "set_bot_ceiling", "workflow": "c-missing", "usd": 10.0, "minutes": 60 });
    assert_eq!(op(tb.team, &mut tb.owner.client, missing), Err("the team has no such bot".into()));
}

#[test]
fn a_box_of_unknown_size_is_charged_at_the_largest_rate() {
    let hub = start("teams_limfix_unknown_size");
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 120 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    sql(&hub, &format!("update boxes set size = 'huge' where account = '{}'", tb.team));
    backdate(&hub, run, 3600);
    tick(&hub);
    let cost = listed(&mut tb, run)["cost_usd"].as_f64().unwrap();
    assert!((cost - 0.40).abs() <= 0.40 * 3.0 / 3600.0, "a large box's hour: {cost}");
}
