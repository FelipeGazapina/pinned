//! Teams, hardening B: a team's events reach only the app connections that asked about teams,
//! a team op runs only on the app's own path, invite codes never expire, a member who leaves
//! hands back what it held, and a team's box acts only for the run bound to it.

use super::teams_calls::{design, op, patient, rows, sql, TeamBox};
use super::*;
use uuid::Uuid;

/// A member of `tb`'s team in group `design`, signed up here so its login can open more sockets.
fn member_with_token(hub: &TestHub, tb: &TeamBox, email: &str) -> (Uuid, String) {
    let (token, account) = account::signup(&hub.url, email, "correct horse").unwrap();
    let id = Uuid::parse_str(&account.id).unwrap();
    sql(hub, &format!(
        "insert into team_members (team, member, role) values ('{0}', '{1}', 'member');
         insert into team_groups (team, handle) values ('{0}', 'design') on conflict do nothing;
         insert into team_group_members (team, handle, member) values ('{0}', 'design', '{1}')",
        tb.team, id
    ));
    (id, token)
}

/// A raw machine socket signed in with a machine token of the login `token`.
fn raw_machine(hub: &TestHub, token: &str) -> WebSocket<MaybeTlsStream<TcpStream>> {
    let (_, machine_token) = account::register_machine(&hub.url, token, "mini", "linux").unwrap();
    let (mut ws, _) = tungstenite::connect(format!("{}/ws/machine", hub.url.replace("http://", "ws://"))).unwrap();
    if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    }
    ws.send(Message::text(json!({ "type": "auth", "token": machine_token, "name": "mini", "os": "linux", "version": "test" }).to_string())).unwrap();
    read_machine(&mut ws, |frame| frame["type"] == "hello");
    ws
}

/// Every frame the machine socket got up to the reply to a call it sends now: the hub's frames
/// to one socket keep their order.
fn machine_frames_until_a_reply(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, id: u64) -> Vec<Value> {
    ws.send(Message::text(json!({ "type": "call", "id": id, "call": { "cmd": "get_tastes" } }).to_string())).unwrap();
    let (start, mut seen) = (Instant::now(), Vec::new());
    loop {
        match ws.read() {
            Ok(Message::Text(text)) => {
                let frame: Value = serde_json::from_str(&text).unwrap();
                let done = frame["type"] == "reply" && frame["id"] == id;
                seen.push(frame);
                if done {
                    return seen;
                }
            }
            Ok(Message::Ping(payload)) => ws.send(Message::Pong(payload)).unwrap(),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                assert!(start.elapsed() < WAIT, "timed out waiting for the machine's reply");
            }
            Err(e) => panic!("{e}"),
        }
    }
}

fn team_frames(frames: &[Value]) -> Vec<Value> {
    frames.iter().filter(|f| f["type"] == "team").cloned().collect()
}

