//! Boxes at once: how many boxes the account's agents may have open at once. The Orchestrator, the
//! Captains, their workers and the workflows count and are refused past it; a box the user opens
//! from the app is neither. Only the user changes the value. Literal values throughout.

use super::*;

const ZERO: &str = "Boxes at once is 0, so agents cannot start a box on this account. Run it on a connected machine, or raise Boxes at once in Settings › Way of working.";
const ONE: &str = "1 box already runs on this account, the most it allows at once. Wait for one to end, or raise Boxes at once in Settings › Way of working.";
const TWO: &str = "2 boxes already run on this account, the most it allows at once. Wait for one to end, or raise Boxes at once in Settings › Way of working.";
const USER_ONLY: &str = "boxes_at_once is changed only by the user, on the Way of working page; nothing changed";

/// A box account whose machine `mac` has the project site on claude, with every box permission
/// fresh, the global orchestrator, the site's Captain and a plain session there.
struct Fleet {
    hub: TestHub,
    ana: BoxAccount,
    _mac: Node,
    dir: PathBuf,
    global: String,
    captain: String,
    session: String,
}

impl Fleet {
    fn start(name: &str) -> Fleet {
        let hub = TestHub::start_with_boxes(name, Signup::Open, Policy { max_open: 12, ..fast() });
        let mut ana = BoxAccount::new(&hub);
        hub.boxes.state.lock().unwrap().checkout = Some(("site".into(), SITE.into()));
        let dir = temp(&format!("{name}-mac"));
        let (mac, mac_id, site) = machine_with_site(&hub.url, &mut ana, "ana@acme.dev", "mac", &dir, "claude");
        let all_fresh = json!({ "workflow": { "mode": "fresh" }, "session": { "mode": "fresh" }, "worker": { "mode": "fresh" }, "orchestrator": { "mode": "fresh" } });
        ana.client.hub(json!({ "cmd": "set_box_access", "access": all_fresh })).unwrap();
        ana.client.call(&mac_id, json!({ "cmd": "set_global_orchestrator_settings", "settings": { "host": true } })).unwrap();
        let global = ana.client.call(&mac_id, json!({ "cmd": "open_global_orchestrator" })).unwrap().as_str().unwrap().to_owned();
        let captain = ana.client.call(&mac_id, json!({ "cmd": "open_orchestrator", "path": site })).unwrap().as_str().unwrap().to_owned();
        let session = mac.machine.open_tab(Some(site.clone()), false).unwrap();
        ana.client.wait("the session's tab at the hub", |f| f["type"] == "event" && f["event"]["kind"] == "tabs" && f["event"]["tabs"].as_array().unwrap().iter().any(|t| t["id"] == session.as_str()));
        Fleet { hub, ana, _mac: mac, dir, global, captain, session }
    }

    /// The user's own change, from the app.
    fn boxes_at_once(&mut self, n: u32) {
        self.ana.client.hub(json!({ "cmd": "set_taste", "scope": { "kind": "captains" }, "patch": { "boxes_at_once": n } })).unwrap();
    }

    fn worker(&self, title: &str) -> Result<Value, String> {
        delegate_from(&self.dir, &self.captain, json!({ "task": "Run the suite.", "title": title, "target": { "box": true } }))
    }

    fn captain_box(&self, request_id: &str) -> Result<Value, String> {
        session_tool(&self.dir, &self.global, json!({ "name": "send_to_captain", "project": "github.com/acme/site", "message": "Ship it.", "request_id": request_id, "box": true }))
    }

    fn agents_box(&self) -> Result<Value, String> {
        act(&self.dir, &self.session, "open_box", json!({ "repo": "acme/site", "size": "small", "budget_usd": 2.0 }))
    }

    fn workflow(&self, id: &str) -> Result<(), String> {
        self.hub.runtime.block_on(self.hub.hub().open(self.ana.id, self.ana.box_run(issue(id))))
    }

    fn users_box(&mut self) -> Result<Value, String> {
        self.ana.client.hub(json!({ "cmd": "open_box", "repo": "acme/site", "size": "small", "budget_usd": 2.0 }))
    }
}

#[test]
fn boxes_at_once_2_refuses_the_third_agent_box_and_never_the_users() {
    let mut fleet = Fleet::start("boxes_at_once_two");
    fleet.boxes_at_once(2);
    assert_eq!(fleet.worker("One").unwrap()["workState"], "working");
    assert_eq!(fleet.users_box().unwrap()["phase"], "creating", "the user's box neither counts nor is refused");
    assert_eq!(fleet.worker("Two").unwrap()["workState"], "working");
    assert_eq!(fleet.worker("Three"), Err(TWO.to_owned()), "the third agent box");
    assert_eq!(fleet.captain_box("r-1"), Err(TWO.to_owned()), "a Captain's box counts the same");
    assert_eq!(fleet.agents_box(), Err(TWO.to_owned()), "an agent's open_box counts the same");
    assert_eq!(fleet.workflow("w-1"), Err(TWO.to_owned()), "a workflow's box counts the same");
    assert_eq!(fleet.users_box().unwrap()["phase"], "creating", "the user still opens one past the agents' limit");
    assert_eq!(fleet.ana.rows(&fleet.hub), 4, "two agent boxes and the user's two");
}

