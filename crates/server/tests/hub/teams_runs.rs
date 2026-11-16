//! pinned Teams, runs: every way a run ends, the box a run is recorded with before its session
//! starts, bots turned off when their project leaves the team, and the ask reply a box reads.

use super::*;
use pinned_core::teams::{Bot, BotLimits, CallRules, Run as TeamRun, Situation, TeamOp, TeamPolicy, Who};
use pinned_server::hub::Account;
use pinned_server::teams;
use uuid::Uuid;

const SITE: &str = "github.com/acme/site";
const DOCS: &str = "github.com/acme/docs";

/// A hub with a team that keeps the Upstash key and Claude token: ana owns it, bob is a member.
struct Crew {
    hub: TestHub,
    team: Uuid,
    ana: Arc<Account>,
    bob: Arc<Account>,
    folder: PathBuf,
}

impl Crew {
    fn new(name: &str, policy: Policy) -> Crew {
        let patient = TeamPolicy { escalate_after_secs: 3600, give_up_after_secs: 7200, ..TeamPolicy::default() };
        let hub = TestHub::start_with_team_policy(name, policy, patient);
        let sign_up = |email: &str| {
            let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
            hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
        };
        let (ana, bob) = (sign_up("ana@example.test"), sign_up("bob@example.test"));
        let made = hub.runtime.block_on(teams::call(hub.hub(), &ana, None, TeamOp::Create { name: "Acme".into() })).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", bob.id));
        let folder = temp(&format!("teams-runs-{name}"));
        let crew = Crew { hub, team, ana, bob, folder };
        crew.group("design", &crew.bob);
        crew.hub_op(&crew.ana, json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN })).unwrap();
        crew
    }

    fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), op))
    }

    fn op(&self, who: &Arc<Account>, op: Value) -> Result<Value, String> {
        self.call(who, serde_json::from_value(op).unwrap())
    }

    fn hub_op(&self, who: &Arc<Account>, call: Value) -> Result<Value, String> {
        self.call(who, TeamOp::Hub { call })
    }

    /// A bot that is on, its workflow working in `repo` (empty: no project), asking @design in doubt.
    fn on_bot(&self, name: &str, repo: &str, once: bool) -> String {
        let saved = self
            .hub_op(&self.bob, json!({ "cmd": "save_connector", "connector": {
                "name": name, "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
                "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
                "instructions": "", "machine": { "kind": "upstash-box", "size": "small" }, "repo": repo,
            } }))
            .unwrap();
        let id = saved["id"].as_str().unwrap().to_owned();
        let calls = CallRules { in_doubt: Who::Group { handle: "design".into() }, ..CallRules::default() };
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls, limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once, off_note: None };
        self.call(&self.bob, TeamOp::SaveBot { bot }).unwrap();
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.clone(), on: true }).unwrap();
        id
    }

    fn group(&self, handle: &str, member: &Arc<Account>) {
        admin_on(&self.hub.database, &format!("insert into team_groups (team, handle) values ('{}', '{handle}')", self.team));
        admin_on(&self.hub.database, &format!("insert into team_group_members (team, handle, member) values ('{}', '{handle}', '{}')", self.team, member.id));
    }

    fn run_now(&self, workflow: &str) -> TeamRun {
        serde_json::from_value(self.call(&self.bob, TeamOp::RunNow { workflow: workflow.into() }).unwrap()).unwrap()
    }

    /// The team's runs as a member lists them: (state, outcome) of each, newest first.
    fn runs(&self) -> Vec<(String, Option<String>)> {
        let runs = self.op(&self.bob, json!({ "op": "runs" })).unwrap();
        runs.as_array().unwrap().iter().map(|r| (r["state"].as_str().unwrap().to_owned(), r["outcome"].as_str().map(str::to_owned))).collect()
    }

    /// The bots as a member lists them: (name, on, off_note).
    fn bots(&self) -> Vec<(String, bool, Option<String>)> {
        let bots: Vec<Bot> = serde_json::from_value(self.call(&self.bob, TeamOp::Bots).unwrap()).unwrap();
        bots.into_iter().map(|b| (b.name, b.on, b.off_note)).collect()
    }

    /// The node of the team's one box, once it runs one.
    fn node(&self) -> Option<Arc<Node>> {
        self.hub.boxes.state.lock().unwrap().boxes.values().find_map(|b| b.node.clone())
    }

    /// What the box's session sends the hub to ask people, over its machine link.
    fn ask(&self, question: &str) -> Result<Value, String> {
        let op = TeamOp::AskPeople { situation: Situation::InDoubt, question: question.into(), choices: vec![], step: None };
        self.node().expect("a box runs a node").link.as_ref().unwrap().call_hub(HubCall::Team { team: None, op })
    }
}

