//! A box is a machine of the account that opened it, and may only pull the account's files: the
//! hub refuses every push, executable mark and share a box machine sends, while a person's other
//! machine of the same account syncs as before. Literal values throughout.

use super::*;

const REFUSED: &str = "the account's synced projects would pass 0 MB from a box: a box only pulls the account's files";
const NOT_SHARED: &str = "a box may not change the account's shared projects";
const NOT_WORKFLOWS: &str = "a box may not change the account's workflows";
const NOT_SETTINGS: &str = "a box may not change the account's settings";
const NOT_SESSIONS: &str = "a box may not change the account's sessions on its other machines";
const NOT_MACHINES: &str = "a box may not change the account's machines";
const NOTICE: &str = "Sync stopped: your synced projects would pass 1024 MB. Nothing was lost; this machine keeps everything.";
const PROJECT: &str = "github.com/acme/site";

/// A hub with one account, a box machine `vps` (its row as the hub writes one for a box) and a
/// person's machine `mac`: the hub, the box's machine id and token, the mac's id and token.
struct Setup {
    hub: TestHub,
    account: String,
    token: String,
    box_id: String,
    box_token: String,
    mac_id: String,
    mac_token: String,
}

fn setup(name: &str) -> Setup {
    let hub = TestHub::start(name);
    let (token, account) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    let (vps, box_token) = account::register_machine(&hub.url, &token, "vps", "linux").unwrap();
    let (mac, mac_token) = account::register_machine(&hub.url, &token, "mac", "macos").unwrap();
    add_box_row(&hub, &account.id, &vps.id);
    Setup { hub, account: account.id, token, box_id: vps.id, box_token, mac_id: mac.id, mac_token }
}

fn add_box_row(hub: &TestHub, account: &str, machine: &str) {
    admin_on(&hub.database, &format!(
        "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase) values ('box-1', '{account}', '{machine}', 'box_123', 'c1', '{{\"issue\":\"i1\"}}', '{{\"folder\":\"/w\",\"args\":[]}}', 'small', 'running')",
    ));
}

fn count(hub: &TestHub, sql: &str) -> i64 {
    hub.runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        client.query_one(sql, &[]).await.unwrap().get::<_, i64>(0)
    })
}

fn push(project: &str, path: &str, body: &[u8]) -> Value {
    let packed = gzip_bytes(body);
    json!({
        "cmd": "sync_push",
        "project": project,
        "files": [{
            "path": path,
            "sha": sha256_hex(body),
            "modified": 1_700_000_000_000u64,
            "size": packed.len(),
            "executable": false,
            "data": base64::engine::general_purpose::STANDARD.encode(&packed),
        }],
    })
}

fn skill_push(body: &[u8]) -> Value {
    push(sync::PLUGINS_KEY, "user-skills/ship/SKILL.md", body)
}

fn mark() -> Value {
    json!({ "cmd": "sync_mark_executable", "project": sync::PLUGINS_KEY, "repairs": [{ "path": "user-skills/ship/SKILL.md", "sha": sha256_hex(b"from the box") }] })
}

fn share() -> Value {
    json!({ "cmd": "share_project", "project": { "key": PROJECT, "name": "site", "repo": "git@github.com:acme/site.git" } })
}

fn listed(hub: &str, token: &str, project: &str) -> Vec<String> {
    let list = call_machine(hub, token, json!({ "cmd": "sync_list", "project": project }));
    list.as_array().unwrap().iter().map(|entry| entry["path"].as_str().unwrap().to_owned()).collect()
}

/// A hub call over a machine socket that stays open, the way a node sends its calls.
fn send_call(node: &mut MachineSock, id: u64, call: Value) -> Result<Value, String> {
    node.send(json!({ "type": "call", "id": id, "call": call }));
    let start = Instant::now();
    loop {
        if let Some(frame) = node.try_text() {
            if frame["type"] == "reply" && frame["id"] == id {
                return match frame.get("err") {
                    Some(err) => Err(err.as_str().unwrap().to_owned()),
                    None => Ok(frame["ok"].clone()),
                };
            }
        }
        assert!(start.elapsed() < WAIT, "timed out waiting for reply {id}");
    }
}

