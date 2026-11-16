//! The hub's lock waits: a lock another connection holds (another hub during a deploy, or a slow
//! transaction) never stalls the hub. Each test holds a lock on a Postgres connection of its own,
//! then checks that an unrelated call of another account is still answered within 5 s, and that
//! the call that needs the lock is refused, or gets its try-lock answer, within 5 s too.
use super::teams_admission::{none, Crew};
use super::*;
use pinned_core::teams::TeamOp;
use pinned_server::teams;
use uuid::Uuid;

/// What a call refused for a lock it waited on too long answers.
const BUSY: &str = "the hub is busy with this right now; try again in a moment";
/// How long the hub may take to answer while a lock is held.
const ANSWER: Duration = Duration::from_secs(5);

/// A transaction on a connection of the test's own that holds what `statements` lock, until dropped.
pub(super) struct Held {
    runtime: tokio::runtime::Runtime,
    db: Option<tokio_postgres::Client>,
}

impl Held {
    pub(super) fn new(hub: &TestHub, statements: &str) -> Held {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let url = format!("{}/{}", postgres(), hub.database);
        let db = runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.batch_execute(&format!("begin; {statements}")).await.unwrap();
            db
        });
        Held { runtime, db: Some(db) }
    }

    /// How many server processes of the hub's database wait on a lock now.
    pub(super) fn waiters(&self) -> i64 {
        let db = self.db.as_ref().unwrap();
        self.runtime.block_on(db.batch_execute("select pg_stat_clear_snapshot()")).unwrap();
        self.runtime.block_on(db.query_one("select count(*) from pg_stat_activity where datname = current_database() and wait_event_type = 'Lock'", &[])).unwrap().get(0)
    }

    /// Ends the transaction, and with it what it held.
    pub(super) fn release(mut self) {
        let db = self.db.take().unwrap();
        self.runtime.block_on(db.batch_execute("rollback")).unwrap();
    }
}

/// Runs `call` on the hub's runtime and answers it, or fails the test when it takes over 5 s.
fn within<T>(hub: &TestHub, what: &str, call: impl std::future::Future<Output = T>) -> T {
    hub.runtime.block_on(async { tokio::time::timeout(ANSWER, call).await }).unwrap_or_else(|_| panic!("{what} was not answered within 5 s"))
}

/// A person who is in no team, as the hub knows it.
pub(super) fn person(hub: &TestHub, email: &str) -> Arc<pinned_server::hub::Account> {
    let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
    hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
}

/// Another account's own calls, on the shared statement connection (its schedules) and on the
/// shared transaction connection (its session marks), are answered within 5 s.
pub(super) fn others_answered(hub: &TestHub, carl: &Arc<pinned_server::hub::Account>) {
    let hub_arc = hub.hub().clone();
    let listed = within(hub, "another account's schedules", hub_arc.hub_call(carl, serde_json::from_value(json!({ "cmd": "list_schedules" })).unwrap()));
    assert_eq!(listed, Ok(json!([])));
    let marked = within(hub, "another account's session mark", hub_arc.hub_call(carl, serde_json::from_value(json!({ "cmd": "mark_sessions", "changes": [{ "session": "c1", "mark": { "pin": 0 } }] })).unwrap()));
    assert_eq!(marked, Ok(json!([{ "session": "c1", "pin": 0 }])));
}

/// Starts team op `op` of `who` on the hub's runtime without waiting for it.
fn spawn_op(crew: &Crew, who: &Arc<pinned_server::hub::Account>, op: TeamOp) -> tokio::task::JoinHandle<Result<Value, String>> {
    let (hub, who, team) = (crew.hub.hub().clone(), who.clone(), crew.team);
    crew.hub.runtime.spawn(async move { teams::call(&hub, &who, Some(team), teams::Op::read(op)?).await })
}

/// Waits until a statement of the hub waits on the lock `held` holds.
fn waiting_on(held: &Held) {
    wait_for("the hub's statement to wait on the held lock", || held.waiters() >= 1);
}

/// The team's turn lock (`queue::TURN_KEY`) as another hub holds it while it decides.
fn turn_of(team: Uuid) -> String {
    format!("select pg_advisory_lock(hashtextextended('pinned-team-runs:' || '{team}'::uuid::text, 0))")
}

// ---- the team's turn ----

