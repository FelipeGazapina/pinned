//! A Captain's plans live on its machine. The hub passes their glances in the tabs it sends the
//! account's clients and forwards `Call::Plan` to that machine, and another account sees nothing of them.
use super::*;

/// A node of the account resumed from sessions.json with a Captain of Claude session `cap-1` and a
/// plain thread of session `plain-1`, listening on its socket: the node, its folder, its machine id,
/// the Captain's tab and the thread's tab.
fn resumed_with_a_captain(url: &str, token: &str, email: &str, client: &mut Client, name: &str) -> (Node, PathBuf, String, String, String) {
    let dir = temp(&format!("plans-{name}"));
    let project = dir.join("app");
    std::fs::create_dir_all(&project).unwrap();
    let (machine, machine_token) = account::register_machine(url, token, name, "linux").unwrap();
    let creds = Credentials { hub: url.into(), email: email.into(), user_token: token.into(), machine_id: machine.id.clone(), machine_token };
    let project = project.display().to_string();
    let saved = json!([
        { "cwd": project, "session": "cap-1", "env": [["PINNED_ROLE", "orchestrator"], ["PINNED_PROJECT", project]], "bypass": false, "interrupted": false },
        { "cwd": project, "session": "plain-1", "env": [], "bypass": false, "interrupted": false },
    ]);
    std::fs::write(dir.join("sessions.json"), saved.to_string()).unwrap();
    let node = Node::resume(&dir, creds);
    node.machine.listen(|_| Err("unexpected".into())).unwrap();
    client.wait_machines(&[(name, true)]);
    let tab_of = |session: &str| node.machine.tabs().into_iter().find(|t| t.session.as_deref() == Some(session)).unwrap().id;
    let (captain, thread) = (tab_of("cap-1"), tab_of("plain-1"));
    (node, dir, machine.id, captain, thread)
}

/// The tab `tab` of `machine` in a hello or machines frame.
fn tab_in(frame: &Value, machine: &str, tab: &str) -> Option<Value> {
    frame["tabs"][machine].as_array()?.iter().find(|t| t["id"] == tab).cloned()
}

#[test]
fn plans_reach_the_accounts_clients_through_the_hub_and_never_another_account() {
    let hub = TestHub::start_with("plans_apart", Signup::Invite);
    let url = hub.url.clone();
    let (ana_token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut ana_client = signed_in_client(&url, &ana_token);
    let code = ana_client.hub(json!({ "cmd": "create_invite" })).unwrap()["code"].as_str().unwrap().to_owned();
    let (bruno_token, _) = account::signup_invited(&url, "bruno@acme.dev", "correct horse", Some(&code)).unwrap();
    let mut bruno_client = signed_in_client(&url, &bruno_token);

    let (mut ana_node, ana_dir, ana_machine, captain, thread) = resumed_with_a_captain(&url, &ana_token, "ana@acme.dev", &mut ana_client, "ana-mac");
    let bruno_dir = temp("plans-bruno-mac");
    let (bruno_machine, bruno_machine_token) = account::register_machine(&url, &bruno_token, "bruno-mac", "linux").unwrap();
    let bruno_creds = Credentials { hub: url.clone(), email: "bruno@acme.dev".into(), user_token: bruno_token.clone(), machine_id: bruno_machine.id.clone(), machine_token: bruno_machine_token };
    let mut bruno_node = Node::start(&bruno_dir, bruno_creds);
    bruno_client.wait_machines(&[("bruno-mac", true)]);

    // The Captain writes its plan the way its MCP tool does: over its machine's socket, as its tab.
    let set = json!({ "kind": "project", "tab": captain, "tool": { "name": "plan_set", "title": "Ship the form", "lists": [{ "name": "Build", "steps": [{ "name": "Write" }, { "name": "Test" }] }] } });
    assert_eq!(ask_socket(&ana_dir, &set), json!({ "Ok": "p1 rev 1: Ship the form\nl1 Build: s1 pending, s2 pending" }));
    let glances = json!([{ "id": "p1", "title": "Ship the form", "state": "idle", "done": 0, "total": 2, "strips": ["pp"] }]);
    ana_client.wait("the Captain's glance", |f| {
        f["type"] == "event" && f["machine"] == ana_machine && f["event"]["kind"] == "tabs" && f["event"]["tabs"].as_array().is_some_and(|tabs| tabs.iter().any(|t| t["id"] == captain && t["plans"] == glances))
    });

    // A client of the account that connects now gets the glance in its hello, on the Captain only.
    let mut late = Client::connect(&url, &ana_token);
    let hello = late.wait("hello", |f| f["type"] == "hello");
    assert_eq!(tab_in(&hello, &ana_machine, &captain).map(|t| t["plans"].clone()), Some(glances.clone()), "{hello}");
    assert_eq!(tab_in(&hello, &ana_machine, &thread).map(|t| t.get("plans").cloned()), Some(None), "a plain thread sends no plans: {hello}");

    // Call::Plan reaches the machine that holds the Captain.
    let plan = |tab: &str| json!({ "cmd": "plan", "tab": tab });
    let mut plans = late.call(&ana_machine, plan(&captain)).unwrap();
    assert!(plans[0]["asked_at"].as_u64().is_some_and(|at| at > 0), "{plans}");
    plans[0].as_object_mut().unwrap().remove("asked_at");
    let step = |id: &str, name: &str| json!({ "id": id, "name": name, "what": "", "state": "pending", "why": null, "cost": null, "task": null, "fixes": [] });
    assert_eq!(plans, json!([{ "id": "p1", "rev": 1, "title": "Ship the form", "finished_at": null, "closed": null, "lists": [{ "id": "l1", "name": "Build", "task": null, "steps": [step("s1", "Write"), step("s2", "Test")] }] }]));
    assert_eq!(late.call(&ana_machine, json!({ "cmd": "plan", "tab": captain, "plan": "p1" })).unwrap().as_array().map(Vec::len), Some(1));
    assert_eq!(late.call(&ana_machine, plan(&thread)), Ok(json!([])), "a tab that is not a Captain keeps no plan");

    // Another account on the same hub reaches none of it.
    let refused = Err(format!("this account has no machine {ana_machine}"));
    assert_eq!(bruno_client.call(&ana_machine, plan(&captain)), refused);
    let own = bruno_client.call(&bruno_machine.id, plan(&captain));
    assert!(own.as_ref().map_or(true, |plans| *plans == json!([])), "Bruno's own machine knows no plan of Ana's tab: {own:?}");
    let mut bruno_late = Client::connect(&url, &bruno_token);
    let hello = bruno_late.wait("Bruno's hello", |f| f["type"] == "hello");
    assert!(hello["tabs"].get(&ana_machine).is_none(), "{hello}");
    bruno_client.pump();
    bruno_late.pump();
    let leaked: Vec<&Value> = bruno_client.frames.iter().chain(&bruno_late.frames).filter(|f| f["type"] != "reply" && (f.to_string().contains(&ana_machine) || f.to_string().contains("Ship the form"))).collect();
    assert!(leaked.is_empty(), "Bruno's sockets carried Ana's plan: {leaked:?}");
    ana_node.kill();
    bruno_node.kill();
}
