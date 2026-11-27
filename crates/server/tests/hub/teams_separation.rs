//! pinned Teams: a member's own account is unreachable from a team, and a team stays out of every
//! place that lists, counts or signs in accounts. Plain English texts and literal values throughout.
//!
//! Where a unit this one is built beside is still a stub, its op answers `not built yet`: the
//! reads below accept that answer (there is nothing to leak) and `stubs_seen` prints which ones.
//! The `join` op may be a stub too, so a member is put on a team with SQL (`join_team`).

use super::*;
use uuid::Uuid;

const B_TOKEN: &str = "sk-ant-oat01-bprivatesecret";
const B_HINT: &str = "…cret";
const B_TOKEN_NAME: &str = "b-token-name";
const B_FLOW: &str = "b-private-flow";
const B_SCHEDULE: &str = "b-private-schedule";
const B_MACHINE: &str = "b-secret-laptop";
const B_EMAIL: &str = "b@example.test";
const B_PROJ_1: &str = "github.com/b-corp/b-proj-1";
const B_PROJ_2: &str = "github.com/b-corp/b-proj-2";
const NOT_BUILT: &str = "not built yet";
/// The reads still stubbed on this base, the only ones that may answer `NOT_BUILT`.
const STUBS: [&str; 3] = ["bots", "runs", "calls"];
const NOT_MEMBER: &str = "you are not a member of that team";
const OWNER_ONLY: &str = "only the team's owner runs";

fn sql_text(hub: &TestHub, sql: &str) -> String {
    hub.runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        client.query_one(sql, &[]).await.unwrap().get::<_, String>(0)
    })
}

fn run_sql(hub: &TestHub, sql: &str) {
    admin_on(&hub.database, sql);
}

fn project(key: &str) -> Value {
    json!({ "key": key, "name": key.rsplit('/').next().unwrap(), "repo": format!("git@{}.git", key.replacen('/', ":", 1)) })
}

fn team_op(client: &mut Client, team: &str, op: Value) -> Result<Value, String> {
    client.hub(json!({ "cmd": "team", "team": team, "op": op }))
}

/// A personal call run on the team's account.
fn via(client: &mut Client, team: &str, call: Value) -> Result<Value, String> {
    team_op(client, team, json!({ "op": "hub", "call": call }))
}

fn join_team(hub: &TestHub, team: &str, member: &str) {
    run_sql(hub, &format!("insert into team_members (team, member, role) values ('{team}', '{member}', 'member')"));
}

/// Two people, B with a lot of personal data, and a team A owns.
struct Cast {
    hub: TestHub,
    a: Client,
    a_id: String,
    b: Client,
    b_id: String,
    b_machine: String,
    team: String,
}

fn cast(name: &str, b_joins: bool) -> Cast {
    let hub = TestHub::start(name);
    let url = hub.url.clone();
    let (_, a_id, mut a) = signed_in(&url, "a@example.test");
    let (b_token, b_id, mut b) = signed_in(&url, B_EMAIL);

    let workflow = json!({ "name": B_FLOW, "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
        "folder": "/w/b-private", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "" });
    b.hub(json!({ "cmd": "save_connector", "connector": workflow })).unwrap();
    // A named Claude token is kept only once Upstash Box is connected.
    b.hub(json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN })).unwrap();
    assert_eq!(b.hub(json!({ "cmd": "set_box_claude_token", "name": B_TOKEN_NAME, "token": B_TOKEN })), Ok(json!({ B_TOKEN_NAME: B_HINT })));
    save_schedule(&mut b, task_body(B_SCHEDULE)).unwrap();
    for key in [B_PROJ_1, B_PROJ_2] {
        b.hub(json!({ "cmd": "share_project", "project": project(key) })).unwrap();
    }
    let (machine, _) = account::register_machine(&url, &b_token, B_MACHINE, "linux").unwrap();

    let team = a.hub(json!({ "cmd": "team", "op": { "op": "create", "name": "Acme bots" } })).unwrap()["id"].as_str().unwrap().to_owned();
    if b_joins {
        join_team(&hub, &team, &b_id);
    }
    Cast { hub, a, a_id, b, b_id, b_machine: machine.id, team }
}