#[test]
fn run_now_is_refused_while_another_hub_holds_the_teams_turn_and_the_hub_answers_others() {
    let crew = Crew::new("tl_turn");
    let carl = person(&crew.hub, "carl@example.test");
    let bot = crew.on_bot("Pages");
    let held = Held::new(&crew.hub, &turn_of(crew.team));

    let asked = spawn_op(&crew, &crew.bob, TeamOp::RunNow { workflow: bot.clone() });
    waiting_on(&held);
    others_answered(&crew.hub, &carl);
    assert_eq!(within(&crew.hub, "Run now", asked).unwrap(), Err(BUSY.to_owned()));
    assert_eq!(crew.live(), 0, "no run was reserved");
    held.release();

    let started = crew.call(&crew.bob, TeamOp::RunNow { workflow: bot.clone() }).unwrap();
    assert_eq!((started["workflow"].as_str(), started["state"].as_str()), (Some(bot.as_str()), Some("working")), "the next Run now starts");
}

#[test]
fn run_nows_queued_behind_a_turn_another_hub_holds_are_all_refused_within_three_seconds() {
    let crew = Crew::new("tl_turn_wait");
    let bot = crew.on_bot("Pages");
    let held = Held::new(&crew.hub, &turn_of(crew.team));
    let since = Instant::now();
    let asked: Vec<_> = [&crew.bob, &crew.ana, &crew.bob].into_iter().map(|who| spawn_op(&crew, who, TeamOp::RunNow { workflow: bot.clone() })).collect();
    for (n, ask) in asked.into_iter().enumerate() {
        assert_eq!(within(&crew.hub, "a Run now", ask).unwrap(), Err(BUSY.to_owned()), "Run now {n}");
    }
    assert!(since.elapsed() < Duration::from_secs(3), "all three were refused within 3 s, not {:?}", since.elapsed());
    held.release();
}

#[test]
fn the_teams_turn_is_not_held_while_a_box_boots() {
    let crew = Crew::new("tl_boot");
    let bot = crew.on_bot("Pages");
    let gate = Arc::new(tokio::sync::Notify::new());
    crew.hub.boxes.state.lock().unwrap().gate = Some(gate.clone());
    let booted = crew.boots();

    let first = spawn_op(&crew, &crew.bob, TeamOp::RunNow { workflow: bot.clone() });
    wait_for("the first run's box to boot", || crew.boots() > booted);
    // Its box waits at the gate: another Run now of the team still decides, and waits behind it.
    let second = within(&crew.hub, "a second Run now while the first box boots", spawn_op(&crew, &crew.ana, TeamOp::RunNow { workflow: bot.clone() })).unwrap();
    assert_eq!(second.map(|w| (w["waiting"].clone(), w["workflow"].clone(), w["place"].clone())), Ok((json!(true), json!(bot), json!(1))));

    gate.notify_one();
    let started = within(&crew.hub, "the first Run now", first).unwrap().unwrap();
    assert_eq!((started["workflow"].as_str(), started["state"].as_str()), (Some(bot.as_str()), Some("working")));
    assert_eq!(crew.boots() - booted, 1, "one box for the first run; the second waits in line");
}

#[test]
fn a_trigger_event_refused_by_a_busy_turn_runs_later_exactly_once() {
    let crew = Crew::new("tl_trigger");
    let bot = crew.on_bot("Pages");
    let held = Held::new(&crew.hub, &turn_of(crew.team));

    let refused = crew.found(crew.labeled(&bot, 1));
    assert_eq!(refused, Err(format!("could not open a session in {}: {BUSY}; the next check tries again", crew.folder.display())));
    assert_eq!((crew.outside(1), crew.live(), crew.waiting(&bot)), (none(), 0, 0), "nothing outside pinned, no run, nothing waits");
    held.release();

    // The next check finds it again.
    assert_eq!(crew.found(crew.labeled(&bot, 1)), Ok(()));
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(crew.outside(1), vec!["model".to_owned(), "linear move".to_owned()], "each outside step exactly once");
    assert_eq!(crew.live_of(&bot).len(), 1, "one run");
}

#[test]
fn an_event_left_waiting_by_a_busy_turn_after_a_run_ends_starts_on_the_next_reaper_pass() {
    let crew = Crew::new("tl_strand");
    let (holder, pages) = (crew.on_bot("Docs"), crew.on_bot("Pages"));
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    assert_eq!(crew.found(crew.labeled(&holder, 9)), Ok(()));
    assert_eq!(crew.found(crew.labeled(&pages, 1)), Ok(()));
    assert_eq!(crew.waiting(&pages), 1, "ENG-1 waits");

    let held = Held::new(&crew.hub, &turn_of(crew.team));
    crew.stop(&crew.live_of(&holder)[0]);
    // The start after the end waits on the turn, and is refused.
    waiting_on(&held);
    wait_for("the start after the end to give up the turn", || held.waiters() == 0);
    held.release();
    assert_eq!((crew.waiting(&pages), crew.outside(1)), (1, none()), "ENG-1 still waits");

    crew.reaper_pass();
    wait_for("ENG-1 to run", || crew.outside(1).len() == 2 && crew.live_of(&pages).len() == 1);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(crew.outside(1), vec!["model".to_owned(), "linear move".to_owned()], "each outside step exactly once");
    assert_eq!(crew.waiting(&pages), 0);
}