#[test]
fn boxes_at_once_0_refuses_every_agent_path_and_opens_the_users_box() {
    let mut fleet = Fleet::start("boxes_at_once_zero");
    fleet.boxes_at_once(0);
    assert_eq!(fleet.worker("One"), Err(ZERO.to_owned()), "delegate_task with a box");
    assert_eq!(fleet.captain_box("r-1"), Err(ZERO.to_owned()), "send_to_captain with a box");
    assert_eq!(fleet.agents_box(), Err(ZERO.to_owned()), "an agent's open_box");
    assert_eq!(fleet.workflow("w-1"), Err(ZERO.to_owned()), "a workflow's box");
    assert_eq!(fleet.ana.rows(&fleet.hub), 0, "no agent box was reserved");
    assert_eq!(fleet.users_box().unwrap()["phase"], "creating", "the user's own box opens at 0");
    assert_eq!(fleet.ana.rows(&fleet.hub), 1);
}

#[test]
fn boxes_at_once_1_lets_one_of_two_agent_boxes_opened_at_the_same_time_through() {
    let hub = TestHub::start_with_boxes("boxes_at_once_race", Signup::Open, Policy { max_open: 12, ..fast() });
    let mut ana = BoxAccount::new(&hub);
    set_workflow(&mut ana.client, "fresh");
    ana.client.hub(json!({ "cmd": "set_taste", "scope": { "kind": "captains" }, "patch": { "boxes_at_once": 1 } })).unwrap();
    let shared = hub.hub();
    let (first, second) = hub.runtime.block_on(async { tokio::join!(shared.open(ana.id, ana.box_run(issue("r-1"))), shared.open(ana.id, ana.box_run(issue("r-2")))) });
    let mut answers = vec![first, second];
    answers.sort();
    assert_eq!(answers, vec![Ok(()), Err(ONE.to_owned())], "exactly one passes the limit");
    assert_eq!(ana.rows(&hub), 1);
}

#[test]
fn boxes_at_once_is_8_by_default_and_only_the_user_changes_it() {
    let hub = TestHub::start("boxes_at_once_owner");
    let url = hub.url.clone();
    let (token, owner) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut ana = agent_of(&url, &token, "ana@acme.dev", "boxes-owner-mac");
    let (machine, tab) = (ana.machine.clone(), ana.tab.clone());
    let captains = json!({ "kind": "captains" });
    let stored = |client: &mut Client| {
        let tastes = client.hub(json!({ "cmd": "get_tastes" })).unwrap();
        (tastes["boxes_at_once"].clone(), tastes["box_cap"].clone())
    };
    let asks = |client: &mut Client| client.hub(json!({ "cmd": "confirmations" })).unwrap();

    assert_eq!(stored(&mut ana.client), (json!(8), json!(8)), "a fresh account reads 8, and the hub's cap of 8");
    ana.client.hub(json!({ "cmd": "set_taste", "scope": captains, "patch": { "parallel": 2 } })).unwrap();
    assert_eq!(stored(&mut ana.client), (json!(8), json!(8)), "a change from an app that predates the field keeps 8");

    assert_eq!(act(&ana.dir, &tab, "set_taste", json!({ "scope": captains, "patch": { "boxes_at_once": 3 } })), Err(USER_ONLY.into()), "an agent's set_taste");
    assert_eq!(act(&ana.dir, &tab, "set_taste", json!({ "scope": captains, "patch": { "parallel": 3, "boxes_at_once": 3 } })), Err(USER_ONLY.into()), "a mixed patch is refused whole");
    assert_eq!(act(&ana.dir, &tab, "reset_taste_field", json!({ "scope": captains, "field": "boxes_at_once" })), Err(USER_ONLY.into()), "an agent's reset");
    assert_eq!(asks(&mut ana.client), json!([]), "a refused act held no ask for the owner");

    // An ask held before the refusal, approved by the owner, changes nothing either.
    let asked = Confirmation { id: String::new(), at: 1, machine: machine.clone(), machine_name: "boxes-owner-mac".into(), tab: tab.clone(), title: "Way of working".into(), role: ActorRole::Session, action: "set_taste".into(), about: String::new(), args: json!({ "scope": captains, "patch": { "boxes_at_once": 3 } }), target: None, request_id: Some("held-boxes".into()), decision: None, decided_at: None, outcome: None };
    let (account_id, machine_id) = (uuid::Uuid::parse_str(&owner.id).unwrap(), uuid::Uuid::parse_str(&machine).unwrap());
    let held = hub.runtime.block_on(hub.hub().db.hold_confirmation(account_id, machine_id, None, &asked)).unwrap().0.to_string();
    ana.client.hub(json!({ "cmd": "unlock", "password": "correct horse" })).unwrap();
    let approved = ana.client.hub(json!({ "cmd": "decide_confirmation", "id": held, "approve": true })).unwrap();
    assert_eq!(approved["outcome"], format!("failed: {USER_ONLY}"), "an approved agent change still fails: {approved}");
    assert_eq!(stored(&mut ana.client), (json!(8), json!(8)), "no agent changed it");

    ana.client.hub(json!({ "cmd": "set_taste", "scope": captains, "patch": { "boxes_at_once": 3 } })).unwrap();
    assert_eq!(stored(&mut ana.client), (json!(3), json!(8)), "the user's own change is saved");
    ana.client.hub(json!({ "cmd": "set_taste", "scope": captains, "patch": { "boxes_at_once": 20 } })).unwrap();
    assert_eq!(stored(&mut ana.client), (json!(20), json!(8)), "a value past the cap is kept; every reader takes the cap");
}

