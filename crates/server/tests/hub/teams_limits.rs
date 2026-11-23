//! Teams: a run's limits. Waiting on a call counts toward neither its minutes nor its dollars and
//! keeps its box from idling out, and an unanswered call ends it at give-up; one wait at a time;
//! the run's max life counts waiting and ends it; a run at a limit calls whom `on_limit` names;
//! a bot whose limit is not set does not start; the owner sets a bot's ceiling up to 4x the policy.
//!
//! Times are moved with SQL on the run's row (`started_at`, `waiting_since`, the call's
//! `created_at`) instead of sleeping them out.

use super::teams_calls::{calls_of, choice, design, op, patient, rows, sql, TeamBox};
use super::teams_separation_fixes::{credential, locked_app, make_team, team_op, via, PASSWORD};
use super::*;
use pinned_core::teams::TeamPolicy;
use uuid::Uuid;

/// The calls tick every second, nothing gives up unless a test moves a call's time, and a run
/// lives `max_life_secs`. Read from JSON, as the hub's own config is.
fn limits_policy(max_life_secs: u64) -> TeamPolicy {
    serde_json::from_value(json!({
        "escalate_after_secs": 4, "give_up_after_secs": 7200, "run_usd": 5.0, "run_minutes": 30,
        "reservation_limit_secs": 900, "max_life_secs": max_life_secs,
    }))
    .unwrap()
}

/// The box's bot with `limits`, asking the owner when in doubt and `on_limit` at a limit.
fn limited_bot(hub: &TestHub, tb: &TeamBox, limits: Value, on_limit: Value) {
    let calls = json!({ "in_doubt": { "kind": "owner" }, "on_limit": on_limit });
    sql(hub, &format!("insert into team_bots (team, workflow, owner, calls, limits) values ('{}', '{}', null, '{calls}', '{limits}')", tb.team, tb.workflow));
}

/// Moves the run's start (and, with `waiting`, its wait's start) `secs` into the past.
fn backdate(hub: &TestHub, run: Uuid, secs: u32, waiting: bool) {
    let wait = if waiting { format!(", waiting_since = waiting_since - interval '{secs} seconds'") } else { String::new() };
    sql(hub, &format!("update team_runs set started_at = started_at - interval '{secs} seconds'{wait} where id = '{run}'"));
}

/// The run as the owner lists it.
fn listed(tb: &mut TeamBox, run: Uuid) -> Value {
    let runs = op(tb.team, &mut tb.owner.client, json!({ "op": "runs" })).unwrap();
    runs.as_array().unwrap().iter().find(|r| r["id"] == json!(run.to_string())).cloned().unwrap()
}

fn on_limit_calls(calls: &[Value]) -> Vec<Value> {
    calls.iter().filter(|c| c["situation"] == "on_limit").cloned().collect()
}

#[test]
fn waiting_on_a_call_does_not_count_toward_the_runs_minutes() {
    let hub = TestHub::start_with_team_policy("teams_limits_wait", fast(), limits_policy(14_400));
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 1 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    backdate(&hub, run, 50, false);
    let call = tb.ask(&hub, "Red or blue footer?", &[]).unwrap()["call"].clone();
    // 80 s since the start, 30 of them waiting: past the minute on the wall, 50 s of work.
    backdate(&hub, run, 30, true);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(listed(&mut tb, run)["state"], json!("waiting"), "still alive while it waits");
    assert_eq!(on_limit_calls(&calls_of(tb.team, &mut tb.owner.client)), Vec::<Value>::new(), "no limit reached while waiting");

    op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": choice("", "blue") })).unwrap();
    let after = listed(&mut tb, run);
    assert_eq!(after["state"], json!("working"), "it goes on once answered");
    let (active, waiting) = (after["active_secs"].as_u64().unwrap(), after["waiting_secs"].as_u64().unwrap_or(0));
    assert!((30..45).contains(&waiting), "the wait is kept apart: {after}");
    assert!((50..58).contains(&active), "active time leaves the wait out: {after}");

    // Ten more seconds of work reach the minute: the time after the answer counts again.
    backdate(&hub, run, 10, false);
    let mut limit = vec![];
    wait_for("the limit call", || {
        limit = on_limit_calls(&calls_of(tb.team, &mut tb.owner.client));
        !limit.is_empty()
    });
    assert_eq!((&limit[0]["asked"], &limit[0]["question"]), (&json!([{ "kind": "owner" }]), &json!("The run reached its limit: 1 minute of work. Let it go on until the 4 hour limit, or stop it?")));
    assert_eq!(listed(&mut tb, run)["state"], json!("waiting"));
}

