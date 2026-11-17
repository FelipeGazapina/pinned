//! Teams: guards the code has and no test proved. Each test asserts what a member or a box sees,
//! and names the guard it holds; the mutation row of each is in the commit that added it.

use super::teams_admission::{Crew, POLLS_HELD};
use super::teams_calls::{calls_of, choice, design, op, patient, rows, sql, TeamBox};
use super::teams_go_ahead::{answer, ask, asked, open_calls, released, try_step, DEPLOY};
use pinned_core::teams::{Answer, AnswerKind, Run as TeamRun, RunState, Situation, TeamOp};
use super::*;
use pinned_server::boxes::End;
use pinned_server::db::{Db, Sealer};
use pinned_server::teams;
use uuid::Uuid;

const NOT_A_BOX: &str = "only a session in one of a team's boxes asks people";

/// A run of `workflow` as the teams' other ops would write it.
fn run_row(workflow: &str) -> TeamRun {
    TeamRun {
        id: Uuid::new_v4().to_string(),
        workflow: workflow.to_owned(),
        state: RunState::Working,
        controller: None,
        cost_usd: 0.0,
        active_secs: 0, waiting_secs: 0,
        started_at: 0,
        ended_at: None,
        outcome: None,
        started_by: "trigger:linear_label".into(),
        kind: pinned_core::teams::RunKind::Bot, project: None, machine: None, tab: None,
    }
}

/// The op a box's session sends to ask the team, in `situation`.
fn ask_in(situation: Situation, question: &str) -> TeamOp {
    TeamOp::AskPeople { situation, question: question.into(), choices: vec![], step: None }
}

// ---- a. every call and run read is filtered by the team ----