#[test]
fn a_box_machine_may_only_pull_the_accounts_files() {
    let s = setup("boxgate_pull_only");
    let url = &s.hub.url;
    let refused = Err(REFUSED.to_owned());
    assert_eq!(machine_result(url, &s.box_token, skill_push(b"from the box")), refused, "a skill or plugin file");
    assert_eq!(machine_result(url, &s.box_token, push(PROJECT, "claude/memory/notes.md", b"from the box")), refused, "a project's file");
    assert_eq!(machine_result(url, &s.box_token, mark()), refused, "an executable mark");
    assert_eq!(machine_result(url, &s.box_token, share()), Err(NOT_SHARED.to_owned()), "a project share");
    assert_eq!(machine_result(url, &s.box_token, json!({ "cmd": "sync_list", "project": sync::PLUGINS_KEY })), Ok(json!([])));
    assert_eq!(machine_result(url, &s.box_token, json!({ "cmd": "sync_pull", "project": sync::PLUGINS_KEY, "paths": [] })), Ok(json!([])));

    // A person's machine of the same account syncs as before, and the box pulls what it sent.
    assert_eq!(machine_result(url, &s.mac_token, skill_push(b"from the mac")), Ok(Value::Null));
    assert_eq!(listed(url, &s.mac_token, sync::PLUGINS_KEY), vec!["user-skills/ship/SKILL.md".to_owned()]);
    let pulled = machine_result(url, &s.box_token, json!({ "cmd": "sync_pull", "project": sync::PLUGINS_KEY, "paths": ["user-skills/ship/SKILL.md"] })).unwrap();
    assert_eq!(pulled[0]["sha"], json!(sha256_hex(b"from the mac")));

    // Nothing the box sent is in the account's set.
    assert_eq!(listed(url, &s.mac_token, PROJECT), Vec::<String>::new());
    assert_eq!(count(&s.hub, &format!("select count(*) from shared_projects where account = '{}'", s.account)), 0);
    assert_eq!(count(&s.hub, &format!("select count(*) from project_files where account = '{}' and sha = '{}'", s.account, sha256_hex(b"from the box"))), 0);
    assert_eq!(machine_result(url, &s.mac_token, share()), Ok(Value::Null), "a person's machine shares as before");
}

#[test]
fn a_box_whose_row_goes_while_it_is_connected_stays_a_box_until_it_reconnects() {
    let s = setup("boxgate_row_gone");
    let url = &s.hub.url;
    let mut node = MachineSock::connect(url, &s.box_token, "0.18.6");
    assert_eq!(send_call(&mut node, 1, skill_push(b"from the box")), Err(REFUSED.to_owned()));
    admin_on(&s.hub.database, &format!("delete from boxes where machine = '{}'", s.box_id));
    assert_eq!(send_call(&mut node, 2, skill_push(b"from the box")), Err(REFUSED.to_owned()), "the connection it opened as a box");
    assert_eq!(send_call(&mut node, 3, share()), Err(NOT_SHARED.to_owned()));
    assert_eq!(listed(url, &s.mac_token, sync::PLUGINS_KEY), Vec::<String>::new());
    drop(node);
    // Its next connection reads the boxes table again: with no row it is no box.
    assert_eq!(machine_result(url, &s.box_token, skill_push(b"no box now")), Ok(Value::Null));
}