#[test]
fn a_team_event_reaches_only_the_app_connections_that_sent_a_team_call_and_never_a_machine() {
    let hub = TestHub::start_with_team_policy("teams_b_push", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let (_, token) = member_with_token(&hub, &tb, "dina@example.test");
    let mut asked = Client::connect(&hub.url, &token);
    asked.wait("hello", |f| f["type"] == "hello");
    let mut quiet = Client::connect(&hub.url, &token);
    quiet.wait("hello", |f| f["type"] == "hello");
    let mut machine = raw_machine(&hub, &token);
    op(tb.team, &mut asked, json!({ "op": "runs" })).unwrap();
    quiet.hub(json!({ "cmd": "get_tastes" })).unwrap();
    tb.bot(&hub, design(), None);
    tb.run(&hub);

    let call = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap()["call"].as_str().unwrap().to_owned();
    let pushed = asked.wait("the call pushed", |f| f["type"] == "team" && f["event"]["kind"] == "call_opened");
    assert_eq!(pushed["event"]["call"]["id"], json!(call));
    // A reply on the same socket comes after any frame the hub queued before it.
    quiet.hub(json!({ "cmd": "get_tastes" })).unwrap();
    assert_eq!(team_frames(&quiet.frames), Vec::<Value>::new(), "the connection that sent no team call heard nothing");
    let machine_saw = machine_frames_until_a_reply(&mut machine, 7);
    assert_eq!(team_frames(&machine_saw), Vec::<Value>::new(), "a machine of the member never hears a team event");

    // Once it sends a team call, the second connection hears the next event too; the machine still does not.
    op(tb.team, &mut quiet, json!({ "op": "calls" })).unwrap();
    op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": { "kind": { "kind": "choice" }, "choice": "red", "text": "" } })).unwrap();
    quiet.wait("the answer pushed", |f| f["type"] == "team" && f["event"]["kind"] == "call_resolved");
    let machine_saw = machine_frames_until_a_reply(&mut machine, 8);
    assert_eq!(team_frames(&machine_saw), Vec::<Value>::new());
}

const AGENTS_NO_TEAM: &str = "team is not open to agents: a member's own calls about a team, which only the app sends";

#[test]
fn a_team_op_from_an_agent_a_team_call_or_a_machine_runs_nothing() {
    let hub = TestHub::start("teams_b_hub_call");
    let (token, _) = account::signup(&hub.url, "ana@example.test", "correct horse").unwrap();
    let mut ana = agent_of(&hub.url, &token, "ana@example.test", "teams-b-mac");
    let team = ana.client.hub(json!({ "cmd": "team", "op": { "op": "create", "name": "Acme" } })).unwrap()["id"].as_str().unwrap().to_owned();
    let counts = |hub: &TestHub| {
        let found = super::teams_calls::rows(hub, "select (select count(*) from team_invites)::text, (select count(*) from accounts where kind = 'team')::text, (select string_agg(name, ',') from teams)::text");
        found[0].iter().map(|c| c.clone().unwrap_or_default()).collect::<Vec<_>>()
    };
    let before = counts(&hub);
    assert_eq!(before, ["0", "1", "Acme"]);

    let invite = json!({ "team": team, "op": { "op": "invite" } });
    let create = json!({ "op": { "op": "create", "name": "Agents" } });
    let rename = json!({ "team": team, "op": { "op": "rename", "name": "Taken" } });
    for args in [&invite, &create, &rename] {
        assert_eq!(act(&ana.dir, &ana.tab, "team", args.clone()), Err(AGENTS_NO_TEAM.into()), "an agent's act {args}");
    }
    // A team op inside a team's own hub call.
    let nested = json!({ "cmd": "team", "team": team, "op": { "op": "hub", "call": { "cmd": "team", "team": team, "op": { "op": "invite" } } } });
    assert_eq!(ana.client.hub(nested), Err("team does not run on a team".into()));
    // A person's machine sending a team op itself.
    let (_, machine_token) = account::register_machine(&hub.url, &token, "raw", "linux").unwrap();
    assert_eq!(
        machine_result(&hub.url, &machine_token, json!({ "cmd": "team", "team": team, "op": { "op": "invite" } })),
        Err("a machine may not send the team op invite".into())
    );
    assert_eq!(counts(&hub), before, "nothing changed");
}

const NO_INVITE: &str = "that invite code is unknown, used or revoked";
const FULL: &str = "this team has 20 open invite codes; revoke some first";

/// Moves invite `code`'s row `age` into the past.
fn backdate(hub: &TestHub, code: &str, age: &str) {
    admin_on(&hub.database, &format!("update team_invites set created_at = now() - interval '{age}' where code_hash = sha256(convert_to('{}', 'UTF8'))", code.replace('-', "")));
}

#[test]
fn an_old_unused_invite_still_works_and_a_used_or_revoked_one_answers_like_an_unknown_one() {
    use super::teams_membership::{join_team, make_team, new_invite, person, team_op};
    let hub = TestHub::start("teams_b_invite_age");
    let (mut ana, _) = person(&hub.url, "ana@example.test");
    let (mut bruno, _) = person(&hub.url, "bruno@example.test");
    let (mut carla, _) = person(&hub.url, "carla@example.test");
    let team = make_team(&mut ana, "Acme");

    let old = new_invite(&mut ana, &team);
    backdate(&hub, &old, "3650 days");
    assert_eq!(join_team(&mut bruno, &old).unwrap()["id"], json!(team), "a ten year old unused code still joins");

    let revoked = new_invite(&mut ana, &team);
    backdate(&hub, &revoked, "400 days");
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "revoke_invite", "hint": &revoked[..4] })), Ok(Value::Null), "an old code can still be revoked");
    let unknown = join_team(&mut carla, "ZZZZ-ZZZZ-ZZZZ-0000");
    let used = join_team(&mut carla, &old);
    let gone = join_team(&mut carla, &revoked);
    assert_eq!((unknown.clone(), used, gone), (Err(NO_INVITE.into()), Err(NO_INVITE.into()), Err(NO_INVITE.into())), "used and revoked answer exactly as unknown");
    assert_eq!(super::teams_membership::roster(&mut carla, &team), Err("you are not a member of that team".into()));
}