/// Makes B's machine a box of B's own account, so B's own `box_status` and `extend_box` find it.
fn make_b_machine_a_box(s: &Cast) {
    run_sql(&s.hub, &format!(
        "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase) values ('b-box', '{}', '{}', 'box_b', 'c-b', '{{\"issue\":\"b1\"}}', '{{\"folder\":\"/w\",\"args\":[]}}', 'small', 'running')",
        s.b_id, s.b_machine
    ));
}

/// Everything B's own account holds that a team must never take or change.
fn personal(b: &mut Client) -> Value {
    json!({
        "settings": b.hub(json!({ "cmd": "get_settings" })).unwrap(),
        "projects": b.hub(json!({ "cmd": "shared_projects" })).unwrap(),
        "schedules": b.hub(json!({ "cmd": "list_schedules" })).unwrap(),
        "machines": b.hub(json!({ "cmd": "machines" })).unwrap(),
    })
}

/// B's own data as `personal` reads it, before any team call: the fixture every test starts from.
fn assert_b_personal_is_whole(snapshot: &Value) {
    let text = snapshot.to_string();
    for needle in [B_FLOW, B_SCHEDULE, B_HINT, B_TOKEN_NAME, B_PROJ_1, B_PROJ_2, B_MACHINE] {
        assert!(text.contains(needle), "B's own account lost {needle}: {text}");
    }
    assert!(!text.contains("bprivatesecret"), "even B's own settings show only the hint: {text}");
}

#[test]
fn nothing_of_a_members_own_account_shows_in_what_the_team_answers() {
    let mut s = cast("sep_reads", true);
    make_b_machine_a_box(&s);
    let before = personal(&mut s.b);
    assert_b_personal_is_whole(&before);
    let team = s.team.clone();

    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "attach_project", "key": B_PROJ_1 })), Ok(Value::Null));

    let reads: Vec<(&str, Value)> = vec![
        ("members", json!({ "op": "members" })),
        ("projects", json!({ "op": "projects" })),
        ("groups", json!({ "op": "groups" })),
        ("bots", json!({ "op": "bots" })),
        ("runs", json!({ "op": "runs" })),
        ("calls", json!({ "op": "calls" })),
        ("hub get_settings", json!({ "op": "hub", "call": { "cmd": "get_settings" } })),
        ("hub list_schedules", json!({ "op": "hub", "call": { "cmd": "list_schedules" } })),
    ];
    let secrets = [B_FLOW, B_SCHEDULE, "bprivatesecret", B_HINT, B_TOKEN_NAME, B_PROJ_2, "b-proj-2", B_MACHINE, s.b_machine.as_str()];
    let mut stubs_seen = vec![];
    for (who, client) in [("A", &mut s.a), ("B", &mut s.b)] {
        for (label, op) in &reads {
            let answer = team_op(client, &team, op.clone());
            let text = match &answer {
                Ok(value) => value.to_string(),
                // A team's settings and schedules are no member's to read: refused, naming the call only.
                Err(e) if label.starts_with("hub ") => {
                    assert_eq!(e, &format!("{} does not run on a team", &label[4..]), "{who} {label}");
                    e.clone()
                }
                Err(e) if e == NOT_BUILT && STUBS.contains(label) => {
                    stubs_seen.push(format!("{who} {label}"));
                    continue;
                }
                Err(e) => panic!("{who} {label} answered the error {e}"),
            };
            for secret in secrets {
                assert!(!text.contains(secret), "{who} {label} shows {secret}: {text}");
            }
            if *label != "members" {
                assert!(!text.contains(B_EMAIL), "{who} {label} shows B's email outside the members: {text}");
            }
        }
    }
    eprintln!("stubs seen on this base, which pass until built: {stubs_seen:?}");

    // What did cross: the key, the name, the repo and who attached it, nothing else.
    for client in [&mut s.a, &mut s.b] {
        assert_eq!(
            team_op(client, &team, json!({ "op": "projects" })),
            Ok(json!([{ "key": B_PROJ_1, "name": "b-proj-1", "repo": "git@github.com:b-corp/b-proj-1.git", "member": s.b_id }]))
        );
    }

    // A box's status takes a machine id: the team has none of B's, though B's own account finds its box.
    let unknown = "6f1c0b1e-0000-4000-8000-0000000000ee";
    let status = json!({ "cmd": "box_status", "machine": s.b_machine });
    assert_eq!(s.b.hub(status.clone()).map(|box_status| box_status["phase"].is_string()), Ok(true), "B's machine is a box of B's own account");
    assert_eq!(via(&mut s.a, &team, json!({ "cmd": "box_status", "machine": unknown })), Err("this machine is no box".into()));
    for client in [&mut s.a, &mut s.b] {
        assert_eq!(via(client, &team, status.clone()), Err("this machine is no box".into()), "B's machine answers as a machine nobody has: the team has none of its own");
    }
    // B's real workflow, run on the team by B itself, is one the team's account does not have.
    let b_flow = before["settings"]["connectors"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(via(&mut s.b, &team, json!({ "cmd": "run_workflow", "id": b_flow })), Err("run_workflow does not run on a team".into()));
    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "run_now", "workflow": b_flow })), Err(format!("this team has no workflow {b_flow}")));

    assert_eq!(personal(&mut s.b), before, "B's own account is as it was");
}