/// Asks `machine` through the hub for its sync limit notice, answering for `node` as a node from
/// 0.18.5 on does once the hub refused its push.
fn sync_limit_shown(client: &mut Client, node: &mut MachineSock, machine: &str, id: u64) -> Result<Value, String> {
    client.send(json!({ "type": "call", "id": id, "machine": machine, "call": { "cmd": "sync_limit" } }));
    let start = Instant::now();
    loop {
        client.pump();
        if let Some(reply) = client.frames.iter().find(|f| f["type"] == "reply" && f["id"] == id) {
            return match reply.get("err") {
                Some(err) => Err(err.as_str().unwrap().to_owned()),
                None => Ok(reply["ok"].clone()),
            };
        }
        if let Some(frame) = node.try_text() {
            if frame["type"] == "call" && frame["call"]["cmd"] == "sync_limit" {
                node.send(json!({ "type": "reply", "id": frame["id"], "ok": NOTICE }));
            }
        }
        assert!(start.elapsed() < WAIT, "timed out waiting for the sync limit of {machine}");
    }
}

#[test]
fn a_box_machines_refused_push_shows_no_limit_notice_to_the_accounts_apps() {
    let s = setup("boxgate_notice");
    let url = &s.hub.url;
    let mut client = Client::connect(url, &s.token);
    client.wait("hello", |f| f["type"] == "hello");
    let mut vps = MachineSock::connect(url, &s.box_token, "0.18.6");
    let mut mac = MachineSock::connect(url, &s.mac_token, "0.18.6");
    client.wait_machines(&[("vps", true), ("mac", true)]);
    assert_eq!(send_call(&mut vps, 1, skill_push(b"from the box")), Err(REFUSED.to_owned()));

    assert_eq!(sync_limit_shown(&mut client, &mut vps, &s.box_id, 901), Ok(Value::Null), "the hub answers for a box");
    assert_eq!(sync_limit_shown(&mut client, &mut mac, &s.mac_id, 902), Ok(json!(NOTICE)), "a person's machine at the limit still says so");
}

#[test]
fn a_box_sessions_act_may_not_share_a_project_either() {
    let s = setup("boxgate_act");
    let url = &s.hub.url;
    let mut client = Client::connect(url, &s.token);
    client.wait("hello", |f| f["type"] == "hello");
    let tab = json!({ "id": "tab-1", "cwd": "/w/site", "bypass": false, "running": true, "status": "done", "title": "Fix login", "message": null, "resume_at": null, "opened": 1, "session": "conv-1", "role": "worker" });
    let mut vps = MachineSock::connect(url, &s.box_token, "0.18.6");
    let mut mac = MachineSock::connect(url, &s.mac_token, "0.18.6");
    for node in [&mut vps, &mut mac] {
        node.send(json!({ "type": "event", "event": { "kind": "tabs", "tabs": [tab.clone()] } }));
    }
    client.wait("the box's tabs", |f| f["type"] == "event" && f["machine"] == json!(s.box_id) && f.to_string().contains("conv-1"));
    client.wait("the mac's tabs", |f| f["type"] == "event" && f["machine"] == json!(s.mac_id) && f.to_string().contains("conv-1"));
    let act = json!({ "cmd": "act", "act": { "tab": "tab-1", "action": "share_project", "args": { "project": share()["project"] } } });

    assert_eq!(send_call(&mut vps, 1, act.clone()), Err(NOT_SHARED.to_owned()));
    assert_eq!(count(&s.hub, &format!("select count(*) from shared_projects where account = '{}'", s.account)), 0);
    assert_eq!(send_call(&mut mac, 1, act).unwrap()["outcome"], "done", "a person's machine's session shares as before");
}

/// A tab as a node reports it, with `extra` fields on top.
fn tab(id: &str, role: &str, extra: Value) -> Value {
    let mut tab = json!({ "id": id, "cwd": "/w/site", "bypass": false, "running": true, "status": "done", "title": "Fix login", "message": null, "resume_at": null, "opened": 1, "session": format!("conv-{id}"), "role": role });
    tab.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    tab
}

/// A machine socket that reported `tabs`, once the hub holds them.
fn reporting(s: &Setup, token: &str, machine: &str, tabs: Vec<Value>) -> MachineSock {
    let mut node = MachineSock::connect(&s.hub.url, token, "0.18.6");
    let reported = tabs.len();
    node.send(json!({ "type": "event", "event": { "kind": "tabs", "tabs": tabs } }));
    let account = s.hub.runtime.block_on(s.hub.hub().account(uuid::Uuid::parse_str(&s.account).unwrap())).unwrap();
    wait_for("the hub to hold the machine's tabs", || account.tabs_of(machine).is_some_and(|tabs| tabs.len() == reported));
    node
}

