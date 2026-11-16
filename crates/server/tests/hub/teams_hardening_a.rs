//! pinned Teams, hardening A: who may change a team, and the one way into its workflows.
//!
//! A team's settings, schedules and workflows are no personal call's: `TeamOp::Hub` refuses them
//! for every role, and a member saves a team's workflow only with its bot (`save_bot` with a
//! `connector`), always as a draft, and deletes it only by `delete_bot`. A locked login unlocks
//! before every op that changes the team or steers a run; a stop, giving a run back and the reads
//! do not ask. Each test drives the wire: the JSON an app sends and the text it reads back.

use super::teams_separation_fixes::{bot_on_at, locked_app, make_team, sql_text, team_box_with_on_bot, team_op, team_workflow, via, PASSWORD};
use super::*;
use uuid::Uuid;

const NO_RUN: &str = "6f1c0b1e-0000-4000-8000-000000000001";

fn workflow_json(name: &str) -> Value {
    json!({ "name": name, "enabled": true, "trigger": { "kind": "linear-label", "label": "agent" },
        "folder": "/w/team", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "Fix the bug.", "machine": "" })
}

/// The bots the team lists, as `(workflow, name, on, owner)`.
fn listed(client: &mut Client, team: &str) -> Vec<(String, String, bool, Option<String>)> {
    let bots = team_op(client, team, json!({ "op": "bots" })).unwrap();
    bots.as_array().unwrap().iter().map(|b| (b["workflow"].as_str().unwrap().to_owned(), b["name"].as_str().unwrap().to_owned(), b["on"] == json!(true), b["owner"].as_str().map(str::to_owned))).collect()
}

/// An unlocked member `email` of `team`: its client and account id.
fn member(hub: &TestHub, team: &str, email: &str) -> (Client, String) {
    let (mut client, _, id) = locked_app(&hub.url, email);
    admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{id}', 'member')"));
    client.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    (client, id)
}

fn the_nine() -> Vec<Value> {
    vec![
        json!({ "cmd": "get_settings" }),
        json!({ "cmd": "list_schedules" }),
        json!({ "cmd": "schedule_runs", "id": "t1" }),
        json!({ "cmd": "run_workflow", "id": "c1" }),
        json!({ "cmd": "save_connector", "connector": workflow_json("sneak") }),
        json!({ "cmd": "delete_connector", "id": "c1" }),
        json!({ "cmd": "save_schedule", "task": task_body("team-schedule") }),
        json!({ "cmd": "pause_schedule", "id": "t1", "paused": true }),
        json!({ "cmd": "delete_schedule", "id": "t1" }),
    ]
}

// ---- 1. The team's settings, schedules and workflows by hub: refused for every role ----

#[test]
fn the_teams_settings_schedules_and_workflow_calls_are_refused_by_hub_for_the_owner_and_a_member() {
    let hub = TestHub::start("hard_a_nine");
    let (mut ana, _, _) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    ana.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    let (mut bruno, _) = member(&hub, &team, "bruno@example.test");
    let texts = [
        "get_settings does not run on a team",
        "list_schedules does not run on a team",
        "schedule_runs does not run on a team",
        "run_workflow does not run on a team",
        "save_connector does not run on a team",
        "delete_connector does not run on a team",
        "save_schedule does not run on a team",
        "pause_schedule does not run on a team",
        "delete_schedule does not run on a team",
    ];
    for (who, client) in [("owner", &mut ana), ("member", &mut bruno)] {
        for (call, text) in the_nine().into_iter().zip(texts) {
            assert_eq!(via(client, &team, call.clone()), Err(text.to_owned()), "{who} {call}");
        }
    }
    assert_eq!(team_workflows(&hub, Uuid::parse_str(&team).unwrap()), Vec::<Value>::new(), "nothing was saved on the team");
    assert_eq!(sql_text(&hub, &format!("select count(*)::text from tasks where account = '{team}'")), "0", "and no schedule");
}

