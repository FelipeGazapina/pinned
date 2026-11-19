//! The account's copy on the hub: who may keep one, what a hold does, and what a handback writes.
use super::*;
use pinned_core::account_copy::{Changes, Copy, Edit, Item, Mirrored, Progress, RunSet, Taken, Version, WorkflowProgress, COPY_EVERY};
use pinned_core::connectors::{Connector, Keys};
use pinned_server::hub::copies;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

const LINEAR_KEY: &str = "lin_api_mirror_key_LLLL";
const UPSTASH_KEY: &str = "box_mirror_upstash_secret";
const OFFLINE_KEY: &str = "lin_api_OFFLINE_EDIT_KEY";

fn person(name: &str) -> (TestHub, String, String) {
    let hub = TestHub::start(name);
    let (token, view) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    (hub, token, view.id)
}

fn machine(hub: &TestHub, token: &str, name: &str) -> (String, String) {
    let (view, machine_token) = account::register_machine(&hub.url, token, name, "macos").unwrap();
    (view.id, machine_token)
}

fn id_of(account: &str) -> uuid::Uuid {
    uuid::Uuid::parse_str(account).unwrap()
}

fn loaded(hub: &TestHub, account: uuid::Uuid) -> Arc<pinned_server::hub::Account> {
    hub.runtime.block_on(hub.hub().account(account)).unwrap()
}

fn connectors_of(hub: &TestHub, account: uuid::Uuid) -> Arc<pinned_core::connectors::Connectors> {
    let account = loaded(hub, account);
    hub.runtime.block_on(hub.hub().connectors(&account)).unwrap()
}

fn store_json() -> Value {
    json!({
        "connectors": [{
            "id": "wf",
            "name": "Triage",
            "enabled": true,
            "trigger": { "kind": "manual" },
            "folder": "/repo",
            "model": "",
            "effort": "high",
            "permissionMode": "acceptEdits",
            "worktree": false,
            "instructions": "",
            "since": "2026-01-01T00:00:00+00:00",
            "handled": ["ISS-1"]
        }],
        "linear": { "name": "Ana", "email": "ana@acme.dev", "organization": "Acme" }
    })
}

fn import_store(hub: &TestHub, account: uuid::Uuid) {
    let connectors = connectors_of(hub, account);
    assert!(connectors.import(store_json(), Keys { linear: Some(LINEAR_KEY.into()), ..Keys::default() }).unwrap(), "the account already had a store");
    connectors.connect_upstash(&pinned_core::upstash::Keys { api_key: UPSTASH_KEY.into(), claude_token: "claude-token-not-in-the-copy".into() }).unwrap();
    hub.runtime.block_on(hub.hub().db.save_tastes(account, &Tastes::default())).unwrap();
}

fn mirror_in(hub: &TestHub, account: &Arc<pinned_server::hub::Account>, machine: &str, have: &str, login: &str) -> Mirrored {
    hub.runtime.block_on(copies::mirror(hub.hub(), account, machine, have, login, false)).unwrap_or_else(|err| panic!("{err}"))
}

fn mirror_socket(hub: &TestHub, machine_token: &str, login: &str, have: &str) -> Mirrored {
    let value = machine_result(&hub.url, machine_token, json!({ "cmd": "mirror", "have": have, "login": login })).unwrap_or_else(|err| panic!("{err}"));
    serde_json::from_value(value).unwrap_or_else(|err| panic!("mirror did not read: {err}"))
}

fn handback_in(hub: &TestHub, account: &Arc<pinned_server::hub::Account>, machine: &str, changes: &Changes, login: &str) -> Taken {
    hub.runtime.block_on(copies::handback(hub.hub(), account, machine, changes, login, false)).unwrap_or_else(|err| panic!("{err}"))
}

fn handback_socket(hub: &TestHub, machine_token: &str, login: &str, changes: &Changes) -> Taken {
    let value = machine_result(&hub.url, machine_token, json!({ "cmd": "handback", "login": login, "changes": changes })).unwrap_or_else(|err| panic!("{err}"));
    serde_json::from_value(value).unwrap_or_else(|err| panic!("handback did not read: {err}"))
}

