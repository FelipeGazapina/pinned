//! pinned Teams, a flood of events: a bot has at most one live run and a team at most 3. Every
//! other event, a trigger's or a member's Run now, waits in its bot's line, at most 20, and starts
//! when a run ends. A waiting or dropped event is answered to the poller as consumed (`Ok(None)`),
//! so Linear and Slack are left as they are; each event that starts no run is counted on its bot.
//!
//! The hub reaches no fake Linear or Slack (their API addresses are fixed), so each test hands
//! `fired` the `Work` the team's poller would produce for the event; what a poller does with a
//! consumed answer is `a_consumed_run_moves_no_issue_and_reacts_to_nothing` in pinned-core.

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
        let patient = TeamPolicy { escalate_after_secs: 3600, give_up_after_secs: 7200, ..TeamPolicy::default() };
        let hub = TestHub::start_with_team_policy(name, fast(), patient);
        let sign_up = |email: &str| {
            let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
            hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
        };
        let (ana, bob) = (sign_up("ana@example.test"), sign_up("bob@example.test"));
        let made = hub.runtime.block_on(teams::call(hub.hub(), &ana, None, teams::Op::Team(TeamOp::Create { name: "Acme".into() }))).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", bob.id));
        let folder = temp(&format!("teams-flood-{name}"));
        let crew = Crew { hub, team, ana, bob, folder };
        crew.call(&crew.ana, TeamOp::Hub { call: json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN }) }).unwrap();
        crew
    }

    fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        let op = teams::Op::read(op)?;
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), op))
    }

    /// The JSON a member's app sends as a team op.
    fn op(&self, who: &Arc<Account>, op: Value) -> Result<Value, String> {
        self.call(who, serde_json::from_value(op).unwrap())
    }

    /// A bot of a workflow with `trigger` that bob saved and ana turned on.
    fn bot_with(&self, name: &str, trigger: Value) -> String {
        let saved = self
            .call(&self.bob, TeamOp::Hub { call: json!({ "cmd": "save_connector", "connector": {
                "name": name, "enabled": false, "trigger": trigger,
                "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
                "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
            } }) })
            .unwrap();
        let id = saved["id"].as_str().unwrap().to_owned();
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot }).unwrap();
        self.turn_on(&id, true);
        id
    }

    /// A bot of a Linear label workflow that ana turned on.
    fn on_bot(&self, name: &str) -> String {
        self.bot_with(name, json!({ "kind": "linear-label", "label": "agent" }))
    }

    fn turn_on(&self, id: &str, on: bool) {
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.to_owned(), on }).unwrap();
    }

    /// The run the team's Linear poller hands over for issue `issue` (`eng-3` is "ENG-3 Fix page 3").
    fn labeled(&self, id: &str, issue: &str) -> Work {
        let n = issue.trim_start_matches("eng-");
        Work::Run(Run {
            cwd: self.folder.clone(),
            repo: None,
            args: vec!["--name".into(), format!("ENG-{n} Fix page {n}")],
            connector: id.to_owned(),
            subject: super::issue(issue),
            mentioned: vec![],
            account: None,
            runs_on: RunsOn::FreshBox(BoxStep { size: Some(BoxSize::Small) }),
            input: format!("ENG-{n}: Fix page {n}\nhttps://linear.example.test/acme/issue/ENG-{n}\n\n(no description)"),
        })
    }

    fn fired(&self, work: Work) -> Result<Option<Placement>, String> {
        self.hub.runtime.block_on(teams::triggers::fired(self.hub.hub(), self.team, work))
    }

    /// The team's runs as a member lists them: (state, started_by) of each, newest first.
    fn runs(&self) -> Vec<(String, String)> {
        let runs = self.call(&self.bob, TeamOp::Runs).unwrap();
        runs.as_array().unwrap().iter().map(|r| (r["state"].as_str().unwrap().to_owned(), r["started_by"].as_str().unwrap().to_owned())).collect()
    }

    /// The ids of the team's live runs of bot `id`.
    fn live_of(&self, id: &str) -> Vec<String> {
        let runs = self.call(&self.bob, TeamOp::Runs).unwrap();
        runs.as_array().unwrap().iter().filter(|r| r["workflow"] == id && r["state"] == "working").map(|r| r["id"].as_str().unwrap().to_owned()).collect()
    }

    fn live(&self) -> usize {
        self.runs().iter().filter(|(state, _)| state == "working").count()
    }

    fn stop(&self, run: &str) {
        self.op(&self.ana, json!({ "op": "stop_run", "run": run })).unwrap();
    }

    /// The name of the box whose tab runs bot `id`'s session on issue `issue`, once one does.
    fn box_of(&self, id: &str, issue: &str) -> Option<String> {
        let team = self.hub.runtime.block_on(self.hub.hub().account(self.team)).unwrap();
        let (machine, _) = team.placed(id, &super::issue(issue))?;
        team.name_of(&machine)
    }

    /// How many events wait in bot `id`'s line.
    fn waiting(&self, id: &str) -> i64 {
        self.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), self.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.query_one("select count(*) from team_run_queue where team = $1 and workflow = $2", &[&self.team, &id]).await.unwrap().get::<_, i64>(0)
        })
    }

    /// The skipped counts of bot `id` as the bot page lists them.
    fn skipped(&self, id: &str) -> Value {
        let bots = self.call(&self.bob, TeamOp::Bots).unwrap();
        bots.as_array().unwrap().iter().find(|b| b["workflow"] == id).unwrap()["skipped"].clone()
    }
}