/// How many runs of bot `bot` are live and opened their session (no longer only reserved).
fn opened(crew: &Crew, bot: &str) -> i64 {
    let rows = super::teams_calls::rows(&crew.hub, &format!("select count(*)::text from team_runs where team = '{}' and workflow = '{bot}' and state = 'working' and not reserved", crew.team));
    rows[0][0].as_deref().unwrap_or("0").parse().unwrap()
}

#[test]
fn a_waiting_run_reserved_by_a_turn_that_then_fails_still_starts_at_once() {
    let crew = Crew::new("tl_reserved_start");
    let (holder, pages, notes) = (crew.on_bot("Docs"), crew.on_bot("Pages"), crew.on_bot("Notes"));
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    assert_eq!(crew.found(crew.labeled(&holder, 9)), Ok(()));
    for bot in [&pages, &notes] {
        let reply = crew.call(&crew.bob, TeamOp::RunNow { workflow: bot.clone() }).unwrap();
        assert_eq!(reply["waiting"], json!(true), "a Run now waits behind the holder");
    }
    // Two places once the holder ends: the turn reserves Pages's run first, then reads Notes's bot
    // row, which no longer reads, and fails after its reservation committed.
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 2 where id = '{}'; update team_bots set limits = '\"unreadable\"' where team = '{}' and workflow = '{notes}'", crew.team, crew.team));
    let boxes = crew.boxes_created();
    crew.stop(&crew.live_of(&holder)[0]);

    wait_for("the reserved Pages run to open its session in a box of its own", || opened(&crew, &pages) == 1 && crew.boxes_created() > boxes);
    assert_eq!(crew.reserved_at(&pages), Vec::<String>::new(), "no Pages run is left only holding its place");
}

#[test]
fn a_run_now_refused_after_its_turn_queued_it_is_queued_and_counted_exactly_once_on_retry() {
    let crew = Crew::new("tl_queue_once");
    let pages = crew.on_bot("Pages");
    assert_eq!(crew.found(crew.labeled(&pages, 1)), Ok(()));
    // Pages's line is full of ana's waiting Run nows.
    let ana = format!("{{\"by\": \"member\", \"member\": \"{}\"}}", crew.ana.id);
    admin_on(&crew.hub.database, &format!("insert into team_run_queue (team, workflow, start) select '{}', '{pages}', '{ana}'::jsonb from generate_series(1, 20)", crew.team));
    let line = || super::teams_calls::rows(&crew.hub, &format!("select start->>'member', count(*)::text from team_run_queue where team = '{}' group by 1 order by 2", crew.team));
    // Counting the dropped oldest waits on another connection's lock and is refused: bob's Run now
    // was queued in the same turn, so it goes back out with it.
    let held = Held::new(&crew.hub, "lock table team_bot_skips in share mode");
    assert_eq!(crew.call(&crew.bob, TeamOp::RunNow { workflow: pages.clone() }), Err(BUSY.to_owned()));
    held.release();
    assert_eq!(line(), vec![vec![Some(crew.ana.id.to_string()), Some("20".to_owned())]], "the refused turn left the line as it was");

    let waiting = crew.call(&crew.bob, TeamOp::RunNow { workflow: pages.clone() }).unwrap();
    assert_eq!(waiting["waiting"], json!(true));
    assert_eq!(line(), vec![vec![Some(crew.bob.id.to_string()), Some("1".to_owned())], vec![Some(crew.ana.id.to_string()), Some("19".to_owned())]], "bob's Run now waits once");
    assert_eq!(crew.skipped(&pages), json!([{ "reason": "queue_full", "count": 1 }]), "one oldest dropped, counted once");
}

