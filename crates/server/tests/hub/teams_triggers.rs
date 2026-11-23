//! pinned Teams, triggers: a team's runs open only through its bots. `open_placed` refuses a run
//! on a team's account, and what the team's own pollers find goes to `teams::triggers::fired`,
//! which starts a run only for a bot that is on.
//!
//! The hub reaches no fake Linear or Slack (their API addresses are fixed), so each test hands
//! `fired` the `Work` the team's poller would produce for the event.

use super::*;
use pinned_core::connectors::Placement;
use pinned_core::teams::{Bot, BotLimits, CallRules, TeamOp, TeamPolicy};
use pinned_server::hub::Account;
use pinned_server::teams;
use uuid::Uuid;

/// A hub with a team that keeps the Upstash key and Claude token: ana owns it, bob is a member.
struct Crew {
    hub: TestHub,
    team: Uuid,
    ana: Arc<Account>,
    bob: Arc<Account>,
    folder: PathBuf,
}

impl Crew {
    fn new(name: &str) -> Crew {
        Crew::with_policy(name, fast())
    }

    fn with_policy(name: &str, policy: Policy) -> Crew {
        let patient = TeamPolicy { escalate_after_secs: 3600, give_up_after_secs: 7200, ..TeamPolicy::default() };
        let hub = TestHub::start_with_team_policy(name, policy, patient);
        let sign_up = |email: &str| {
            let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
            hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
        };
        let (ana, bob) = (sign_up("ana@example.test"), sign_up("bob@example.test"));
        let made = hub.runtime.block_on(teams::call(hub.hub(), &ana, None, teams::Op::Team(TeamOp::Create { name: "Acme".into() }))).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", bob.id));
        let folder = temp(&format!("teams-triggers-{name}"));
        let crew = Crew { hub, team, ana, bob, folder };
        crew.hub_op(&crew.ana, json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN })).unwrap();
        crew
    }

    fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        let op = teams::Op::read(op)?;
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), op))
    }

    fn hub_op(&self, who: &Arc<Account>, call: Value) -> Result<Value, String> {
        self.call(who, TeamOp::Hub { call })
    }

    /// A bot of a workflow with `trigger`, saved by bob: a draft until someone turns it on.
    fn draft_bot(&self, name: &str, trigger: Value) -> String {
        let id = save_team_workflow(&self.hub, &self.bob, self.team, json!({
            "name": name, "enabled": false, "trigger": trigger,
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
        }))
        .unwrap();
        self.save_bot(&id);
        id
    }

    /// A workflow bob saves with a graph of `steps` after its trigger, one after another, each
    /// `(id, step)` without its name and position. Its trigger watches `watch`.
    fn graph_workflow(&self, name: &str, watch: Value, steps: &[(&str, Value)]) -> String {
        save_team_workflow(&self.hub, &self.bob, self.team, self.graph(name, watch, steps)).unwrap()
    }

    /// The workflow `graph_workflow` saves, as the JSON a save sends.
    fn graph(&self, name: &str, watch: Value, steps: &[(&str, Value)]) -> Value {
        let mut nodes = vec![json!({ "id": "t", "name": "Trigger", "position": [0, 0], "type": "trigger", "watch": watch })];
        let mut edges = vec![];
        let mut from = "t".to_owned();
        for (i, (id, step)) in steps.iter().enumerate() {
            let mut node = step.clone();
            node["id"] = json!(id);
            node["name"] = json!(format!("Step {i}"));
            node["position"] = json!([i + 1, 0]);
            nodes.push(node);
            edges.push(json!({ "from": from, "port": "out", "to": id }));
            from = (*id).to_owned();
        }
        json!({
            "name": name, "enabled": false, "trigger": watch,
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
            "graph": { "nodes": nodes, "edges": edges },
        })
    }

    fn try_save_bot(&self, id: &str) -> Result<Value, String> {
        let bot = Bot { workflow: id.to_owned(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot, connector: None })
    }

    fn try_turn_on(&self, id: &str) -> Result<Value, String> {
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.to_owned(), on: true })
    }

    /// bob saves the bot of workflow `id` again, which puts it back to a draft.
    fn save_bot(&self, id: &str) {
        let bot = Bot { workflow: id.to_owned(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot, connector: None }).unwrap();
    }

    fn turn_on(&self, id: &str, on: bool) {
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.to_owned(), on }).unwrap();
    }

    /// A bot of a Linear label workflow that ana turned on.
    fn on_bot(&self, name: &str) -> String {
        let id = self.draft_bot(name, json!({ "kind": "linear-label", "label": "agent" }));
        self.turn_on(&id, true);
        id
    }

    /// The run the team's Linear poller hands over for issue `issue` labeled for workflow `id`.
    fn labeled(&self, id: &str, issue: &str) -> Work {
        Work::Run(Run {
            reservation: None,
            cwd: self.folder.clone(),
            repo: None,
            args: vec!["--name".into(), "ENG-7 Fix the footer".into(), "Fix it.\n\nLinear issue ENG-7: Fix the footer\nhttps://linear.example.test/acme/issue/ENG-7\n\n(no description)".into()],
            connector: id.to_owned(),
            subject: super::issue(issue),
            mentioned: vec![],
            account: None,
            runs_on: RunsOn::FreshBox(BoxStep { size: Some(BoxSize::Small) }),
            input: "ENG-7: Fix the footer\nhttps://linear.example.test/acme/issue/ENG-7\n\n(no description)".into(),
        })
    }

    fn fired(&self, work: Work) -> Result<Option<Placement>, String> {
        self.hub.runtime.block_on(teams::triggers::fired(self.hub.hub(), self.team, work))
    }

    /// How many `team_runs` rows the team has, whatever their state.
    fn team_runs(&self) -> i64 {
        self.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), self.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.query_one("select count(*) from team_runs where team = $1", &[&self.team]).await.unwrap().get::<_, i64>(0)
        })
    }

    /// The team's runs as a member lists them: (state, started_by) of each, newest first.
    fn runs(&self) -> Vec<(String, String)> {
        let runs = self.call(&self.bob, TeamOp::Runs).unwrap();
        runs.as_array().unwrap().iter().map(|r| (r["state"].as_str().unwrap().to_owned(), r["started_by"].as_str().unwrap().to_owned())).collect()
    }
}

