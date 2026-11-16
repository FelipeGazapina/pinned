//! pinned Teams, runs lifecycle: no run or box is left behind. The calls tick ends a team's box
//! whose run ended, fails a run whose box never started or is gone, and moves on a run left waiting
//! with no open call; a run stopped while its box boots never gets that box; a box that fails to
//! boot fails its run; a passed call keeps its first clocks; and a person's machine keeps no record
//! of its gone sessions.

use super::*;
use pinned_core::teams::{Bot, BotLimits, CallRules, Run as TeamRun, Situation, TeamOp, TeamPolicy, Who};
use pinned_server::hub::Account;
use pinned_server::teams;
use uuid::Uuid;

/// The calls' timings these tests run with: nothing escalates or gives up unless a test says so.
fn patient() -> TeamPolicy {
    TeamPolicy { escalate_after_secs: 3600, give_up_after_secs: 7200, ..TeamPolicy::default() }
}

/// A hub with a team that keeps the Upstash key and Claude token: ana owns it, bob is a member of
/// it and of its group @design.
struct Crew {
    hub: TestHub,
    team: Uuid,
    ana: Arc<Account>,
    bob: Arc<Account>,
    folder: PathBuf,
}

impl Crew {
    fn new(name: &str, policy: Policy, team_policy: TeamPolicy) -> Crew {
        let hub = TestHub::start_with_team_policy(name, policy, team_policy);
        let sign_up = |email: &str| {
            let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
            hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
        };
        let (ana, bob) = (sign_up("ana@example.test"), sign_up("bob@example.test"));
        let made = hub.runtime.block_on(teams::call(hub.hub(), &ana, None, teams::Op::Team(TeamOp::Create { name: "Acme".into() }))).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", bob.id));
        let folder = temp(&format!("teams-lifecycle-{name}"));
        let crew = Crew { hub, team, ana, bob, folder };
        for handle in ["design", "legal"] {
            crew.sql(&format!("insert into team_groups (team, handle) values ('{}', '{handle}')", crew.team));
            crew.sql(&format!("insert into team_group_members (team, handle, member) values ('{}', '{handle}', '{}')", crew.team, crew.bob.id));
        }
        crew.hub_op(&crew.ana, json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN })).unwrap();
        crew
    }

    fn sql(&self, sql: &str) {
        admin_on(&self.hub.database, sql);
    }

    fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        let op = teams::Op::read(op)?;
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), op))
    }

    /// A member's op as the app sends it, as JSON.
    fn op(&self, who: &Arc<Account>, op: Value) -> Result<Value, String> {
        self.call(who, serde_json::from_value(op).unwrap())
    }

    fn hub_op(&self, who: &Arc<Account>, call: Value) -> Result<Value, String> {
        self.call(who, TeamOp::Hub { call })
    }

    /// A bot that is on, asking @design in doubt.
    fn on_bot(&self, name: &str) -> String {
        let saved = self
            .hub_op(&self.bob, json!({ "cmd": "save_connector", "connector": {
                "name": name, "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
                "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
                "instructions": "", "machine": { "kind": "upstash-box", "size": "small" }, "repo": "",
            } }))
            .unwrap();
        let id = saved["id"].as_str().unwrap().to_owned();
        let calls = CallRules { in_doubt: Who::Group { handle: "design".into() }, ..CallRules::default() };
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls, limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None };
        self.call(&self.bob, TeamOp::SaveBot { bot }).unwrap();
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.clone(), on: true }).unwrap();
        id
    }

    fn run_now(&self, workflow: &str) -> TeamRun {
        serde_json::from_value(self.op(&self.bob, json!({ "op": "run_now", "workflow": workflow })).unwrap()).unwrap()
    }

    /// A run of a new bot whose box's session is up.
    fn running(&self) -> TeamRun {
        let id = self.on_bot("Footer");
        let run = self.run_now(&id);
        wait_for("the box's session", || self.node().is_some_and(|n| !n.machine.tabs().is_empty()));
        run
    }

    /// The team's runs as a member lists them: (state, outcome) of each, newest first.
    fn runs(&self) -> Vec<(String, Option<String>)> {
        let runs = self.op(&self.bob, json!({ "op": "runs" })).unwrap();
        runs.as_array().unwrap().iter().map(|r| (r["state"].as_str().unwrap().to_owned(), r["outcome"].as_str().map(str::to_owned))).collect()
    }

    /// The team's calls as a member lists them: (question, asked, answer kind) of each, open first.
    fn calls(&self) -> Vec<(String, Value, Value)> {
        let calls = self.op(&self.bob, json!({ "op": "calls" })).unwrap();
        calls.as_array().unwrap().iter().map(|c| (c["question"].as_str().unwrap().to_owned(), c["asked"].clone(), c["answer"]["kind"].clone())).collect()
    }

    fn node(&self) -> Option<Arc<Node>> {
        self.hub.boxes.state.lock().unwrap().boxes.values().find_map(|b| b.node.clone())
    }

    /// What the box's session sends the hub to ask people, over its machine link.
    fn ask(&self, question: &str) -> Result<Value, String> {
        let op = TeamOp::AskPeople { situation: Situation::InDoubt, question: question.into(), choices: vec![], step: None };
        self.node().expect("a box runs a node").link.as_ref().unwrap().call_hub(HubCall::Team { team: None, op })
    }

    /// One pass of the hub's calls tick, as `teams::spawn_tick` runs it.
    fn tick(&self) {
        self.hub.runtime.block_on(teams::calls::tick(self.hub.hub(), &self.hub.team_policy)).unwrap();
    }

    /// A connection to the hub's database, for what a test reads or holds there.
    fn connect(&self) -> tokio_postgres::Client {
        let url = format!("{}/{}", postgres(), self.hub.database);
        let (client, connection) = self.hub.runtime.block_on(tokio_postgres::connect(&url, tokio_postgres::NoTls)).unwrap();
        self.hub.runtime.spawn(connection);
        client
    }

    /// Holds every write to `table` whose new row matches `when` (SQL on `new`) until the answered
    /// connection runs `select pg_advisory_unlock(<key>)`, so a test orders two things the hub does.
    fn hold(&self, key: i64, table: &str, when: &str) -> tokio_postgres::Client {
        let held = self.connect();
        self.hub.runtime.block_on(held.batch_execute(&format!(
            "select pg_advisory_lock({key});
             create function hold_{table}() returns trigger language plpgsql as $$ begin
               if {when} then perform pg_advisory_lock({key}); perform pg_advisory_unlock({key}); end if;
               return new;
             end $$;
             create trigger hold_{table} before insert or update on {table} for each row execute function hold_{table}();"
        )))
        .unwrap();
        held
    }

    /// The box the run is recorded with.
    fn run_box(&self, run: &str) -> Option<String> {
        let row = self.hub.runtime.block_on(self.connect().query_one("select box from team_runs where id = $1::text::uuid", &[&run])).unwrap();
        row.get(0)
    }

    /// A run the tests write straight into the database: `state`, started `age` ago, in box `machine`.
    fn insert_run(&self, workflow: &str, state: &str, age: &str, machine: Option<&str>) -> Uuid {
        let id = Uuid::new_v4();
        let machine = machine.map_or("null".to_owned(), |m| format!("'{m}'"));
        self.sql(&format!(
            "insert into team_runs (id, team, workflow, box, started_by, state, started_at, waiting_since)
             values ('{id}', '{}', '{workflow}', {machine}, '\"trigger:linear_label\"', '{state}', now() - interval '{age}', now() - interval '{age}')",
            self.team
        ));
        id
    }
}