fn act(tab: &str, action: &str, args: Value, request_id: &str) -> Value {
    json!({ "cmd": "act", "act": { "tab": tab, "action": action, "args": args, "request_id": request_id } })
}

fn sql_quoted(text: &str) -> String {
    text.replace('\'', "''")
}

#[test]
fn a_box_still_asks_to_extend_itself_and_only_the_owner_decides() {
    let s = setup("boxgate_extend_ask");
    let mut vps = reporting(&s, &s.box_token, &s.box_id, vec![tab("tab-1", "worker", json!({}))]);
    let extend = |machine: &str, request_id: &str| act("tab-1", "extend_box", json!({ "machine": machine, "extra_usd": 1.0 }), request_id);
    let waiting = |machine: &str| count(&s.hub, &format!("select count(*) from agent_confirmations where account = '{}' and action = 'extend_box' and decided_at is null and args->>'machine' = '{machine}'", s.account));

    let asked = send_call(&mut vps, 1, extend(&s.box_id, "x-1")).unwrap();
    assert_eq!(asked["outcome"], "pending", "the ask waits for the owner: {asked}");
    assert_eq!(waiting(&s.box_id), 1);
    assert_eq!(send_call(&mut vps, 2, extend(&s.mac_id, "x-2")), Err(NOT_MACHINES.to_owned()), "never for another machine");
    assert_eq!(waiting(&s.mac_id), 0);
    assert_eq!(
        send_call(&mut vps, 3, json!({ "cmd": "decide_confirmation", "id": asked["id"], "approve": true })),
        Err("only the account's owner decides, in the app".to_owned())
    );
    assert_eq!(send_call(&mut vps, 4, json!({ "cmd": "extend_box", "machine": s.box_id, "extra_usd": 1.0 })), Err(NOT_MACHINES.to_owned()), "its own call extends nothing");
    assert_eq!(waiting(&s.box_id), 1);
}

#[test]
fn a_refused_sync_push_reads_as_the_size_limit_and_every_other_refusal_says_what_a_box_may_not_change() {
    let s = setup("boxgate_texts");
    let url = &s.hub.url;
    assert!(REFUSED.starts_with(sync::LIMIT_REFUSAL));
    assert_eq!(machine_result(url, &s.box_token, skill_push(b"from the box")), Err(REFUSED.to_owned()));
    assert_eq!(machine_result(url, &s.box_token, mark()), Err(REFUSED.to_owned()));

    let calls = [
        (share(), NOT_SHARED),
        (json!({ "cmd": "delete_connector", "id": "c1" }), NOT_WORKFLOWS),
        (json!({ "cmd": "run_workflow", "id": "c1" }), NOT_WORKFLOWS),
        (json!({ "cmd": "connect_linear", "key": "lin_api_box" }), NOT_SETTINGS),
        (json!({ "cmd": "disconnect_linear" }), NOT_SETTINGS),
    ];
    for (call, plain) in calls {
        let refused = machine_result(url, &s.box_token, call.clone()).unwrap_err();
        assert_eq!(refused, plain, "{call}");
        assert!(!refused.starts_with(sync::LIMIT_REFUSAL), "{call}");
    }

    let mut vps = reporting(&s, &s.box_token, &s.box_id, vec![tab("tab-1", "worker", json!({}))]);
    let acts = [
        (act("tab-1", "share_project", json!({ "project": share()["project"] }), "a-1"), NOT_SHARED),
        (act("tab-1", "delete_connector", json!({ "id": "c1" }), "a-2"), NOT_WORKFLOWS),
        (act("tab-1", "disconnect_linear", json!({}), "a-3"), NOT_SETTINGS),
        (act("tab-1", "rename_machine", json!({ "id": s.mac_id, "name": "renamed" }), "a-4"), NOT_MACHINES),
    ];
    for (id, (call, plain)) in acts.into_iter().enumerate() {
        let refused = send_call(&mut vps, id as u64 + 1, call.clone()).unwrap_err();
        assert_eq!(refused, plain, "{call}");
        assert!(!refused.starts_with(sync::LIMIT_REFUSAL), "{call}");
    }
    assert_eq!(count(&s.hub, &format!("select count(*) from agent_confirmations where account = '{}'", s.account)), 0, "no refused act waits for the owner");
}

