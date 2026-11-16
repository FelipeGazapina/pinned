//! Teams: a "Go ahead" on a step the team-box hook asked about lets exactly that step run, once,
//! in the same run, within 10 minutes of the answer. Any other step, a second try, another run, a
//! late try or any other answer asks again.
//!
//! The bot's step goes through the real hook (`team_hook::decide`) and the box's node, as in a
//! team's box: what the bot reads is the hook's output, what people see is the team's calls.

use super::teams_calls::{calls_of, choice, on_node, op, patient, rows, sql, TeamBox};
use super::*;
use pinned_core::preauth::team_hook::{self, HookEnv};
use pinned_core::teams::{Answer, AnswerKind, Situation, TeamOp};
use pinned_server::teams;
use uuid::Uuid;

const DEPLOY: &str = "vercel deploy --prod site-a";

/// What the bot reads when it runs `command` in the team's box: `None` when the step runs, else
/// the hook's reason for denying it.
fn try_step(hub: &TestHub, tb: &TeamBox, command: &str) -> Option<String> {
    let payload = json!({ "tool_name": "Bash", "tool_input": { "command": command }, "cwd": "/work/site" }).to_string();
    let env = HookEnv { team_box: true, project_dir: Some(PathBuf::from("/work/site")), home: PathBuf::from("/home/box") };
    let out = on_node(hub, &tb.box_id, |node| team_hook::decide(&payload, &env, |m| node.link.as_ref().unwrap().forward(m)))?;
    let out: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    Some(out["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().to_owned())
}

/// What the bot reads when its step went to the team as call `id`.
fn asked(id: &str) -> String {
    format!("Asked the team before this step (call {id}). End your turn and wait for the answer; only if it says go ahead, make this exact tool call again, unchanged.")
}

/// The team's open calls as its owner lists them: id and question.
fn open_calls(tb: &mut TeamBox) -> Vec<(String, String)> {
    let listed = calls_of(tb.team, &mut tb.owner.client);
    listed.iter().filter(|c| c["answer"].is_null()).map(|c| (c["id"].as_str().unwrap().to_owned(), c["question"].as_str().unwrap().to_owned())).collect()
}

/// The bot tries `command`, which goes to the team as one new call; its id.
fn ask(hub: &TestHub, tb: &mut TeamBox, command: &str) -> String {
    ask_shown(hub, tb, command, command)
}

/// The bot tries `command`, which goes to the team as one new call showing `shown`; its id.
fn ask_shown(hub: &TestHub, tb: &mut TeamBox, command: &str, shown: &str) -> String {
    let reason = try_step(hub, tb, command).expect("the step is asked about, not run");
    let open = open_calls(tb);
    assert_eq!(open.len(), 1, "{open:?} {reason}");
    let (id, question) = open[0].clone();
    assert_eq!(question, format!("deploy: {shown}"));
    assert_eq!(reason, asked(&id));
    id
}

/// `AskPeople` for `command` as the hook sends it, showing `question`.
fn ask_op(command: &str, question: &str) -> TeamOp {
    TeamOp::AskPeople {
        situation: Situation::BeforeIrreversible,
        question: question.to_owned(),
        choices: vec!["Go ahead".into(), "Don't".into()],
        step: Some(team_hook::step_hash("Bash", &json!({ "command": command }))),
    }
}

/// The team's owner answers call `call`.
fn answer(tb: &mut TeamBox, call: &str, answer: Value) -> Value {
    op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": answer })).unwrap()
}

/// Whether the "Go ahead" on call `call` was used.
fn released(hub: &TestHub, call: &str) -> bool {
    rows(hub, &format!("select (released_at is not null)::text from team_calls where id = '{call}'"))[0][0].as_deref() == Some("true")
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[test]
fn after_go_ahead_the_same_step_in_the_same_run_runs_and_opens_no_call() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_runs", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let run = tb.run(&hub);
    let call = ask(&hub, &mut tb, DEPLOY);
    assert_eq!(tb.run_state(&hub, run).0, "waiting");
    let stored = rows(&hub, &format!("select step_hash from team_calls where id = '{call}'"));
    assert_eq!(stored[0][0], Some(team_hook::step_hash("Bash", &json!({ "command": DEPLOY }))));
    assert!(!released(&hub, &call));

    answer(&mut tb, &call, choice("Go ahead", ""));
    assert_eq!(try_step(&hub, &tb, DEPLOY), None, "the step runs");
    assert_eq!(open_calls(&mut tb), []);
    assert_eq!(calls_of(tb.team, &mut tb.owner.client).len(), 1, "no call was opened");
    assert!(released(&hub, &call));
    assert_eq!(tb.run_state(&hub, run), ("working".to_owned(), None));
}

#[test]
fn a_go_ahead_is_used_once_and_the_second_try_asks_again() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_once", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.run(&hub);
    let call = ask(&hub, &mut tb, DEPLOY);
    answer(&mut tb, &call, choice("Go ahead", ""));
    assert_eq!(try_step(&hub, &tb, DEPLOY), None);

    let again = ask(&hub, &mut tb, DEPLOY);
    assert_ne!(again, call);
    assert!(!released(&hub, &again));
}

#[test]
fn a_go_ahead_does_not_let_a_step_one_character_apart_run() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_other_step", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.run(&hub);
    // One hidden character apart: the people saw the same text, the hash tells the steps apart.
    let hidden = "vercel deploy --prod --token=[hidden] site-a";
    let call = ask_shown(&hub, &mut tb, "vercel deploy --prod --token=abc1 site-a", hidden);
    answer(&mut tb, &call, choice("Go ahead", ""));
    let other = ask_shown(&hub, &mut tb, "vercel deploy --prod --token=abc2 site-a", hidden);
    assert_ne!(other, call);

    answer(&mut tb, &other, choice("Go ahead", ""));
    let visible = ask_shown(&hub, &mut tb, "vercel deploy --prod --token=abc2 site-b", "vercel deploy --prod --token=[hidden] site-b");
    assert_ne!(visible, other);
    assert!(!released(&hub, &call) && !released(&hub, &other), "the asked steps' go-aheads are still unused");
}

