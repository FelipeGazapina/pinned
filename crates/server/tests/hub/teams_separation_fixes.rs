//! pinned Teams: a team's personal call is read once into its type, a locked login unlocks before
//! a team change, a team's box sends only what its session needs, and nothing that lets an agent
//! or an owner in takes a team's account. Plain English texts and literal values throughout.

use super::*;

const NOT_AN_OBJECT: &str = "not a hub call: a hub call is a JSON object";
const NOT_BUILT: &str = "not built yet";
const PASSWORD: &str = "correct horse";

fn team_op(client: &mut Client, team: &str, op: Value) -> Result<Value, String> {
    client.hub(json!({ "cmd": "team", "team": team, "op": op }))
}

fn via(client: &mut Client, team: &str, call: Value) -> Result<Value, String> {
    team_op(client, team, json!({ "op": "hub", "call": call }))
}

fn sql_text(hub: &TestHub, sql: &str) -> String {
    hub.runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        client.query_one(sql, &[]).await.unwrap().get::<_, String>(0)
    })
}

/// A person signed up as `email`, signed in the way the app signs in now (a locked login): the
/// client, its login token and its account id.
fn locked_app(url: &str, email: &str) -> (Client, String, String) {
    let (token, view) = account::signup(url, email, PASSWORD).unwrap();
    (Client::app(url, &token), token, view.id)
}

fn make_team(client: &mut Client, name: &str) -> String {
    client.hub(json!({ "cmd": "team", "op": { "op": "create", "name": name } })).unwrap()["id"].as_str().unwrap().to_owned()
}

fn credential(upstash: bool, claude: bool) -> Value {
    json!({ "upstash": upstash, "claude": claude, "github": false, "slack": false })
}

#[test]
fn a_teams_hub_call_as_an_array_or_in_the_wrong_case_is_refused_and_extra_keys_unlock_as_the_plain_form() {
    let hub = TestHub::start("sepfix_forms");
    let url = hub.url.clone();
    let (mut ana, _, _) = locked_app(&url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let plain = json!({ "cmd": "disconnect_upstash" });
    let extra = json!({ "cmd": "disconnect_upstash", "note": "hi", "cmd_alias": "get_settings" });
    let array = json!(["disconnect_upstash"]);
    let wrong_case = json!({ "cmd": "Disconnect_Upstash" });

    // Locked: the plain form and the one with extra keys ask for the password; the others are no call.
    assert_eq!(via(&mut ana, &team, plain.clone()), Err(LOCKED.into()));
    assert_eq!(via(&mut ana, &team, extra.clone()), Err(LOCKED.into()), "extra keys change nothing");
    assert_eq!(via(&mut ana, &team, array.clone()), Err(NOT_AN_OBJECT.into()), "an array is no call, locked or not");
    let refused = via(&mut ana, &team, wrong_case.clone()).unwrap_err();
    assert!(refused.starts_with("not a hub call: unknown variant `Disconnect_Upstash`"), "{refused}");

    ana.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    let connect = json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN });
    assert_eq!(via(&mut ana, &team, connect), Ok(credential(true, true)), "a key call answers which keys are set, never a hint");
    assert_eq!(via(&mut ana, &team, array), Err(NOT_AN_OBJECT.into()), "unlocked, the array still runs nothing");
    assert!(via(&mut ana, &team, wrong_case).unwrap_err().starts_with("not a hub call: unknown variant"));
    assert!(!via(&mut ana, &team, json!({ "cmd": "get_settings" })).unwrap()["upstash"].is_null(), "the team is still connected");
    assert_eq!(via(&mut ana, &team, extra), Ok(credential(false, false)), "extra keys run as the plain form");
    assert!(via(&mut ana, &team, json!({ "cmd": "get_settings" })).unwrap()["upstash"].is_null());
}