#[test]
fn open_placed_refuses_a_run_on_a_teams_account_and_opens_no_box() {
    let crew = Crew::new("tt_gate");
    let id = crew.on_bot("Footer");

    let refused = crew.hub.runtime.block_on(crew.hub.hub().open_placed(crew.team, crew.labeled(&id, "eng-7")));
    assert_eq!(refused, Err("a team's runs start only through its bots".to_owned()));
    let refused = crew.hub.runtime.block_on(crew.hub.hub().open(crew.team, crew.labeled(&id, "eng-7")));
    assert_eq!(refused, Err("a team's runs start only through its bots".to_owned()));
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new(), "no box opens");
    assert_eq!(crew.runs(), vec![], "no run is recorded");
}

#[test]
fn an_on_bots_linear_label_fires_one_run_in_a_fresh_box() {
    let crew = Crew::new("tt_linear");
    let id = crew.on_bot("Footer");

    let placed = crew.fired(crew.labeled(&id, "eng-7")).unwrap().expect("the run's box took it");
    assert_eq!(crew.runs(), vec![("working".to_owned(), "trigger:linear_label".to_owned())]);
    assert_eq!(crew.hub.boxes.live().len(), 1, "the run opened in a fresh box");
    let row = crew.hub.runtime.block_on(crew.hub.hub().db.boxes(crew.team)).unwrap().into_iter().next().unwrap().unwrap();
    assert_eq!(placed.machine, row.machine.to_string(), "the run is in the team's box");

    let again = crew.fired(crew.labeled(&id, "eng-7"));
    assert_eq!(again, Ok(Some(placed.clone())), "the same event goes to the live run's tab");
    assert_eq!(crew.team_runs(), 1, "the same event starts no second run");
    assert_eq!(crew.runs(), vec![("working".to_owned(), "trigger:linear_label".to_owned())]);
    assert_eq!(crew.hub.boxes.live().len(), 1);
}