#[test]
fn a_member_still_asks_a_box_status_on_the_team() {
    let hub = TestHub::start("hard_a_box_status");
    let (mut ana, _, _) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let (mut bruno, _) = member(&hub, &team, "bruno@example.test");
    // The call passes the team's gate and reaches the box lookup, which knows no such box.
    assert_eq!(via(&mut bruno, &team, json!({ "cmd": "box_status", "machine": "6f1c0b1e-0000-4000-8000-0000000000ee" })), Err("this machine is no box".into()));
}

#[test]
fn save_connector_through_hub_is_refused_and_the_bot_and_its_workflow_stay_as_they_were() {
    let (hub, team, id, mut bruno) = team_box_with_on_bot("hard_a_end_a");
    let before = team_workflow(&hub, &team, &id);
    let mut changed = before.clone();
    changed["instructions"] = json!("Push straight to main.");
    assert_eq!(via(&mut bruno, &team, json!({ "cmd": "save_connector", "connector": changed })), Err("save_connector does not run on a team".into()));
    assert_eq!(team_workflow(&hub, &team, &id), before, "the workflow is as it was");
    assert_eq!(bot_on_at(&hub, &team, &id), "set", "the bot is still on");
    assert_eq!(listed(&mut bruno, &team).iter().map(|b| b.2).collect::<Vec<_>>(), vec![true]);
}

// ---- 2. save_bot with its workflow, and delete_bot ----

#[test]
fn a_member_saves_a_bot_with_its_workflow_as_a_draft_and_the_team_lists_it() {
    let hub = TestHub::start("hard_a_save_bot");
    let (mut ana, _, _) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let (mut bruno, bruno_id) = member(&hub, &team, "bruno@example.test");
    let saved = team_op(&mut bruno, &team, json!({ "op": "save_bot", "bot": { "workflow": "" }, "connector": workflow_json("Fix bugs") }));
    let flows = team_workflows(&hub, Uuid::parse_str(&team).unwrap());
    assert_eq!(flows.len(), 1);
    let id = flows[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(saved, Ok(json!({ "workflow": id })));
    assert_eq!((flows[0]["name"].clone(), flows[0]["enabled"].clone(), flows[0]["machine"].clone()), (json!("Fix bugs"), json!(false), json!({ "kind": "upstash-box" })), "saved off and in a box, whatever the member asked");
    assert_eq!(listed(&mut bruno, &team), vec![(id.clone(), "Fix bugs".to_owned(), false, Some(bruno_id.clone()))], "a draft bot, answered for by bruno");
    assert_eq!(bot_on_at(&hub, &team, &id), "null");
    // On the team's account, not bruno's.
    assert_eq!(bruno.hub(json!({ "cmd": "get_settings" })).unwrap()["connectors"], json!([]));
}

#[test]
fn save_bot_on_a_bot_that_is_on_puts_it_back_to_a_draft_with_or_without_its_workflow() {
    let (hub, team, id, mut bruno) = team_box_with_on_bot("hard_a_end_b");
    let bot = json!({ "workflow": id });
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "save_bot", "bot": bot })), Ok(json!({ "workflow": id })));
    assert_eq!(bot_on_at(&hub, &team, &id), "null", "off");
    assert_eq!(team_workflow(&hub, &team, &id)["enabled"], false, "its workflow with it");
    assert_eq!(listed(&mut bruno, &team).iter().map(|b| b.2).collect::<Vec<_>>(), vec![false]);

    let mut ana = Client::app(&hub.url, &owner_token(&hub));
    ana.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "turn_bot_on", "workflow": id, "on": true })), Ok(Value::Null));
    assert_eq!(bot_on_at(&hub, &team, &id), "set");
    let mut changed = team_workflow(&hub, &team, &id);
    changed["instructions"] = json!("Fix the bug and open a pull request.");
    changed["enabled"] = json!(true);
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "save_bot", "bot": bot, "connector": changed })), Ok(json!({ "workflow": id })));
    let stored = team_workflow(&hub, &team, &id);
    assert_eq!((stored["instructions"].clone(), stored["enabled"].clone()), (json!("Fix the bug and open a pull request."), json!(false)));
    assert_eq!(bot_on_at(&hub, &team, &id), "null", "a changed workflow is a draft until someone turns it on");
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "run_now", "workflow": id })), Err("this bot is a draft: it has to be turned on before it can run".into()));
}