#[test]
fn five_events_for_one_bot_start_one_run_and_the_next_starts_with_its_own_event() {
    let crew = Crew::new("tf_line");
    let id = crew.on_bot("Pages");

    assert!(crew.fired(crew.labeled(&id, "eng-1")).unwrap().is_some(), "the first event starts a run");
    for n in 2..=5 {
        assert_eq!(crew.fired(crew.labeled(&id, &format!("eng-{n}"))), Ok(None), "event {n} waits: consumed for now");
    }
    assert_eq!(crew.runs(), vec![("working".to_owned(), "trigger:linear_label".to_owned())], "one live run");
    assert_eq!(crew.waiting(&id), 4);
    assert_eq!(crew.hub.boxes.live().len(), 1, "one box");
    wait_for("the first run's tab", || crew.box_of(&id, "eng-1").is_some());
    assert_eq!(crew.box_of(&id, "eng-1").as_deref(), Some("Box · ENG-1 Fix page 1"));

    let first = crew.live_of(&id);
    crew.stop(&first[0]);
    wait_for("the next waiting event to start", || crew.box_of(&id, "eng-2").is_some());
    assert_eq!(crew.box_of(&id, "eng-2").as_deref(), Some("Box · ENG-2 Fix page 2"), "the run of the oldest waiting event, with its own text");
    assert_eq!(crew.live(), 1, "still one live run");
    assert_eq!(crew.waiting(&id), 3);
    assert_eq!(crew.box_of(&id, "eng-3"), None);
}

#[test]
fn a_run_now_behind_a_live_run_waits_and_the_member_is_told() {
    let crew = Crew::new("tf_run_now");
    let id = crew.on_bot("Pages");
    assert!(crew.fired(crew.labeled(&id, "eng-1")).unwrap().is_some());

    let reply = crew.op(&crew.bob, json!({ "op": "run_now", "workflow": id })).unwrap();
    assert_eq!(
        reply,
        json!({ "waiting": true, "workflow": id, "place": 1, "message": "This bot is busy or the team has as many runs going as it may: your run waits in line (number 1) and starts when one ends." })
    );
    assert_eq!(crew.runs(), vec![("working".to_owned(), "trigger:linear_label".to_owned())], "no second run");
    assert_eq!(crew.hub.boxes.live().len(), 1);

    let first = crew.live_of(&id);
    crew.stop(&first[0]);
    wait_for("bob's run to start", || crew.runs().first() == Some(&("working".to_owned(), crew.bob.id.to_string())));
    assert_eq!(crew.live(), 1);
    assert_eq!(crew.waiting(&id), 0);
}

