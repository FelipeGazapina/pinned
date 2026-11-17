//! Teams: a member reaches a team session only through `TeamOp::Session`, gated by
//! `session_rule`. Every member reads it; only its driver drives it, and only while the owner has
//! turned team sessions on for that member. The session and its box are seeded here: a row in
//! `team_runs` and a scripted machine of the team's account that logs every call it gets.

use super::teams_calls::{op, patient, person, sql};
use super::*;
use uuid::Uuid;

const TAB: &str = "tab-session";
const NOT_A_MEMBER: &str = "you are not a member of that team";
const NO_SUCH_SESSION: &str = "the team has no such session";

fn session(team: Uuid, client: &mut Client, run: &str, call: Value) -> Result<Value, String> {
    op(team, client, json!({ "op": "session", "run": run, "call": call }))
}

fn access(team: Uuid, client: &mut Client, member: Uuid, on: bool) -> Result<Value, String> {
    op(team, client, json!({ "op": "set_session_access", "member": member.to_string(), "on": on }))
}

#[test]
fn a_member_reads_a_team_session_and_only_its_driver_with_access_drives_it() {
    let hub = TestHub::start_with_team_policy("teams_sessions_gate", fast(), patient());
    let mut olga = person(&hub, "olga@example.test");
    let made = olga.client.hub(json!({ "cmd": "team", "op": { "op": "create", "name": "Acme" } })).unwrap();
    let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
    let (mut bea, mut caio, mut sam) = (person(&hub, "bea@example.test"), person(&hub, "caio@example.test"), person(&hub, "sam@example.test"));
    for member in [bea.id, caio.id] {
        sql(&hub, &format!("insert into team_members (team, member, role) values ('{team}', '{member}', 'member')"));
    }

    // Only the owner turns team sessions on for a member, and each member starts with them off.
    assert_eq!(access(team, &mut bea.client, caio.id, true), Err("only the team's owner does this".into()));
    let members = access(team, &mut olga.client, caio.id, true).unwrap();
    let on = |id: Uuid| members.as_array().unwrap().iter().find(|m| m["account"] == id.to_string()).unwrap()["session_access"] == json!(true);
    assert_eq!((on(caio.id), on(bea.id)), (true, false));

    // The team's box: a machine of the team's account that answers every call with its name.
    let (machine, token) = hub.runtime.block_on(hub.hub().db.create_machine(team, "box", "linux")).unwrap();
    let raw = RawMachine::start(&hub.url, &token, |call| match call["cmd"].as_str().unwrap() {
        "tabs" => RawReply::Ok(json!([{ "id": "tab-other" }, { "id": TAB }])),
        cmd => RawReply::Ok(json!({ "answered": cmd })),
    });
    wait_for("the team's box online", || hub.runtime.block_on(hub.hub().account(team)).unwrap().online().iter().any(|(m, _)| *m == machine.id));
    let (run, bot_run) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
    sql(&hub, &format!(
        "insert into team_runs (id, team, workflow, box, started_by, state, kind, project, tab)
           values ('{run}', '{team}', 'session', '{0}', to_jsonb('{1}'::text), 'working', 'session', 'github.com/acme/site', '{TAB}');
         insert into team_runs (id, team, workflow, box, started_by, state) values ('{bot_run}', '{team}', 'w1', '{0}', to_jsonb('{1}'::text), 'working')",
        machine.id, caio.id
    ));
    let thread = json!({ "cmd": "thread", "tab": TAB, "from": 0 });
    let prompt = json!({ "cmd": "prompt", "tab": TAB, "text": "Fix the footer" });

    // Its driver with access reads and drives it; another member only reads it.
    assert_eq!(session(team, &mut caio.client, &run, thread.clone()), Ok(json!({ "answered": "thread" })));
    assert_eq!(session(team, &mut caio.client, &run, prompt.clone()), Ok(json!({ "answered": "prompt" })));
    assert_eq!(session(team, &mut bea.client, &run, thread.clone()), Ok(json!({ "answered": "thread" })));
    assert_eq!(session(team, &mut bea.client, &run, prompt.clone()), Err("only the member driving this session does prompt in it".into()));

    // A person's tabs, terminal and accounts never run in a team session, its driver's call neither.
    assert_eq!(session(team, &mut caio.client, &run, json!({ "cmd": "open_tab", "path": null, "bypass": false })), Err("open_tab does not run in a team session".into()));
    assert_eq!(session(team, &mut caio.client, &run, json!({ "cmd": "write_input", "tab": TAB, "data": "ls\n" })), Err("write_input does not run in a team session".into()));
    assert_eq!(
        session(team, &mut caio.client, &run, json!({ "cmd": "switch_account", "tab": TAB, "account": "claude", "cols": 80, "rows": 24 })),
        Err("switch_account does not run in a team session".into())
    );
    // A call names the session's own tab; the hub puts in no other.
    assert_eq!(session(team, &mut caio.client, &run, json!({ "cmd": "thread", "tab": "tab-other", "from": 0 })), Err("that call names another tab than this session's".into()));
    // A bot run is no session, and someone who is not a member is refused before anything else.
    assert_eq!(session(team, &mut caio.client, &bot_run, thread.clone()), Err(NO_SUCH_SESSION.into()));
    assert_eq!(session(team, &mut caio.client, &Uuid::new_v4().to_string(), thread.clone()), Err(NO_SUCH_SESSION.into()));
    assert_eq!(session(team, &mut sam.client, &run, thread.clone()), Err(NOT_A_MEMBER.into()));
    // The tab list answers the session's tab only.
    assert_eq!(session(team, &mut caio.client, &run, json!({ "cmd": "tabs" })), Ok(json!([{ "id": TAB }])));

    // Once the owner turns its access off, the driver only reads its own session.
    access(team, &mut olga.client, caio.id, false).unwrap();
    assert_eq!(session(team, &mut caio.client, &run, prompt.clone()), Err("the team's owner has not turned on team sessions for you".into()));
    assert_eq!(session(team, &mut caio.client, &run, thread.clone()), Ok(json!({ "answered": "thread" })));

    // The box got exactly the calls let through, in order, and nothing else.
    let got = raw.calls.lock().unwrap().clone();
    assert_eq!(got, vec![thread.clone(), prompt, thread.clone(), json!({ "cmd": "tabs" }), thread]);
}
