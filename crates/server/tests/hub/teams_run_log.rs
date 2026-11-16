//! Teams: a member opens a run and reads what its bot did, from the session in the run's own box
//! only, masked, short and one line each.

use super::teams_calls::{on_node, op, person, rows, sql, TeamBox};
use super::*;
use pinned_core::teams::{Run as TeamRun, RunState, TeamPolicy};
use pinned_server::teams;
use uuid::Uuid;

/// Unix seconds of 2026-11-09T12:00:00Z, where the transcripts below start.
const NOON: i64 = 1794225600;
const NO_SUCH_RUN: &str = "the team has no such run";

/// An RFC 3339 time `secs` after noon.
fn clock(secs: i64) -> String {
    format!("2026-11-09T12:{:02}:{:02}Z", secs / 60, secs % 60)
}

fn entry(kind: &str, content: Value, secs: i64) -> String {
    json!({ "type": kind, "uuid": format!("{kind}-{secs}"), "timestamp": clock(secs), "message": { "role": kind, "content": content } }).to_string()
}

fn typed(text: &str, secs: i64) -> String {
    entry("user", json!(text), secs)
}

fn said(text: &str, secs: i64) -> String {
    entry("assistant", json!([{ "type": "text", "text": text }]), secs)
}

fn ran(tool: &str, input: Value, secs: i64) -> String {
    entry("assistant", json!([{ "type": "tool_use", "id": format!("tool-{secs}"), "name": tool, "input": input }]), secs)
}

/// Gives the box's session `lines` as its transcript, in the claude home of the box's own node
/// (`.claude` in the box's folder), under a session id of this test's own.
fn transcript(hub: &TestHub, tb: &TeamBox, lines: &[String]) {
    let folder = tb.dir.join(".claude/projects/box");
    std::fs::create_dir_all(&folder).unwrap();
    let session = format!("s-{}", hub.database.replace('_', "-"));
    std::fs::write(folder.join(format!("{session}.jsonl")), lines.join("\n") + "\n").unwrap();
    hook(&tb.dir, json!({ "kind": "agent", "status": "working", "tab": tb.tab, "session": session }));
}

fn line(at: i64, who: &str, text: &str) -> Value {
    json!({ "at": at, "who": who, "text": text })
}

fn log(team: Uuid, client: &mut Client, run: &str) -> Result<Vec<Value>, String> {
    op(team, client, json!({ "op": "run_log", "run": run })).map(|log| log.as_array().unwrap().clone())
}

/// A run of `team` working in the box on machine `box_machine`, written with SQL.
fn run_in(hub: &TestHub, team: Uuid, started_by: Uuid, box_machine: Option<&str>) -> Uuid {
    let id = Uuid::new_v4();
    let run = TeamRun {
        id: id.to_string(),
        workflow: "w1".into(),
        state: RunState::Working,
        controller: None,
        cost_usd: 0.0,
        active_secs: 0,
        started_at: 0,
        ended_at: None,
        outcome: None,
        started_by: started_by.to_string(),
    };
    hub.runtime.block_on(teams::db::insert_run(&hub.hub().db, team, &run, box_machine)).unwrap();
    id
}

