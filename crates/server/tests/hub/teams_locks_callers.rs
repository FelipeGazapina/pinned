//! What each caller sees when the hub refuses a statement that waited too long on a lock another
//! connection holds (another hub during a deploy, or a slow transaction), and that it recovers on its
//! own. One test per class of caller, each through the wire path that caller uses: the app's socket,
//! a machine's socket, a team box's hook, the HTTP routes of sign up, login, pairing and the agents'
//! OAuth, and the hub's own store writer. Each holds the lock on a connection of its own, checks the
//! plain refusal within 5 s, releases it, and checks that the retry, or the next send, lands.
//! Last, two hubs' reaper passes at once start no more than the team's free places.
use super::teams_admission::Crew;
use super::teams_calls::{choice, patient, rows, TeamBox};
use super::teams_go_ahead::{answer, ask, released, try_step, DEPLOY};
use super::*;
use pinned_server::teams;

/// What a call refused for a lock it waited on too long answers.
const BUSY: &str = "the hub is busy with this right now; try again in a moment";
/// How long the hub may take to answer while a lock is held.
const ANSWER: Duration = Duration::from_secs(5);

/// A transaction on a connection of the test's own that holds what `statements` lock, until released.
struct Held {
    runtime: tokio::runtime::Runtime,
    db: Option<tokio_postgres::Client>,
}