#[test]
fn a_team_has_at_most_20_open_invites_and_only_unused_unrevoked_ones_count_however_old() {
    use super::teams_membership::{db_text, join_team, make_team, new_invite, person, team_op};
    let hub = TestHub::start("teams_b_invite_cap");
    let (mut ana, _) = person(&hub.url, "ana@example.test");
    let (mut bruno, _) = person(&hub.url, "bruno@example.test");
    let (mut carla, _) = person(&hub.url, "carla@example.test");
    let team = make_team(&mut ana, "Acme");
    let open = || db_text(&hub.database, "select count(*)::text from team_invites where used_by is null and not revoked");

    // One used and one revoked code: neither counts.
    join_team(&mut bruno, &new_invite(&mut ana, &team)).unwrap();
    let revoked = new_invite(&mut ana, &team);
    team_op(&mut ana, &team, json!({ "op": "revoke_invite", "hint": &revoked[..4] })).unwrap();
    let mut codes: Vec<String> = (0..20).map(|_| new_invite(&mut ana, &team)).collect();
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "invite" })), Err(FULL.into()), "the 21st open one");
    assert_eq!(open(), vec!["20"]);
    assert_eq!(db_text(&hub.database, "select count(*)::text from team_invites"), vec!["22"]);

    // Old open codes still count: none of them lapses.
    admin_on(&hub.database, "update team_invites set created_at = now() - interval '3650 days' where used_by is null");
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "invite" })), Err(FULL.into()), "old open codes still fill the cap");

    let last = codes.pop().unwrap();
    team_op(&mut ana, &team, json!({ "op": "revoke_invite", "hint": &last[..4] })).unwrap();
    codes.push(new_invite(&mut ana, &team));
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "invite" })), Err(FULL.into()), "a revoked one made room for exactly one");
    assert_eq!(join_team(&mut carla, &codes[0]).unwrap()["id"], json!(team), "an old code joins");
    new_invite(&mut ana, &team);
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "invite" })), Err(FULL.into()), "a used one made room for exactly one");
    assert_eq!(open(), vec!["20"]);
}