// ---- 1. the reaper in the calls tick ----

#[test]
fn the_tick_ends_a_team_box_whose_run_ended() {
    let crew = Crew::new("tl_box_run_ended", fast(), patient());
    let run = crew.running();
    crew.tick();
    assert_eq!(crew.hub.boxes.live().len(), 1, "a box whose run works stays");

    crew.sql(&format!("update team_runs set state = 'done', ended_at = now(), outcome = 'its session finished' where id = '{}'", run.id));
    crew.tick();
    wait_for("the box to be deleted", || crew.hub.boxes.live().is_empty());
    assert_eq!(crew.runs(), vec![("done".to_owned(), Some("its session finished".to_owned()))], "the run stays as it ended");
}

#[test]
fn the_tick_ends_a_team_box_that_no_run_holds() {
    let crew = Crew::new("tl_box_no_run", fast(), patient());
    let run = crew.running();
    crew.sql(&format!("delete from team_runs where id = '{}'", run.id));
    crew.tick();
    wait_for("the box to be deleted", || crew.hub.boxes.live().is_empty());
    assert_eq!(crew.runs(), vec![]);
}

#[test]
fn the_tick_fails_a_run_whose_box_never_started() {
    let crew = Crew::new("tl_never_started", fast(), patient());
    let bot = crew.on_bot("Footer");
    crew.insert_run(&bot, "working", "2 hours", None);
    crew.tick();
    assert_eq!(crew.runs(), vec![("failed".to_owned(), Some("its box never started".to_owned()))]);

    crew.insert_run(&bot, "working", "1 minute", None);
    crew.tick();
    assert_eq!(crew.runs()[0], ("working".to_owned(), None), "a run whose box may still boot goes on");
    assert_eq!(crew.hub.boxes.state.lock().unwrap().created.len(), 0, "the fake box API was asked for nothing");
}