#[test]
fn a_capitao_on_a_box_reports_to_the_orchestrator_and_hands_workers_only_to_boxes() {
    let s = setup("boxgate_captain");
    let url = &s.hub.url;
    let mut client = Client::connect(url, &s.token);
    client.wait("hello", |f| f["type"] == "hello");
    let host = RawMachine::start(url, &s.mac_token, |call| match call["cmd"].as_str() {
        Some("global_orchestrator_settings") => RawReply::Ok(json!({ "host": true })),
        Some("orchestrator_report") => RawReply::Ok(json!({ "outcome": "iniciado", "tab": "tab-g" })),
        _ => RawReply::Err("not in this test".into()),
    });
    let mut vps = reporting(&s, &s.box_token, &s.box_id, vec![tab("tab-c", "orchestrator", json!({}))]);
    client.wait_machines(&[("vps", true), ("mac", true)]);

    let report = json!({ "cmd": "relay", "request_id": "rep-1", "to": { "kind": "orchestrator", "report": {
        "machine": "", "machine_name": "vps", "project": "site", "path": "/w/site", "tab": "tab-c", "text": "Done; should I open the pull request?", "answers": [] } } });
    assert_eq!(send_call(&mut vps, 1, report), Ok(json!({ "outcome": "iniciado", "tab": "tab-g" })), "its report and its question reach the orchestrator");
    assert_eq!(host.calls("orchestrator_report"), 1);
    let to_captain = json!({ "cmd": "relay", "request_id": "r-2", "to": { "kind": "captain", "machine": s.mac_id, "path": "/w/site", "text": "Push it.", "sender": "Captain" } });
    assert_eq!(send_call(&mut vps, 2, to_captain), Err(NOT_SESSIONS.to_owned()));

    let delegate = |target: Value| {
        let request = serde_json::to_value(pinned_core::delegate::parse_request(&json!({ "task": "Run the suite.", "target": target })).unwrap()).unwrap();
        json!({ "cmd": "delegate", "tab": "tab-c", "task": "t1", "request": request, "access": "auto" })
    };
    assert_eq!(
        send_call(&mut vps, 3, delegate(json!({ "box": true }))),
        Err("Upstash Box is not connected: connect it in pinned's Connectors settings".to_owned()),
        "a worker for a box goes on to the account's box connection"
    );
    assert_eq!(send_call(&mut vps, 4, delegate(json!({ "machine": "mac" }))), Err(NOT_SESSIONS.to_owned()));
    assert_eq!(host.calls("delegate") + host.calls("start_claude"), 0);
}

#[test]
fn a_box_run_still_replies_and_files_issues_under_the_run_vouch() {
    let s = setup("boxgate_run_acts");
    let run = tab("tab-r", "worker", json!({ "connector": "c1", "subject": { "channel": "C1", "thread": "1.2" } }));
    let mut vps = reporting(&s, &s.box_token, &s.box_id, vec![run, tab("tab-o", "worker", json!({}))]);
    // Past the gate each goes on to the account's workflow and connections, which this account lacks.
    let acts = [
        ("slack_reply", json!({ "connector": "c1", "channel": "C1", "thread": "1.2", "text": "On it." }), "the workflow was deleted"),
        ("linear_create", json!({ "connector": "c1", "channel": "C1", "thread": "1.2", "title": "Fix login", "description": "From Slack." }), "the workflow was deleted"),
        ("linear_take", json!({ "connector": "c1", "channel": "C1", "thread": "1.2", "issue": "ENG-7" }), "Linear is not connected: connect it in pinned's Connectors settings"),
    ];
    for (id, (action, args, past_the_gate)) in acts.into_iter().enumerate() {
        let id = id as u64 * 2 + 1;
        let ran = send_call(&mut vps, id, act("tab-r", action, args.clone(), &format!("r-{id}")));
        assert_eq!(ran, Err(past_the_gate.to_owned()), "{action} from the run's own session");
        let other = send_call(&mut vps, id + 1, act("tab-o", action, args, &format!("r-{}", id + 1)));
        assert_eq!(other, Err("only the session that runs this workflow can do that".to_owned()), "{action} from another session");
    }
}