#[test]
fn a_locked_login_unlocks_before_every_team_change_and_reads_without_it() {
    let hub = TestHub::start("sepfix_unlock");
    let url = hub.url.clone();
    let (mut ana, _, _) = locked_app(&url, "ana@example.test");
    let (mut bruno, _, bruno_id) = locked_app(&url, "bruno@example.test");
    let team = make_team(&mut ana, "Acme");
    admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{bruno_id}', 'member')"));
    let workflow = json!({ "name": "team-flow", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
        "folder": "/w/team", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "" });

    let changes = [
        json!({ "op": "rename", "name": "Acme 2" }),
        json!({ "op": "invite" }),
        json!({ "op": "revoke_invite", "hint": "ABCD" }),
        json!({ "op": "remove_member", "account": bruno_id }),
        json!({ "op": "save_group", "group": { "handle": "design" } }),
        json!({ "op": "save_bot", "bot": { "workflow": "c1" } }),
        json!({ "op": "turn_bot_on", "workflow": "c1", "on": true }),
        json!({ "op": "hub", "call": { "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN } }),
        json!({ "op": "hub", "call": { "cmd": "set_box_claude_token", "name": "n", "token": "sk-ant-oat01-teamtoken" } }),
        json!({ "op": "hub", "call": { "cmd": "disconnect_github" } }),
        json!({ "op": "hub", "call": { "cmd": "set_box_access", "access": { "workflow": { "mode": "ask" } } } }),
        json!({ "op": "hub", "call": { "cmd": "extend_box", "machine": "6f1c0b1e-0000-4000-8000-0000000000ee", "extra_usd": 1.0 } }),
        json!({ "op": "hub", "call": { "cmd": "save_connector", "connector": workflow } }),
        json!({ "op": "hub", "call": { "cmd": "delete_connector", "id": "c1" } }),
        json!({ "op": "hub", "call": { "cmd": "save_schedule", "task": task_body("team-schedule") } }),
        json!({ "op": "hub", "call": { "cmd": "pause_schedule", "id": "t1", "paused": true } }),
        json!({ "op": "hub", "call": { "cmd": "delete_schedule", "id": "t1" } }),
        json!({ "op": "hub", "call": { "cmd": "remove_box_claude_token", "name": "n" } }),
        json!({ "op": "hub", "call": { "cmd": "disconnect_slack" } }),
        json!({ "op": "hub", "call": { "cmd": "disconnect_upstash" } }),
    ];
    // They would reach GitHub and Slack once unlocked, so they are only refused while locked.
    let network = [
        json!({ "op": "hub", "call": { "cmd": "connect_github", "token": "ghp_team" } }),
        json!({ "op": "hub", "call": { "cmd": "connect_slack", "token": "xoxb-team" } }),
    ];
    for op in changes.iter().chain(&network) {
        assert_eq!(team_op(&mut ana, &team, op.clone()), Err(LOCKED.into()), "{op}");
    }
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "leave" })), Err(LOCKED.into()));
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "join", "code": "ABCD-EFGH-JKMN-PQRS" })), Err(LOCKED.into()));
    // The same answer a person's own confirm call gets on a locked login.
    assert_eq!(ana.hub(json!({ "cmd": "remove_machine", "id": "6f1c0b1e-0000-4000-8000-0000000000ee" })), Err(LOCKED.into()));
    assert_eq!(sql_text(&hub, &format!("select name from teams where id = '{team}'")), "Acme", "nothing ran");
    assert_eq!(sql_text(&hub, &format!("select count(*)::text from team_invites where team = '{team}'")), "0");

    let reads = [
        json!({ "op": "list" }),
        json!({ "op": "members" }),
        json!({ "op": "projects" }),
        json!({ "op": "groups" }),
        json!({ "op": "hub", "call": { "cmd": "get_settings" } }),
        json!({ "op": "hub", "call": { "cmd": "list_schedules" } }),
    ];
    for op in &reads {
        assert!(team_op(&mut bruno, &team, op.clone()).is_ok(), "{op}");
    }
    // Still stubs on this base: they answer that, never the unlock.
    for op in [json!({ "op": "bots" }), json!({ "op": "runs" }), json!({ "op": "calls" })] {
        assert_eq!(team_op(&mut bruno, &team, op.clone()), Err(NOT_BUILT.into()), "{op}");
    }

    // Unlocked, every change gets past the unlock: each answers what its op answers, never LOCKED.
    bruno.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    assert_ne!(team_op(&mut bruno, &team, json!({ "op": "join", "code": "ABCD-EFGH-JKMN-PQRS" })), Err(LOCKED.into()));
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "leave" })), Ok(Value::Null), "bruno leaves");
    ana.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    assert_eq!(team_op(&mut ana, &team, changes[0].clone()), Ok(Value::Null));
    assert!(team_op(&mut ana, &team, changes[1].clone()).unwrap()["code"].is_string());
    assert_eq!(team_op(&mut ana, &team, changes[7].clone()), Ok(credential(true, true)));
    assert_eq!(team_op(&mut ana, &team, changes[12].clone()).unwrap()["name"], "team-flow");
    for op in &changes[2..] {
        assert_ne!(team_op(&mut ana, &team, op.clone()), Err(LOCKED.into()), "{op}");
    }
    assert_eq!(sql_text(&hub, &format!("select name from teams where id = '{team}'")), "Acme 2");
}