/// Every personal-only call, as a `cmd` the hub reads, which a team refuses.
fn personal_only() -> Vec<Value> {
    let request = serde_json::to_value(pinned_core::delegate::parse_request(&json!({ "task": "Fix it." })).unwrap()).unwrap();
    vec![
        json!({ "cmd": "machines" }),
        json!({ "cmd": "pair_code" }),
        json!({ "cmd": "unlock", "password": "correct horse" }),
        json!({ "cmd": "import", "store": { "connectors": [] }, "linear_key": null, "slack_key": null }),
        json!({ "cmd": "delegate", "tab": "tab-1", "task": "t1", "request": request, "access": "auto" }),
        json!({ "cmd": "sync_push", "project": B_PROJ_1, "files": [] }),
        json!({ "cmd": "sync_pull", "project": B_PROJ_1, "paths": [] }),
        json!({ "cmd": "relay", "request_id": "r1", "to": { "kind": "captain", "machine": "m1", "path": "/w", "text": "hi" } }),
        json!({ "cmd": "read_session", "tab": "tab-1" }),
        json!({ "cmd": "create_invite" }),
        json!({ "cmd": "decide_confirmation", "id": "c1", "approve": true }),
        json!({ "cmd": "external_agents" }),
        json!({ "cmd": "shared_projects" }),
        json!({ "cmd": "share_project", "project": project(B_PROJ_2) }),
        json!({ "cmd": "sync_list", "project": B_PROJ_1 }),
        json!({ "cmd": "remove_machine", "id": "m1" }),
        json!({ "cmd": "team", "op": { "op": "list" } }),
        json!({ "cmd": "team", "team": "t", "op": { "op": "hub", "call": { "cmd": "get_settings" } } }),
    ]
}

