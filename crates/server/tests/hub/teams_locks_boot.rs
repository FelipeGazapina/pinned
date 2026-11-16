//! A hub's start beside another hub's locks, as during a deploy: a schema statement that would
//! change nothing takes no lock that waits on the running hub's rows, and one that is needed waits
//! as long as it must instead of failing on the hub's short lock timeout.
use super::teams_locks::{others_answered, person, Held};
use super::*;
use std::sync::mpsc;
use uuid::Uuid;

/// How long a hub may take to start beside another hub's row locks.
const STARTED: Duration = Duration::from_secs(10);
/// How long the running hub may take to answer while the other starts.
const ANSWER: Duration = Duration::from_secs(5);

/// Starts a hub with `config` on a thread of its own and answers how its start ended. That hub
/// stops again as soon as it is up.
fn start_aside(config: pinned_server::Config) -> mpsc::Receiver<Result<(), String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _ = tx.send(runtime.block_on(pinned_server::start(config)).map(|_| ()));
    });
    rx
}

/// A team of `owner`'s with one run and one queued start, as the hub's database keeps them.
fn team_rows(hub: &TestHub, owner: Uuid) {
    let run = Uuid::new_v4();
    admin_on(
        &hub.database,
        &format!(
            "insert into teams (id, name, owner) values ('{owner}', 'crew', '{owner}');
             insert into team_runs (id, team, workflow, started_by, state) values ('{run}', '{owner}', 'triage', '{{}}', 'running');
             insert into team_run_queue (team, workflow, start) values ('{owner}', 'triage', '{{}}');"
        ),
    );
}

/// Whether `table` of the hub's database has `column`.
fn column_exists(hub: &TestHub, table: &str, column: &str) -> bool {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        let found = db.query_one("select count(*) from information_schema.columns where table_name = $1 and column_name = $2", &[&table, &column]).await.unwrap();
        found.get::<_, i64>(0) == 1
    })
}

#[test]
fn a_hub_starts_while_another_hub_holds_row_locks_on_every_table() {
    let hub = TestHub::start("locks_boot_rows");
    let ann = person(&hub, "ann@example.test");
    let carl = person(&hub, "carl@example.test");
    team_rows(&hub, ann.id);
    let held = Held::new(
        &hub,
        &format!(
            "select 1 from accounts where id = '{}' for update;
             select 1 from logins where account = '{}' for update;
             select 1 from teams for update;
             select 1 from team_runs for update;
             select 1 from team_run_queue for update;
             update team_runs set state = state;",
            ann.id, ann.id
        ),
    );
    let since = Instant::now();
    let started = start_aside(hub.config("127.0.0.1:0"));
    let mut start = None;
    wait_for("the second hub to start or wait on a lock", || {
        if start.is_none() {
            start = started.try_recv().ok();
        }
        start.is_some() || held.waiters() >= 1
    });
    // The running hub still answers other accounts while the second starts: a new account signs up.
    let url = hub.url.clone();
    let (tx, signed) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(account::signup(&url, "dana@example.test", "correct horse").map(|_| ()));
    });
    assert_eq!(signed.recv_timeout(ANSWER).expect("a signup on the running hub was not answered within 5 s"), Ok(()));
    others_answered(&hub, &carl);
    let start = start.or_else(|| started.recv_timeout(STARTED.saturating_sub(since.elapsed())).ok());
    assert_eq!(start.expect("the second hub was not up within 10 s"), Ok(()));
    held.release();
}

#[test]
fn a_hub_whose_schema_needs_a_change_waits_for_it_and_starts() {
    let mut hub = TestHub::start("locks_boot_change");
    let ann = person(&hub, "ann@example.test");
    team_rows(&hub, ann.id);
    hub.stop();
    admin_on(&hub.database, "alter table team_run_queue drop column granted_at");
    let held = Held::new(&hub, "select 1 from team_run_queue for update");
    let started = start_aside(hub.config("127.0.0.1:0"));
    wait_for("the starting hub to wait on the held lock", || held.waiters() >= 1);
    // Past the 2 s any other statement of the hub may wait on a lock, the column still waits.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(started.try_recv(), Err(mpsc::TryRecvError::Empty), "the hub's start ended while the column waited on the lock");
    held.release();
    assert_eq!(started.recv_timeout(STARTED).expect("the hub was not up within 10 s of the lock's release"), Ok(()));
    assert!(column_exists(&hub, "team_run_queue", "granted_at"), "the hub added the missing column");
}
