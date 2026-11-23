//! The global orchestrator settings page's last save: the hub sends it to each machine that missed
//! it when that machine connects, keeps the machine's own host, and lets only an app record it.
use super::*;

/// A hub with Ana signed in on a client that runs every call (an app before the lock).
fn ana(name: &str) -> (TestHub, String, Client) {
    let hub = TestHub::start(name);
    let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&hub.url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    (hub, token, client)
}

/// The settings the page sends with a save: every shared field, Auto-merge as `auto_merge`.
fn shared(name: &str, auto_merge: bool) -> Value {
    json!({ "name": name, "busy": "queue", "reports": "auto", "group_reports": true, "read_aloud": false, "auto_merge": auto_merge, "approval_profile": "" })
}

fn record(client: &mut Client, settings: Value, took: &[&str]) -> Value {
    client.hub(json!({ "cmd": "record_global_orchestrator_save", "settings": settings, "took": took })).unwrap()
}

fn pending(client: &mut Client) -> Vec<String> {
    let save = client.hub(json!({ "cmd": "global_orchestrator_save" })).unwrap();
    serde_json::from_value(save["pending"].clone()).unwrap()
}

fn wait_taken(client: &mut Client, machine: &str) {
    wait_for("the machine to take the save", || !pending(client).iter().any(|id| id == machine));
}

/// A paired node that ran once with `settings`, then went offline: its dir and credentials.
fn offline_node(client: &mut Client, hub: &TestHub, name: &str, settings: Value) -> (PathBuf, Credentials) {
    let dir = node_dir(name);
    let creds = pair(&hub.url, client, name, &dir);
    let mut node = Node::start(&dir, creds.clone());
    wait_for("the node online", || node.online());
    client.call(&creds.machine_id, json!({ "cmd": "set_global_orchestrator_settings", "settings": settings })).unwrap();
    node.kill();
    wait_for("the node offline", || client.hub(json!({ "cmd": "machines" })).unwrap().as_array().unwrap().iter().any(|m| m["name"] == name && m["online"] == false));
    (dir, creds)
}

#[test]
fn a_machine_offline_for_a_save_gets_it_when_it_connects() {
    let (hub, _, mut client) = ana("go_save_offline");
    let mac = pair(&hub.url, &mut client, "mac", &node_dir("mac")).machine_id;
    let (dir, creds) = offline_node(&mut client, &hub, "vps", json!({ "auto_merge": true }));
    let vps = creds.machine_id.clone();

    let save = record(&mut client, shared("Orchestrator", false), &[&mac]);
    assert_eq!(save["pending"], json!([vps]), "the page wrote mac, so only vps waits");
    assert_eq!(save["settings"]["host"], false);
    assert_eq!(pending(&mut client), vec![vps.clone()]);

    let node = Node::start(&dir, creds);
    wait_taken(&mut client, &vps);
    assert!(!node.machine.global_settings().auto_merge, "Auto-merge off reached the machine that was offline");
}

#[test]
fn a_second_save_before_the_machine_connects_is_the_one_it_gets() {
    let (hub, _, mut client) = ana("go_save_latest");
    let (dir, creds) = offline_node(&mut client, &hub, "vps", json!({ "auto_merge": false }));
    let vps = creds.machine_id.clone();
    let first = record(&mut client, shared("Admiral", false), &[]);
    let second = record(&mut client, shared("Commander", true), &[]);
    assert_ne!(first["id"], second["id"]);
    assert_eq!(client.hub(json!({ "cmd": "global_orchestrator_save" })).unwrap()["id"], second["id"], "the second save replaced the first");

    let node = Node::start(&dir, creds);
    wait_taken(&mut client, &vps);
    let settings = node.machine.global_settings();
    assert_eq!((settings.name.as_str(), settings.auto_merge), ("Commander", true));
}

