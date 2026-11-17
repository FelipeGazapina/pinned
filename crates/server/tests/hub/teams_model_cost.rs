//! Teams: a run's model cost. A team box's node reports the running total of its session's model
//! cost (`TeamOp::RunModelCost`); the hub keeps the most it was told for that run, adds it to the
//! box cost in the run's `cost_usd`, and holds the run's dollars limit to the sum. Only the box
//! the run is bound to may report for it. A node that never reports keeps box cost only.

use super::teams_calls::{calls_of, op, sql, TeamBox};
use super::*;
use pinned_core::teams::{TeamOp, TeamPolicy};
use uuid::Uuid;

fn policy() -> TeamPolicy {
    serde_json::from_value(json!({
        "escalate_after_secs": 4, "give_up_after_secs": 7200, "run_usd": 5.0, "run_minutes": 30,
        "reservation_limit_secs": 900, "max_life_secs": 14_400,
    }))
    .unwrap()
}

/// The box's bot, with `usd` and 30 minutes, asking the owner at its limit.
fn bot(hub: &TestHub, tb: &TeamBox, usd: f64) {
    let calls = json!({ "in_doubt": { "kind": "owner" }, "on_limit": { "kind": "owner" } });
    let limits = json!({ "usd": usd, "minutes": 30 });
    sql(hub, &format!("insert into team_bots (team, workflow, owner, calls, limits) values ('{}', '{}', null, '{calls}', '{limits}')", tb.team, tb.workflow));
}

/// A run of the box's bot on issue `team-1`, the subject its session works on.
fn run(hub: &TestHub, tb: &TeamBox) -> Uuid {
    let run = tb.run(hub);
    sql(hub, &format!("update team_runs set subject = '{{\"issue\":\"team-1\"}}' where id = '{run}'"));
    run
}

/// What the node of box `box_id` sends the hub: its session's model cost so far.
fn report(hub: &TestHub, box_id: &str, workflow: &str, usd: f64) -> Result<Value, String> {
    let op = TeamOp::RunModelCost { workflow: workflow.into(), subject: issue("team-1"), usd };
    super::teams_calls::on_node(hub, box_id, |node| node.link.as_ref().unwrap().call_hub(HubCall::Team { team: None, op }))
}

fn listed(tb: &mut TeamBox, run: Uuid) -> Value {
    let runs = op(tb.team, &mut tb.owner.client, json!({ "op": "runs" })).unwrap();
    runs.as_array().unwrap().iter().find(|r| r["id"] == json!(run.to_string())).cloned().unwrap()
}

fn limit_calls(tb: &mut TeamBox) -> Vec<Value> {
    calls_of(tb.team, &mut tb.owner.client).into_iter().filter(|c| c["situation"] == "on_limit").collect()
}

fn model_usd(hub: &TestHub, run: Uuid) -> String {
    super::teams_separation_fixes::sql_text(hub, &format!("select model_usd::text from team_runs where id = '{run}'"))
}

#[test]
fn a_runs_reported_model_cost_counts_toward_its_limit_and_calls_once() {
    let hub = TestHub::start_with_team_policy("teams_model_cost_limit", fast(), policy());
    let mut tb = TeamBox::new(&hub);
    bot(&hub, &tb, 0.5);
    let run = run(&hub, &tb);
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 0.2), Ok(Value::Null));
    let listed_run = listed(&mut tb, run);
    assert_eq!(listed_run["model_usd"], json!(0.2), "{listed_run}");
    let box_usd = listed_run["cost_usd"].as_f64().unwrap() - 0.2;
    assert!((0.0..0.01).contains(&box_usd), "cost is box plus model: {listed_run}");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(limit_calls(&mut tb), Vec::<Value>::new(), "$0.20 is under the $0.50 limit");

    report(&hub, &tb.box_id, &tb.workflow, 0.6).unwrap();
    let mut limit = vec![];
    wait_for("the limit call", || {
        limit = limit_calls(&mut tb);
        !limit.is_empty()
    });
    assert_eq!(limit[0]["question"], json!("The run reached its limit: $0.50 spent on its box and model. Let it go on until the 4 hour limit, or stop it?"));
    tb.told(&hub, "[The run reached its limit: $0.50 spent on its box and model. Stop and wait for the answer]");
    report(&hub, &tb.box_id, &tb.workflow, 0.9).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(limit_calls(&mut tb).len(), 1, "one limit call, however far past it goes");
}