#[test]
fn the_tick_fails_a_run_whose_box_is_gone() {
    let crew = Crew::new("tl_box_gone", fast(), patient());
    let bot = crew.on_bot("Footer");
    crew.insert_run(&bot, "working", "1 minute", Some(&Uuid::new_v4().to_string()));
    crew.tick();
    assert_eq!(crew.runs(), vec![("failed".to_owned(), Some("its box is gone".to_owned()))]);
    let stopped = crew.op(&crew.ana, json!({ "op": "stop_run", "run": crew.op(&crew.bob, json!({ "op": "runs" })).unwrap()[0]["id"] }));
    assert_eq!(stopped, Err("this run has ended".into()));
}

#[test]
fn the_tick_moves_a_run_left_waiting_with_no_open_call_back_to_work() {
    let crew = Crew::new("tl_stale_wait", fast(), patient());
    let run = crew.running();
    crew.ask("Red or blue footer?").unwrap();
    let (old_wait, new_wait) = ("waiting_since = now() - interval '1 hour'", "waiting_since = now()");
    let wait = |since: &str| crew.sql(&format!("update team_runs set {since} where id = '{}'", run.id));
    wait(old_wait);
    crew.tick();
    assert_eq!(crew.runs(), vec![("waiting".to_owned(), None)], "a run whose call is open waits on");

    // The call closed as a give-up whose run never ended, as when the hub restarted in between.
    crew.sql(&format!("update team_calls set answered_at = now(), outcome = 'gave-up' where run = '{}'", run.id));
    crew.tick();
    assert_eq!(crew.runs(), vec![("waiting".to_owned(), None)], "a call that just closed may still move its run on");

    crew.sql(&format!("update team_calls set answered_at = now() - interval '1 hour' where run = '{}'", run.id));
    wait(new_wait);
    crew.tick();
    assert_eq!(crew.runs(), vec![("waiting".to_owned(), None)], "a run that just began to wait may be opening its call");

    wait(old_wait);
    crew.tick();
    assert_eq!(crew.runs(), vec![("working".to_owned(), None)]);
    assert_eq!(crew.hub.boxes.live().len(), 1, "its box goes on");
    let asked = crew.ask("Red or blue header?").unwrap();
    assert_eq!(asked["asked"], json!(["@design"]), "it can ask again");
}

#[test]
fn the_tick_ends_a_run_left_waiting_whose_box_is_gone() {
    let crew = Crew::new("tl_stale_wait_gone", fast(), patient());
    let bot = crew.on_bot("Footer");
    crew.insert_run(&bot, "waiting", "1 hour", Some(&Uuid::new_v4().to_string()));
    crew.tick();
    assert_eq!(crew.runs(), vec![("failed".to_owned(), Some("its box is gone".to_owned()))]);
}

// ---- 2. a run stopped while its box boots ----

#[test]
fn a_run_stopped_while_its_box_boots_stays_stopped_and_its_box_ends_without_a_session() {
    let crew = Crew::new("tl_stop_booting", fast(), patient());
    let id = crew.on_bot("Footer");
    let gate = Arc::new(tokio::sync::Notify::new());
    crew.hub.boxes.state.lock().unwrap().gate = Some(gate.clone());
    let (hub, bob, team, workflow) = (crew.hub.hub().clone(), crew.bob.clone(), crew.team, id.clone());
    let started = crew.hub.runtime.spawn(async move { teams::call(&hub, &bob, Some(team), teams::Op::Team(TeamOp::RunNow { workflow })).await });
    wait_for("the box's bootstrap to start", || crew.hub.boxes.state.lock().unwrap().scripts.iter().any(|s| s.contains("start.sh")));
    let runs = crew.op(&crew.bob, json!({ "op": "runs" })).unwrap();
    assert_eq!((&runs[0]["state"], &runs[0]["outcome"]), (&json!("working"), &Value::Null), "the run is listed while its box boots");

    let stopped = crew.op(&crew.ana, json!({ "op": "stop_run", "run": runs[0]["id"] })).unwrap();
    assert_eq!((&stopped["state"], &stopped["outcome"]), (&json!("stopped"), &json!("stopped by ana")));
    gate.notify_one();
    let answered = crew.hub.runtime.block_on(started).unwrap();
    // Only the box's claim of its run answers this, and it comes before the session's run is sent.
    assert_eq!(answered, Err("Upstash Box: the run ended before its box started, so its box was ended".into()), "no session opened");

    wait_for("the box to be deleted", || crew.hub.boxes.live().is_empty());
    assert_eq!(crew.runs(), vec![("stopped".to_owned(), Some("stopped by ana".to_owned()))]);
    assert_eq!(crew.run_box(runs[0]["id"].as_str().unwrap()), None, "the run never got the box");

    crew.run_now(&id);
    wait_for("the next run's session, with no cooldown", || crew.node().is_some_and(|n| !n.machine.tabs().is_empty()));
}