#[test]
fn a_fourth_run_on_a_team_waits_until_one_of_three_ends() {
    let crew = Crew::new("tf_cap");
    let bots: Vec<String> = (1..=4).map(|n| crew.on_bot(&format!("Bot {n}"))).collect();
    for (n, id) in bots.iter().enumerate().take(3) {
        assert!(crew.fired(crew.labeled(id, &format!("eng-{}", n + 1))).unwrap().is_some(), "run {} starts", n + 1);
    }
    assert_eq!(crew.fired(crew.labeled(&bots[3], "eng-4")), Ok(None), "the 4th waits");
    assert_eq!(crew.live(), 3);
    assert_eq!(crew.waiting(&bots[3]), 1);
    assert_eq!(crew.hub.boxes.live().len(), 3);

    let second = crew.live_of(&bots[1]);
    crew.stop(&second[0]);
    wait_for("the 4th run to start", || crew.box_of(&bots[3], "eng-4").is_some());
    assert_eq!(crew.box_of(&bots[3], "eng-4").as_deref(), Some("Box · ENG-4 Fix page 4"));
    assert_eq!(crew.live(), 3, "still 3 live runs");
    assert_eq!(crew.waiting(&bots[3]), 0);
}

#[test]
fn a_21st_waiting_event_drops_the_oldest_and_counts_it() {
    let crew = Crew::new("tf_full");
    let id = crew.on_bot("Pages");
    assert!(crew.fired(crew.labeled(&id, "eng-1")).unwrap().is_some());
    for n in 2..=22 {
        assert_eq!(crew.fired(crew.labeled(&id, &format!("eng-{n}"))), Ok(None), "event {n} is answered as consumed");
    }
    assert_eq!(crew.waiting(&id), 20);
    assert_eq!(crew.skipped(&id), json!([{ "reason": "queue_full", "count": 1 }]));
    assert_eq!(crew.live(), 1);

    let first = crew.live_of(&id);
    crew.stop(&first[0]);
    wait_for("the next waiting event to start", || crew.box_of(&id, "eng-3").is_some());
    assert_eq!(crew.box_of(&id, "eng-2"), None, "eng-2 was the oldest waiting, dropped: it never runs");
    assert_eq!(crew.box_of(&id, "eng-3").as_deref(), Some("Box · ENG-3 Fix page 3"));
}

#[test]
fn events_that_start_no_run_are_counted_by_why() {
    let crew = Crew::new("tf_counts");
    let off = crew.on_bot("Off");
    crew.turn_on(&off, false);
    let manual = crew.bot_with("Manual", json!({ "kind": "manual" }));

    assert_eq!(crew.fired(crew.labeled(&off, "eng-1")), Ok(None));
    assert_eq!(crew.fired(crew.labeled(&off, "eng-2")), Ok(None));
    assert_eq!(crew.fired(crew.labeled(&manual, "eng-3")), Ok(None));
    assert_eq!(crew.skipped(&off), json!([{ "reason": "off", "count": 2 }]));
    assert_eq!(crew.skipped(&manual), json!([{ "reason": "manual", "count": 1 }]));
    assert_eq!(crew.runs(), vec![]);
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());
}

#[test]
fn a_waiting_event_of_a_bot_turned_off_before_its_turn_is_dropped_and_counted() {
    let crew = Crew::new("tf_off_later");
    let id = crew.on_bot("Pages");
    assert!(crew.fired(crew.labeled(&id, "eng-1")).unwrap().is_some());
    assert_eq!(crew.fired(crew.labeled(&id, "eng-2")), Ok(None));
    assert_eq!(crew.waiting(&id), 1);

    // Turning it off leaves the live run; ending that run is the waiting event's turn.
    crew.turn_on(&id, false);
    let first = crew.live_of(&id);
    crew.stop(&first[0]);
    wait_for("the waiting event to be dropped", || crew.waiting(&id) == 0);
    wait_for("the count", || crew.skipped(&id) == json!([{ "reason": "off", "count": 1 }]));
    assert_eq!(crew.live(), 0, "nothing started");
    assert_eq!(crew.box_of(&id, "eng-2"), None);
}