#[test]
fn a_report_sent_twice_or_out_of_order_keeps_the_most_told() {
    let hub = TestHub::start_with_team_policy("teams_model_cost_max", fast(), policy());
    let mut tb = TeamBox::new(&hub);
    bot(&hub, &tb, 5.0);
    let run = run(&hub, &tb);
    report(&hub, &tb.box_id, &tb.workflow, 0.3).unwrap();
    report(&hub, &tb.box_id, &tb.workflow, 0.3).unwrap();
    assert_eq!(model_usd(&hub, run), "0.3", "a repeated total adds nothing");
    report(&hub, &tb.box_id, &tb.workflow, 0.1).unwrap();
    assert_eq!(model_usd(&hub, run), "0.3", "an older total arriving late lowers nothing");
    report(&hub, &tb.box_id, &tb.workflow, 0.45).unwrap();
    assert_eq!(listed(&mut tb, run)["model_usd"], json!(0.45));
}

#[test]
fn only_the_box_that_runs_the_run_reports_its_model_cost() {
    let hub = TestHub::start_with_team_policy("teams_model_cost_owner", fast(), policy());
    let mut tb = TeamBox::new(&hub);
    let other = TeamBox::new_as(&hub, "rita@example.test");
    bot(&hub, &tb, 5.0);
    let run = run(&hub, &tb);
    let refused = Err("this box runs no live run of that workflow on that subject".to_owned());
    assert_eq!(report(&hub, &other.box_id, &tb.workflow, 4.0), refused, "another team's box");
    // The run moved to another machine of the team: this box no longer runs it.
    sql(&hub, &format!("update team_runs set box = '{}' where id = '{run}'", Uuid::new_v4()));
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 4.0), refused, "a box the run is not bound to");
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, -1.0), Err("a model cost is a number of dollars, 0 or more".to_owned()));
    assert_eq!(model_usd(&hub, run), "0", "nothing was stored");
    assert_eq!(listed(&mut tb, run)["model_usd"], json!(0.0));
}

#[test]
fn a_node_that_reports_nothing_is_charged_box_cost_only_and_a_wait_costs_nothing() {
    let hub = TestHub::start_with_team_policy("teams_model_cost_none", fast(), policy());
    let mut tb = TeamBox::new(&hub);
    bot(&hub, &tb, 5.0);
    let run = run(&hub, &tb);
    sql(&hub, &format!("update team_runs set started_at = started_at - interval '36 seconds' where id = '{run}'"));
    let mut working = Value::Null;
    // A small box is $0.10 an hour: 36 s of work is $0.001.
    wait_for("the box charge", || {
        working = listed(&mut tb, run);
        working["cost_usd"].as_f64().unwrap() >= 0.001
    });
    assert_eq!(working["model_usd"], json!(0.0), "{working}");
    assert!(working["cost_usd"].as_f64().unwrap() < 0.002, "box cost only: {working}");

    report(&hub, &tb.box_id, &tb.workflow, 0.25).unwrap();
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    let waiting = listed(&mut tb, run);
    assert_eq!(waiting["state"], json!("waiting"));
    // The session spends no tokens while it waits: its node tells the same total again.
    report(&hub, &tb.box_id, &tb.workflow, 0.25).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    let later = listed(&mut tb, run);
    assert_eq!((&later["cost_usd"], &later["model_usd"]), (&waiting["cost_usd"], &json!(0.25)), "a wait costs nothing: {later}");
}