impl Held {
    fn new(hub: &TestHub, statements: &str) -> Held {
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

    /// Ends the transaction, and with it what it held.
    fn release(mut self) {
        let db = self.db.take().unwrap();
        self.runtime.block_on(db.batch_execute("rollback")).unwrap();
    }
}

/// Runs `call` and answers what it answered, failing the test when it took 5 s or more.
fn timed<T>(what: &str, call: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let answered = call();
    assert!(started.elapsed() < ANSWER, "{what} was not answered within 5 s");
    answered
}

/// `POST path` with a JSON body, as the app and the node send it: the status and the body.
fn post_json(url: &str, path: &str, body: Value) -> (u16, Value) {
    let response = reqwest::blocking::Client::new().post(format!("{url}{path}")).header("content-type", "application/json").body(body.to_string()).send().unwrap();
    let status = response.status().as_u16();
    (status, serde_json::from_str(&response.text().unwrap()).unwrap_or(Value::Null))
}

/// The refusal an `/api` route answers for a busy lock: the client shows it and tries again.
fn busy() -> (u16, Value) {
    (503, json!({ "error": BUSY }))
}

/// The refusal `/oauth/token` answers for a busy lock, in OAuth's error shape.
fn busy_oauth() -> (u16, Value) {
    (503, json!({ "error": "temporarily_unavailable", "error_description": BUSY }))
}

// ---- A. a person's app ----

#[test]
fn an_app_call_refused_for_a_busy_lock_says_so_and_the_persons_retry_lands() {
    let hub = TestHub::start("tlc_app");
    let (_, ana_id, mut ana) = signed_in(&hub.url, "ana@example.test");
    ana.hub(mark(json!([{ "session": "c1", "mark": { "pin": 0 } }]))).unwrap();
    let held = Held::new(&hub, &format!("select 1 from session_marks where account = '{ana_id}' for update"));

    let refused = timed("the app's mark", || ana.hub(mark(json!([{ "session": "c1", "mark": { "pin": 1 } }]))));
    assert_eq!(refused, Err(BUSY.to_owned()));
    assert_eq!(ana.hub(json!({ "cmd": "list_schedules" })), Ok(json!([])), "the socket still answers");
    held.release();

    assert_eq!(ana.hub(mark(json!([{ "session": "c1", "mark": { "pin": 1 } }]))), Ok(json!([{ "session": "c1", "pin": 1 }])), "the person's retry lands");
}

// ---- B. an agent's act, and the app's release protection, on the account row ----

#[test]
fn an_agent_task_and_release_protection_refused_on_a_busy_account_row_land_on_retry() {
    let mut s = Schedules::start("tlc_account_row");
    s.ana.hub(json!({ "cmd": "unlock", "password": "correct horse" })).unwrap();
    let held = Held::new(&s._hub, "select 1 from accounts for update");

    let acted = timed("the agent's task", || s.act("save_schedule", json!({ "task": task_body("Agent digest") })));
    assert_eq!(acted, Err(BUSY.to_owned()));
    let protected = timed("release protection", || s.ana.hub(json!({ "cmd": "set_release_protection", "on": true })));
    assert_eq!(protected, Err(BUSY.to_owned()));
    held.release();

    assert_eq!(s.titles(), Vec::<String>::new(), "the refused task was not kept");
    assert_eq!(s.act("save_schedule", json!({ "task": task_body("Agent digest") })).unwrap()["outcome"], "done", "the agent's retry lands");
    assert_eq!(s.titles(), vec!["Agent digest".to_owned()]);
    assert_eq!(s.ana.hub(json!({ "cmd": "set_release_protection", "on": true })), Ok(json!(true)), "the person's retry lands");
}

// ---- C. a machine's call ----

#[test]
fn a_machine_write_refused_for_a_busy_lock_keeps_its_socket_and_its_next_send_lands() {
    let hub = TestHub::start("tlc_machine");
    let url = hub.url.clone();
    let (token, _, mut ana) = signed_in(&url, "ana@example.test");
    let (_, machine_token) = account::register_machine(&url, &token, "ana-mini", "linux").unwrap();
    let tab = json!({ "id": "tab-1", "cwd": "/w/site", "bypass": false, "running": true, "status": "done", "title": "Fix login", "message": null, "resume_at": null, "opened": 1, "session": "conv-1", "role": "worker" });
    let raw = RawMachine::start_with(&url, &machine_token, vec![json!({ "kind": "tabs", "tabs": [tab] })], |_| RawReply::Ok(Value::Null));
    ana.wait_machines(&[("ana-mini", true)]);
    ana.wait("the tabs", |f| f["type"] == "event" && f["event"]["kind"] == "tabs" && f.to_string().contains("conv-1"));
    // Another hub writes marks: the machine's archive waits on its lock.
    let held = Held::new(&hub, "lock table session_marks in exclusive mode");

    let refused = timed("the machine's auto archive", || raw.ask(json!({ "cmd": "auto_archive", "tab": "tab-1" })));
    assert_eq!(refused, Err(BUSY.to_owned()));
    assert_eq!(raw.calls("close_tab"), 0, "nothing was closed");
    held.release();

    // The machine keeps the tab and asks again on a later check, on the same socket.
    let row = raw.ask(json!({ "cmd": "auto_archive", "tab": "tab-1" })).unwrap();
    assert_eq!((row["session"].as_str(), row["archived"]["name"].as_str()), (Some("conv-1"), Some("Fix login")));
    assert_eq!(raw.calls("close_tab"), 1);
}

// ---- D. a team box's go-ahead ----

/// What the bot reads when the hub was too busy to ask the team about its step.
const ASK_AGAIN: &str = "The hub is busy right now and could not ask the team. Wait a moment, then make this exact tool call again, unchanged.";

#[test]
fn a_go_ahead_refused_for_a_busy_call_row_denies_the_step_plainly_and_the_bots_retry_runs_it() {
    let hub = TestHub::start_with_team_policy("tlc_go_ahead", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    tb.run(&hub);
    let call = ask(&hub, &mut tb, DEPLOY);
    answer(&mut tb, &call, choice("Go ahead", ""));
    let held = Held::new(&hub, &format!("select 1 from team_calls where id = '{call}' for update"));

    let refused = timed("the box's go-ahead", || try_step(&hub, &tb, DEPLOY));
    assert_eq!(refused, Some(ASK_AGAIN.to_owned()), "a plain refusal the bot retries, not \"do not do this step\"");
    assert!(!released(&hub, &call), "the go-ahead is still there to use");
    held.release();

    assert_eq!(try_step(&hub, &tb, DEPLOY), None, "the bot's retry runs the step");
    assert!(released(&hub, &call));
}

// ---- E. sign up by invite ----

#[test]
fn a_sign_up_by_invite_refused_for_a_busy_invite_is_a_plain_refusal_and_the_retry_signs_up() {
    let hub = TestHub::start_with("tlc_invite", Signup::Invite);
    let url = hub.url.clone();
    let (status, ana) = post_json(&url, "/api/signup", json!({ "email": "ana@example.test", "password": "correct horse" }));
    assert_eq!(status, 200);
    let mut ana = signed_in_client(&url, ana["token"].as_str().unwrap());
    let code = ana.hub(json!({ "cmd": "create_invite" })).unwrap()["code"].as_str().unwrap().to_owned();
    let bruno = json!({ "email": "bruno@example.test", "password": "correct horse", "invite": code });
    let held = Held::new(&hub, "select 1 from invites for update");

    assert_eq!(timed("the sign up", || post_json(&url, "/api/signup", bruno.clone())), busy());
    held.release();

    let (status, signed) = post_json(&url, "/api/signup", bruno);
    assert_eq!((status, signed["account"]["email"].as_str()), (200, Some("bruno@example.test")), "the retry signs up with the same code");
}

// ---- F. login and pairing a machine ----

#[test]
fn a_login_and_a_machine_pairing_refused_for_a_busy_row_are_plain_refusals_and_retries_land() {
    let hub = TestHub::start("tlc_login");
    let url = hub.url.clone();
    let (_, ana_id, mut ana) = signed_in(&url, "ana@example.test");
    let login = json!({ "email": "ana@example.test", "password": "correct horse" });
    let held = Held::new(&hub, &format!("select 1 from accounts where id = '{ana_id}' for update"));
    assert_eq!(timed("the login", || post_json(&url, "/api/login", login.clone())), busy());
    held.release();
    let (status, session) = post_json(&url, "/api/login", login);
    assert_eq!((status, session["account"]["id"].as_str()), (200, Some(ana_id.as_str())), "the retry logs in");

    let code = ana.hub(json!({ "cmd": "pair_code" })).unwrap()["code"].as_str().unwrap().to_owned();
    let pairing = json!({ "code": code, "name": "vps", "os": "linux" });
    let held = Held::new(&hub, "select 1 from pair_codes for update");
    assert_eq!(timed("the pairing", || post_json(&url, "/api/pair", pairing.clone())), busy());
    held.release();
    let (status, paired) = post_json(&url, "/api/pair", pairing);
    assert_eq!((status, paired["machine"]["name"].as_str()), (200, Some("vps")), "the retry pairs with the same code");
}

// ---- G. an agent's OAuth: code and refresh ----

/// A registered OAuth client that ana let in: the hub's address, the HTTP client, the client id
/// and the approved code with its verifier. Her approval is written as the app's Approve writes it.
fn approved_code(hub: &TestHub) -> (String, reqwest::blocking::Client, String, String, String) {
    let url = hub.url.clone();
    let (_, ana_id, _) = signed_in(&url, "ana@example.test");
    let http = agent_http();
    let client_id = register_agent(&url, &http);
    let (verifier, challenge) = pkce();
    let request = authorize(&url, &http, &client_id, &challenge, "st-1");
    let db = &hub.hub().db;
    let code = hub.runtime.block_on(async {
        let row = db.take_agent_request(&request).await.unwrap().unwrap();
        db.create_agent_grant(uuid::Uuid::parse_str(&ana_id).unwrap(), &row, &["sessions".to_owned()], &[]).await.unwrap()
    });
    (url, http, client_id, code, verifier)
}

#[test]
fn an_agent_code_exchange_refused_for_a_busy_code_is_a_plain_oauth_refusal_and_the_retry_gets_tokens() {
    let hub = TestHub::start("tlc_agent_code");
    let (url, http, client_id, code, verifier) = approved_code(&hub);
    let form = [("grant_type", "authorization_code"), ("client_id", client_id.as_str()), ("code", code.as_str()), ("code_verifier", verifier.as_str()), ("redirect_uri", AGENT_REDIRECT)];
    let held = Held::new(&hub, "select 1 from agent_codes for update");
    assert_eq!(timed("the code exchange", || exchange(&url, &http, &form)), busy_oauth());
    held.release();

    let (status, tokens) = exchange(&url, &http, &form);
    assert_eq!((status, tokens["token_type"].as_str()), (200, Some("Bearer")), "the retry with the same code gets tokens: {tokens}");
}

#[test]
fn an_agent_refresh_refused_for_a_busy_token_is_a_plain_oauth_refusal_and_the_retry_gets_tokens() {
    let hub = TestHub::start("tlc_agent_refresh");
    let (url, http, client_id, code, verifier) = approved_code(&hub);
    let (status, tokens) = exchange(&url, &http, &[("grant_type", "authorization_code"), ("client_id", &client_id), ("code", &code), ("code_verifier", &verifier), ("redirect_uri", AGENT_REDIRECT)]);
    assert_eq!(status, 200, "{tokens}");
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let form = [("grant_type", "refresh_token"), ("client_id", client_id.as_str()), ("refresh_token", refresh.as_str())];
    let held = Held::new(&hub, "select 1 from agent_tokens for update");
    assert_eq!(timed("the refresh", || exchange(&url, &http, &form)), busy_oauth());
    held.release();

    let (status, tokens) = exchange(&url, &http, &form);
    assert_eq!((status, tokens["token_type"].as_str()), (200, Some("Bearer")), "the retry with the same refresh token gets tokens: {tokens}");
}

// ---- H. the hub's store writer ----

#[test]
fn a_store_write_refused_for_a_busy_account_row_is_kept_and_lands_once_the_row_is_free() {
    let hub = TestHub::start("tlc_store");
    let url = hub.url.clone();
    let (token, ana_id, mut ana) = signed_in(&url, "ana@example.test");
    let (machine, _) = account::register_machine(&url, &token, "ana-mini", "linux").unwrap();
    let stored = || rows(&hub, &format!("select coalesce(store::text, '') from accounts where id = '{ana_id}'"))[0][0].clone().unwrap_or_default();
    let held = Held::new(&hub, &format!("select 1 from accounts where id = '{ana_id}' for update"));

    let saved = timed("the app's save", || {
        ana.hub(json!({ "cmd": "save_connector", "connector": {
            "name": "Bugs", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": "/w/site", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": machine.id,
        } }))
    });
    assert!(saved.is_ok(), "{saved:?}");
    std::thread::sleep(Duration::from_secs(3));
    assert!(!stored().contains("Bugs"), "the row is held: nothing is written yet");
    held.release();

    wait_for("the kept write to land", || stored().contains("Bugs"));
}

// ---- I. two hubs' reaper passes ----

/// The team's live runs: (bot, run) of each.
fn live_runs(crew: &Crew) -> Vec<(String, String)> {
    rows(&crew.hub, &format!("select workflow, id::text from team_runs where team = '{}' and state in ('working', 'waiting', 'taken_over')", crew.team))
        .into_iter()
        .map(|r| (r[0].clone().unwrap(), r[1].clone().unwrap()))
        .collect()
}

fn waiting_total(crew: &Crew) -> i64 {
    rows(&crew.hub, &format!("select count(*)::text from team_run_queue where team = '{}' and granted_at is null", crew.team))[0][0].as_deref().unwrap().parse().unwrap()
}

#[test]
fn two_hubs_reaper_passes_start_at_most_the_teams_free_places() {
    let crew = Crew::new("tlc_reapers");
    let holder = crew.on_bot("Docs");
    let bots = [crew.on_bot("Pages"), crew.on_bot("Blog"), crew.on_bot("Shop")];
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    assert_eq!(crew.found(crew.labeled(&holder, 9)), Ok(()));
    // Five events wait for three bots behind the one live run.
    let events = [(&bots[0], 1), (&bots[0], 2), (&bots[1], 3), (&bots[1], 4), (&bots[2], 5)];
    for (bot, n) in events {
        assert_eq!(crew.found(crew.labeled(bot, n)), Ok(()));
    }
    assert_eq!((live_runs(&crew).len(), waiting_total(&crew)), (1, 5));

    // Two places free up, and two hubs on the database run their reaper pass at the same time.
    let (twin, running) = crew.hub.twin();
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 3 where id = '{}'", crew.team));
    let policy = crew.hub.team_policy.clone();
    let (one, other) = (crew.hub.hub().clone(), running.hub.clone());
    let gate = Arc::new(tokio::sync::Barrier::new(2));
    let (gate_one, gate_other, policy_other) = (gate.clone(), gate.clone(), policy.clone());
    let first = crew.hub.runtime.spawn(async move {
        gate_one.wait().await;
        teams::calls::tick(&one, &policy).await
    });
    let second = twin.spawn(async move {
        gate_other.wait().await;
        teams::calls::tick(&other, &policy_other).await
    });
    assert_eq!(crew.hub.runtime.block_on(first).unwrap(), Ok(()));
    assert_eq!(twin.block_on(second).unwrap(), Ok(()));

    // Until what started settles: never more live runs than the cap, never two of one bot.
    let check = |crew: &Crew| {
        let live = live_runs(crew);
        assert!(live.len() <= 3, "at most the cap of 3 live runs: {live:?}");
        let mut of_bot: Vec<&String> = live.iter().map(|(bot, _)| bot).collect();
        of_bot.sort();
        let before = of_bot.len();
        of_bot.dedup();
        assert_eq!(of_bot.len(), before, "no bot has two live runs: {live:?}");
        live.len()
    };
    wait_for("the two free places to fill", || check(&crew) == 3);
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_secs(4) {
        check(&crew);
        std::thread::sleep(Duration::from_millis(100));
    }
    // A run another hub's box check ends frees its place for the next: the line may move on.
    assert!(waiting_total(&crew) <= 3, "the two free places took two events");
    // Each event started at most once: no step outside pinned twice. A run another hub's box
    // check ended may stop before its last step.
    for (_, n) in events {
        let outside = crew.outside(n);
        let mut once = outside.clone();
        once.sort();
        once.dedup();
        assert_eq!(once.len(), outside.len(), "ENG-{n} ran at most once: {outside:?}");
    }
    twin.shutdown_background();
}