fn copy_of(mirrored: Mirrored) -> Copy {
    match mirrored {
        Mirrored::Copy { copy, .. } => *copy,
        Mirrored::Unchanged { .. } => panic!("the mirror answered unchanged"),
    }
}

fn workflow_version(copy: &Copy) -> Version {
    copy.items.get(&Item::workflow("wf")).unwrap().clone()
}

fn parallel_item() -> Item {
    Item::try_from("taste/captains/parallel".to_owned()).unwrap()
}

fn hold_message(machine: &str) -> String {
    format!("workflows wait for {machine} to hand back what it ran while the hub was off")
}

/// Ages the beat and holds. The beat task may rewrite `at` between the two, so a miss is tried again.
fn away(hub: &TestHub, account: uuid::Uuid, interval: &str) -> Option<String> {
    for _ in 0..3 {
        admin_on(&hub.database, &format!("insert into hub_beat (one, at) values (true, now() - interval '{interval}') on conflict (one) do update set at = excluded.at"));
        hub.runtime.block_on(copies::hold_after_absence(hub.hub())).unwrap();
        if let Some(held) = connectors_of(hub, account).held_for() {
            return Some(held);
        }
    }
    None
}

fn offline_edits_text(hub: &TestHub) -> String {
    hub.runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        let row = client.query_one("select coalesce(string_agg(item || ' ' || recorded::text, '\n'), '') from offline_edits", &[]).await.unwrap();
        row.get(0)
    })
}

fn add_box_row(hub: &TestHub, account: &str, machine: &str) {
    admin_on(&hub.database, &format!(
        "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase) values ('box-1', '{account}', '{machine}', 'box_123', 'c1', '{{\"issue\":\"i1\"}}', '{{\"folder\":\"/w\",\"args\":[]}}', 'small', 'running')"
    ));
}

#[test]
fn a_progress_only_copy_waits_out_copy_every_and_an_edit_does_not() {
    pinned_core::machine::refuse_real_home();
    let (tag, items, progressed) = ("tag-old", "items-same", "tag-new");
    assert!(!copies::due(tag, tag, items, None), "the machine already has this copy");
    assert!(copies::due("have-nothing", tag, items, None), "a machine with nothing is due a copy");
    assert!(!copies::due(tag, progressed, items, Some((tag, items, COPY_EVERY - Duration::from_secs(1)))), "progress alone waits out the interval");
    assert!(copies::due(tag, progressed, items, Some((tag, items, COPY_EVERY))), "progress past the interval is due");
    assert!(copies::due(tag, progressed, "items-new", Some((tag, items, Duration::from_secs(1)))), "an edit of the items is due at once");
}