#[test]
fn a_failed_start_whose_count_is_busy_still_starts_what_waits_next() {
    let crew = Crew::new("tl_start_cleanup");
    let pages = crew.on_bot("Pages");
    assert_eq!(crew.found(crew.labeled(&pages, 1)), Ok(()));
    for who in [&crew.bob, &crew.ana] {
        let reply = crew.call(who, TeamOp::RunNow { workflow: pages.clone() }).unwrap();
        assert_eq!(reply["waiting"], json!(true));
    }
    // Bob's run's box does not boot, and counting that waits on another connection's lock.
    crew.hub.boxes.state.lock().unwrap().failing = Some(Exec { exit_code: 22, output: String::new(), error: "curl: (22) The requested URL returned error: 404\n".into() });
    let held = Held::new(&crew.hub, "lock table team_bot_skips in share mode");
    crew.stop(&crew.live_of(&pages)[0]);

    wait_for("ana's waiting run to open its session", || opened(&crew, &pages) == 1);
    assert_eq!(crew.waiting(&pages), 0, "nothing is left waiting for the reaper");
    held.release();
}

// ---- the claim of a granted event ----

#[test]
fn a_claim_skips_a_granted_row_another_holds_and_the_hub_answers_others() {
    let crew = Crew::new("tl_claim");
    let carl = person(&crew.hub, "carl@example.test");
    let (holder, pages) = (crew.on_bot("Docs"), crew.on_bot("Pages"));
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    assert_eq!(crew.found(crew.labeled(&holder, 9)), Ok(()));
    assert_eq!(crew.found(crew.labeled(&pages, 1)), Ok(()));
    // Another hub's claim holds the row; the grant itself does not wait on it.
    let held = Held::new(&crew.hub, &format!("select id from team_run_queue where team = '{}' for key share", crew.team));
    crew.stop(&crew.live_of(&holder)[0]);
    let granted = || -> Vec<i64> {
        let rows = crew.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), crew.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.query("select id from team_run_queue where team = $1 and granted_at is not null", &[&crew.team]).await.unwrap()
        });
        rows.iter().map(|r| r.get(0)).collect()
    };
    wait_for("ENG-1's grant", || granted().len() == 1);

    let (hub, team) = (crew.hub.hub().clone(), crew.team);
    let id = granted()[0];
    let claimed = within(&crew.hub, "the claim", async { teams::queue::claim(&hub, &hub.account(team).await.unwrap(), id).await });
    assert_eq!(claimed.map(|event| event.is_some()), Ok(false), "the claim that holds the row runs it, not this one");
    others_answered(&crew.hub, &carl);
    assert_eq!(held.waiters(), 0, "no claim waits on the held row");
    held.release();
    // The poller's own claim may have skipped the row while it was held, and it looks again only
    // on its next check (a minute): wake it, as a grant does, so the event does not wait for that.
    crew.hub.runtime.block_on(async { hub.connectors(&hub.account(team).await.unwrap()).await.unwrap().wake() });

    wait_for("ENG-1 to run", || crew.outside(1).len() == 2 && crew.live_of(&pages).len() == 1);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(crew.outside(1), vec!["model".to_owned(), "linear move".to_owned()], "each outside step exactly once");
}

// ---- row locks and the other explicit locks ----

#[test]
fn stopping_a_run_whose_row_another_holds_is_refused_and_the_hub_answers_others() {
    let crew = Crew::new("tl_runrow");
    let carl = person(&crew.hub, "carl@example.test");
    let bot = crew.on_bot("Pages");
    assert_eq!(crew.found(crew.labeled(&bot, 1)), Ok(()));
    let run = crew.live_of(&bot)[0].clone();
    let held = Held::new(&crew.hub, &format!("select 1 from team_runs where id = '{run}' for update"));

    let stopped = spawn_op(&crew, &crew.ana, serde_json::from_value(json!({ "op": "stop_run", "run": run })).unwrap());
    waiting_on(&held);
    others_answered(&crew.hub, &carl);
    assert_eq!(within(&crew.hub, "the stop", stopped).unwrap(), Err(BUSY.to_owned()));
    held.release();
    assert_eq!(crew.live_of(&bot), vec![run], "the run lives until a stop gets through");
}

#[test]
fn an_invite_is_refused_while_another_hub_counts_the_teams_invites() {
    let crew = Crew::new("tl_invite");
    let carl = person(&crew.hub, "carl@example.test");
    let held = Held::new(&crew.hub, &format!("select pg_advisory_xact_lock(hashtext('team_invites:{}'))", crew.team));

    let invited = spawn_op(&crew, &crew.ana, TeamOp::Invite);
    waiting_on(&held);
    others_answered(&crew.hub, &carl);
    assert_eq!(within(&crew.hub, "the invite", invited).unwrap(), Err(BUSY.to_owned()));
    held.release();
    assert!(crew.call(&crew.ana, TeamOp::Invite).is_ok(), "the next invite is made");
}