#[test]
fn an_event_for_a_run_its_box_has_not_placed_yet_is_given_back_and_starts_nothing() {
    let crew = Crew::new("tt_starting");
    let id = crew.on_bot("Footer");
    let subject = serde_json::to_string(&super::issue("eng-7")).unwrap();
    admin_on(
        &crew.hub.database,
        &format!("insert into team_runs (id, team, workflow, started_by, state, subject) values ('{}', '{}', '{id}', '\"trigger:linear_label\"', 'working', '{subject}')", Uuid::new_v4(), crew.team),
    );

    let again = crew.fired(crew.labeled(&id, "eng-7"));
    assert_eq!(again, Err(format!("the bot of workflow {id} already has a live run on this issue that is still starting; the next check tries again")));
    assert_eq!(crew.team_runs(), 1, "no second run");
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new(), "no box opens");
}

#[test]
fn an_event_for_a_bot_turned_off_is_consumed_and_never_runs_after_it_is_turned_on() {
    let crew = Crew::new("tt_off");
    let id = crew.on_bot("Footer");
    crew.turn_on(&id, false);

    assert_eq!(crew.fired(crew.labeled(&id, "eng-7")), Ok(None), "consumed: the poller does not try it again");
    assert_eq!(crew.runs(), vec![]);
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());

    crew.turn_on(&id, true);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(crew.runs(), vec![], "the consumed event is not picked up once the bot is on");
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());
}

#[test]
fn an_event_for_a_draft_bot_starts_nothing() {
    let crew = Crew::new("tt_draft");
    let id = crew.draft_bot("Footer", json!({ "kind": "linear-label", "label": "agent" }));

    assert_eq!(crew.fired(crew.labeled(&id, "eng-7")), Ok(None));
    assert_eq!(crew.runs(), vec![]);
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());
}

#[test]
fn a_bot_saved_again_is_a_draft_and_its_next_event_starts_nothing() {
    let crew = Crew::new("tt_saved");
    let id = crew.on_bot("Footer");
    crew.save_bot(&id);

    assert_eq!(crew.fired(crew.labeled(&id, "eng-7")), Ok(None));
    assert_eq!(crew.runs(), vec![]);
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());
}

#[test]
fn new_thread_messages_for_a_bot_with_no_live_run_are_dropped_and_start_nothing() {
    let crew = Crew::new("tt_delivery");
    let id = crew.draft_bot("Answer", json!({ "kind": "slack-mention", "channel": "C0123ABCD" }));
    crew.turn_on(&id, true);
    let delivery = Delivery {
        connector: id.clone(),
        channel: "C0123ABCD".into(),
        thread: "1792400050.000200".into(),
        messages: vec!["[Slack reply from Dana] Any news on the footer?".into()],
        cursor: "1792400060.000300".into(),
        mentioned: vec![],
    };

    assert_eq!(crew.fired(Work::Delivery(delivery)), Ok(None), "dropped, so the poller does not open a session for it");
    assert_eq!(crew.runs(), vec![]);
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());
}

#[test]
fn a_manual_bot_gets_no_run_from_its_poller_and_runs_only_by_run_now() {
    let crew = Crew::new("tt_manual");
    let id = crew.draft_bot("Release notes", json!({ "kind": "manual" }));
    let checked = || team_settings(&crew.hub, crew.team)["checked"].clone();
    let before = checked();
    // Turning the bot on saves its workflow, which wakes the team's poller for a check.
    crew.turn_on(&id, true);
    wait_for("a check of the team's poller", || checked() != before && !checked().is_null());
    assert_eq!(checked()["error"], Value::Null, "the poller checks nothing for a manual workflow");
    assert_eq!(crew.runs(), vec![]);
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());

    assert_eq!(crew.fired(crew.labeled(&id, "eng-7")), Ok(None), "a run found for it is consumed");
    assert_eq!(crew.runs(), vec![]);
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());

    crew.call(&crew.bob, TeamOp::RunNow { workflow: id.clone() }).unwrap();
    assert_eq!(crew.runs(), vec![("working".to_owned(), crew.bob.id.to_string())]);
    assert_eq!(crew.hub.boxes.live().len(), 1);
}