#[test]
fn a_box_a_team_and_a_wrong_login_keep_no_copy() {
    let (hub, token, account) = person("refusals");
    let (mac, mac_token) = machine(&hub, &token, "MacBook");
    let (vps, vps_token) = machine(&hub, &token, "vps");
    add_box_row(&hub, &account, &vps);
    let refused = "a box may not keep a copy of the account's keys";
    assert_eq!(machine_result(&hub.url, &vps_token, json!({ "cmd": "mirror", "have": "", "login": token })).unwrap_err(), refused);
    assert_eq!(machine_result(&hub.url, &vps_token, json!({ "cmd": "handback", "login": token, "changes": {} })).unwrap_err(), refused);
    let owner = loaded(&hub, id_of(&account));
    assert_eq!(hub.runtime.block_on(copies::mirror(hub.hub(), &owner, &mac, "", &token, true)).unwrap_err(), refused);

    let (bob, _) = account::signup(&hub.url, "bob@acme.dev", "correct horse").unwrap();
    assert_eq!(hub.runtime.block_on(copies::mirror(hub.hub(), &owner, &mac, "", &bob, false)).unwrap_err(), "that login is not the account's");
    assert_eq!(hub.runtime.block_on(copies::mirror(hub.hub(), &owner, &mac, "", "", false)).unwrap_err(), "that login is not the account's");
    assert_eq!(hub.runtime.block_on(copies::mirror(hub.hub(), &owner, "not-a-machine", "", &token, false)).unwrap_err(), "this account has no machine not-a-machine");

    let team = hub.runtime.block_on(pinned_server::teams::db::create_team(&hub.hub().db, id_of(&account), "Acme")).unwrap();
    let (team_machine, team_token) = hub.runtime.block_on(hub.hub().db.create_machine(team, "team-box", "linux")).unwrap();
    assert_eq!(machine_result(&hub.url, &team_token, json!({ "cmd": "mirror", "have": "", "login": token })).unwrap_err(), "a team's box may not send mirror");
    let team_account = loaded(&hub, team);
    assert_eq!(hub.runtime.block_on(copies::mirror(hub.hub(), &team_account, &team_machine.id, "", &token, false)).unwrap_err(), "a team's account keeps no copy");

    let mut client = Client::connect(&hub.url, &token).signed_in().unwrap();
    assert_eq!(client.hub(json!({ "cmd": "mirror", "have": "", "login": token })).unwrap_err(), "only a signed-in desktop keeps a copy of the account");
    assert_eq!(client.hub(json!({ "cmd": "set_offline_runner", "machine": mac })).unwrap_err(), pinned_core::surface::LOCKED);
    assert_eq!(client.hub(json!({ "cmd": "release_workflows" })).unwrap_err(), pinned_core::surface::LOCKED);
    assert_eq!(machine_result(&hub.url, &mac_token, json!({ "cmd": "set_offline_runner", "machine": mac })).unwrap_err(), "a machine may only forward a workflow session's callbacks");
    assert_eq!(machine_result(&hub.url, &mac_token, json!({ "cmd": "release_workflows" })).unwrap_err(), "a machine may only forward a workflow session's callbacks");
    let missing = "no-such-machine".to_owned();
    assert_eq!(
        hub.runtime.block_on(hub.hub().hub_call(&owner, HubCall::SetOfflineRunner { machine: Some(missing.clone()) })).unwrap_err(),
        format!("this account has no machine {missing}")
    );
}

#[test]
fn the_owners_mirror_holds_the_linear_key_and_no_upstash_key() {
    let (hub, token, account) = person("copykeys");
    let (mac, mac_token) = machine(&hub, &token, "MacBook");
    import_store(&hub, id_of(&account));
    let mirrored = mirror_socket(&hub, &mac_token, &token, "");
    let Mirrored::Copy { copy, stopping } = mirrored else { panic!("the first mirror parked") };
    assert!(!stopping);
    assert_eq!(copy.runner.as_deref(), Some(mac.as_str()), "the first mirror names the machine the runner");
    let body = serde_json::to_string(&copy).unwrap();
    assert!(body.contains(LINEAR_KEY), "the copy dropped the linear key");
    assert!(!body.contains(UPSTASH_KEY), "the copy carries the upstash key");
    assert!(body.contains("taste/"), "the copy dropped tastes");
    assert_eq!(connectors_of(&hub, id_of(&account)).runner().as_deref(), Some(mac.as_str()));
}

#[test]
fn a_three_minute_outage_lifts_when_that_machine_mirrors() {
    let (hub, token, account) = person("outage3");
    let account_id = id_of(&account);
    let (mac, mac_token) = machine(&hub, &token, "MacBook");
    let (_mini, mini_token) = machine(&hub, &token, "Mini");
    import_store(&hub, account_id);
    let owner = loaded(&hub, account_id);
    hub.runtime.block_on(hub.hub().hub_call(&owner, HubCall::SetOfflineRunner { machine: Some(mac.clone()) })).unwrap();
    assert_eq!(away(&hub, account_id, "3 minutes").as_deref(), Some(mac.as_str()));

    let started = Instant::now();
    let mini = mirror_socket(&hub, &mini_token, &token, "");
    assert!(started.elapsed() < Duration::from_secs(5), "another machine's mirror parked for the hold");
    assert!(matches!(mini, Mirrored::Copy { stopping: false, .. }));
    assert_eq!(connectors_of(&hub, account_id).held_for().as_deref(), Some(mac.as_str()), "another machine's mirror left the hold");

    let started = Instant::now();
    let mac_copy = mirror_socket(&hub, &mac_token, &token, "");
    assert!(started.elapsed() < Duration::from_secs(5), "the holder's mirror parked for the hold");
    assert!(matches!(mac_copy, Mirrored::Copy { stopping: false, .. }));
    assert_eq!(connectors_of(&hub, account_id).held_for(), None, "the holder's mirror lifted the hold");
}