/// `teams/db.rs` `call`, `run` and `answer_call` read `team = $1`: a member of team A who sends the
/// id of team B's call or run gets the not-found text, and B's call and run stay as they were.
#[test]
fn a_member_of_one_team_cannot_reach_the_call_or_run_of_another() {
    let hub = TestHub::start_with_team_policy("tg_other_team", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.bot(&hub, design(), None);
    let run_b = tb.run(&hub);
    let call_b = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap()["call"].as_str().unwrap().to_owned();
    let mut dina = super::teams_calls::person(&hub, "dina@example.test");
    let made = dina.client.hub(json!({ "cmd": "team", "op": { "op": "create", "name": "Other" } })).unwrap();
    let team_a = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();

    let no_call = Err("the team has no such call".to_owned());
    let no_run = Err("the team has no such run".to_owned());
    assert_eq!(op(team_a, &mut dina.client, json!({ "op": "answer_call", "id": call_b, "answer": choice("red", "") })), no_call);
    for name in ["stop_run", "take_over", "give_back"] {
        assert_eq!(op(team_a, &mut dina.client, json!({ "op": name, "run": run_b.to_string() })), no_run, "{name}");
    }
    assert_eq!(op(team_a, &mut dina.client, json!({ "op": "calls" })).unwrap(), json!([]), "team A lists nothing of team B");
    assert_eq!(op(team_a, &mut dina.client, json!({ "op": "runs" })).unwrap(), json!([]));

    // Team B's call and run are as the box left them.
    let listed = calls_of(tb.team, &mut tb.owner.client);
    assert_eq!((listed.len(), &listed[0]["id"], &listed[0]["answer"]), (1, &json!(call_b), &Value::Null));
    assert_eq!(tb.run_state(&hub, run_b), ("waiting".to_owned(), None));
    let stored = rows(&hub, &format!("select (answered_at is null)::text, outcome from team_calls where id = '{call_b}'"));
    assert_eq!(stored, [[Some("true".to_owned()), None]]);

    // The reads themselves, with team B as the control that the rows are there.
    let db = &hub.hub().db;
    let (call_id, run_id) = (Uuid::parse_str(&call_b).unwrap(), run_b);
    assert!(hub.runtime.block_on(teams::db::call(db, team_a, call_id)).unwrap().is_none());
    assert!(hub.runtime.block_on(teams::db::run(db, team_a, run_id)).unwrap().is_none());
    assert!(hub.runtime.block_on(teams::db::call(db, tb.team, call_id)).unwrap().is_some());
    assert!(hub.runtime.block_on(teams::db::run(db, tb.team, run_id)).unwrap().is_some());
    let red = Answer { by: dina.id.to_string(), choice: Some("red".into()), text: String::new(), kind: AnswerKind::Choice, at: 0 };
    assert!(hub.runtime.block_on(teams::db::answer_call(db, team_a, call_id, &red, "answered")).unwrap().is_none(), "team A's answer writes nothing");
    let stored = rows(&hub, &format!("select (answered_at is null)::text, outcome from team_calls where id = '{call_b}'"));
    assert_eq!(stored, [[Some("true".to_owned()), None]], "still open");
    let answered = hub.runtime.block_on(teams::db::answer_call(db, tb.team, call_id, &red, "answered")).unwrap();
    assert!(answered.is_some(), "team B answers it");
}

// ---- b. a go-ahead of another situation is not used for an irreversible step ----

/// `use_go_ahead` takes only `situation = 'before_irreversible'`: a "Go ahead" on a call of the
/// same step and question that is of another situation lets nothing run.
#[test]
fn a_go_ahead_given_in_another_situation_does_not_let_an_irreversible_step_run() {
    let hub = TestHub::start_with_team_policy("tg_go_ahead_situation", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let run = tb.run(&hub);
    let call = ask(&hub, &mut tb, DEPLOY);
    sql(&hub, &format!("update team_calls set situation = 'in_doubt' where id = '{call}'"));
    answer(&mut tb, &call, choice("Go ahead", ""));
    assert_eq!(tb.run_state(&hub, run), ("working".to_owned(), None));

    let reason = try_step(&hub, &tb, DEPLOY).expect("the step is asked about again, not run");
    let open = open_calls(&mut tb);
    assert_eq!(open.len(), 1, "one new call: {open:?}");
    assert_ne!(open[0].0, call);
    assert_eq!(open[0].1, format!("deploy: {DEPLOY}"));
    assert_eq!(reason, asked(&open[0].0));
    assert!(!released(&hub, &call), "the go-ahead was not used");

    // The same go-ahead, given on a call of the situation, is the one that lets the step run.
    sql(&hub, &format!("update team_calls set situation = 'before_irreversible' where id = '{call}'"));
    assert_eq!(try_step(&hub, &tb, DEPLOY), None, "the step runs");
    assert!(released(&hub, &call));
}

// ---- c. two asks at once ----

/// `start_waiting` moves only a run that works: of two asks at once on one run exactly one call
/// opens, and the other box session is told the run already waits.
#[test]
fn of_two_asks_at_once_on_one_run_one_call_opens_and_the_run_waits_once() {
    let hub = TestHub::start_with_team_policy("tg_two_asks", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let account = hub.runtime.block_on(hub.hub().account(tb.team)).unwrap();
    for round in 0..5 {
        let ask = |q: &str| teams::machine_call(hub.hub(), &account, &tb.machine, ask_in(Situation::InDoubt, q));
        let (first, second) = hub.runtime.block_on(async { tokio::join!(ask("Red?"), ask("Blue?")) });
        let (opened, refused): (Vec<_>, Vec<_>) = [first, second].into_iter().partition(Result::is_ok);
        assert_eq!((opened.len(), refused.len()), (1, 1), "round {round}: one call opens, one ask is refused");
        assert_eq!(refused[0], Err("this run already waits on a call".to_owned()));
        let call = opened[0].as_ref().unwrap()["call"].as_str().unwrap().to_owned();
        let open: Vec<_> = calls_of(tb.team, &mut tb.owner.client).into_iter().filter(|c| c["answer"].is_null()).collect();
        assert_eq!(open.len(), 1, "round {round}: one open call");
        assert_eq!(open[0]["id"], json!(call));
        assert_eq!(tb.run_state(&hub, run), ("waiting".to_owned(), None));
        op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": { "kind": { "kind": "choice" }, "text": "go" } })).unwrap();
        assert_eq!(tb.run_state(&hub, run), ("working".to_owned(), None));
    }

    // The statement itself, from two connections: exactly one moves the run.
    let url = format!("{}/{}", postgres(), hub.database);
    let other = hub.runtime.block_on(Db::connect(&url, Sealer::load(&temp(&format!("guard-{}", hub.database)).join("hub.key")).unwrap())).unwrap();
    let db = &hub.hub().db;
    for _ in 0..10 {
        let (first, second) = hub.runtime.block_on(async { tokio::join!(teams::db::start_waiting(db, tb.team, run), teams::db::start_waiting(&other, tb.team, run)) });
        assert_eq!([first.unwrap(), second.unwrap()].iter().flatten().count(), 1);
        hub.runtime.block_on(teams::db::stop_waiting(db, tb.team, run)).unwrap().unwrap();
    }
}

// ---- d. a box that boots for an ended run opens no session ----

/// `claim_run_box` answers false for a run that ended (or one of another team), so the box
/// opens no session for it; the run stays as it ended and never gets the box.
#[test]
fn a_box_that_boots_for_an_ended_run_opens_no_session_for_it() {
    let crew = Crew::new("tg_claim_box");
    let id = crew.on_bot("Footer");
    let gate = Arc::new(tokio::sync::Notify::new());
    crew.hub.boxes.state.lock().unwrap().gate = Some(gate.clone());
    let (hub, bob, team, workflow) = (crew.hub.hub().clone(), crew.bob.clone(), crew.team, id.clone());
    let started = crew.hub.runtime.spawn(async move { teams::call(&hub, &bob, Some(team), teams::Op::Team(TeamOp::RunNow { workflow })).await });
    wait_for("the box's bootstrap to start", || crew.hub.boxes.state.lock().unwrap().scripts.iter().any(|s| s.contains("start.sh")));
    let runs = crew.call(&crew.bob, TeamOp::Runs).unwrap();
    let run = runs[0]["id"].as_str().unwrap().to_owned();
    crew.stop(&run);
    gate.notify_one();
    // Only the box's claim of its run answers this, and it comes before the session's run is sent.
    let answered = crew.hub.runtime.block_on(started).unwrap();
    assert_eq!(answered, Err("Upstash Box: the run ended before its box started, so its box was ended".into()), "no session opened");
    wait_for("the box to be deleted", || crew.hub.boxes.live().is_empty());
    assert_eq!(rows(&crew.hub, &format!("select state, box from team_runs where id = '{run}'")), [[Some("stopped".to_owned()), None]], "the run never got the box");

    // The claim itself: a live run of the team takes the box, an ended one and another team's run do not.
    let db = &crew.hub.hub().db;
    let (live, ended, subject) = (run_row(&id), run_row(&id), issue("eng-1"));
    // The ended run ends before the live one is written: a bot has one live run at a time.
    for (row, subject) in [(&ended, subject.clone()), (&live, issue("eng-live"))] {
        crew.hub.runtime.block_on(teams::db::insert_run(db, crew.team, row, None)).unwrap();
        crew.hub.runtime.block_on(teams::db::set_run_subject(db, crew.team, Uuid::parse_str(&row.id).unwrap(), &subject)).unwrap();
        if row.id == ended.id {
            crew.hub.runtime.block_on(teams::db::set_run_state(db, crew.team, Uuid::parse_str(&ended.id).unwrap(), RunState::Stopped, None, Some("stopped"))).unwrap();
        }
    }
    let claim = |account: Uuid, subject: &Subject| crew.hub.runtime.block_on(teams::db::claim_run_box(db, account, &id, subject, "machine-1")).unwrap();
    assert!(!claim(crew.team, &subject), "a run that ended takes no box");
    assert!(!claim(crew.ana.id, &issue("eng-live")), "another account's claim finds no run");
    assert!(claim(crew.team, &issue("eng-live")), "a live run takes the box");
    assert_eq!(rows(&crew.hub, &format!("select box from team_runs where id = '{}'", ended.id)), [[None]]);
}

// ---- e. a run that cannot be ended keeps its box ----

/// The `?` on `box_ended` in `fail_held_runs`: when the team's run cannot be ended, ending the box
/// fails with it, so the box stays (and so does its run, still listed as it is) instead of
/// being deleted under a run that goes on; the next pass ends both.
#[test]
fn when_the_run_cannot_be_ended_its_box_is_not_deleted_under_it() {
    let hub = TestHub::start_with_team_policy("tg_box_end_fails", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let run = tb.run(&hub);
    sql(&hub, "create function block_end() returns trigger language plpgsql as $$ begin
                 if new.state in ('done', 'stopped', 'failed', 'gave_up') then raise exception 'blocked'; end if; return new; end $$;
               create trigger block_end before update on team_runs for each row execute function block_end()");
    let account = hub.runtime.block_on(hub.hub().account(tb.team)).unwrap();
    let machine = Uuid::parse_str(&tb.machine).unwrap();
    let ended = hub.runtime.block_on(hub.hub().boxes.end_machine(hub.hub(), &account, machine, End::Removed));
    let why = ended.expect_err("the box does not end while its run cannot");
    assert!(why.starts_with("could not end the team run of box"), "{why}");
    assert_eq!(hub.boxes.live().len(), 1, "the box is not deleted");
    assert_eq!(tb.run_state(&hub, run), ("working".to_owned(), None), "the run is not left looking ended");
    let listed = op(tb.team, &mut tb.owner.client, json!({ "op": "runs" })).unwrap();
    assert_eq!((&listed[0]["state"], &listed[0]["outcome"]), (&json!("working"), &Value::Null));

    sql(&hub, "drop trigger block_end on team_runs");
    hub.runtime.block_on(hub.hub().boxes.reap(hub.hub(), &account));
    wait_for("the box to be deleted", || hub.boxes.live().is_empty());
    let listed = op(tb.team, &mut tb.owner.client, json!({ "op": "runs" })).unwrap();
    assert_eq!((&listed[0]["state"], &listed[0]["outcome"]), (&json!("stopped"), &json!("its box was removed")));
}

// ---- f. a take-over answer by a member who does not control the run ----

/// An answer that takes over a run someone else controls is refused with who has it, and the
/// run keeps its controller; a member the call does not ask is refused before that.
#[test]
fn a_member_who_does_not_control_a_run_cannot_answer_its_call_as_controller() {
    let hub = TestHub::start_with_team_policy("tg_take_over_answer", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut dina = tb.member(&hub, "dina@example.test", &["design"]);
    let mut omar = tb.member(&hub, "omar@example.test", &["ops"]);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let take_over = json!({ "kind": { "kind": "take_over" }, "text": "mine" });

    let first = tb.ask(&hub, "Red or blue footer?", &[]).unwrap()["call"].clone();
    op(tb.team, &mut dina.client, json!({ "op": "answer_call", "id": first, "answer": take_over })).unwrap();
    assert_eq!(tb.run_state(&hub, run), ("taken_over".to_owned(), Some(dina.id.to_string())));
    // The box asks again: the call goes to the one who controls the run.
    let second = tb.ask(&hub, "Ship it?", &[]).unwrap();
    assert_eq!(second["asked"], json!(["dina"]));
    let call = second["call"].clone();

    let answer = |a: &Value| json!({ "op": "answer_call", "id": call, "answer": a });
    assert_eq!(op(tb.team, &mut omar.client, answer(&take_over)), Err("this call is not for you".into()));
    assert_eq!(op(tb.team, &mut tb.owner.client, answer(&take_over)), Err("dina already took over".into()));
    assert_eq!(tb.run_state(&hub, run), ("waiting".to_owned(), Some(dina.id.to_string())), "dina still controls it, while its call waits");
    let open: Vec<_> = calls_of(tb.team, &mut tb.owner.client).into_iter().filter(|c| c["answer"].is_null()).collect();
    assert_eq!(open.len(), 1, "the call is still open");
    assert_eq!(open[0]["id"], call);
    let own = op(tb.team, &mut dina.client, answer(&take_over)).unwrap();
    assert_eq!(own["answer"]["by"], json!(dina.id.to_string()), "the controller answers it");
}

// ---- g. a box asks people only for its own run ----

/// `box_of` in `ask_people`: the call a box opens is on the run in that box, and a machine that
/// is not one of the team's boxes asks nothing, even when a live run of the team names it.
#[test]
fn a_box_asks_people_only_for_its_own_run() {
    let hub = TestHub::start_with_team_policy("tg_box_of", fast(), patient());
    let mut a = TeamBox::new(&hub);
    let mut b = TeamBox::new_as(&hub, "bea@example.test");
    for tb in [&a, &b] {
        tb.bot(&hub, design(), None);
    }
    let (run_a, run_b) = (a.run(&hub), b.run(&hub));

    let asked = b.ask(&hub, "Which footer?", &[]).unwrap();
    let listed = calls_of(b.team, &mut b.owner.client);
    assert_eq!((listed.len(), &listed[0]["id"], &listed[0]["run"]), (1, &asked["call"], &json!(run_b.to_string())), "team B's box asks on team B's run");
    assert_eq!(a.run_state(&hub, run_a), ("working".to_owned(), None));
    assert_eq!(calls_of(a.team, &mut a.owner.client), Vec::<Value>::new());

    // Team A's run names team B's box as its own; B's box is still no box of team A.
    sql(&hub, &format!("update team_runs set box = '{}' where id = '{run_a}'", b.machine));
    let account_a = hub.runtime.block_on(hub.hub().account(a.team)).unwrap();
    for machine in [b.machine.clone(), Uuid::new_v4().to_string(), "not-a-machine".to_owned()] {
        let sent = hub.runtime.block_on(teams::machine_call(hub.hub(), &account_a, &machine, ask_in(Situation::InDoubt, "Which footer?")));
        assert_eq!(sent, Err(NOT_A_BOX.to_owned()), "{machine}");
    }
    assert_eq!(a.run_state(&hub, run_a), ("working".to_owned(), None), "team A's run does not wait");
    assert_eq!(calls_of(a.team, &mut a.owner.client), Vec::<Value>::new(), "no call opened on team A's run");
    assert_eq!(calls_of(b.team, &mut b.owner.client).len(), 1);
}

// ---- h. a restart after a box failed ----

/// A box that fails to boot while the hub cannot end its run leaves the run working. A hub
/// that restarts after it does not leave it so: one pass of the reaper ends it failed, and
/// its place goes to the event that waits for it.
#[test]
fn a_restart_after_a_box_failed_does_not_leave_its_run_working_forever() {
    let mut crew = Crew::new("tg_restart_failed");
    let (holder, pages) = (crew.on_bot("Docs"), crew.on_bot("Pages"));
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    // The database refuses to end or delete a run, as when it is down for the moment the box fails.
    admin_on(
        &crew.hub.database,
        "create function block_end() returns trigger language plpgsql as $$ begin
           if tg_op = 'DELETE' or new.state in ('done', 'stopped', 'failed', 'gave_up') then raise exception 'blocked'; end if; return new; end $$;
         create trigger block_end before update or delete on team_runs for each row execute function block_end()",
    );
    crew.hub.boxes.state.lock().unwrap().failing = Some(Exec { exit_code: 22, output: "Downloading pinned-server\n".into(), error: "curl: (22) The requested URL returned error: 404\n".into() });
    let started = crew.call(&crew.bob, TeamOp::RunNow { workflow: holder.clone() });
    assert!(started.is_err(), "the box did not boot: {started:?}");
    assert_eq!(crew.found(crew.labeled(&pages, 1)), Ok(()));
    let listed = |crew: &Crew, id: &str| -> Vec<(String, Option<String>)> {
        let runs = crew.call(&crew.bob, TeamOp::Runs).unwrap();
        runs.as_array().unwrap().iter().filter(|r| r["workflow"] == id).map(|r| (r["state"].as_str().unwrap().to_owned(), r["outcome"].as_str().map(str::to_owned))).collect()
    };
    assert_eq!(listed(&crew, &holder), [("working".to_owned(), None)], "its run is left working");
    assert_eq!((crew.waiting(&pages), crew.live_of(&pages).len()), (1, 0), "ENG-1 waits for its place");

    crew.hub.restart();
    admin_on(&crew.hub.database, "drop trigger block_end on team_runs");
    // Run now's run holds a reserved place until its session opens: the backstop for a
    // reservation that never opened ends it, past its limit.
    admin_on(&crew.hub.database, &format!("update team_runs set started_at = now() - interval '1 hour', reserved_at = now() - interval '16 minutes' where team = '{}'", crew.team));
    crew.hub.runtime.block_on(teams::calls::tick(crew.hub.hub(), &crew.hub.team_policy)).unwrap();
    assert_eq!(listed(&crew, &holder), [("failed".to_owned(), Some("it never started".to_owned()))]);
    wait_for("ENG-1 to take the freed place", || crew.live_of(&pages).len() == 1 && crew.waiting(&pages) == 0);
}

// ---- reservation races ----

/// Ends a reserved run of the team the moment its box is recorded on it, as a stop or the backstop
/// does between the box's claim of its run and its session's open: in the same transaction, so
/// the claim answers that the run is live, and the run ended right after.
fn end_reserved_run_after_its_box_claim(crew: &Crew) {
    admin_on(
        &crew.hub.database,
        "create function end_after_claim() returns trigger language plpgsql as $$ begin
           update team_runs set state = 'failed', ended_at = now(), outcome = 'it never started' where id = new.id; return null; end $$;
         create trigger end_after_claim after update on team_runs for each row
           when (new.reserved and old.box is null and new.box is not null and new.state = 'working') execute function end_after_claim()",
    );
}

/// `open_reserved` writes the box only while the run is live and reserved: a reserved run that
/// ended (stopped, or by the backstop) while its box booted gets no session, its box ends, and
/// the run stays listed as it ended.
#[test]
fn a_reserved_run_that_ended_while_its_box_booted_gets_no_session_and_its_box_ends() {
    let crew = Crew::new("tg_open_reserved");
    let bot = crew.on_bot("Pages");
    end_reserved_run_after_its_box_claim(&crew);
    let found = crew.found(crew.labeled(&bot, 1)).expect_err("its session did not open");
    assert!(found.contains("the run ended before its box started, so its box was ended"), "no session opened for it: {found}");
    wait_for("its box to be deleted", || crew.hub.boxes.live().is_empty());
    let listed = crew.call(&crew.bob, TeamOp::Runs).unwrap();
    let listed: Vec<_> = listed.as_array().unwrap().iter().map(|r| (r["state"].clone(), r["outcome"].clone())).collect();
    assert_eq!(listed, [(json!("failed"), json!("it never started"))], "the run stays listed as it ended");
    assert_eq!(crew.live_of(&bot), Vec::<String>::new());
}

/// `claim` takes a granted event only while its reserved run is live and reserved: the run ended
/// (as the backstop ends it) before the poller's claim, so the claim answers nothing, and nothing
/// outside pinned is called for the event.
#[test]
fn a_claim_after_its_reserved_run_ended_gets_nothing_and_calls_nothing_outside_pinned() {
    let crew = Crew::new("tg_claim_ended");
    let (holder, triage) = (crew.on_bot("Docs"), crew.graph_bot("Triage"));
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    assert_eq!(crew.found(crew.labeled(&holder, 9)), Ok(()));
    assert_eq!(crew.found(crew.graph_labeled(&triage, 1)), Ok(()));
    assert_eq!(crew.waiting(&triage), 1, "ENG-1 waits");
    // No check of the team's triggers gets past its Linear poll, so none claims the grant yet.
    POLLS_HELD.lock().unwrap().push("tg_claim_ended".into());
    let held = crew.live_of(&holder);
    crew.stop(&held[0]);
    wait_for("ENG-1's grant", || crew.reserved_at(&triage) == vec!["set".to_owned()]);
    assert_eq!(crew.waiting(&triage), 1, "granted, its row waits for a claim");
    let boxes_before = crew.boxes_created();
    admin_on(
        &crew.hub.database,
        &format!("update team_runs set state = 'failed', ended_at = now(), outcome = 'it never started' where team = '{}' and workflow = '{triage}' and reserved", crew.team),
    );
    POLLS_HELD.lock().unwrap().retain(|t| t != "tg_claim_ended");

    // The held check goes on, and a few more pass.
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(crew.outside(1), Vec::<String>::new(), "nothing outside pinned was called for ENG-1");
    assert_eq!(crew.boxes_created() - boxes_before, 0, "no session opened");
    assert_eq!(crew.live_of(&triage), Vec::<String>::new());
    let runs = crew.call(&crew.bob, TeamOp::Runs).unwrap();
    let ended: Vec<_> = runs.as_array().unwrap().iter().filter(|r| r["workflow"] == json!(triage)).map(|r| (r["state"].clone(), r["outcome"].clone())).collect();
    assert_eq!(ended, [(json!("failed"), json!("it never started"))], "the run stays listed as it ended");
}