// ---- A team's box: deny by default ----

const TEAM_BOX_TOKEN: &str = "team-box-machine-token";

/// A team with a machine of its own account, as a team's box has, signed in by `TEAM_BOX_TOKEN`.
fn team_box(name: &str) -> (TestHub, String) {
    let hub = TestHub::start(name);
    let (mut ana, _, _) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    admin_on(
        &hub.database,
        &format!("insert into machines (id, account, name, os, token_hash) values ('6f1c0b1e-0000-4000-8000-00000000b0b0', '{team}', 'team box', 'linux', sha256('{TEAM_BOX_TOKEN}'::bytea))"),
    );
    (hub, team)
}

fn refused(cmd: &str) -> Result<Value, String> {
    Err(format!("a team's box may not send {cmd}"))
}

#[test]
fn a_team_box_may_not_import_a_store() {
    let (hub, team) = team_box("sepfix_box_import");
    let store = json!({ "connectors": [{ "name": "sneak", "enabled": false, "trigger": { "kind": "linear-label", "label": "x" },
        "folder": "/w", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "" }] });
    let call = json!({ "cmd": "import", "store": store, "linear_key": "lin_api_sneak", "slack_key": null });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, call), refused("import"));
    assert_eq!(sql_text(&hub, &format!("select count(*)::text from accounts where id = '{team}' and coalesce(store::text, '') like '%sneak%'")), "0");
}

#[test]
fn a_team_box_may_not_import_tastes() {
    let (hub, _) = team_box("sepfix_box_tastes");
    let call = json!({ "cmd": "import_tastes", "tastes": { "projects": { "github.com/acme/other": { "parallel": 7 } } } });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, call), refused("import_tastes"));
}

#[test]
fn a_team_box_may_not_share_a_project() {
    let (hub, team) = team_box("sepfix_box_share");
    let call = json!({ "cmd": "share_project", "project": { "key": "github.com/acme/site", "name": "site", "repo": "git@github.com:acme/site.git" } });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, call), refused("share_project"));
    assert_eq!(sql_text(&hub, &format!("select count(*)::text from shared_projects where account = '{team}'")), "0");
}

#[test]
fn a_team_box_may_not_delegate() {
    let (hub, _) = team_box("sepfix_box_delegate");
    let request = serde_json::to_value(pinned_core::delegate::parse_request(&json!({ "task": "Fix it." })).unwrap()).unwrap();
    let call = json!({ "cmd": "delegate", "tab": "tab-1", "task": "t1", "request": request, "access": "auto" });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, call), refused("delegate"));
}

#[test]
fn a_team_box_may_not_sync_a_projects_files() {
    let (hub, team) = team_box("sepfix_box_sync");
    let call = json!({ "cmd": "sync_push", "project": "github.com/acme/site", "files": [] });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, call), refused("sync_push"));
    let call = json!({ "cmd": "sync_list", "project": "github.com/acme/site" });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, call), refused("sync_list"));
    assert_eq!(sql_text(&hub, &format!("select count(*)::text from project_files where account = '{team}'")), "0");
}

#[test]
fn a_team_box_may_not_load_mcp_secrets() {
    let (hub, _) = team_box("sepfix_box_mcp");
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, json!({ "cmd": "load_mcp_servers" })), refused("load_mcp_servers"));
}

#[test]
fn a_team_box_may_not_read_tastes_or_machine_states() {
    let (hub, _) = team_box("sepfix_box_tastes_read");
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, json!({ "cmd": "get_tastes" })), refused("get_tastes"));
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, json!({ "cmd": "machine_states" })), refused("machine_states"));
}

#[test]
fn a_team_box_may_not_act_as_an_agent() {
    let (hub, _) = team_box("sepfix_box_act");
    let act = json!({ "cmd": "act", "act": { "action": "get_settings", "args": null, "tab": "tab-1" } });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, act), refused("act"));
}