/// The login token of the owner `team_box_with_on_bot` made: it signs ana in again.
fn owner_token(hub: &TestHub) -> String {
    account::login(&hub.url, "ana@example.test", PASSWORD).unwrap().0
}

#[test]
fn a_workflow_id_the_team_does_not_keep_is_refused_as_an_unknown_workflow() {
    let hub = TestHub::start("hard_a_foreign");
    let (mut ana, _, _) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let (mut bruno, _) = member(&hub, &team, "bruno@example.test");
    // Bruno's own workflow, on his own account.
    let own = bruno.hub(json!({ "cmd": "save_connector", "connector": workflow_json("Mine") })).unwrap();
    let own_id = own["id"].as_str().unwrap().to_owned();
    let mut foreign = workflow_json("Mine, now the team's");
    foreign["id"] = json!(own_id);
    for bot in [json!({ "workflow": own_id }), json!({ "workflow": "" })] {
        let op = json!({ "op": "save_bot", "bot": bot, "connector": foreign });
        assert_eq!(team_op(&mut bruno, &team, op), Err(format!("this team has no workflow {own_id}")));
    }
    let mut invented = workflow_json("Invented");
    invented["id"] = json!("c-nope");
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "save_bot", "bot": { "workflow": "" }, "connector": invented })), Err("this team has no workflow c-nope".into()));
    assert_eq!(team_workflows(&hub, Uuid::parse_str(&team).unwrap()), Vec::<Value>::new(), "the team stored nothing");
    assert_eq!(listed(&mut bruno, &team), vec![]);
    let mine = bruno.hub(json!({ "cmd": "get_settings" })).unwrap();
    assert_eq!(mine["connectors"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["Mine"], "bruno's own is as it was");
}