#[test]
fn an_unanswered_call_ends_the_run_at_give_up() {
    let hub = TestHub::start_with_team_policy("teams_limits_give_up", fast(), limits_policy(14_400));
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    let call = tb.ask(&hub, "Red or blue footer?", &[]).unwrap()["call"].as_str().unwrap().to_owned();
    sql(&hub, &format!("update team_calls set created_at = now() - interval '3 hours' where id = '{call}'"));
    wait_for("the give-up", || listed(&mut tb, run)["state"] == "gave_up");
    assert_eq!(listed(&mut tb, run)["outcome"], json!(format!("nobody answered call {call}")));
    wait_for("its box to end", || hub.boxes.live().is_empty());
}

#[test]
fn a_run_that_keeps_opening_calls_ends_at_its_max_life() {
    let hub = TestHub::start_with_team_policy("teams_limits_max_life", fast(), limits_policy(8));
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    let run = tb.run(&hub);
    for _ in 0..2 {
        let call = tb.ask(&hub, "Go on?", &[]).unwrap()["call"].clone();
        op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": choice("", "yes") })).unwrap();
    }
    tb.ask(&hub, "Go on?", &[]).unwrap();
    wait_long("the run to end at its max life", Duration::from_secs(20), || listed(&mut tb, run)["state"] == "failed");
    assert_eq!(listed(&mut tb, run)["outcome"], json!("it reached the most time a run may take, waiting included"));
    wait_for("its box to end", || hub.boxes.live().is_empty());
}

#[test]
fn a_run_may_not_open_a_second_wait_while_one_is_open() {
    let hub = TestHub::start_with_team_policy("teams_limits_one_wait", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    tb.run(&hub);
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    assert_eq!(tb.ask(&hub, "And the header?", &[]), Err("this run already waits on a call".into()));
    assert_eq!(calls_of(tb.team, &mut tb.owner.client).len(), 1);
}

#[test]
fn a_run_that_reaches_its_cost_limit_calls_who_on_limit_names() {
    let hub = TestHub::start_with_team_policy("teams_limits_cost", fast(), limits_policy(14_400));
    let mut tb = TeamBox::new(&hub);
    let mut dina = tb.member(&hub, "dina@example.test", &["design"]);
    // A small box costs $0.10 an hour: 40 s of work is past $0.001.
    limited_bot(&hub, &tb, json!({ "usd": 0.001, "minutes": 30 }), design());
    let run = tb.run(&hub);
    backdate(&hub, run, 40, false);
    let mut limit = vec![];
    wait_for("the limit call", || {
        limit = on_limit_calls(&calls_of(tb.team, &mut dina.client));
        !limit.is_empty()
    });
    assert_eq!((&limit[0]["asked"], &limit[0]["question"]), (&json!([design()]), &json!("The run reached its limit: $0.001 spent on its box and model. Let it go on until the 4 hour limit, or stop it?")));
    let spent = listed(&mut tb, run);
    assert_eq!(spent["state"], json!("waiting"));
    assert!(spent["cost_usd"].as_f64().unwrap() >= 0.001, "{spent}");
    tb.told(&hub, "[The run reached its limit: $0.001 spent on its box and model. Stop and wait for the answer]");
    assert_eq!(tb.ask(&hub, "One more step?", &[]), Err("this run already waits on a call".into()), "no work while it waits");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(listed(&mut tb, run)["cost_usd"], spent["cost_usd"], "nothing more is spent while it waits");

    let id = limit[0]["id"].as_str().unwrap().to_owned();
    sql(&hub, &format!("update team_calls set created_at = now() - interval '3 hours' where id = '{id}'"));
    wait_for("the give-up", || listed(&mut tb, run)["state"] == "gave_up");
    assert_eq!(listed(&mut tb, run)["outcome"], json!(format!("nobody answered call {id}")));
    wait_for("its box to end", || hub.boxes.live().is_empty());
}

#[test]
fn a_bot_whose_limit_is_not_set_does_not_start() {
    let (hub, team, workflow, mut bruno) = super::teams_separation_fixes::team_box_with_on_bot("teams_limits_start");
    sql(&hub, &format!("update team_bots set limits = '{{\"usd\":0,\"minutes\":30}}' where team = '{team}'"));
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "run_now", "workflow": workflow })), Err("this bot has no limit to run with: the team's owner sets one".into()));
    assert_eq!(rows(&hub, &format!("select count(*)::text from team_runs where team = '{team}'")), vec![vec![Some("0".to_owned())]]);
}