#[test]
fn a_call_that_belongs_to_a_person_is_refused_on_a_team_for_its_owner_and_its_members() {
    let mut s = cast("sep_personal", true);
    let before = personal(&mut s.b);
    let team = s.team.clone();
    // A decision needs the password even on a team's call: until the app unlocks, the hub says so.
    let decide = json!({ "cmd": "decide_confirmation", "id": "c1", "approve": true });
    assert_eq!(via(&mut s.a, &team, decide.clone()), Err(LOCKED.into()));
    for client in [&mut s.a, &mut s.b] {
        client.hub(json!({ "cmd": "unlock", "password": "correct horse" })).unwrap();
    }
    for call in personal_only() {
        let cmd = call["cmd"].as_str().unwrap().to_owned();
        for (who, client) in [("owner", &mut s.a), ("member", &mut s.b)] {
            assert_eq!(via(client, &team, call.clone()), Err(format!("{cmd} does not run on a team")), "{who} {cmd}");
        }
    }
    assert_eq!(personal(&mut s.b), before);
    assert_eq!(s.hub.runtime.block_on(s.hub.hub().db.shared_projects(s.a_id.parse().unwrap())).unwrap().len(), 0, "A's own projects are no one's business either");
}

fn no_tokens(tokens: &Value) -> bool {
    tokens.is_null() || tokens == &json!({})
}

#[test]
fn a_member_cannot_set_the_teams_keys_but_its_owner_can() {
    let mut s = cast("sep_owner", true);
    make_b_machine_a_box(&s);
    let before = personal(&mut s.b);
    let team = s.team.clone();
    let owner_calls = [
        json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN }),
        json!({ "cmd": "set_box_claude_token", "name": "team-claude", "token": "sk-ant-oat01-teamtoken" }),
        json!({ "cmd": "connect_github", "token": "ghp_faketoken" }),
        json!({ "cmd": "connect_slack", "token": "xoxb-faketoken" }),
        json!({ "cmd": "set_box_access", "access": { "workflow": { "mode": "ask" } } }),
        json!({ "cmd": "extend_box", "machine": s.b_machine, "extra_usd": 1.0 }),
        json!({ "cmd": "disconnect_upstash" }),
        json!({ "cmd": "remove_box_claude_token", "name": B_TOKEN_NAME }),
    ];
    for call in &owner_calls {
        let cmd = call["cmd"].as_str().unwrap();
        assert_eq!(via(&mut s.b, &team, call.clone()), Err(format!("{OWNER_ONLY} {cmd} on the team")), "member {cmd}");
    }
    // Refused before it ran: the team holds none of it.
    let team_id = Uuid::parse_str(&team).unwrap();
    let team_settings = team_settings(&s.hub, team_id);
    assert_eq!(team_settings["upstash"], Value::Null);
    assert_eq!(no_tokens(&team_settings["box_claude_tokens"]), true);

    // The owner's same calls pass the gate. connect_github and connect_slack would reach GitHub and
    // Slack, so they are only refused above, never run for the owner here.
    // A key call answers which keys are set, never a hint of one.
    let connected = via(&mut s.a, &team, owner_calls[0].clone()).unwrap();
    assert_eq!(connected, json!({ "upstash": true, "claude": true, "github": false, "slack": false, "linear": false }));
    assert_eq!(via(&mut s.a, &team, owner_calls[1].clone()), Ok(connected));
    via(&mut s.a, &team, owner_calls[4].clone()).unwrap();
    let extended = via(&mut s.a, &team, owner_calls[5].clone());
    let unknown = via(&mut s.a, &team, json!({ "cmd": "extend_box", "machine": "6f1c0b1e-0000-4000-8000-0000000000ee", "extra_usd": 1.0 }));
    assert_eq!(s.b.hub(json!({ "cmd": "box_status", "machine": s.b_machine })).map(|status| status["phase"].is_string()), Ok(true), "B's machine is a box of B's own account");
    assert_eq!(extended, Err("this machine is no box".into()), "the owner passes the gate and finds B's machine is no team box");
    assert_eq!(extended, unknown, "B's machine answers as one nobody has");

    let team_settings = self::team_settings(&s.hub, team_id);
    assert_eq!(team_settings["box_claude_tokens"], json!({ "team-claude": "…oken" }));
    assert!(!team_settings["upstash"].is_null());
    assert!(!team_settings.to_string().contains(BOX_KEY) && !team_settings.to_string().contains("teamtoken"), "the team's keys show only as hints: {team_settings}");

    // A's own account got none of it, and B's is as it was.
    let a_own = s.a.hub(json!({ "cmd": "get_settings" })).unwrap();
    assert!(a_own["upstash"].is_null());
    assert_eq!(no_tokens(&a_own["box_claude_tokens"]), true);
    assert_eq!(personal(&mut s.b), before);
}