#[test]
fn a_member_who_leaves_while_controlling_a_live_run_gives_it_back_to_its_bot_at_once() {
    let hub = TestHub::start_with_team_policy("teams_b_leave_run", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut dina = tb.member(&hub, "dina@example.test", &[]);
    tb.bot(&hub, design(), Some(dina.id));
    let run = tb.run(&hub);
    op(tb.team, &mut tb.owner.client, json!({ "op": "runs" })).unwrap();
    let taken = op(tb.team, &mut dina.client, json!({ "op": "take_over", "run": run.to_string() })).unwrap();
    assert_eq!((&taken["state"], &taken["controller"]), (&json!("taken_over"), &json!(dina.id.to_string())));

    assert_eq!(op(tb.team, &mut dina.client, json!({ "op": "leave" })), Ok(Value::Null));
    assert_eq!(tb.run_state(&hub, run), ("working".to_owned(), None), "the run is back with its bot");
    let id = json!(run.to_string());
    let pushed = tb.owner.client.wait("the run given back", |f| f["type"] == "team" && f["event"]["kind"] == "run_changed" && f["event"]["run"]["id"] == id && f["event"]["run"]["state"] == "working");
    assert_eq!(pushed["event"]["run"]["controller"], Value::Null);
    let runs = op(tb.team, &mut tb.owner.client, json!({ "op": "runs" })).unwrap();
    let listed = runs.as_array().unwrap().iter().find(|r| r["id"] == json!(run.to_string())).unwrap();
    assert_eq!((&listed["state"], &listed["controller"]), (&json!("working"), &Value::Null), "the runs page shows it working, with nobody in control");
    assert!(tb.told(&hub, "[dina gave the run back: continue]").contains("[dina gave the run back: continue]"));
}

#[test]
fn a_call_whose_only_asked_member_is_removed_goes_to_the_owner_at_once() {
    let hub = TestHub::start_with_team_policy("teams_b_remove_call", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let dina = tb.member(&hub, "dina@example.test", &["design"]);
    let omar = tb.member(&hub, "omar@example.test", &["ops"]);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    op(tb.team, &mut tb.owner.client, json!({ "op": "runs" })).unwrap();
    let call = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap()["call"].as_str().unwrap().to_owned();
    let asked = |tb: &mut TeamBox| {
        let listed = super::teams_calls::calls_of(tb.team, &mut tb.owner.client);
        (listed[0]["id"].clone(), listed[0]["asked"].clone(), listed[0]["escalated"].clone())
    };

    // Removing someone the call does not ask changes nothing.
    assert_eq!(op(tb.team, &mut tb.owner.client, json!({ "op": "remove_member", "account": omar.id.to_string() })), Ok(Value::Null));
    assert_eq!(asked(&mut tb), (json!(call), json!([design()]), json!(false)));

    assert_eq!(op(tb.team, &mut tb.owner.client, json!({ "op": "remove_member", "account": dina.id.to_string() })), Ok(Value::Null));
    assert_eq!(asked(&mut tb), (json!(call), json!([design(), { "kind": "owner" }]), json!(true)), "the owner is asked at once");
    let pushed = tb.owner.client.wait("the escalation pushed", |f| f["type"] == "team" && f["event"]["kind"] == "call_opened" && f["event"]["call"]["escalated"] == json!(true));
    assert_eq!(pushed["event"]["call"]["id"], json!(call));
    assert_eq!(tb.run_state(&hub, run), ("waiting".to_owned(), None));
    let answered = op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": { "kind": { "kind": "choice" }, "choice": "blue", "text": "" } })).unwrap();
    assert_eq!(answered["answer"]["choice"], json!("blue"));
}

const NO_RUN: &str = "this box has no live run of the team";
const NOT_ITS_RUN: &str = "this box's run is not of that workflow on that subject";

/// Signs in a machine of the team's own account, as a team's box is, with `token`, and reports
/// `tabs` as running sessions of (workflow, subject) pairs that the box itself claims.
fn team_machine(hub: &TestHub, team: Uuid, id: &str, token: &str, tabs: &[(&str, Value)]) -> WebSocket<MaybeTlsStream<TcpStream>> {
    admin_on(&hub.database, &format!("insert into machines (id, account, name, os, token_hash) values ('{id}', '{team}', 'box {id}', 'linux', sha256('{token}'::bytea)) on conflict do nothing"));
    let (mut ws, _) = tungstenite::connect(format!("{}/ws/machine", hub.url.replace("http://", "ws://"))).unwrap();
    if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    }
    ws.send(Message::text(json!({ "type": "auth", "token": token, "name": "box", "os": "linux", "version": "test" }).to_string())).unwrap();
    read_machine(&mut ws, |frame| frame["type"] == "hello");
    let tabs: Vec<Value> = tabs
        .iter()
        .enumerate()
        .map(|(n, (workflow, subject))| {
            let mut tab = json!({ "id": format!("tab-{n}"), "cwd": "/w", "bypass": false, "running": true, "status": null, "title": null, "message": null,
                "resume_at": null, "connector": workflow, "opened": 1 });
            tab["subject"] = subject.clone();
            tab
        })
        .collect();
    ws.send(Message::text(json!({ "type": "event", "event": { "kind": "tabs", "tabs": tabs } }).to_string())).unwrap();
    let account = hub.runtime.block_on(hub.hub().account(team)).unwrap();
    wait_for("the machine's tabs at the hub", || account.tabs_of(id).is_some_and(|t| t.len() == tabs.len()));
    ws
}

/// A hub call the machine socket sends, and the hub's answer.
fn machine_call(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, id: u64, call: Value) -> Result<Value, String> {
    ws.send(Message::text(json!({ "type": "call", "id": id, "call": call }).to_string())).unwrap();
    let reply = read_machine(ws, |frame| frame["type"] == "reply" && frame["id"] == id);
    match reply.get("err") {
        Some(err) => Err(err.as_str().unwrap().to_owned()),
        None => Ok(reply["ok"].clone()),
    }
}

/// Every per-run callback a session sends, for `workflow` on Slack thread `C0B00001`/`1.2` or
/// on Linear issue `issue`.
fn callbacks(workflow: &str, issue: &str) -> Vec<Value> {
    vec![
        json!({ "cmd": "stage", "connector": workflow, "issue": issue, "stage": "coding" }),
        json!({ "cmd": "run_set", "connector": workflow, "issue": issue, "name": "title", "value": "hijacked" }),
        json!({ "cmd": "slack_reply", "connector": workflow, "channel": "C0B00001", "thread": "1.2", "text": "from the wrong box" }),
        json!({ "cmd": "linear_create", "connector": workflow, "channel": "C0B00001", "thread": "1.2", "title": "Sneak", "description": "" }),
        json!({ "cmd": "linear_take", "connector": workflow, "channel": "C0B00001", "thread": "1.2", "issue": "ENG-1" }),
    ]
}

/// A team box (bot B, with its run bound to its own box) whose team has a Slack bot on the
/// stand-in Slack, and machine A of the same team with a live run of its own workflow `wf-a` on
/// issue `team-2`.
fn two_boxes(name: &str, slack_token: &str) -> (TestHub, TeamBox, Uuid) {
    super::teams_slack_notice::slack_standin();
    let hub = TestHub::start_with_team_policy(name, fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let team = tb.team.to_string();
    tb.owner.client.hub(json!({ "cmd": "team", "team": team, "op": { "op": "hub", "call": { "cmd": "connect_slack", "token": slack_token } } })).unwrap();
    tb.bot(&hub, design(), None);
    let run_b = tb.run(&hub);
    sql(&hub, &format!(
        "update team_runs set subject = '{{\"issue\":\"team-1\"}}'::jsonb where id = '{run_b}';
         insert into team_runs (id, team, workflow, box, started_by, state, subject)
           values ('{}', '{team}', 'wf-a', '{BOX_A}', '\"{}\"'::jsonb, 'working', '{{\"issue\":\"team-2\"}}'::jsonb)",
        Uuid::new_v4(),
        tb.owner.id
    ));
    (hub, tb, run_b)
}

const BOX_A: &str = "6f1c0b1e-0000-4000-8000-0000000000aa";
const BOX_A_TOKEN: &str = "team-box-a-machine-token";

/// The team account's stored workflow runs, with their variables.
fn stored_runs(hub: &TestHub, team: Uuid) -> String {
    rows(hub, &format!("select coalesce(store->'runs', 'null'::jsonb)::text from accounts where id = '{team}'"))[0][0].clone().unwrap()
}

#[test]
fn a_team_box_cannot_act_for_another_bots_run_by_reporting_its_tab() {
    let (hub, tb, _) = two_boxes("teams_b_box_vouch", "xoxb-dev-vouch-one");
    let thread = json!({ "channel": "C0B00001", "thread": "1.2" });
    let mut a = team_machine(&hub, tb.team, BOX_A, BOX_A_TOKEN, &[("wf-a", json!({ "issue": "team-2" })), (&tb.workflow, json!({ "issue": "team-1" })), (&tb.workflow, thread)]);
    let before = stored_runs(&hub, tb.team);

    let set = json!({ "cmd": "run_set", "connector": tb.workflow, "issue": "team-1", "name": "title", "value": "hijacked" });
    let reply = json!({ "cmd": "slack_reply", "connector": tb.workflow, "channel": "C0B00001", "thread": "1.2", "text": "from the wrong box" });
    let (set, reply) = (machine_call(&mut a, 1, set), machine_call(&mut a, 2, reply));
    std::thread::sleep(Duration::from_millis(500));
    let posted = super::teams_slack_notice::posts("xoxb-dev-vouch-one").len();
    assert_eq!((set, reply, posted), (Err(NOT_ITS_RUN.into()), Err(NOT_ITS_RUN.into()), 0), "RunSet for bot B's run, SlackReply as bot B, and Slack posts");
    for (n, call) in callbacks(&tb.workflow, "team-1").into_iter().enumerate() {
        assert_eq!(machine_call(&mut a, 10 + n as u64, call.clone()), Err(NOT_ITS_RUN.into()), "{call}");
    }
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(super::teams_slack_notice::posts("xoxb-dev-vouch-one").len(), 0, "nothing was posted to Slack");
    assert_eq!(stored_runs(&hub, tb.team), before, "bot B's run variables are untouched");
    assert!(!stored_runs(&hub, tb.team).contains("hijacked"));

    // Its own bound run's callback passes the box check and reaches the workflow's own answer.
    let own = json!({ "cmd": "run_set", "connector": "wf-a", "issue": "team-2", "name": "title", "value": "mine" });
    assert_eq!(machine_call(&mut a, 30, own), Err("this session was not started by a workflow run, so it has no run variables to set".into()));
}

#[test]
fn a_second_socket_of_the_same_box_is_held_to_the_run_bound_to_that_machine() {
    let (hub, tb, _) = two_boxes("teams_b_box_mirror", "xoxb-dev-vouch-two");
    let first = team_machine(&hub, tb.team, BOX_A, BOX_A_TOKEN, &[("wf-a", json!({ "issue": "team-2" }))]);
    // The same machine's second socket claims a tab of bot B's workflow it does not have.
    let mut second = team_machine(&hub, tb.team, BOX_A, BOX_A_TOKEN, &[("wf-a", json!({ "issue": "team-2" })), (&tb.workflow, json!({ "issue": "team-1" })), ("wf-a", json!({ "issue": "team-9" }))]);
    drop(first);
    let before = stored_runs(&hub, tb.team);
    for (n, call) in callbacks(&tb.workflow, "team-1").into_iter().enumerate() {
        assert_eq!(machine_call(&mut second, 1 + n as u64, call.clone()), Err(NOT_ITS_RUN.into()), "{call}");
    }
    let other_subject = json!({ "cmd": "run_set", "connector": "wf-a", "issue": "team-9", "name": "title", "value": "hijacked" });
    assert_eq!(machine_call(&mut second, 20, other_subject), Err(NOT_ITS_RUN.into()), "its own workflow on a subject its run is not on");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(super::teams_slack_notice::posts("xoxb-dev-vouch-two").len(), 0);
    assert_eq!(stored_runs(&hub, tb.team), before);
}

#[test]
fn a_team_machine_with_no_live_run_bound_to_it_gets_every_session_callback_refused() {
    let (hub, tb, _) = two_boxes("teams_b_box_unbound", "xoxb-dev-vouch-three");
    let idle = "6f1c0b1e-0000-4000-8000-0000000000cc";
    let mut c = team_machine(&hub, tb.team, idle, "team-box-c-machine-token", &[(&tb.workflow, json!({ "issue": "team-1" })), (&tb.workflow, json!({ "channel": "C0B00001", "thread": "1.2" }))]);
    let before = stored_runs(&hub, tb.team);
    for (n, call) in callbacks(&tb.workflow, "team-1").into_iter().chain(callbacks("wf-a", "team-2")).enumerate() {
        assert_eq!(machine_call(&mut c, 1 + n as u64, call.clone()), Err(NO_RUN.into()), "{call}");
    }
    // A run that ended no longer binds its box either.
    sql(&hub, &format!("update team_runs set box = '{idle}', state = 'stopped' where workflow = 'wf-a'"));
    assert_eq!(machine_call(&mut c, 40, callbacks("wf-a", "team-2")[1].clone()), Err(NO_RUN.into()));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(super::teams_slack_notice::posts("xoxb-dev-vouch-three").len(), 0);
    assert_eq!(stored_runs(&hub, tb.team), before);
}