#[test]
fn set_bot_ceiling_is_the_owners_up_to_four_times_the_policy() {
    let hub = TestHub::start("teams_limits_ceiling");
    let (mut ana, ana_token, _) = locked_app(&hub.url, "ana@example.test");
    let (mut bruno, _, bruno_id) = locked_app(&hub.url, "bruno@example.test");
    let team = make_team(&mut ana, "Acme");
    admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{bruno_id}', 'member')"));
    ana.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    bruno.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    assert_eq!(via(&mut ana, &team, json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN })), Ok(credential(true, true)));
    let workflow = json!({ "name": "Fix bugs", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
        "folder": "/w/team", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "Fix the bug." });
    team_op(&mut ana, &team, json!({ "op": "save_bot", "bot": { "workflow": "" }, "connector": workflow })).unwrap();
    let id = team_workflows(&hub, Uuid::parse_str(&team).unwrap())[0]["id"].as_str().unwrap().to_owned();
    let ceiling = |usd: Value, minutes: Value| json!({ "op": "set_bot_ceiling", "workflow": id, "usd": usd, "minutes": minutes });
    let limits = |client: &mut Client| team_op(client, &team, json!({ "op": "bots" })).unwrap()[0]["limits"].clone();

    // The policy gives $5 and 30 minutes: the owner may set up to $20 and 120 minutes.
    assert_eq!(team_op(&mut ana, &team, ceiling(json!(20.0), json!(120))), Ok(Value::Null));
    assert_eq!(limits(&mut bruno), json!({ "usd": 20.0, "minutes": 120 }), "every member lists it");
    const ABOVE: &str = "a bot's ceiling is at most $20 and 120 minutes";
    const POSITIVE: &str = "a bot's ceiling needs dollars and whole minutes above 0";
    for (usd, minutes, refused) in [
        (json!(20.5), json!(60), ABOVE),
        (json!(10.0), json!(121), ABOVE),
        (json!(0), json!(60), POSITIVE),
        (json!(10.0), json!(0), POSITIVE),
        (json!(-1.0), json!(60), POSITIVE),
        (json!(10.0), json!(-5), POSITIVE),
        (json!(10.0), json!(2.5), POSITIVE),
    ] {
        assert_eq!(team_op(&mut ana, &team, ceiling(usd.clone(), minutes.clone())), Err(refused.into()), "{usd} {minutes}");
    }
    // JSON carries no NaN or infinity (`JSON.stringify` sends null): null is no number.
    assert!(team_op(&mut ana, &team, ceiling(Value::Null, json!(60))).is_err());
    assert_eq!(team_op(&mut bruno, &team, ceiling(json!(10.0), json!(60))), Err("only the team's owner does this".into()));
    // Saving the bot may not pass the ceiling either.
    let over = json!({ "op": "save_bot", "bot": { "workflow": id, "limits": { "usd": 50.0, "minutes": 30 } } });
    assert_eq!(team_op(&mut ana, &team, over), Err(ABOVE.into()));
    assert_eq!(limits(&mut ana), json!({ "usd": 20.0, "minutes": 120 }), "nothing refused was saved");

    // A locked login asks for the password first.
    let mut locked = Client::app(&hub.url, &ana_token);
    assert_eq!(team_op(&mut locked, &team, ceiling(json!(10.0), json!(60))), Err(LOCKED.into()));
    assert_eq!(limits(&mut ana), json!({ "usd": 20.0, "minutes": 120 }));
}

#[test]
fn the_box_idle_timer_waits_while_its_run_waits_and_runs_again_after() {
    let idle = Policy { idle: Duration::from_secs(1), ..fast() };
    let hub = TestHub::start_with_team_policy("teams_limits_idle", idle, patient());
    let mut tb = TeamBox::new(&hub);
    limited_bot(&hub, &tb, json!({ "usd": 5.0, "minutes": 30 }), json!({ "kind": "owner" }));
    tb.run(&hub);
    let call = tb.ask(&hub, "Red or blue footer?", &[]).unwrap()["call"].clone();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(hub.boxes.live().len(), 1, "a box whose run waits outlasts its idle time");
    op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": choice("", "blue") })).unwrap();
    wait_for("the idle box to end once answered", || hub.boxes.live().is_empty());
}

/// JSON carries no NaN or infinity, so the typed op, as the hub reads it, brings them to the check.
#[test]
fn set_bot_ceiling_refuses_nan_and_infinity_as_the_hub_reads_them() {
    let hub = TestHub::start("teams_limits_ceiling_nan");
    let (mut ana, _, ana_id) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let (team, ana_id) = (Uuid::parse_str(&team).unwrap(), Uuid::parse_str(&ana_id).unwrap());
    for (usd, minutes) in [(f64::NAN, 60.0), (f64::INFINITY, 60.0), (f64::NEG_INFINITY, 60.0), (10.0, f64::NAN), (10.0, f64::INFINITY)] {
        let op = pinned_core::teams::TeamOp::SetBotCeiling { workflow: "c1".into(), usd, minutes };
        let refused = hub.runtime.block_on(async {
            let owner = hub.hub().account(ana_id).await.unwrap();
            pinned_server::teams::call(hub.hub(), &owner, Some(team), pinned_server::teams::Op::Team(op)).await
        });
        assert_eq!(refused, Err("a bot's ceiling needs dollars and whole minutes above 0".into()), "{usd} {minutes}");
    }
}