#[test]
fn a_go_ahead_on_a_question_that_does_not_show_the_step_lets_nothing_run() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_forged", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.run(&hub);
    let account = hub.runtime.block_on(hub.hub().account(tb.team)).unwrap();
    // An ask from the box with the deploy's hash under a harmless-looking question.
    let forged = hub.runtime.block_on(teams::machine_call(hub.hub(), &account, &tb.machine, ask_op(DEPLOY, "deploy: echo hello"))).unwrap();
    let forged = forged["call"].as_str().unwrap().to_owned();
    assert_eq!(open_calls(&mut tb), [(forged.clone(), "deploy: echo hello".to_owned())]);
    answer(&mut tb, &forged, choice("Go ahead", ""));

    let real = ask(&hub, &mut tb, DEPLOY);
    assert_ne!(real, forged);
    assert!(!released(&hub, &forged));
}

#[test]
fn a_go_ahead_does_not_carry_over_to_another_run_of_the_team() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_other_run", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let first = tb.run(&hub);
    let call = ask(&hub, &mut tb, DEPLOY);
    answer(&mut tb, &call, choice("Go ahead", ""));
    sql(&hub, &format!("update team_runs set state = 'done', ended_at = now(), outcome = 'done' where id = '{first}'"));
    let second = tb.run(&hub);

    let next = ask(&hub, &mut tb, DEPLOY);
    let listed = calls_of(tb.team, &mut tb.owner.client);
    assert_eq!((&listed[0]["id"], &listed[0]["run"]), (&json!(next), &json!(second.to_string())));
    assert!(!released(&hub, &call));
}

#[test]
fn a_go_ahead_expires_ten_minutes_after_the_answer() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_expires", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.run(&hub);
    let late = ask(&hub, &mut tb, DEPLOY);
    answer(&mut tb, &late, choice("Go ahead", ""));
    let answered_ago = |call: &str, secs: i64| {
        sql(&hub, &format!(
            "update team_calls set answer = jsonb_set(answer, '{{at}}', to_jsonb((answer ->> 'at')::bigint - {secs})),
               answered_at = answered_at - make_interval(secs => {secs}) where id = '{call}'"
        ))
    };
    answered_ago(&late, 601);
    let next = ask(&hub, &mut tb, DEPLOY);
    assert_ne!(next, late);
    assert!(!released(&hub, &late));

    answer(&mut tb, &next, choice("Go ahead", ""));
    answered_ago(&next, 590);
    assert_eq!(try_step(&hub, &tb, DEPLOY), None, "still within 10 minutes");
    assert!(released(&hub, &next));
}