#[test]
fn a_bot_whose_workflow_reaches_its_session_twice_is_refused_and_a_one_session_bot_turns_on() {
    let crew = Crew::new("tt_sessions");
    let linear = json!({ "kind": "linear-label", "label": "agent" });
    let talk = |prompt: &str| json!({ "type": "talk", "prompt": prompt });
    let two = crew.graph("Two talks", linear.clone(), &[("a", talk("Look.")), ("b", json!({ "type": "session", "prompt": "Fix." }))]);
    let helper = crew.graph_workflow("Helper", json!({ "kind": "called" }), &[("h", talk("Help."))]);
    let calls = crew.graph("Talk and call", linear.clone(), &[("a", talk("Look.")), ("c", json!({ "type": "call", "workflow": helper }))]);
    let one = crew.graph_workflow("One session", linear, &[("s", json!({ "type": "session", "prompt": "Fix." }))]);
    let refused = Err("a bot's workflow may have only one session or talk step, counting the workflows it calls: a workflow with more is not available as a bot yet".to_owned());

    // A member's save of the workflow with its bot is refused, and nothing is stored.
    for workflow in [&two, &calls] {
        assert_eq!(save_team_workflow(&crew.hub, &crew.bob, crew.team, workflow.clone()).map(drop), refused, "saving the workflow with its bot");
    }
    assert_eq!(team_workflows(&crew.hub, crew.team).len(), 2, "only Helper and One session are stored");
    // Such workflows stored before (as an older hub let a member) are refused as bots.
    let (two, calls) = (store_on_team(&crew.hub, crew.team, two), store_on_team(&crew.hub, crew.team, calls));
    for id in [&two, &calls] {
        assert_eq!(crew.try_save_bot(id).map(drop), refused, "saving the bot");
        assert_eq!(crew.try_turn_on(id).map(drop), refused, "turning it on");
    }
    assert_eq!(crew.call(&crew.bob, TeamOp::Bots).unwrap().as_array().unwrap().iter().filter(|b| b["on"] == json!(true)).count(), 0, "no bot is on");

    assert_eq!(crew.try_save_bot(&one).map(drop), Ok(()));
    assert_eq!(crew.try_turn_on(&one).map(drop), Ok(()), "a one-session workflow still turns on");
    let migrated = crew.on_bot("Footer");
    let on: Vec<String> = crew.call(&crew.bob, TeamOp::Bots).unwrap().as_array().unwrap().iter().filter(|b| b["on"] == json!(true)).map(|b| b["workflow"].as_str().unwrap().to_owned()).collect();
    assert_eq!(on.len(), 2);
    assert!(on.contains(&one) && on.contains(&migrated), "{on:?}");
}

#[test]
fn a_teams_bot_is_held_to_the_hubs_cap_not_to_a_boxes_at_once_it_cannot_set() {
    let crew = Crew::with_policy("tt_team_cap", Policy { max_open: 12, ..fast() });
    let id = crew.on_bot("Footer");
    for n in 0..11 {
        admin_on(
            &crew.hub.database,
            &format!("insert into boxes (id, account, machine, connector, subject, spec, size, phase) values ('held-{n}', '{}', '{}', 'other', '{{\"issue\":\"held-{n}\"}}', '{{\"folder\":\"/w\",\"args\":[]}}', 'small', 'running')", crew.team, Uuid::new_v4()),
        );
    }
    let placed = crew.fired(crew.labeled(&id, "eng-7"));
    assert!(placed.as_ref().is_ok_and(|p| p.is_some()), "the 12th box of a team with 11 open opens: {placed:?}");
    assert_eq!(crew.hub.runtime.block_on(crew.hub.hub().db.boxes(crew.team)).unwrap().len(), 12);
}