#[test]
fn a_box_act_still_in_flight_when_its_row_and_connection_go_is_still_refused() {
    let s = setup("boxgate_act_in_flight");
    let url = &s.hub.url;
    let mut client = Client::connect(url, &s.token);
    client.wait("hello", |f| f["type"] == "hello");
    let mut vps = reporting(&s, &s.box_token, &s.box_id, vec![tab("tab-1", "worker", json!({}))]);
    client.wait_machines(&[("vps", true), ("mac", false)]);

    // The audit table is held, so the act waits after the hub read its session and before it is admitted.
    let lock = s.hub.runtime.block_on(async {
        let (lock, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), s.hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        lock.batch_execute("begin; lock table agent_actions in access exclusive mode").await.unwrap();
        lock
    });
    admin_on(&s.hub.database, &format!("delete from boxes where machine = '{}'", s.box_id));
    vps.send(json!({ "type": "call", "id": 1, "call": act("tab-1", "share_project", json!({ "project": share()["project"] }), "in-flight") }));
    wait_for("the act to wait on the audit table", || {
        count(&s.hub, "select count(*) from pg_stat_activity where wait_event_type = 'Lock' and query like 'select outcome%agent_actions%'") == 1
    });
    drop(vps);
    // Read from the hub's own memory: its clients hear of it only once the table is free.
    let account = s.hub.runtime.block_on(s.hub.hub().account(uuid::Uuid::parse_str(&s.account).unwrap())).unwrap();
    wait_for("the box to be offline", || account.online().iter().all(|(id, _)| *id != s.box_id));
    s.hub.runtime.block_on(lock.batch_execute("commit")).unwrap();

    wait_for("the act's audit row", || count(&s.hub, "select count(*) from agent_actions where request_id = 'in-flight'") == 1);
    assert_eq!(count(&s.hub, &format!("select count(*) from shared_projects where account = '{}'", s.account)), 0, "the act is held to the connection it came on");
    let refused = format!("select count(*) from agent_actions where request_id = 'in-flight' and outcome = 'refused' and detail = '{}'", sql_quoted(NOT_SHARED));
    assert_eq!(count(&s.hub, &refused), 1);
}

