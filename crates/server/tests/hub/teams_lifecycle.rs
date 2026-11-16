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
        let id = store_on_team(&self.hub, self.team, json!({
            "name": name, "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" }, "repo": "",
        }));
        let calls = CallRules { in_doubt: Who::Group { handle: "design".into() }, ..CallRules::default() };
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls, limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot, connector: None }).unwrap();
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

// ---- 1b. what the reaper spares: each test ends one box in the same tick as its control ----

/// The box rows the reaper tests write straight into the database, beside a fake Upstash box each.
impl Crew {
    /// A hub whose own box pass runs once and then sleeps an hour, so only the calls tick touches the rows.
    fn still(name: &str) -> Crew {
        Crew::new(name, Policy { tick: Duration::from_secs(3600), ..fast() }, patient())
    }

    /// A `boxes` row of `account` in `phase` (and `reason`) for `connector` and the `subject` json, naming the fake Upstash box `upstash`.
    fn insert_box(&self, account: Uuid, id: &str, machine: Uuid, phase: &str, reason: Option<&str>, connector: &str, subject: &str) {
        let reason = reason.map_or("null".to_owned(), |r| format!("'{r}'"));
        self.hub.boxes.add(&format!("up-{id}"), &[]);
        self.sql(&format!(
            "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase, reason)
             values ('{id}', '{account}', '{machine}', 'up-{id}', '{connector}', '{subject}', '{{\"folder\":\"/w\",\"args\":[]}}', 'small', '{phase}', {reason})"
        ));
    }

    /// A live run of the team that holds the box `machine`, or, with none, the one box a run opened for `workflow` and `subject`.
    fn insert_holding_run(&self, workflow: &str, state: &str, machine: Option<Uuid>, subject: &str) {
        let machine = machine.map_or("null".to_owned(), |m| format!("'{m}'"));
        self.sql(&format!(
            "insert into team_runs (id, team, workflow, box, subject, started_by, state, started_at, waiting_since)
             values ('{}', '{}', '{workflow}', {machine}, '{subject}', '\"trigger:linear_label\"', '{state}', now() - interval '1 second', now() - interval '1 second')",
            Uuid::new_v4(),
            self.team
        ));
    }