#[test]
fn offline_edits_records_the_hash_and_no_key() {
    let (hub, token, account) = person("editskey");
    let account_id = id_of(&account);
    let (_mac, mac_token) = machine(&hub, &token, "MacBook");
    import_store(&hub, account_id);
    let copy = copy_of(mirror_socket(&hub, &mac_token, &token, ""));
    let item = Item::integration("linear");
    let base = copy.items.get(&item).unwrap().clone();
    let mut ours = base.clone();
    ours.secrets.insert("key".into(), OFFLINE_KEY.into());
    ours.value["name"] = json!("Ana Offline");
    let changes = Changes { edits: BTreeMap::from([(item, Edit { at: 9, base: Some(base), ours: Some(ours) })]), ..Changes::default() };
    let taken = handback_socket(&hub, &mac_token, &token, &changes);
    let kept = taken.copy.items.get(&Item::integration("linear")).and_then(|version| version.secrets.get("key")).map(String::as_str);
    assert!(kept == Some(OFFLINE_KEY), "the handed-back copy did not take the linear key");
    let blob = offline_edits_text(&hub);
    assert!(!blob.contains(OFFLINE_KEY), "offline_edits holds a key byte");
    assert!(blob.contains("linear"), "offline_edits dropped the linear stamp");
}

#[test]
fn a_repeated_handback_replays_no_run_set() {
    let (hub, token, account) = person("repeat");
    let account_id = id_of(&account);
    let (mac, _) = machine(&hub, &token, "MacBook");
    import_store(&hub, account_id);
    let owner = loaded(&hub, account_id);
    let id = uuid::Uuid::new_v4();
    let changes = Changes {
        id,
        run_sets: vec![RunSet { connector: "wf".into(), subject: Subject::LinearIssue { issue: "ISS-missing".into() }, name: "status".into(), value: "done".into() }],
        ..Changes::default()
    };
    let first = handback_in(&hub, &owner, &mac, &changes, &token);
    assert!(first.notes.iter().any(|note| note.text.contains("no run variables")), "a missing run dropped its note");
    let second = handback_in(&hub, &owner, &mac, &changes, &token);
    assert!(second.notes.is_empty(), "a repeated handback replayed the run set");
    assert_eq!(first.copy.tag(), second.copy.tag());
}