#[test]
fn a_run_once_bot_is_gone_after_its_run_is_stopped() {
    let crew = Crew::new("tr_once_stop", fast());
    let once = crew.on_bot("Once", "", true);
    crew.on_bot("Keep", "", false);
    let run = crew.run_now(&once);
    wait_for("the run's box", || crew.hub.boxes.live().len() == 1);

    let stopped = crew.op(&crew.ana, json!({ "op": "stop_run", "run": run.id })).unwrap();
    assert_eq!((&stopped["state"], &stopped["outcome"]), (&json!("stopped"), &json!("stopped by ana")));
    assert_eq!(crew.bots(), vec![("Keep".to_owned(), true, None)], "the run-once bot and its workflow are gone");
    wait_for("the box to be deleted", || crew.hub.boxes.live().is_empty());
    assert_eq!(crew.runs(), vec![("stopped".to_owned(), Some("stopped by ana".to_owned()))]);
}

#[test]
fn a_box_that_ends_at_max_life_while_its_run_waits_ends_the_run_and_closes_its_call() {
    let short = Policy { max_life: Duration::from_secs(5), ..fast() };
    let crew = Crew::new("tr_max_life", short);
    let id = crew.on_bot("Footer", "", false);
    crew.run_now(&id);
    let asked = crew.ask("Red or blue footer?").unwrap();
    assert_eq!(asked["asked"], json!(["@design"]));
    assert_eq!(crew.runs(), vec![("waiting".to_owned(), None)]);

    wait_long("the box to end at its max life", Duration::from_secs(15), || crew.hub.boxes.live().is_empty());
    wait_for("the run to end", || crew.runs()[0].0 != "waiting");
    assert_eq!(crew.runs(), vec![("failed".to_owned(), Some("its box ran out of time or budget".to_owned()))]);
    let calls = crew.op(&crew.bob, json!({ "op": "calls" })).unwrap();
    let call = calls.as_array().unwrap().iter().find(|c| c["id"] == asked["call"]).unwrap().clone();
    assert_eq!((&call["answer"]["kind"], &call["answer"]["text"]), (&json!({ "kind": "stop" }), &json!("its box ran out of time or budget")));
    let run = crew.op(&crew.bob, json!({ "op": "runs" })).unwrap()[0]["id"].clone();
    assert_eq!(crew.op(&crew.ana, json!({ "op": "stop_run", "run": run })), Err("this run has ended".into()));
}

/// Starts a run of a new bot and answers its box's node, the box's machine id and its session's tab.
fn running_session(crew: &Crew) -> (Arc<Node>, String, String) {
    let id = crew.on_bot("Footer", "", false);
    crew.run_now(&id);
    let mut tab = String::new();
    wait_for("the box's session", || {
        tab = crew.node().and_then(|n| n.machine.tabs().first().map(|t| t.id.clone())).unwrap_or_default();
        !tab.is_empty()
    });
    let creds = crew.hub.boxes.state.lock().unwrap().boxes.values().find_map(|b| b.files.get(&format!("{BOX_DIR}/account.json")).cloned()).unwrap();
    let creds: Credentials = serde_json::from_str(&creds).unwrap();
    (crew.node().unwrap(), creds.machine_id, tab)
}

/// The status of `tab` as the hub last heard it from the box's `machine`.
fn hub_status(crew: &Crew, machine: &str, tab: &str) -> Option<pinned_core::agent::AgentStatus> {
    let team = crew.hub.runtime.block_on(crew.hub.hub().account(crew.team)).unwrap();
    team.tabs_of(machine)?.into_iter().find(|t| t.id == tab)?.status
}

/// Claude's Stop hook for `tab`, through the node's hook socket as the session's hook sends it:
/// the session ended its own turn. Waits until the hub has heard it.
fn turn_ends(crew: &Crew, node: &Node, machine: &str, tab: &str) {
    use std::io::Write;
    node.machine.listen(|_| Err("not in this test".into())).unwrap();
    let event = pinned_core::agent::AgentEvent { tab: Some(tab.into()), ..pinned_core::hooks::event_from(Some("done"), "{}").unwrap() };
    let sock = crew.hub.boxes.root.join(&crew.hub.boxes.live()[0]).join("sock");
    let mut stream = std::os::unix::net::UnixStream::connect(sock).unwrap();
    writeln!(stream, "{}", serde_json::to_string(&pinned_core::agent::Message::Agent(event)).unwrap()).unwrap();
    wait_for("the hub to hear the turn end", || hub_status(crew, machine, tab) == Some(pinned_core::agent::AgentStatus::Done));
}

