//! Teams, hardening B: a team's events reach only the app connections that asked about teams,
//! a team op runs only on the app's own path, invite codes never expire, a member who leaves
//! hands back what it held, and a team's box acts only for the run bound to it.

use super::teams_calls::{design, op, patient, sql, TeamBox};
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