#[test]
fn a_returning_hub_holds_until_the_handback_or_the_release() {
    let (mut hub, token, account) = person("boothold");
    let account_id = id_of(&account);
    let (mac, _) = machine(&hub, &token, "MacBook");
    import_store(&hub, account_id);
    let owner = loaded(&hub, account_id);
    hub.runtime.block_on(hub.hub().hub_call(&owner, HubCall::SetOfflineRunner { machine: Some(mac.clone()) })).unwrap();
    let addr = hub.stop();
    std::thread::sleep(Duration::from_millis(200));
    admin_on(&hub.database, "insert into hub_beat (one, at) values (true, now() - interval '10 minutes') on conflict (one) do update set at = excluded.at");
    hub.serve(&addr);

    let connectors = connectors_of(&hub, account_id);
    assert_eq!(connectors.held_for().as_deref(), Some(mac.as_str()));
    let opened = AtomicUsize::new(0);
    let open = |_work: Work| {
        opened.fetch_add(1, AtomicOrdering::SeqCst);
        async { Err::<(), String>("a held hub opened a run".into()) }
    };
    let check = hub.runtime.block_on(connectors.check_once(&open));
    assert_eq!(opened.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(check.error.as_deref(), Some(hold_message(&mac).as_str()));
    assert_eq!(connectors.run_now("wf", "Ana").unwrap_err(), hold_message(&mac));

    let owner = loaded(&hub, account_id);
    handback_in(&hub, &owner, &mac, &Changes::default(), &token);
    assert_eq!(connectors_of(&hub, account_id).held_for(), None, "the handback left the hold");

    assert_eq!(away(&hub, account_id, "10 minutes").as_deref(), Some(mac.as_str()));
    let owner = loaded(&hub, account_id);
    hub.runtime.block_on(hub.hub().hub_call(&owner, HubCall::ReleaseWorkflows)).unwrap();
    assert_eq!(connectors_of(&hub, account_id).held_for(), None, "the release left the hold");
}

#[test]
fn a_held_hub_checks_nothing_and_keeps_the_handed_issue() {
    let (hub, token, account) = person("heldchk");
    let account_id = id_of(&account);
    let (mac, _) = machine(&hub, &token, "MacBook");
    import_store(&hub, account_id);
    let owner = loaded(&hub, account_id);
    hub.runtime.block_on(hub.hub().hub_call(&owner, HubCall::SetOfflineRunner { machine: Some(mac.clone()) })).unwrap();
    assert_eq!(away(&hub, account_id, "3 minutes").as_deref(), Some(mac.as_str()));
    let connectors = connectors_of(&hub, account_id);
    let opened = AtomicUsize::new(0);
    let open = |_work: Work| {
        opened.fetch_add(1, AtomicOrdering::SeqCst);
        async { Err::<(), String>("a held hub opened a run".into()) }
    };
    let check = hub.runtime.block_on(connectors.check_once(&open));
    assert_eq!(opened.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(check.error.as_deref(), Some(hold_message(&mac).as_str()));

    let mut progress = Progress::default();
    progress.workflows.insert("wf".into(), WorkflowProgress { handled: BTreeSet::from(["I1".into()]), ..WorkflowProgress::default() });
    let owner = loaded(&hub, account_id);
    handback_in(&hub, &owner, &mac, &Changes { progress, ..Changes::default() }, &token);
    let handled = connectors_of(&hub, account_id).get_settings().connectors.into_iter().find(|connector| connector.id == "wf").unwrap().handled;
    assert!(handled.contains(&"I1".into()), "the handed issue was dropped");
    let opened = AtomicUsize::new(0);
    let open = |_work: Work| {
        opened.fetch_add(1, AtomicOrdering::SeqCst);
        async { Err::<(), String>("the next check started a run".into()) }
    };
    let _check = hub.runtime.block_on(connectors_of(&hub, account_id).check_once(&open));
    assert_eq!(opened.load(AtomicOrdering::SeqCst), 0, "the next check started a run for the handed issue");
}

struct Desk {
    winner: String,
    aside_name: String,
    aside_id: String,
    aside_enabled: bool,
    parallel: u32,
}

fn desk(name: &str, first: &str) -> Desk {
    let (hub, token, account) = person(name);
    let account_id = id_of(&account);
    let (mac, _) = machine(&hub, &token, "MacBook");
    let (mini, _) = machine(&hub, &token, "Mini");
    import_store(&hub, account_id);
    let owner = loaded(&hub, account_id);
    let copy = copy_of(mirror_in(&hub, &owner, &mac, "", &token));
    let workflow = Item::workflow("wf");
    let base_workflow = workflow_version(&copy);
    let base_parallel = copy.items.get(&parallel_item()).unwrap().clone();
    let edit = |at: u64, name: &str, parallel: u64| {
        let mut workflow_ours = base_workflow.clone();
        workflow_ours.value["name"] = json!(name);
        let parallel_ours = Version { value: json!(parallel), secrets: BTreeMap::new() };
        Changes {
            id: uuid::Uuid::new_v4(),
            edits: BTreeMap::from([
                (workflow.clone(), Edit { at, base: Some(base_workflow.clone()), ours: Some(workflow_ours) }),
                (parallel_item(), Edit { at, base: Some(base_parallel.clone()), ours: Some(parallel_ours) }),
            ]),
            ..Changes::default()
        }
    };
    let mac_changes = edit(10, "From MacBook", 2);
    let mini_changes = edit(12, "From Mini", 4);
    let (first_machine, first_changes, second_machine, second_changes) = match first {
        "MacBook" => (&mac, &mac_changes, &mini, &mini_changes),
        _ => (&mini, &mini_changes, &mac, &mac_changes),
    };
    handback_in(&hub, &owner, first_machine, first_changes, &token);
    handback_in(&hub, &owner, second_machine, second_changes, &token);
    let settings = connectors_of(&hub, account_id).get_settings();
    let winner = settings.connectors.iter().find(|connector| connector.id == "wf").unwrap();
    let aside = settings.connectors.iter().find(|connector| connector.id != "wf").unwrap();
    let tastes = hub.runtime.block_on(hub.hub().db.tastes(account_id)).unwrap().unwrap();
    Desk { winner: winner.name.clone(), aside_name: aside.name.clone(), aside_id: aside.id.clone(), aside_enabled: aside.enabled, parallel: tastes.captains.parallel }
}

#[test]
fn two_desktops_hand_back_the_same_store_in_either_order() {
    let (ab, ba) = (desk("deskab", "MacBook"), desk("deskba", "Mini"));
    assert_eq!(ab.winner, "From Mini");
    assert_eq!(ab.aside_name, "From MacBook (MacBook while the hub was off)");
    assert!(!ab.aside_enabled);
    assert_eq!(ab.parallel, 4);
    assert_eq!(ab.winner, ba.winner);
    assert_eq!(ab.aside_name, ba.aside_name);
    assert_eq!(ab.aside_id, ba.aside_id);
    assert_eq!(ab.aside_enabled, ba.aside_enabled);
    assert_eq!(ab.parallel, ba.parallel);
}

#[test]
fn mirror_hold_wait_answers_unchanged_then_a_save_and_a_stop_send_the_copy() {
    let (hub, token, account) = person("holdwait");
    let account_id = id_of(&account);
    let (mac, _) = machine(&hub, &token, "MacBook");
    import_store(&hub, account_id);
    let connectors = connectors_of(&hub, account_id);
    connectors.stop_finding();
    let started = Instant::now();
    while connectors.checks_open() > 0 && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let owner = loaded(&hub, account_id);
    let first = copy_of(mirror_in(&hub, &owner, &mac, "", &token));
    assert!(first.progress.workflows["wf"].handled.contains("ISS-1"));
    let tag = first.tag();

    let (hub_for_save, owner_for_save, mac_for_save, token_for_save) = (hub.hub().clone(), owner.clone(), mac.clone(), token.clone());
    let parked = hub.runtime.spawn(async move { copies::mirror(&hub_for_save, &owner_for_save, &mac_for_save, &tag, &token_for_save, false).await });
    let saw = wait_parked(&hub, account_id);
    let mut connector: Connector = serde_json::from_value(store_json()["connectors"][0].clone()).unwrap();
    connector.name = "Renamed".into();
    hub.runtime.block_on(hub.hub().hub_call(&owner, HubCall::SaveConnector { connector })).unwrap();
    let saved = hub.runtime.block_on(parked).unwrap().unwrap();
    assert!(saw.elapsed() < Duration::from_secs(5), "a save left the mirror parked");
    let saved_copy = copy_of(saved);
    assert_eq!(saved_copy.items.get(&Item::workflow("wf")).unwrap().value["name"], json!("Renamed"));
    let tag = saved_copy.tag();
    let have = tag.clone();

    let (hub_for_hold, owner_for_hold, mac_for_hold, token_for_hold) = (hub.hub().clone(), owner.clone(), mac.clone(), token.clone());
    let holding = hub.runtime.spawn(async move { copies::mirror(&hub_for_hold, &owner_for_hold, &mac_for_hold, &have, &token_for_hold, false).await });
    let saw = wait_parked(&hub, account_id);
    let unchanged = hub.runtime.block_on(holding).unwrap().unwrap();
    let waited = saw.elapsed();
    assert!(waited >= Duration::from_secs(45) && waited < Duration::from_secs(58), "the hold waited {waited:?}");
    assert!(matches!(unchanged, Mirrored::Unchanged { stopping: false }));

    hub.runtime.block_on(copies::stop(hub.hub()));
    let stopped = mirror_in(&hub, &owner, &mac, &tag, &token);
    let Mirrored::Copy { copy, stopping } = stopped else { panic!("a stopping hub answered unchanged") };
    assert!(stopping);
    assert!(copy.progress.workflows["wf"].handled.contains("ISS-1"));
    assert_eq!(copy.items.get(&Item::workflow("wf")).unwrap().value["name"], json!("Renamed"));
}

fn wait_parked(hub: &TestHub, account: uuid::Uuid) -> Instant {
    let started = Instant::now();
    loop {
        if hub.hub().copies.parked(account) > 0 {
            return Instant::now();
        }
        assert!(started.elapsed() < Duration::from_secs(5), "the mirror did not park");
        std::thread::sleep(Duration::from_millis(20));
    }
}