/// A box's act that the gate refuses never joins the account's line of extends: while a person's
/// machine's ask waits there, held behind another hub's lock of the account, the box's ask to
/// extend another box is refused at once and opens no turn connection of its own.
#[test]
fn a_refused_box_act_takes_no_place_in_the_accounts_line_of_extends() {
    let s = setup("boxgate_refused_no_line");
    let (other, _) = account::register_machine(&s.hub.url, &s.token, "vps-2", "linux").unwrap();
    admin_on(&s.hub.database, &format!(
        "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase) values ('box-2', '{}', '{}', 'box_456', 'c1', '{{\"issue\":\"i2\"}}', '{{\"folder\":\"/w\",\"args\":[]}}', 'small', 'running')",
        s.account, other.id,
    ));
    let mut vps = reporting(&s, &s.box_token, &s.box_id, vec![tab("tab-1", "worker", json!({}))]);
    let _mac = reporting(&s, &s.mac_token, &s.mac_id, vec![tab("tab-1", "worker", json!({}))]);
    let id = uuid::Uuid::parse_str(&s.account).unwrap();
    let waiting = || count(&s.hub, &format!("select count(*) from agent_confirmations where account = '{}' and action = 'extend_box' and decided_at is null", s.account));
    let lock = ExtendLockHolder::take_account(&s.hub.database, id);

    let account = s.hub.runtime.block_on(s.hub.hub().account(id)).unwrap();
    let (running, origin) = (s.hub.hub().clone(), s.mac_id.clone());
    let ask = serde_json::from_value(json!({ "tab": "tab-1", "action": "extend_box", "args": { "machine": s.box_id, "extra_usd": 1.0 }, "request_id": "m-1" })).unwrap();
    let in_line = s.hub.runtime.spawn(async move { pinned_server::surface::act(&running, &account, &origin, false, ask).await });
    wait_for("the mac's ask to wait for the account's lock", || lock.turns() == 1);
    let before = (lock.turns(), waiting());

    let started = Instant::now();
    let refused = send_call(&mut vps, 1, act("tab-1", "extend_box", json!({ "machine": other.id, "extra_usd": 1.0 }), "x-1"));
    assert_eq!(refused, Err(NOT_MACHINES.to_owned()));
    assert!(started.elapsed() < Duration::from_millis(1000), "refused at once, not after a turn: {:?}", started.elapsed());
    assert_eq!((lock.turns(), waiting()), before, "only the mac's ask holds a turn connection, and nothing more waits for the owner");

    assert_eq!(s.hub.runtime.block_on(in_line).unwrap(), Err(pinned_core::wire::HUB_BUSY.to_owned()), "the mac's ask still waited its own turn");
    lock.release();
    assert_eq!(waiting(), 0);
}

const NOT_TASTES: &str = "a box may not change the account's way of working";

#[test]
fn a_boxs_node_act_on_a_persons_machine_is_refused_when_it_writes_and_goes_through_when_it_reads() {
    let s = setup("boxgate_node_act");
    let host = RawMachine::start(&s.hub.url, &s.mac_token, |call| match call["cmd"].as_str() {
        Some("list_projects") => RawReply::Ok(json!([])),
        _ => RawReply::Err("not in this test".into()),
    });
    let mut vps = reporting(&s, &s.box_token, &s.box_id, vec![tab("tab-1", "worker", json!({}))]);
    let on_mac = |action: &str, args: Value, request_id: &str| {
        let mut call = act("tab-1", action, args, request_id);
        call["act"]["machine"] = json!(s.mac_id);
        call
    };

    assert_eq!(send_call(&mut vps, 1, on_mac("make_folder", json!({ "path": "/w/new" }), "n-1")), Err(NOT_MACHINES.to_owned()));
    assert_eq!(host.calls("make_folder"), 0, "the write never reaches the person's machine");
    let read = send_call(&mut vps, 2, on_mac("list_projects", Value::Null, "n-2")).unwrap();
    assert_eq!(read["outcome"], "done", "a read of the person's machine passes the gate");
    assert_eq!(host.calls("list_projects"), 1);
}

#[test]
fn a_taste_chats_own_set_taste_from_a_box_is_refused_while_a_persons_machine_changes_the_taste() {
    let s = setup("boxgate_taste_chat");
    let chat = || vec![tab("tab-t", "worker", json!({ "taste_chat": true }))];
    let mut vps = reporting(&s, &s.box_token, &s.box_id, chat());
    let mut mac = reporting(&s, &s.mac_token, &s.mac_id, chat());
    let set = json!({ "cmd": "set_taste", "scope": { "kind": "captains" }, "patch": { "parallel": 2 } });

    assert_eq!(send_call(&mut vps, 1, set.clone()), Err(NOT_TASTES.to_owned()));
    let reset = json!({ "cmd": "reset_taste_field", "scope": { "kind": "captains" }, "field": "parallel" });
    assert_eq!(send_call(&mut vps, 2, reset), Err(NOT_TASTES.to_owned()));
    assert!(send_call(&mut mac, 1, set).is_ok(), "the same call from a person's machine with a taste chat open goes through");
}