#[test]
fn a_session_that_finishes_normally_ends_its_run_as_done() {
    let crew = Crew::new("tr_done", fast());
    let (node, machine, tab) = running_session(&crew);
    turn_ends(&crew, &node, &machine, &tab);
    assert!(node.machine.close_tab(&tab), "the session ends");
    wait_long("the box to end", Duration::from_secs(15), || crew.hub.boxes.live().is_empty());
    wait_for("the run to end", || crew.runs()[0].0 != "working");
    assert_eq!(crew.runs(), vec![("done".to_owned(), Some("its session finished".to_owned()))]);
}

#[test]
fn a_tab_that_goes_away_mid_turn_fails_its_run() {
    let crew = Crew::new("tr_tab_gone", fast());
    let (node, _, tab) = running_session(&crew);
    assert!(node.machine.close_tab(&tab), "the tab goes, with no Stop hook and no person closing it");
    wait_long("the box to end", Duration::from_secs(15), || crew.hub.boxes.live().is_empty());
    wait_for("the run to end", || crew.runs()[0].0 != "working");
    assert_eq!(crew.runs(), vec![("failed".to_owned(), Some("its session's tab went away before it finished".to_owned()))]);
}

#[test]
fn a_tab_a_person_closes_mid_turn_stops_its_run() {
    let crew = Crew::new("tr_tab_closed", fast());
    let (_, machine, tab) = running_session(&crew);
    let team = crew.hub.runtime.block_on(crew.hub.hub().account(crew.team)).unwrap();
    let close = Call::CloseTab { tab };
    crew.hub.runtime.block_on(crew.hub.hub().call_machine(&team, &machine, close, Duration::from_secs(10))).map_err(String::from).unwrap();
    wait_long("the box to end", Duration::from_secs(15), || crew.hub.boxes.live().is_empty());
    wait_for("the run to end", || crew.runs()[0].0 != "working");
    assert_eq!(crew.runs(), vec![("stopped".to_owned(), Some("its session's tab was closed before it finished".to_owned()))]);
}

#[test]
fn a_session_that_asks_as_its_first_act_gets_a_call() {
    let crew = Crew::new("tr_first_ask", fast());
    let id = crew.on_bot("Footer", "", false);
    // Run now records the run's box only after its box moves to running, which comes after the
    // session started. That move waits here, on a lock this test holds until the session has asked,
    // so the ask always comes before Run now has answered, whatever the timing.
    const HOLD: i64 = 7_031;
    let url = format!("{}/{}", postgres(), crew.hub.database);
    let (held, connection) = crew.hub.runtime.block_on(tokio_postgres::connect(&url, tokio_postgres::NoTls)).unwrap();
    crew.hub.runtime.spawn(connection);
    crew.hub.runtime.block_on(held.batch_execute(&format!(
        "select pg_advisory_lock({HOLD});
         create function hold_running() returns trigger language plpgsql as $$ begin
           if new.phase = 'running' then perform pg_advisory_lock({HOLD}); perform pg_advisory_unlock({HOLD}); end if;
           return new;
         end $$;
         create trigger hold_running before update on boxes for each row execute function hold_running();"
    )))
    .unwrap();
    let (hub, bob, team) = (crew.hub.hub().clone(), crew.bob.clone(), crew.team);
    let started = crew.hub.runtime.spawn(async move { teams::call(&hub, &bob, Some(team), TeamOp::RunNow { workflow: id }).await });
    wait_for("the box's session", || crew.node().is_some_and(|n| !n.machine.tabs().is_empty()));
    let asked = crew.ask("Red or blue footer?").unwrap();
    assert!(!started.is_finished(), "the session asked before Run now answered");
    assert_eq!(asked["asked"], json!(["@design"]));
    assert!(asked["call"].as_str().is_some_and(|c| Uuid::parse_str(c).is_ok()), "a call: {asked}");
    crew.hub.runtime.block_on(held.batch_execute(&format!("select pg_advisory_unlock({HOLD})"))).unwrap();
    let run: TeamRun = serde_json::from_value(crew.hub.runtime.block_on(started).unwrap().unwrap()).unwrap();
    let calls = crew.op(&crew.bob, json!({ "op": "calls" })).unwrap();
    assert_eq!((&calls[0]["run"], &calls[0]["question"]), (&json!(run.id), &json!("Red or blue footer?")));
}