#[test]
fn a_team_the_person_is_not_in_refuses_every_op_and_a_removed_member_is_refused_at_once() {
    let mut s = cast("sep_outsider", true);
    let other = s.a.hub(json!({ "cmd": "team", "op": { "op": "create", "name": "Other" } })).unwrap()["id"].as_str().unwrap().to_owned();
    let run = "6f1c0b1e-0000-4000-8000-000000000001";
    let ops = [
        json!({ "op": "rename", "name": "Mine now" }),
        json!({ "op": "members" }),
        json!({ "op": "invite" }),
        json!({ "op": "revoke_invite", "hint": "ABCD" }),
        json!({ "op": "remove_member", "account": s.a_id }),
        json!({ "op": "leave" }),
        json!({ "op": "attach_project", "key": B_PROJ_1 }),
        json!({ "op": "detach_project", "key": B_PROJ_1 }),
        json!({ "op": "projects" }),
        json!({ "op": "groups" }),
        json!({ "op": "save_group", "group": { "handle": "design" } }),
        json!({ "op": "join_group", "handle": "design" }),
        json!({ "op": "leave_group", "handle": "design" }),
        json!({ "op": "bots" }),
        json!({ "op": "save_bot", "bot": { "workflow": "c1" } }),
        json!({ "op": "turn_bot_on", "workflow": "c1", "on": true }),
        json!({ "op": "run_now", "workflow": "c1" }),
        json!({ "op": "runs" }),
        json!({ "op": "calls" }),
        json!({ "op": "answer_call", "id": run, "answer": { "kind": { "kind": "choice" }, "choice": "red" } }),
        json!({ "op": "take_over", "run": run }),
        json!({ "op": "give_back", "run": run }),
        json!({ "op": "stop_run", "run": run }),
        json!({ "op": "chat", "text": "hello" }),
        json!({ "op": "hub", "call": { "cmd": "get_settings" } }),
        json!({ "op": "hub", "call": { "cmd": "save_connector", "connector": { "name": "sneak", "enabled": false, "trigger": { "kind": "linear-label", "label": "x" },
            "folder": "/w", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "" } } }),
        json!({ "op": "hub", "call": { "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN } }),
    ];
    for op in &ops {
        assert_eq!(team_op(&mut s.b, &other, op.clone()), Err(NOT_MEMBER.into()), "{op}");
    }
    assert_eq!(team_op(&mut s.b, "6f1c0b1e-0000-4000-8000-0000000000ff", json!({ "op": "members" })), Err(NOT_MEMBER.into()), "a team that does not exist answers the same");
    assert_eq!(team_op(&mut s.b, "not-a-team", json!({ "op": "members" })), Err("not-a-team is not a team id".into()));
    // The refused calls changed nothing on the other team.
    assert_eq!(team_workflows(&s.hub, Uuid::parse_str(&other).unwrap()), Vec::<Value>::new(), "no sneaked workflow");

    // A member on T reads it; the row goes; the very next call is refused.
    let team = s.team.clone();
    assert!(team_op(&mut s.b, &team, json!({ "op": "members" })).is_ok());
    run_sql(&s.hub, &format!("delete from team_members where team = '{team}' and member = '{}'", s.b_id));
    assert_eq!(via(&mut s.b, &team, json!({ "cmd": "box_status", "machine": "6f1c0b1e-0000-4000-8000-0000000000ee" })), Err(NOT_MEMBER.into()));
    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "projects" })), Err(NOT_MEMBER.into()));
    assert_eq!(team_op(&mut s.a, &team, json!({ "op": "members" })).map(|_| ()), Ok(()), "the owner is still in");
}

