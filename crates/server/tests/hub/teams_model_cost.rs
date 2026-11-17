//! Teams: a run's model cost. A team box's node reports the running total of its session's model
//! cost (`TeamOp::RunModelCost`); the hub keeps the most it was told for that run in its
//! `model_usd`, beside the box cost in its `cost_usd`, and holds the run's dollars limit to the
//! sum. Only the box the live run is bound to may report for it. A node that never reports keeps
//! box cost only.

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
    let box_usd = listed_run["cost_usd"].as_f64().unwrap();
    assert!((0.0..0.01).contains(&box_usd), "cost_usd is the box's only, as an older app reads it: {listed_run}");
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
fn a_model_cost_at_the_cap_is_kept_and_calls_once() {
    let hub = TestHub::start_with_team_policy("teams_model_cost_cap", fast(), policy());
    let mut tb = TeamBox::new(&hub);
    bot(&hub, &tb, 5.0);
    let run = run(&hub, &tb);
    // A node whose total passed the cap sends the cap (`MAX_RUN_MODEL_USD`), each pass.
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 10_000.0), Ok(Value::Null), "exactly the cap is taken");
    assert_eq!(model_usd(&hub, run), "10000");
    wait_for("the limit call", || !limit_calls(&mut tb).is_empty());
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 10_000.0), Ok(Value::Null));
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!((model_usd(&hub, run), limit_calls(&mut tb).len()), ("10000".to_owned(), 1), "one limit call at the cap");
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
    // A box with no live run idles out: each team's run is there before the next box boots.
    let other = TeamBox::new_as(&hub, "rita@example.test");
    bot(&hub, &other, 5.0);
    let other_run = run(&hub, &other);
    let mut tb = TeamBox::new(&hub);
    bot(&hub, &tb, 5.0);
    let run = run(&hub, &tb);
    let refused = Err("this box runs no live run of that workflow on that subject".to_owned());
    assert_eq!(report(&hub, &other.box_id, &tb.workflow, 4.0), refused, "another team's box");
    // Team A's run names team B's box as its own: B's box is still no box of team A.
    sql(&hub, &format!("update team_runs set box = '{}' where id = '{run}'", other.machine));
    assert_eq!(report(&hub, &other.box_id, &tb.workflow, 4.0), refused, "another team's box the run names");
    assert_eq!(model_usd(&hub, other_run), "0");
    // The run moved to another machine of the team: this box no longer runs it.
    sql(&hub, &format!("update team_runs set box = '{}' where id = '{run}'", Uuid::new_v4()));
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 4.0), refused, "a box the run is not bound to");
    assert_eq!(model_usd(&hub, run), "0", "nothing was stored");
    assert_eq!(listed(&mut tb, run)["model_usd"], json!(0.0));
    sql(&hub, &format!("update team_runs set box = '{}' where id = '{run}'", tb.machine));
    let out_of_range = Err("a model cost is a number of dollars, 0 to 10000".to_owned());
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, -1.0), out_of_range);
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 10_000.01), out_of_range);
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, f64::MAX), out_of_range, "a total no later report could correct");
    assert_eq!(model_usd(&hub, run), "0", "nothing was stored");
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 0.5), Ok(Value::Null), "the box that runs it reports");
    assert_eq!((model_usd(&hub, run), model_usd(&hub, other_run)), ("0.5".to_owned(), "0".to_owned()));
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