#[test]
fn a_go_ahead_whose_call_another_holds_is_refused_and_the_hub_answers_others() {
    let crew = Crew::new("tl_goahead");
    let carl = person(&crew.hub, "carl@example.test");
    let held = Held::new(&crew.hub, "lock table team_calls in exclusive mode");

    let (hub, team) = (crew.hub.hub().clone(), crew.team);
    let used = crew.hub.runtime.spawn(async move { teams::db::use_go_ahead(&hub.db, team, Uuid::new_v4(), "step", "May I?", 1_800_000_000).await });
    waiting_on(&held);
    others_answered(&crew.hub, &carl);
    assert_eq!(within(&crew.hub, "the go ahead", used).unwrap(), Err(BUSY.to_owned()));
    held.release();
}

/// Holds `table` as another hub's transaction would, runs `call` of the hub's `Db`, and checks it
/// is refused within 5 s while another account's calls are answered.
fn refused_while_held<F>(name: &str, table: &str, call: impl FnOnce(pinned_server::db::Db) -> F)
where
    F: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    let hub = TestHub::start(name);
    let carl = person(&hub, "carl@example.test");
    let held = Held::new(&hub, &format!("lock table {table} in exclusive mode"));

    let pending = hub.runtime.spawn(call(hub.hub().db.clone()));
    waiting_on(&held);
    others_answered(&hub, &carl);
    assert_eq!(within(&hub, "the call that needs the lock", pending).unwrap(), Err(BUSY.to_owned()));
    held.release();
}

#[test]
fn a_session_mark_whose_row_another_holds_is_refused_and_the_hub_answers_others() {
    let hub = TestHub::start("tl_marks");
    let (carl, dana) = (person(&hub, "carl@example.test"), person(&hub, "dana@example.test"));
    let marked = hub.runtime.block_on(hub.hub().hub_call(&dana, serde_json::from_value(json!({ "cmd": "mark_sessions", "changes": [{ "session": "d1", "mark": { "pin": 1 } }] })).unwrap()));
    assert_eq!(marked, Ok(json!([{ "session": "d1", "pin": 1 }])));
    let held = Held::new(&hub, &format!("select 1 from session_marks where account = '{}' for update", dana.id));

    let (hub_arc, dana2) = (hub.hub().clone(), dana.clone());
    let pending = hub.runtime.spawn(async move { hub_arc.hub_call(&dana2, serde_json::from_value(json!({ "cmd": "mark_sessions", "changes": [{ "session": "d1", "mark": { "pin": 2 } }] })).unwrap()).await });
    waiting_on(&held);
    others_answered(&hub, &carl);
    assert_eq!(within(&hub, "dana's mark", pending).unwrap(), Err(BUSY.to_owned()));
    held.release();
}

#[test]
fn a_sign_up_by_invite_whose_invite_another_holds_is_refused() {
    refused_while_held("tl_signup", "invites", |db| async move { db.create_invited_account(b"code", "erin@example.test", "hash").await.map(drop) });
}

#[test]
fn an_agent_code_another_holds_is_refused() {
    refused_while_held("tl_code", "agent_codes", |db| async move { db.take_agent_code("code").await.map(drop) });
}

#[test]
fn an_agent_refresh_token_another_holds_is_refused() {
    refused_while_held("tl_refresh", "agent_tokens", |db| async move { db.take_agent_refresh("token").await.map(drop) });
}

#[test]
fn an_agent_task_whose_account_another_holds_is_refused() {
    let hub = TestHub::start("tl_task");
    let (carl, erin) = (person(&hub, "carl@example.test"), person(&hub, "erin@example.test"));
    let held = Held::new(&hub, &format!("select 1 from accounts where id = '{}' for update", erin.id));

    let task: pinned_core::schedule::Task = serde_json::from_value(json!({
        "id": Uuid::new_v4().to_string(), "title": "Nightly", "schedule": { "kind": "cron", "expr": "0 9 * * 1-5" }, "tz": "America/Sao_Paulo",
        "target": { "kind": "orchestrator" }, "action": { "kind": "prompt", "text": "Summarise yesterday" },
        "enabled": true, "created_by": { "kind": "user" }, "created_at": 0, "updated_at": 0
    }))
    .unwrap();
    let (db, account) = (hub.hub().db.clone(), erin.id);
    let pending = hub.runtime.spawn(async move { db.add_agent_task(account, &task, 5).await.map(drop) });
    waiting_on(&held);
    others_answered(&hub, &carl);
    assert_eq!(within(&hub, "erin's agent task", pending).unwrap(), Err(BUSY.to_owned()));
    held.release();
}