#[test]
fn a_workflow_a_member_saves_on_the_team_is_the_teams_and_not_the_members() {
    let mut s = cast("sep_store", true);
    let before = personal(&mut s.b);
    let a_before = s.a.hub(json!({ "cmd": "get_settings" })).unwrap();
    let team = s.team.clone();
    let workflow = json!({ "name": "team-flow", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
        "folder": "/w/team", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "" });
    // A team's workflow is saved with its bot; a team keeps no schedules.
    let saved = team_op(&mut s.a, &team, json!({ "op": "save_bot", "bot": { "workflow": "" }, "connector": workflow }));
    let id = team_workflows(&s.hub, Uuid::parse_str(&team).unwrap())[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(saved, Ok(json!({ "workflow": id })));
    assert_eq!(via(&mut s.b, &team, json!({ "cmd": "save_schedule", "task": task_body("team-schedule") })), Err("save_schedule does not run on a team".into()));

    for client in [&mut s.a, &mut s.b] {
        let bots = team_op(client, &team, json!({ "op": "bots" })).unwrap();
        let names = bots.as_array().unwrap().iter().map(|b| (b["workflow"].as_str().unwrap().to_owned(), b["name"].as_str().unwrap().to_owned())).collect::<Vec<_>>();
        assert_eq!(names, [(id.clone(), "team-flow".to_owned())], "both members see the team's workflow, and only it");
    }
    let a_after = s.a.hub(json!({ "cmd": "get_settings" })).unwrap();
    assert_eq!(a_after, a_before, "A's own settings did not get the team's workflow");
    assert!(!a_after.to_string().contains(&id) && !a_after.to_string().contains("team-flow"));
    assert!(!s.a.hub(json!({ "cmd": "list_schedules" })).unwrap().to_string().contains("team-schedule"));
    assert_eq!(personal(&mut s.b), before, "B's own account is as it was");
    // It is stored under the team's account, by SQL as well.
    assert_eq!(sql_text(&s.hub, &format!("select count(*)::text from accounts where id = '{team}' and store::text like '%team-flow%'")), "1");
    assert_eq!(sql_text(&s.hub, &format!("select count(*)::text from accounts where id <> '{team}' and store::text like '%team-flow%'")), "0");
}

#[test]
fn attaching_a_project_copies_its_name_and_repo_and_never_its_files_or_sessions() {
    let mut s = cast("proj_attach", true);
    let team = s.team.clone();
    let file = |path: &str, text: &str| json!({ "path": path, "sha": sha256_hex(text.as_bytes()), "modified": 1, "data": base64::engine::general_purpose::STANDARD.encode(gzip_bytes(text.as_bytes())) });
    s.b.hub(json!({ "cmd": "sync_push", "project": B_PROJ_1, "files": [file("claude/memory/notes.md", "b's private plans")] })).unwrap();

    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "attach_project", "key": "github.com/nobody/none" })), Err("you can only attach a project you have".into()));
    assert_eq!(team_op(&mut s.a, &team, json!({ "op": "attach_project", "key": B_PROJ_1 })), Err("you can only attach a project you have".into()), "A cannot attach B's project by its key");
    assert_eq!(team_op(&mut s.a, &team, json!({ "op": "projects" })), Ok(json!([])));

    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "attach_project", "key": B_PROJ_1 })), Ok(Value::Null));
    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "attach_project", "key": B_PROJ_1 })), Ok(Value::Null), "twice is the same as once");
    let expected = json!([{ "key": B_PROJ_1, "name": "b-proj-1", "repo": "git@github.com:b-corp/b-proj-1.git", "member": s.b_id }]);
    assert_eq!(team_op(&mut s.a, &team, json!({ "op": "projects" })), Ok(expected.clone()), "every member reads it");
    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "projects" })), Ok(expected.clone()));

    // A shares the same key under its own repo; attaching it neither errs nor takes the project over.
    s.a.hub(json!({ "cmd": "share_project", "project": { "key": B_PROJ_1, "name": "mine", "repo": "git@example.test:a/mine.git" } })).unwrap();
    assert_eq!(team_op(&mut s.a, &team, json!({ "op": "attach_project", "key": B_PROJ_1 })), Ok(Value::Null));
    assert_eq!(team_op(&mut s.a, &team, json!({ "op": "projects" })), Ok(expected), "the first attach stands");

    // Nothing but the four columns crossed: no file, no session, no token, and the team shares no project of its own.
    let team_id = &team;
    for table in ["project_files", "shared_projects"] {
        assert_eq!(sql_text(&s.hub, &format!("select count(*)::text from {table} where account = '{team_id}'")), "0", "{table}");
    }
    assert_eq!(sql_text(&s.hub, &format!("select count(*)::text from project_files where account = '{}' and project = '{B_PROJ_1}'", s.b_id)), "1", "B's file stays B's");
    assert_eq!(sql_text(&s.hub, "select count(*)::text from team_projects"), "1");
    let columns = sql_text(&s.hub, "select string_agg(column_name, ',' order by ordinal_position) from information_schema.columns where table_name = 'team_projects'");
    assert_eq!(columns, "team,key,name,repo,member,attached_at");
    assert_eq!(s.b.hub(json!({ "cmd": "shared_projects" })).unwrap().as_array().unwrap().len(), 2, "B still shares both of its own projects");
    assert_eq!(via(&mut s.b, &team, json!({ "cmd": "sync_list", "project": B_PROJ_1 })), Err("sync_list does not run on a team".into()));
}