#[test]
fn a_team_box_may_not_relay_to_a_capitao() {
    let (hub, _) = team_box("sepfix_box_relay");
    let relay = json!({ "cmd": "relay", "request_id": "r1", "to": { "kind": "captain", "machine": "m1", "path": "/w", "text": "hi" } });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, relay), refused("relay"));
}

#[test]
fn a_team_box_may_not_read_a_session() {
    let (hub, _) = team_box("sepfix_box_read_session");
    let read = json!({ "cmd": "read_session", "tab": "tab-1", "machine": "6f1c0b1e-0000-4000-8000-00000000b0b0" });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, read), refused("read_session"));
}

#[test]
fn a_team_box_asks_its_people_and_a_persons_machine_forwards_as_before() {
    let (hub, _) = team_box("sepfix_box_allowed");
    let ask = json!({ "cmd": "team", "op": { "op": "ask_people", "situation": "in_doubt", "question": "Which?", "choices": ["red", "blue"] } });
    assert_eq!(machine_result(&hub.url, TEAM_BOX_TOKEN, ask), Err(NOT_BUILT.into()), "it reaches the op, which is a stub on this base");
    let (token, _) = account::signup(&hub.url, "bruno@example.test", PASSWORD).unwrap();
    let (_, machine_token) = account::register_machine(&hub.url, &token, "bruno-laptop", "linux").unwrap();
    let share = json!({ "cmd": "share_project", "project": { "key": "github.com/b/site", "name": "site", "repo": "git@github.com:b/site.git" } });
    assert!(machine_result(&hub.url, &machine_token, share).is_ok(), "a person's machine still shares its projects");
}

// ---- Only a person lets an agent in or owns a team ----

#[test]
fn a_grant_and_token_made_for_a_team_find_no_agent_and_issue_no_token() {
    let hub = TestHub::start("sepfix_agent");
    let (mut ana, _, _) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let grant = "6f1c0b1e-0000-4000-8000-00000000a9a9";
    admin_on(&hub.database, "insert into agent_clients (id, name, redirect_uris) values ('agent-x', 'Agent X', '[]')");
    admin_on(&hub.database, &format!("insert into agent_grants (id, account, client, scopes, projects) values ('{grant}', '{team}', 'agent-x', '[\"sessions\"]', '[]')"));
    admin_on(&hub.database, &format!("insert into agent_tokens (token_hash, grant_id, kind, expires_at) values (sha256('team-agent-token'::bytea), '{grant}', 'access', now() + interval '1 hour')"));
    let db = &hub.hub().db;
    assert_eq!(hub.runtime.block_on(db.agent_by_token("team-agent-token")).unwrap().map(|g| g.id), None, "the team's token finds no grant");
    assert_eq!(hub.runtime.block_on(db.issue_agent_tokens(grant.parse().unwrap())), Ok(None), "the team's grant issues no token");
    let request = pinned_server::db::AgentRequestRow {
        client: "agent-x".into(),
        client_name: "Agent X".into(),
        redirect_uri: "http://localhost/cb".into(),
        state: None,
        scopes: vec!["sessions".into()],
        code_challenge: "c".into(),
        resource: format!("{}/mcp", hub.url),
        expires_at: 0,
    };
    let team_id: uuid::Uuid = team.parse().unwrap();
    assert_eq!(hub.runtime.block_on(db.create_agent_grant(team_id, &request, &["sessions".into()], &[])), Err(format!("account {team} cannot let an agent in")));
    assert_eq!(sql_text(&hub, &format!("select count(*)::text from agent_grants where account = '{team}'")), "1", "only the grant made by SQL");
    assert_eq!(sql_text(&hub, "select count(*)::text from agent_tokens"), "1", "no token was issued");
}

#[test]
fn a_team_cannot_own_a_team() {
    let hub = TestHub::start("sepfix_owner");
    let (mut ana, _, _) = locked_app(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let made = hub.runtime.block_on(pinned_server::teams::db::create_team(&hub.hub().db, team.parse().unwrap(), "Inner"));
    assert_eq!(made, Err("the team was not made".into()));
    assert_eq!(sql_text(&hub, "select count(*)::text from teams"), "1");
    assert_eq!(sql_text(&hub, "select count(*)::text from accounts where kind = 'team'"), "1");
}