#[test]
fn a_resend_keeps_each_machines_own_host() {
    let (hub, _, mut client) = ana("go_save_host");
    let (host_dir, host_creds) = offline_node(&mut client, &hub, "mini", json!({ "host": true, "auto_merge": true }));
    let (other_dir, other_creds) = offline_node(&mut client, &hub, "vps", json!({ "host": false, "auto_merge": true }));
    let (mini, vps) = (host_creds.machine_id.clone(), other_creds.machine_id.clone());
    record(&mut client, shared("Orchestrator", false), &[]);

    let (host, other) = (Node::start(&host_dir, host_creds), Node::start(&other_dir, other_creds));
    wait_taken(&mut client, &mini);
    wait_taken(&mut client, &vps);
    let (host, other) = (host.machine.global_settings(), other.machine.global_settings());
    assert_eq!((host.host, host.auto_merge), (true, false), "the host stays the host");
    assert_eq!((other.host, other.auto_merge), (false, false), "the other machine does not become one");
}

#[test]
fn a_failed_resend_stays_pending_and_is_sent_again_on_the_next_connect() {
    let (hub, token, mut client) = ana("go_save_retry");
    let (machine, machine_token) = account::register_machine(&hub.url, &token, "flaky", "linux").unwrap();
    record(&mut client, shared("Orchestrator", false), &[]);
    assert_eq!(pending(&mut client), vec![machine.id.clone()]);

    let fail = Arc::new(AtomicBool::new(true));
    let answer = |fail: Arc<AtomicBool>| {
        move |call: &Value| match call["cmd"].as_str().unwrap() {
            "global_orchestrator_settings" => RawReply::Ok(json!({ "name": "Orchestrator", "host": true, "auto_merge": true })),
            "set_global_orchestrator_settings" if fail.load(Ordering::SeqCst) => RawReply::Err("could not write global-orchestrator.json".into()),
            "set_global_orchestrator_settings" => RawReply::Ok(call["settings"].clone()),
            _ => RawReply::Ok(Value::Null),
        }
    };
    let raw = RawMachine::start(&hub.url, &machine_token, answer(fail.clone()));
    wait_for("the resend", || raw.calls("set_global_orchestrator_settings") == 1);
    // The hub marks a machine taken only after its write succeeds, so the failure leaves it pending.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(pending(&mut client), vec![machine.id.clone()], "a failed write leaves the machine pending");
    drop(raw);
    wait_for("flaky offline", || client.hub(json!({ "cmd": "machines" })).unwrap()[0]["online"] == false);

    fail.store(false, Ordering::SeqCst);
    let raw = RawMachine::start(&hub.url, &machine_token, answer(fail));
    wait_taken(&mut client, &machine.id);
    let sent: Vec<Value> = raw.calls.lock().unwrap().iter().filter(|c| c["cmd"] == "set_global_orchestrator_settings").map(|c| c["settings"].clone()).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!((&sent[0]["host"], &sent[0]["auto_merge"]), (&json!(true), &json!(false)), "the save's values, with the host the machine has");
}

#[test]
fn a_pending_save_survives_a_hub_restart() {
    let (mut hub, token, mut client) = ana("go_save_restart");
    let (dir, creds) = offline_node(&mut client, &hub, "vps", json!({ "auto_merge": true }));
    let vps = creds.machine_id.clone();
    record(&mut client, shared("Orchestrator", false), &[]);
    drop(client);

    hub.restart();
    let mut client = Client::connect(&hub.url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    assert_eq!(pending(&mut client), vec![vps.clone()], "the save outlived the hub");
    let node = Node::start(&dir, creds);
    wait_taken(&mut client, &vps);
    assert!(!node.machine.global_settings().auto_merge);
}

#[test]
fn only_an_app_records_a_save() {
    let (hub, token, mut client) = ana("go_save_who");
    let (_, machine_token) = account::register_machine(&hub.url, &token, "mac", "linux").unwrap();
    let call = json!({ "cmd": "record_global_orchestrator_save", "settings": shared("Orchestrator", true), "took": [] });
    let refused = Err("a machine may only forward a workflow session's callbacks".to_owned());
    assert_eq!(machine_result(&hub.url, &machine_token, call.clone()), refused, "a machine socket can't record a save");
    assert_eq!(machine_result(&hub.url, &machine_token, json!({ "cmd": "global_orchestrator_save" })), refused, "nor read one");
    let mut locked = Client::app(&hub.url, &token);
    assert_eq!(locked.hub(call).unwrap_err(), pinned_core::surface::LOCKED, "a locked login needs the unlock");
    assert_eq!(client.hub(json!({ "cmd": "global_orchestrator_save" })), Ok(Value::Null), "nothing was recorded");
}
