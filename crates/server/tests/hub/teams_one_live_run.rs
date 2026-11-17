//! One live run per bot, held by the database as well as by the team's turn: a second live run of
//! a bot is refused by Postgres, a turn that meets one makes its event wait in the bot's line, and
//! a hub whose database already has two live runs of one bot still starts, leaves them as they
//! are, and sets the rule on a later start once they no longer clash.
use super::teams_admission::{none, Crew};
use super::teams_locks::person;
use super::*;
use pinned_core::teams::{Run, RunState};
use pinned_server::db::{Db, Sealer};
use pinned_server::teams;
use uuid::Uuid;

/// The index that holds the rule.
const INDEX: &str = "team_runs_one_live_per_bot";
/// What a hub logs when it starts on a database where one bot has two live runs.
const NOT_ENFORCED: &str = "pinned-server: one live run per bot: 1 bot has more than one live run; the rule is not enforced until they end";
/// The database `opens_the_database_the_boot_test_names` opens, when the boot test runs it.
const CHILD_DATABASE: &str = "PINNED_ONE_LIVE_RUN_DATABASE";

fn run_of(workflow: &str) -> Run {
    Run {
        id: Uuid::new_v4().to_string(),
        workflow: workflow.to_owned(),
        state: RunState::Working,
        controller: None,
        cost_usd: 0.0,
        active_secs: 0,
        waiting_secs: 0,
        started_at: 0,
        ended_at: None,
        outcome: None,
        started_by: "trigger:linear_label".to_owned(),
    }
}

#[test]
fn the_database_refuses_a_second_live_run_of_one_bot() {
    let crew = Crew::new("one_live_db");
    let db = &crew.hub.hub().db;
    let (first, second, other) = (run_of("pages"), run_of("pages"), run_of("docs"));
    let insert = |run: &Run| crew.hub.runtime.block_on(teams::db::insert_run(db, crew.team, run, None));
    assert_eq!(insert(&first), Ok(()));
    assert_eq!(insert(&second), Err("this bot already has a live run; try again once it ends".to_owned()));
    assert_eq!(insert(&other), Ok(()), "another bot's run is its own");
    admin_on(&crew.hub.database, &format!("update team_runs set state = 'done', ended_at = now() where id = '{}'", first.id));
    assert_eq!(insert(&second), Ok(()), "once the first ended, the bot runs again");
}

#[test]
fn a_live_run_written_outside_the_turn_makes_the_turns_event_wait() {
    let crew = Crew::new("one_live_race");
    let id = crew.on_bot("Pages");
    // A live run of the bot written by a path outside the team's turn, its transaction still open:
    // the turn reads no live run of the bot and reserves one.
    let other = crew.hub.runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), crew.hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.batch_execute("begin").await.unwrap();
        db.execute("insert into team_runs (id, team, workflow, started_by, state) values (gen_random_uuid(), $1, $2, '\"trigger:linear_label\"', 'working')", &[&crew.team, &id])
            .await
            .unwrap();
        db
    });
    std::thread::scope(|scope| {
        let ours = scope.spawn(|| crew.found(crew.labeled(&id, 1)));
        std::thread::sleep(Duration::from_secs(1));
        crew.hub.runtime.block_on(other.batch_execute("commit")).unwrap();
        assert_eq!(ours.join().unwrap(), Ok(()));
    });
    assert_eq!(crew.live_of(&id).len(), 1, "one live run of the bot: the other path's");
    assert_eq!(crew.waiting(&id), 1, "the event waits in the bot's line");
    assert_eq!(crew.outside(1), none(), "and touched nothing outside pinned");
}

fn index_exists(hub: &TestHub) -> bool {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.query_one("select to_regclass($1::text) is not null", &[&INDEX]).await.unwrap().get(0)
    })
}

/// The runs' states, by id.
fn states(hub: &TestHub) -> Vec<(Uuid, String)> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.query("select id, state from team_runs order by id", &[]).await.unwrap().iter().map(|r| (r.get(0), r.get(1))).collect()
    })
}

/// What a hub logs while it opens `hub`'s database: `opens_the_database_the_boot_test_names`, run
/// in a process of its own whose log this reads.
fn boot_log(hub: &TestHub) -> String {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["teams_one_live_run::opens_the_database_the_boot_test_names", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_DATABASE, format!("{}/{}", postgres(), hub.database))
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "the database did not open:\n{log}");
    log
}

/// Opens the database the boot test names, as a hub's start does, and nothing else. Without one
/// named it does nothing.
#[test]
fn opens_the_database_the_boot_test_names() {
    let Ok(url) = std::env::var(CHILD_DATABASE) else { return };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(Db::connect(&url, Sealer::load(&temp("one-live-child").join("hub.key")).unwrap())).unwrap();
}

#[test]
fn a_hub_on_two_live_runs_of_one_bot_starts_and_sets_the_rule_once_one_ends() {
    let mut hub = TestHub::start("one_live_boot");
    let ann = person(&hub, "ann@example.test");
    let (first, second, docs) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    // Rows an older hub left: two live runs of bot triage, and one of bot docs.
    admin_on(
        &hub.database,
        &format!(
            "drop index if exists {INDEX};
             insert into teams (id, name, owner) values ('{t}', 'crew', '{t}');
             insert into team_runs (id, team, workflow, started_by, state) values
               ('{first}', '{t}', 'triage', '{{}}', 'working'), ('{second}', '{t}', 'triage', '{{}}', 'waiting'), ('{docs}', '{t}', 'docs', '{{}}', 'working');",
            t = ann.id
        ),
    );
    let before = states(&hub);
    hub.restart();
    assert!(!index_exists(&hub), "the rule is not set while two live runs of one bot clash");
    assert_eq!(states(&hub), before, "no run was ended or changed");
    let log = boot_log(&hub);
    assert!(log.lines().any(|line| line == NOT_ENFORCED), "the start says so:\n{log}");

    admin_on(&hub.database, &format!("update team_runs set state = 'stopped', ended_at = now() where id = '{second}'"));
    hub.restart();
    assert!(index_exists(&hub), "the next start sets the rule");
    assert!(!boot_log(&hub).contains("one live run per bot"), "and says nothing of it");
}