impl Fleet {
    fn every_box(&mut self, mode: &str) {
        let access = json!({ "workflow": { "mode": mode }, "session": { "mode": mode }, "worker": { "mode": mode }, "orchestrator": { "mode": mode } });
        self.ana.client.hub(json!({ "cmd": "set_box_access", "access": access })).unwrap();
    }

    /// What the Upstash card asks the user now.
    fn asks(&mut self) -> Value {
        self.ana.client.hub(json!({ "cmd": "get_settings" })).unwrap()["box_asks"].clone()
    }
}

#[test]
fn boxes_at_once_0_refuses_every_agent_before_an_ask_reaches_the_user() {
    let mut fleet = Fleet::start("boxes_at_once_ask_zero");
    fleet.every_box("ask");
    fleet.boxes_at_once(0);
    assert_eq!(fleet.worker("One"), Err(ZERO.to_owned()), "delegate_task with a box");
    assert_eq!(fleet.captain_box("r-1"), Err(ZERO.to_owned()), "send_to_captain with a box");
    assert_eq!(fleet.agents_box(), Err(ZERO.to_owned()), "an agent's open_box");
    assert_eq!(fleet.workflow("w-1"), Err(ZERO.to_owned()), "a workflow's box");
    assert_eq!(fleet.asks(), json!([]), "no ask card was made");
    assert_eq!(fleet.ana.rows(&fleet.hub), 0);
}

#[test]
fn boxes_at_once_full_refuses_every_agent_before_an_ask_reaches_the_user() {
    let mut fleet = Fleet::start("boxes_at_once_ask_full");
    fleet.boxes_at_once(1);
    assert_eq!(fleet.worker("One").unwrap()["workState"], "working");
    fleet.every_box("ask");
    assert_eq!(fleet.worker("Two"), Err(ONE.to_owned()), "delegate_task with a box");
    assert_eq!(fleet.captain_box("r-1"), Err(ONE.to_owned()), "send_to_captain with a box");
    assert_eq!(fleet.agents_box(), Err(ONE.to_owned()), "an agent's open_box");
    assert_eq!(fleet.workflow("w-1"), Err(ONE.to_owned()), "a workflow's box");
    assert_eq!(fleet.asks(), json!([]), "no ask card was made");
    assert_eq!(fleet.ana.rows(&fleet.hub), 1);
}

#[test]
fn a_captains_message_again_finds_its_box_when_boxes_at_once_is_full() {
    let mut fleet = Fleet::start("boxes_at_once_captain_again");
    fleet.boxes_at_once(1);
    assert_eq!(fleet.captain_box("r-1").unwrap()["outcome"], "iniciado");
    let again = fleet.captain_box("r-2");
    assert_ne!(again, Err(ONE.to_owned()), "the project's own box is no new one");
    assert_eq!(fleet.ana.rows(&fleet.hub), 1);
}

#[test]
fn run_now_is_the_users_act_so_boxes_at_once_neither_refuses_nor_counts_it() {
    let mut fleet = Fleet::start("boxes_at_once_run_now");
    fleet.boxes_at_once(0);
    assert_eq!(fleet.workflow("run-now:a"), Ok(()), "Run now at 0");
    assert_eq!(fleet.workflow("w-1"), Err(ZERO.to_owned()), "a scheduled or triggered run at 0");
    fleet.boxes_at_once(2);
    assert_eq!(fleet.worker("One").unwrap()["workState"], "working");
    assert_eq!(fleet.worker("Two").unwrap()["workState"], "working", "the Run now box is not counted");
    assert_eq!(fleet.workflow("run-now:b"), Ok(()), "Run now with the agents' boxes full");
    assert_eq!(fleet.worker("Three"), Err(TWO.to_owned()), "two agent boxes, not four");
    assert_eq!(fleet.ana.rows(&fleet.hub), 4);
}