/// A second box of `tb`'s team, booted for a run of a second bot of the team on issue `team-2`
/// (a bot runs one run at a time): its machine and box.
fn second_box(hub: &TestHub, tb: &mut TeamBox) -> (String, String) {
    let team = tb.team.to_string();
    let repo = temp(&format!("team-repo-2-{}", hub.database));
    tb.owner.client.hub(json!({ "cmd": "team", "team": team, "op": { "op": "save_bot", "bot": { "workflow": "" }, "connector": {
        "name": "Header", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
        "folder": repo, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
        "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
    } } }))
    .unwrap();
    let workflow = team_workflows(hub, tb.team).iter().map(|w| w["id"].as_str().unwrap().to_owned()).find(|id| *id != tb.workflow).unwrap();
    tb.owner.client.hub(json!({ "cmd": "team", "team": team, "op": { "op": "turn_bot_on", "workflow": workflow, "on": true } })).unwrap();
    let run = Run {
        reservation: None,
        cwd: repo,
        repo: None,
        args: vec!["--name".into(), "Fix the header".into()],
        connector: workflow,
        subject: issue("team-2"),
        mentioned: vec![],
        account: None,
        runs_on: RunsOn::FreshBox(BoxStep { size: Some(BoxSize::Small) }),
        input: String::new(),
    };
    hub.runtime.block_on(pinned_server::teams::triggers::fired(hub.hub(), tb.team, Work::Run(run))).unwrap();
    let mut second = None;
    wait_for("the team's second box and its session", || {
        let boxes = hub.runtime.block_on(hub.hub().db.boxes(tb.team)).unwrap();
        second = boxes.into_iter().flatten().find(|b| b.machine.to_string() != tb.machine).and_then(|b| Some((b.machine.to_string(), b.upstash?.0)));
        second.as_ref().is_some_and(|(machine, _)| hub.runtime.block_on(hub.hub().account(tb.team)).unwrap().tabs_of(machine).is_some_and(|tabs| !tabs.is_empty()))
    });
    second.unwrap()
}

#[test]
fn a_report_for_an_ended_run_from_a_person_a_non_team_a_non_box_or_another_box_of_the_team_is_refused() {
    let hub = TestHub::start_with_team_policy("teams_model_cost_refused", fast(), policy());
    let mut tb = TeamBox::new(&hub);
    bot(&hub, &tb, 5.0);
    let run = run(&hub, &tb);
    let refused = Err("this box runs no live run of that workflow on that subject".to_owned());
    report(&hub, &tb.box_id, &tb.workflow, 0.2).unwrap();

    // A person's client, naming the team or not.
    let sent = json!({ "op": "run_model_cost", "workflow": tb.workflow, "subject": { "issue": "team-1" }, "usd": 3.0 });
    let by_a_person = Err("only a team's box reports what its run's model cost".to_owned());
    assert_eq!(op(tb.team, &mut tb.owner.client, sent.clone()), by_a_person);
    assert_eq!(tb.owner.client.hub(json!({ "cmd": "team", "op": sent })), by_a_person);
    assert_eq!(model_usd(&hub, run), "0.2", "a person's report changes nothing");

    // A second real box of the same team, which runs the bot on another issue.
    let (machine, second) = second_box(&hub, &mut tb);
    assert_ne!(machine, tb.machine);
    assert_eq!(report(&hub, &second, &tb.workflow, 3.0), refused, "the team's other box");
    assert_eq!(model_usd(&hub, run), "0.2", "the other box's report changes nothing");

    // The run ended: its box reports no more for it.
    sql(&hub, &format!("update team_runs set state = 'done', ended_at = now() where id = '{run}'"));
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 3.0), refused, "a run that ended");
    assert_eq!(model_usd(&hub, run), "0.2", "an ended run's cost stays");

    // The run is live again and still names the machine, but the machine is no longer one of
    // the team's boxes.
    sql(&hub, &format!("update team_runs set state = 'working', ended_at = null where id = '{run}'"));
    let moved = Uuid::new_v4();
    sql(&hub, &format!("update boxes set machine = '{moved}' where account = '{}' and machine = '{}'", tb.team, tb.machine));
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 3.0), refused, "a machine that is no box of the team");
    assert_eq!(model_usd(&hub, run), "0.2", "a machine that is no box changes nothing");
    sql(&hub, &format!("update boxes set machine = '{}' where machine = '{moved}'", tb.machine));
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 0.3), Ok(Value::Null), "its box again");

    // The box's account is no team.
    sql(&hub, &format!("update accounts set kind = 'person' where id = '{}'", tb.team));
    assert_eq!(report(&hub, &tb.box_id, &tb.workflow, 3.0), refused, "a machine of an account that is no team");
    assert_eq!(model_usd(&hub, run), "0.3", "a non-team machine's report changes nothing");
}