#[test]
fn delete_bot_refuses_a_bot_that_is_on_or_running_and_only_its_owner_or_the_teams_deletes_a_draft() {
    let (hub, team, ana_flow, mut bruno) = team_box_with_on_bot("hard_a_delete");
    let mut ana = Client::app(&hub.url, &owner_token(&hub));
    ana.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    let (mut carl, _) = member(&hub, &team, "carl@example.test");
    let delete = |id: &str| json!({ "op": "delete_bot", "workflow": id });
    const OFF_FIRST: &str = "turn the bot off first";

    // On: refused for everyone, the owner too.
    assert_eq!(team_op(&mut ana, &team, delete(&ana_flow)), Err(OFF_FIRST.into()));
    assert_eq!(team_op(&mut bruno, &team, delete(&ana_flow)), Err(OFF_FIRST.into()));
    assert_eq!(bot_on_at(&hub, &team, &ana_flow), "set");

    // A draft with a live run: refused too.
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "turn_bot_on", "workflow": ana_flow, "on": false })), Ok(Value::Null));
    let live = Uuid::new_v4();
    admin_on(&hub.database, &format!("insert into team_runs (id, team, workflow, started_by, state) values ('{live}', '{team}', '{ana_flow}', '\"x\"', 'working')"));
    assert_eq!(team_op(&mut ana, &team, delete(&ana_flow)), Err(OFF_FIRST.into()));
    admin_on(&hub.database, &format!("update team_runs set state = 'done', ended_at = now() where id = '{live}'"));

    // A draft: another member gets the text of an unknown bot; the team's owner deletes it.
    let unknown = format!("this team has no workflow {ana_flow}");
    assert_eq!(team_op(&mut bruno, &team, delete(&ana_flow)), Err(unknown.clone()));
    assert_eq!(listed(&mut bruno, &team).len(), 1, "still there");
    assert_eq!(team_op(&mut ana, &team, delete(&ana_flow)), Ok(Value::Null));
    assert_eq!(listed(&mut bruno, &team), vec![]);
    assert_eq!(team_workflows(&hub, Uuid::parse_str(&team).unwrap()), Vec::<Value>::new(), "its workflow went with it");
    assert_eq!(sql_text(&hub, &format!("select count(*)::text from team_bots where team = '{team}'")), "0");
    assert_eq!(team_op(&mut ana, &team, delete(&ana_flow)), Err(unknown), "and it is unknown now");
    // Its past run is still listed.
    let runs = team_op(&mut bruno, &team, json!({ "op": "runs" })).unwrap();
    assert_eq!(runs.as_array().unwrap().iter().map(|r| (r["id"].as_str().unwrap().to_owned(), r["workflow"].as_str().unwrap().to_owned(), r["state"].as_str().unwrap().to_owned())).collect::<Vec<_>>(), vec![(live.to_string(), ana_flow.clone(), "done".to_owned())]);

    // Bruno's own draft: carl gets the unknown text, bruno deletes it.
    team_op(&mut bruno, &team, json!({ "op": "save_bot", "bot": { "workflow": "" }, "connector": workflow_json("Bruno's") })).unwrap();
    let bruno_flow = team_workflows(&hub, Uuid::parse_str(&team).unwrap())[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(team_op(&mut carl, &team, delete(&bruno_flow)), Err(format!("this team has no workflow {bruno_flow}")));
    assert_eq!(team_op(&mut bruno, &team, delete(&bruno_flow)), Ok(Value::Null));
    assert_eq!(listed(&mut bruno, &team), vec![]);

    // Another of bruno's drafts: the team's owner deletes it too.
    team_op(&mut bruno, &team, json!({ "op": "save_bot", "bot": { "workflow": "" }, "connector": workflow_json("Bruno's again") })).unwrap();
    let again = team_workflows(&hub, Uuid::parse_str(&team).unwrap())[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(team_op(&mut ana, &team, delete(&again)), Ok(Value::Null));
    assert_eq!(listed(&mut bruno, &team), vec![]);
}

// ---- 3. What a locked login unlocks for ----

#[test]
fn a_locked_member_unlocks_before_each_team_change_and_stops_gives_back_and_reads_logs_without() {
    let hub = TestHub::start("hard_a_locked");
    let (mut ana, _, _) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let (mut bruno, _, bruno_id) = locked_app(&hub.url, "bruno@example.test");
    admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{bruno_id}', 'member')"));
    let ops = [
        json!({ "op": "attach_project", "key": "github.com/acme/site" }),
        json!({ "op": "detach_project", "key": "github.com/acme/site" }),
        json!({ "op": "join_group", "handle": "design" }),
        json!({ "op": "leave_group", "handle": "design" }),
        json!({ "op": "run_now", "workflow": "c1" }),
        json!({ "op": "answer_call", "id": NO_RUN, "answer": { "kind": { "kind": "choice" }, "choice": "red" } }),
        json!({ "op": "take_over", "run": NO_RUN }),
        json!({ "op": "chat", "text": "hello" }),
        json!({ "op": "delete_bot", "workflow": "c1" }),
    ];
    let pass_to = json!({ "op": "answer_call", "id": NO_RUN, "answer": { "kind": { "kind": "pass_to", "handle": "design" } } });
    for op in ops.iter().chain([&pass_to]) {
        assert_eq!(team_op(&mut bruno, &team, op.clone()), Err(LOCKED.into()), "{op}");
    }
    let free = [json!({ "op": "stop_run", "run": NO_RUN }), json!({ "op": "give_back", "run": NO_RUN }), json!({ "op": "run_log", "run": NO_RUN })];
    let locked: Vec<_> = free.iter().map(|op| team_op(&mut bruno, &team, op.clone())).collect();
    assert_eq!(locked, vec![Err("the team has no such run".to_owned()); 3], "they run locked, and find no such run");

    bruno.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    // Unlocked, each gets past the unlock and answers what its op answers for this empty team.
    let answers = [
        "you can only attach a project you have",
        "the team has no project github.com/acme/site",
        "there is no group @design",
        "you are not in @design",
        "this team has no workflow c1",
        "the team has no such call",
        "the team has no such run",
        "not built yet",
        "this team has no workflow c1",
        "the team has no such call",
    ];
    for (op, answer) in ops.iter().chain([&pass_to]).zip(answers) {
        assert_eq!(team_op(&mut bruno, &team, op.clone()), Err(answer.to_owned()), "{op}");
    }
    let unlocked: Vec<_> = free.iter().map(|op| team_op(&mut bruno, &team, op.clone())).collect();
    assert_eq!(unlocked, locked, "the same answer locked or not");
}