#[test]
fn only_the_owner_or_the_member_who_attached_a_project_detaches_it() {
    let mut s = cast("proj_detach", true);
    let team = s.team.clone();
    s.a.hub(json!({ "cmd": "share_project", "project": project("github.com/a-corp/a-proj") })).unwrap();
    team_op(&mut s.b, &team, json!({ "op": "attach_project", "key": B_PROJ_1 })).unwrap();
    team_op(&mut s.b, &team, json!({ "op": "attach_project", "key": B_PROJ_2 })).unwrap();
    team_op(&mut s.a, &team, json!({ "op": "attach_project", "key": "github.com/a-corp/a-proj" })).unwrap();
    let keys = |client: &mut Client| -> Vec<String> {
        team_op(client, &team, json!({ "op": "projects" })).unwrap().as_array().unwrap().iter().map(|p| p["key"].as_str().unwrap().to_owned()).collect()
    };
    assert_eq!(keys(&mut s.a).len(), 3);

    let refused = "only the team's owner or the member who attached a project detaches it";
    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "detach_project", "key": "github.com/a-corp/a-proj" })), Err(refused.into()), "a member cannot detach another's");
    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "detach_project", "key": "github.com/no/such" })), Err("the team has no project github.com/no/such".into()));
    assert_eq!(team_op(&mut s.b, &team, json!({ "op": "detach_project", "key": B_PROJ_2 })), Ok(Value::Null), "a member detaches its own");
    assert_eq!(team_op(&mut s.a, &team, json!({ "op": "detach_project", "key": B_PROJ_1 })), Ok(Value::Null), "the owner detaches anyone's");
    assert_eq!(keys(&mut s.b), ["github.com/a-corp/a-proj"]);
    assert_eq!(team_op(&mut s.a, &team, json!({ "op": "detach_project", "key": B_PROJ_1 })), Err(format!("the team has no project {B_PROJ_1}")), "detached already");
    assert_eq!(s.b.hub(json!({ "cmd": "shared_projects" })).unwrap().as_array().unwrap().len(), 2, "B's own list is untouched by detaching");
    assert_eq!(s.a.hub(json!({ "cmd": "shared_projects" })).unwrap().as_array().unwrap().len(), 1);
}

