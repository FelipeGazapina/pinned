//! A box is a machine of the account that opened it, and may only pull the account's files: the
//! hub refuses every push, executable mark and share a box machine sends, while a person's other
//! machine of the same account syncs as before. Literal values throughout.

use super::*;

const REFUSED: &str = "the account's synced projects would pass 0 MB from a box: a box only pulls the account's files";
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
        "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase) values ('box-1', '{account}', '{machine}', 'box_123', 'c1', '{{\"issue\":\"i1\"}}', '{{}}', 'small', 'running')",
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
    assert_eq!(machine_result(url, &s.box_token, share()), refused, "a project share");
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
    assert_eq!(send_call(&mut node, 3, share()), Err(REFUSED.to_owned()));
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