#[test]
fn a_take_over_a_pass_or_a_stop_lets_nothing_run_even_with_go_ahead_as_its_choice() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_other_answers", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut omar = tb.member(&hub, "omar@example.test", &["ops"]);
    let run = tb.run(&hub);
    let with_go_ahead = |kind: Value| json!({ "kind": kind, "choice": "Go ahead" });

    let taken = ask(&hub, &mut tb, DEPLOY);
    answer(&mut tb, &taken, with_go_ahead(json!({ "kind": "take_over" })));
    assert_eq!(tb.run_state(&hub, run), ("taken_over".to_owned(), Some(tb.owner.id.to_string())));
    let passed = ask(&hub, &mut tb, DEPLOY);
    assert_ne!(passed, taken);

    answer(&mut tb, &passed, with_go_ahead(json!({ "kind": "pass_to", "handle": "ops" })));
    let to_ops = open_calls(&mut tb);
    assert_eq!(to_ops.len(), 1);
    assert_eq!(try_step(&hub, &tb, DEPLOY), Some("Could not ask the team; do not do this step.".to_owned()), "the run waits on @ops");
    op(tb.team, &mut omar.client, json!({ "op": "answer_call", "id": to_ops[0].0, "answer": choice("Don't", "") })).unwrap();

    // A stop ends the run and its box; written straight to the call here, so its run lives on to try.
    let stopped = ask(&hub, &mut tb, DEPLOY);
    let stop = Answer { by: tb.owner.id.to_string(), choice: Some("Go ahead".into()), text: String::new(), kind: AnswerKind::Stop, at: now() };
    let db = &hub.hub().db;
    hub.runtime.block_on(teams::db::answer_call(db, tb.team, Uuid::parse_str(&stopped).unwrap(), &stop, "stopped")).unwrap().unwrap();
    hub.runtime.block_on(teams::db::stop_waiting(db, tb.team, run)).unwrap().unwrap();
    let after_stop = ask(&hub, &mut tb, DEPLOY);
    assert_ne!(after_stop, stopped);
    for call in [taken, passed, to_ops[0].0.clone(), stopped] {
        assert!(!released(&hub, &call), "{call}");
    }
}

#[test]
fn a_dont_lets_nothing_run_and_the_same_step_asks_again() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_dont", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.run(&hub);
    let call = ask(&hub, &mut tb, DEPLOY);
    answer(&mut tb, &call, choice("Don't", ""));

    let again = ask(&hub, &mut tb, DEPLOY);
    assert_ne!(again, call);
    assert!(!released(&hub, &call));
}

#[test]
fn of_two_identical_tries_at_once_after_go_ahead_exactly_one_runs() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_race", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.run(&hub);
    let account = hub.runtime.block_on(hub.hub().account(tb.team)).unwrap();
    let step = || ask_op(DEPLOY, &format!("deploy: {DEPLOY}"));
    let mut call = ask(&hub, &mut tb, DEPLOY);
    for _ in 0..5 {
        answer(&mut tb, &call, choice("Go ahead", ""));
        let (first, second) =
            hub.runtime.block_on(async { tokio::join!(teams::machine_call(hub.hub(), &account, &tb.machine, step()), teams::machine_call(hub.hub(), &account, &tb.machine, step())) });
        let replies = [first.unwrap(), second.unwrap()];
        let (granted, opened): (Vec<&Value>, Vec<&Value>) = replies.iter().partition(|r| r["granted"] == json!(true));
        assert_eq!(granted, [&json!({ "granted": true, "call": call })], "{replies:?}");
        let next = opened[0]["call"].as_str().unwrap().to_owned();
        assert_eq!(opened, [&json!({ "call": next, "asked": ["the owner"] })]);
        assert_eq!(open_calls(&mut tb), [(next.clone(), format!("deploy: {DEPLOY}"))]);
        call = next;
    }
}

#[test]
fn a_call_passed_on_keeps_its_step_and_the_other_groups_go_ahead_lets_it_run() {
    let hub = TestHub::start_with_team_policy("teams_go_ahead_passed", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut omar = tb.member(&hub, "omar@example.test", &["ops"]);
    tb.run(&hub);
    let call = ask(&hub, &mut tb, DEPLOY);
    answer(&mut tb, &call, json!({ "kind": { "kind": "pass_to", "handle": "ops" } }));
    let to_ops = open_calls(&mut tb);
    assert_eq!(to_ops, [(to_ops[0].0.clone(), format!("deploy: {DEPLOY}"))]);
    op(tb.team, &mut omar.client, json!({ "op": "answer_call", "id": to_ops[0].0, "answer": choice("Go ahead", "") })).unwrap();

    assert_eq!(try_step(&hub, &tb, DEPLOY), None);
    assert!(released(&hub, &to_ops[0].0));
    assert_eq!(open_calls(&mut tb), []);
}