// ---- The accounts sweep: a team is not a person ----

fn make_team_account_row(hub: &TestHub) -> String {
    let id = "6f1c0b1e-0000-4000-8000-00000000aaaa";
    run_sql(hub, &format!("insert into accounts (id, email, password_hash, kind) values ('{id}', 'team-{id}@teams.invalid', '!', 'team')"));
    id.to_owned()
}

#[test]
fn a_team_does_not_count_against_max_accounts() {
    let mut hub = TestHub::start("sweep_cap");
    hub.access.max_accounts = Some(2);
    hub.restart();
    let url = hub.url.clone();
    let (status, ana) = post_with(&url, "/api/signup", &[], new_account(1));
    assert_eq!(status, 200, "{ana}");
    let mut client = signed_in_client(&url, ana["token"].as_str().unwrap());
    for name in ["One", "Two", "Three"] {
        client.hub(json!({ "cmd": "team", "op": { "op": "create", "name": name } })).unwrap();
    }
    assert_eq!(hub.runtime.block_on(hub.hub().db.account_count()), Ok(1), "three teams and one person");
    assert_eq!(post_with(&url, "/api/signup", &[], new_account(2)).0, 200, "the second person fits under a cap of two");
    assert_eq!(post_with(&url, "/api/signup", &[], new_account(3)), (403, json!({ "error": NOT_TAKING })), "the cap still holds for people");
}

#[test]
fn signup_first_looks_at_people_and_a_team_row_is_no_first_account() {
    let hub = TestHub::start_with("sweep_first", Signup::First);
    let url = hub.url.clone();
    make_team_account_row(&hub);
    assert_eq!(hub.runtime.block_on(hub.hub().db.has_accounts()), Ok(false), "a team is no one's sign up");
    let (status, ana) = post_with(&url, "/api/signup", &[], new_account(1));
    assert_eq!(status, 200, "the first person still signs up: {ana}");
    assert_eq!(hub.runtime.block_on(hub.hub().db.has_accounts()), Ok(true));
    assert_eq!(post_with(&url, "/api/signup", &[], new_account(2)), (403, json!({ "error": "this hub already has its account, so sign in with it instead" })));
}

#[test]
fn a_team_has_no_password_to_check_or_set_and_no_login() {
    let hub = TestHub::start("sweep_password");
    let url = hub.url.clone();
    let team = make_team_account_row(&hub);
    let id: uuid::Uuid = team.parse().unwrap();
    // The team's row gets a real hash, the hub's own of a known password, so only its kind refuses it.
    account::signup(&url, "someone@example.test", "correct horse").unwrap();
    run_sql(&hub, &format!("update accounts set password_hash = (select password_hash from accounts where email = 'someone@example.test') where id = '{team}'"));
    let db = &hub.hub().db;
    assert_eq!(hub.runtime.block_on(db.account(id)), Ok(None), "the password check finds no team");
    assert_eq!(account::login(&url, &format!("team-{team}@teams.invalid"), "correct horse"), Err("wrong email or password".into()), "the right password signs no team in");
    assert_eq!(hub.runtime.block_on(db.set_password(id, "$argon2id$not-a-hash")), Ok(()));
    assert_eq!(
        sql_text(&hub, &format!("select password_hash from accounts where id = '{team}'")),
        sql_text(&hub, "select password_hash from accounts where email = 'someone@example.test'"),
        "set_password left the team's row as it was"
    );
    assert_eq!(account::login(&url, &format!("team-{team}@teams.invalid"), "!"), Err("wrong email or password".into()));
    assert_eq!(hub.runtime.block_on(db.create_login(id)), Err(format!("account {team} cannot sign in")));
}