#[test]
fn a_member_opens_a_run_and_reads_what_its_bot_did_oldest_first() {
    let hub = TestHub::start_with_team_policy("run_log_reads", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    let mut dina = tb.member(&hub, "dina@example.test", &[]);
    let run = tb.run(&hub).to_string();
    transcript(
        &hub,
        &tb,
        &[
            typed("Fix the footer", 0),
            said("Looking at the footer.", 10),
            ran("Bash", json!({ "command": "cargo test -p site" }), 20),
            entry("assistant", json!([{ "type": "thinking", "thinking": "private musings" }]), 25),
            entry("user", json!([{ "type": "tool_result", "tool_use_id": "tool-20", "content": "output of the tests" }]), 30),
            ran("Read", json!({ "file_path": "src/footer.rs" }), 40),
            said("Done.\nTests pass.", 50),
        ],
    );
    let expected = vec![
        line(NOON, "person", "Fix the footer"),
        line(NOON + 10, "bot", "Looking at the footer."),
        line(NOON + 20, "Bash", "cargo test -p site"),
        line(NOON + 40, "Read", "src/footer.rs"),
        line(NOON + 50, "bot", "Done. Tests pass."),
    ];
    assert_eq!(log(tb.team, &mut tb.owner.client, &run), Ok(expected.clone()));
    assert_eq!(log(tb.team, &mut dina.client, &run), Ok(expected), "any member reads it");
}

#[test]
fn a_run_of_another_team_is_refused_as_a_run_that_does_not_exist() {
    let hub = TestHub::start_with_team_policy("run_log_other_team", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    let mut eve = person(&hub, "eve@example.test");
    let other = Uuid::parse_str(eve.client.hub(json!({ "cmd": "team", "op": { "op": "create", "name": "Other" } })).unwrap()["id"].as_str().unwrap()).unwrap();
    // The other team's run works in this team's box, the worst case.
    let theirs = run_in(&hub, other, eve.id, Some(&tb.machine));
    transcript(&hub, &tb, &[said("Only this team's members read this.", 10)]);

    assert_eq!(log(tb.team, &mut tb.owner.client, &theirs.to_string()), Err(NO_SUCH_RUN.into()));
    assert_eq!(log(tb.team, &mut tb.owner.client, &Uuid::new_v4().to_string()), Err(NO_SUCH_RUN.into()), "the same words as a run nobody has");
    assert_eq!(log(other, &mut eve.client, &theirs.to_string()), Err(NO_SUCH_RUN.into()), "and its own team does not read this team's box either");
}

#[test]
fn a_run_whose_box_is_a_persons_machine_is_refused_and_that_tab_is_never_read() {
    let hub = TestHub::start_with_team_policy("run_log_person_box", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    let (token, _) = account::signup(&hub.url, "pia@example.test", "correct horse").unwrap();
    let (laptop, _) = account::register_machine(&hub.url, &token, "laptop", "macos").unwrap();
    let run = tb.run(&hub);
    transcript(&hub, &tb, &[said("The team's box.", 10)]);
    assert_eq!(log(tb.team, &mut tb.owner.client, &run.to_string()).unwrap().len(), 1);

    sql(&hub, &format!("update team_runs set box = '{}' where id = '{run}'", laptop.id));
    assert_eq!(log(tb.team, &mut tb.owner.client, &run.to_string()), Err(NO_SUCH_RUN.into()));
    sql(&hub, &format!("update team_runs set box = 'not-a-machine' where id = '{run}'"));
    assert_eq!(log(tb.team, &mut tb.owner.client, &run.to_string()), Err(NO_SUCH_RUN.into()), "a box that is no machine at all");
}

#[test]
fn a_malformed_run_id_is_refused_as_a_run_that_does_not_exist() {
    let hub = TestHub::start_with_team_policy("run_log_malformed", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    for id in ["", "not-a-run", "42", "x' or true --", "00000000-0000-0000-0000-00000000000g", &"a".repeat(5000)] {
        assert_eq!(log(tb.team, &mut tb.owner.client, id), Err(NO_SUCH_RUN.into()), "{id:.20?}");
    }
}

#[test]
fn someone_who_is_not_a_member_gets_the_refusal_every_team_op_gives() {
    let hub = TestHub::start_with_team_policy("run_log_stranger", fast(), TeamPolicy::default());
    let tb = TeamBox::new(&hub);
    let mut sam = person(&hub, "sam@example.test");
    let run = tb.run(&hub).to_string();
    transcript(&hub, &tb, &[said("Members only.", 10)]);
    assert_eq!(log(tb.team, &mut sam.client, &run), Err("you are not a member of that team".into()));
    assert_eq!(log(tb.team, &mut sam.client, "not-a-run"), Err("you are not a member of that team".into()));
}

#[test]
fn a_run_with_no_box_no_tab_or_no_session_yet_answers_an_empty_log() {
    let hub = TestHub::start_with_team_policy("run_log_empty", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    let boxless = run_in(&hub, tb.team, tb.owner.id, None).to_string();
    assert_eq!(log(tb.team, &mut tb.owner.client, &boxless), Ok(vec![]), "no box yet");

    let run = tb.run(&hub).to_string();
    assert_eq!(log(tb.team, &mut tb.owner.client, &run), Ok(vec![]), "a tab whose session has no transcript yet");

    let tab = tb.tab.clone();
    on_node(&hub, &tb.box_id, |node| node.machine.call(Call::CloseTab { tab }).unwrap());
    let team = tb.team;
    wait_for("the hub to hear the box has no tab", || hub.runtime.block_on(hub.hub().account(team)).unwrap().tabs_of(&tb.machine).is_some_and(|tabs| tabs.is_empty()));
    assert_eq!(log(tb.team, &mut tb.owner.client, &run), Ok(vec![]), "a box with no tab");
}

#[test]
fn keys_and_tokens_in_the_log_come_back_hidden() {
    let hub = TestHub::start_with_team_policy("run_log_masked", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    let run = tb.run(&hub).to_string();
    let claude = format!("sk-ant-oat01-{}", "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN");
    let github = format!("ghp_{}", "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ");
    assert_eq!((claude.len(), github.len()), (53, 40));
    transcript(
        &hub,
        &tb,
        &[
            typed(&format!("use {claude} for the login"), 0),
            said(&format!("I found {claude} and {github}."), 10),
            ran("Bash", json!({ "command": format!("git push https://{github}@github.com/acme/site.git") }), 20),
            ran("Bash", json!({ "command": format!("curl -H 'Authorization: Bearer {github}' https://api.example.test") }), 30),
        ],
    );
    let shown = log(tb.team, &mut tb.owner.client, &run).unwrap();
    let all = serde_json::to_string(&shown).unwrap();
    assert!(!all.contains("abcdefghijklmnop"), "no part of a token: {all}");
    assert_eq!(shown[0], line(NOON, "person", "use [hidden] for the login"));
    assert_eq!(shown[1], line(NOON + 10, "bot", "I found [hidden] and [hidden]."));
    assert_eq!(shown[2]["who"], "Bash");
    assert_eq!(shown[3], line(NOON + 30, "Bash", "curl -H 'Authorization: Bearer [hidden]' https://api.example.test"));
    assert_eq!(shown.len(), 4);
}

#[test]
fn a_line_longer_than_500_characters_is_cut_there_with_a_mark() {
    let hub = TestHub::start_with_team_policy("run_log_cut", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    let run = tb.run(&hub).to_string();
    let exact = "xy".repeat(250);
    transcript(
        &hub,
        &tb,
        &[
            said(&"ab".repeat(1000), 10),
            said(&exact, 20),
            said(&format!("{exact}z"), 30),
            said(&"é".repeat(600), 40),
            said(&format!("{}\n{}", "cd".repeat(100), "ef".repeat(100)), 50),
        ],
    );
    let shown = log(tb.team, &mut tb.owner.client, &run).unwrap();
    assert_eq!(shown[0]["text"], json!(format!("{}…", "ab".repeat(250))), "2000 characters: 500 and the mark");
    assert_eq!(shown[1]["text"], json!(exact), "500 characters stay whole");
    assert_eq!(shown[2]["text"], json!(format!("{exact}…")), "501 characters lose the last one");
    assert_eq!(shown[3]["text"], json!(format!("{}…", "é".repeat(500))), "500 characters, not 500 bytes");
    assert_eq!(shown[4]["text"], json!(format!("{} {}", "cd".repeat(100), "ef".repeat(100))), "one line");
    assert_eq!(shown.len(), 5);
}

#[test]
fn three_hundred_lines_come_back_as_the_last_200() {
    let hub = TestHub::start_with_team_policy("run_log_last_200", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    let run = tb.run(&hub).to_string();
    let lines: Vec<String> = (1..=300).map(|i| said(&format!("step {i}"), i)).collect();
    transcript(&hub, &tb, &lines);
    let shown = log(tb.team, &mut tb.owner.client, &run).unwrap();
    let expected: Vec<Value> = (101..=300).map(|i| line(NOON + i, "bot", &format!("step {i}"))).collect();
    assert_eq!(shown.len(), 200);
    assert_eq!((&shown[0], &shown[199]), (&expected[0], &expected[199]));
    assert_eq!(shown, expected);
}

#[test]
fn an_ended_run_still_reads_its_log_while_its_box_exists() {
    let hub = TestHub::start_with_team_policy("run_log_ended", fast(), TeamPolicy::default());
    let mut tb = TeamBox::new(&hub);
    let run = tb.run(&hub).to_string();
    transcript(&hub, &tb, &[said("All done.", 10)]);
    sql(&hub, &format!("update team_runs set state = 'done', ended_at = now(), outcome = 'finished' where id = '{run}'"));
    assert_eq!(rows(&hub, &format!("select state from team_runs where id = '{run}'")), vec![vec![Some("done".to_owned())]]);
    assert_eq!(log(tb.team, &mut tb.owner.client, &run), Ok(vec![line(NOON + 10, "bot", "All done.")]));
}