#[test]
fn taking_a_project_out_of_the_team_turns_off_the_bots_that_work_in_it_and_says_why() {
    let crew = Crew::new("tr_detach", fast());
    for (key, name) in [(SITE, "site"), (DOCS, "docs")] {
        let share = json!({ "cmd": "share_project", "project": { "key": key, "name": name, "repo": format!("https://{key}.git") } });
        crew.hub.runtime.block_on(crew.hub.hub().hub_call(&crew.bob, serde_json::from_value(share).unwrap())).unwrap();
        crew.op(&crew.bob, json!({ "op": "attach_project", "key": key })).unwrap();
    }
    let site = crew.on_bot("Site bot", &format!("https://{SITE}"), false);
    crew.on_bot("Docs bot", &format!("https://{DOCS}"), false);

    assert_eq!(crew.op(&crew.bob, json!({ "op": "detach_project", "key": SITE })), Ok(Value::Null));
    let note = "Turned off: project site was taken out of the team.".to_owned();
    assert_eq!(crew.bots(), vec![("Site bot".to_owned(), false, Some(note)), ("Docs bot".to_owned(), true, None)]);
    assert_eq!(crew.call(&crew.bob, TeamOp::RunNow { workflow: site.clone() }).unwrap_err(), "this bot is a draft: it has to be turned on before it can run");

    crew.call(&crew.ana, TeamOp::TurnBotOn { workflow: site, on: true }).unwrap();
    assert_eq!(crew.bots(), vec![("Site bot".to_owned(), true, None), ("Docs bot".to_owned(), true, None)], "turned on again, the note goes");
}

#[test]
fn the_hubs_ask_reply_is_what_a_box_session_reads() {
    let crew = Crew::new("tr_ask_reply", fast());
    let id = crew.on_bot("Footer", "", false);
    crew.run_now(&id);
    wait_for("the box's node", || crew.node().is_some());
    let reply = crew.ask("Red or blue footer?").unwrap();
    let keys: Vec<&str> = reply.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["call", "asked"]);
    let read = pinned_core::team_call::parse_asked(&reply.to_string()).unwrap();
    assert_eq!((read.id.as_str(), read.who), (reply["call"].as_str().unwrap(), vec!["@design".to_owned()]));
}

#[test]
fn taking_a_project_out_of_the_team_stops_the_live_runs_of_its_bots_and_ends_their_boxes() {
    let crew = Crew::new("tr_detach_runs", fast());
    for (key, name) in [(SITE, "site"), (DOCS, "docs")] {
        let share = json!({ "cmd": "share_project", "project": { "key": key, "name": name, "repo": format!("https://{key}.git") } });
        crew.hub.runtime.block_on(crew.hub.hub().hub_call(&crew.bob, serde_json::from_value(share).unwrap())).unwrap();
        crew.op(&crew.bob, json!({ "op": "attach_project", "key": key })).unwrap();
    }
    let site = crew.on_bot("Site bot", &format!("https://{SITE}"), false);
    let docs = crew.on_bot("Docs bot", &format!("https://{DOCS}"), false);
    let site_run = crew.run_now(&site);
    wait_for("the site run's node", || crew.node().is_some());
    let asked = crew.ask("Red or blue footer?").unwrap();
    let site_box = crew.hub.boxes.live();
    let docs_run = crew.run_now(&docs);
    wait_for("both boxes", || crew.hub.boxes.live().len() == 2);
    let state = |id: &str| {
        let runs = crew.op(&crew.bob, json!({ "op": "runs" })).unwrap();
        let run = runs.as_array().unwrap().iter().find(|r| r["id"] == json!(id)).unwrap().clone();
        (run["state"].as_str().unwrap().to_owned(), run["outcome"].as_str().map(str::to_owned))
    };
    assert_eq!(state(&site_run.id).0, "waiting");

    assert_eq!(crew.op(&crew.bob, json!({ "op": "detach_project", "key": SITE })), Ok(Value::Null));
    let outcome = "Stopped: project site was taken out of the team.".to_owned();
    assert_eq!(state(&site_run.id), ("stopped".to_owned(), Some(outcome.clone())));
    wait_for("the site run's box to be deleted", || crew.hub.boxes.live().len() == 1);
    assert!(!crew.hub.boxes.live().contains(&site_box[0]), "the site run's box is the one gone");
    let calls = crew.op(&crew.bob, json!({ "op": "calls" })).unwrap();
    let call = calls.as_array().unwrap().iter().find(|c| c["id"] == asked["call"]).unwrap().clone();
    assert_eq!((&call["answer"]["kind"], &call["answer"]["text"]), (&json!({ "kind": "stop" }), &json!(outcome)));
    assert_eq!(state(&docs_run.id), ("working".to_owned(), None), "a run of another project's bot goes on");
}