    /// A running box of the team that no run holds: the control, which the tick must end.
    fn insert_control(&self) {
        self.insert_box(self.team, "ctl", Uuid::new_v4(), "running", None, "wf-ctl", r#"{"issue":"ctl"}"#);
    }

    /// One tick, once the control is gone, which says the tick ran its reaper and its spawned ends finished.
    fn tick_until_control_ends(&self) {
        self.tick();
        wait_for("the control box to be deleted", || !self.hub.boxes.live().contains(&"up-ctl".to_owned()));
        wait_for("the control's row to go", || self.box_rows(self.team).iter().all(|(id, _, _)| id != "ctl"));
        // Each end runs in its own spawned task, so a wrongly ended box may be a moment behind the control.
        std::thread::sleep(Duration::from_millis(500));
    }

    /// The account's `boxes` rows as (id, phase, reason), by id.
    fn box_rows(&self, account: Uuid) -> Vec<(String, String, Option<String>)> {
        let rows = self.hub.runtime.block_on(self.connect().query("select id, phase, reason from boxes where account = $1::text::uuid order by id", &[&account.to_string()])).unwrap();
        rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect()
    }

    fn live_upstash(&self) -> Vec<String> {
        let mut live = self.hub.boxes.live();
        live.sort();
        live
    }
}

#[test]
fn the_reaper_spares_the_box_of_a_live_run() {
    let crew = Crew::still("tl_spares_live");
    let (held, early) = (Uuid::new_v4(), Uuid::new_v4());
    crew.insert_box(crew.team, "held", held, "running", None, "wf-held", r#"{"issue":"h1"}"#);
    crew.insert_holding_run("wf-held", "working", Some(held), r#"{"issue":"h1"}"#);
    // A run's box is set once its session starts: until then the run holds the box opened for its workflow and subject.
    crew.insert_box(crew.team, "early", early, "running", None, "wf-early", r#"{"issue":"e1"}"#);
    crew.insert_holding_run("wf-early", "waiting", None, r#"{"issue":"e1"}"#);
    crew.insert_control();
    assert_eq!(crew.live_upstash(), vec!["up-ctl", "up-early", "up-held"]);

    crew.tick_until_control_ends();
    assert_eq!(crew.live_upstash(), vec!["up-early", "up-held"], "no box of a live run was deleted");
    assert_eq!(
        crew.box_rows(crew.team),
        vec![("early".to_owned(), "running".to_owned(), None), ("held".to_owned(), "running".to_owned(), None)],
        "both rows stay running"
    );
    assert_eq!(crew.runs(), vec![("waiting".to_owned(), None), ("working".to_owned(), None)], "and both runs go on");
}

#[test]
fn the_reaper_spares_a_team_box_still_booting() {
    let crew = Crew::still("tl_spares_booting");
    crew.insert_box(crew.team, "booting", Uuid::new_v4(), "booting", None, "wf-booting", r#"{"issue":"b1"}"#);
    crew.insert_control();
    assert_eq!(crew.live_upstash(), vec!["up-booting", "up-ctl"]);

    crew.tick_until_control_ends();
    assert_eq!(crew.live_upstash(), vec!["up-booting"], "a box still booting was not deleted");
    assert_eq!(crew.box_rows(crew.team), vec![("booting".to_owned(), "booting".to_owned(), None)], "its row stays booting");
}

#[test]
fn the_reaper_spares_every_box_of_a_personal_account() {
    let crew = Crew::still("tl_spares_personal");
    let ana = crew.ana.id;
    // One row in each phase the hub stores: `phase` is creating, booting, running or ending, and an ending row
    // names why. No run holds any of them.
    for (id, phase, reason) in [
        ("p-creating", "creating", None),
        ("p-booting", "booting", None),
        ("p-running", "running", None),
        ("p-ending-idle", "ending", Some("idle")),
        ("p-ending-failed", "ending", Some("failed: the bootstrap failed")),
    ] {
        crew.insert_box(ana, id, Uuid::new_v4(), phase, reason, &format!("c-{id}"), &format!(r#"{{"issue":"{id}"}}"#));
    }
    crew.insert_control();
    assert_eq!(crew.live_upstash().len(), 6);

    crew.tick_until_control_ends();
    assert_eq!(crew.live_upstash(), vec!["up-p-booting", "up-p-creating", "up-p-ending-failed", "up-p-ending-idle", "up-p-running"], "nothing was deleted on the fake Upstash");
    assert_eq!(
        crew.box_rows(ana),
        vec![
            ("p-booting".to_owned(), "booting".to_owned(), None),
            ("p-creating".to_owned(), "creating".to_owned(), None),
            ("p-ending-failed".to_owned(), "ending".to_owned(), Some("failed: the bootstrap failed".to_owned())),
            ("p-ending-idle".to_owned(), "ending".to_owned(), Some("idle".to_owned())),
            ("p-running".to_owned(), "running".to_owned(), None),
        ],
        "every row keeps its phase"
    );
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

// ---- 3. a box that fails to boot ----

#[test]
fn a_run_whose_box_fails_to_boot_fails_with_why() {
    let crew = Crew::new("tl_boot_fails", fast(), patient());
    let id = crew.on_bot("Footer");
    let failing = Exec { exit_code: 22, output: "Downloading pinned-server\n".into(), error: "curl: (22) The requested URL returned error: 404\n".into() };
    crew.hub.boxes.state.lock().unwrap().failing = Some(failing);
    let why = "the bootstrap failed: Downloading pinned-server\ncurl: (22) The requested URL returned error: 404";
    assert_eq!(crew.op(&crew.bob, json!({ "op": "run_now", "workflow": id })), Err(format!("Upstash Box: {why}")));
    assert_eq!(crew.runs(), vec![("failed".to_owned(), Some(format!("its box failed: {why}")))], "the run does not stay working");
    wait_for("the box to be deleted", || crew.hub.boxes.live().is_empty());
}

// ---- 4. a passed call ----

/// bob passes call `call` to @legal, as the app sends it.
fn pass(crew: &Crew, call: &Value) -> Result<Value, String> {
    crew.op(&crew.bob, json!({ "op": "answer_call", "id": call, "answer": { "kind": { "kind": "pass_to", "handle": "legal" } } }))
}

#[test]
fn a_passed_call_gives_up_at_the_first_calls_deadline() {
    let crew = Crew::new("tl_pass_clock", fast(), patient());
    crew.running();
    let asked = crew.ask("Red or blue footer?").unwrap();
    // Opened past its give-up (7200 s): the tick would close it now, and the pass does not restart that.
    crew.sql(&format!("update team_calls set created_at = now() - interval '7300 seconds' where id = '{}'", asked["call"].as_str().unwrap()));
    pass(&crew, &asked["call"]).unwrap();
    let calls = crew.op(&crew.bob, json!({ "op": "calls" })).unwrap();
    let (next, first) = (&calls[0], &calls[1]);
    assert_eq!((&next["asked"], &next["answer"]), (&json!([{ "kind": "group", "handle": "legal" }]), &Value::Null), "the passed call is open");
    assert_eq!(next["created_at"], first["created_at"], "it keeps the first call's clock");

    crew.tick();
    wait_for("the run to give up", || crew.runs()[0].0 != "waiting");
    assert_eq!(crew.runs(), vec![("gave_up".to_owned(), Some(format!("nobody answered call {}", next["id"].as_str().unwrap())))]);
}

#[test]
fn a_stop_between_a_pass_and_its_new_call_leaves_no_open_call() {
    let crew = Crew::new("tl_pass_stop", fast(), patient());
    crew.running();
    let asked = crew.ask("Red or blue footer?").unwrap();
    // The hub's statements share one connection, so a lock held on the pass's insert would hold the
    // Stop's statements behind it too. Instead the Stop's end of the run lands in the database the
    // moment the pass closes the call, before the pass opens its new call.
    crew.sql(
        "create function stop_on_pass() returns trigger language plpgsql as $$ begin
           if new.outcome = 'passed' then
             update team_runs set state = 'stopped', ended_at = now(), outcome = 'stopped by ana' where id = new.run;
           end if;
           return new;
         end $$;
         create trigger stop_on_pass after update on team_calls for each row execute function stop_on_pass();",
    );
    assert_eq!(pass(&crew, &asked["call"]), Err("the call was closed, but its run has ended, so @legal was not asked".into()));
    assert_eq!(crew.runs(), vec![("stopped".to_owned(), Some("stopped by ana".to_owned()))]);
    let design = json!([{ "kind": "group", "handle": "design" }]);
    assert_eq!(crew.calls(), vec![("Red or blue footer?".to_owned(), design, json!({ "kind": "pass_to", "handle": "legal" }))], "no call is open on the ended run");
}
