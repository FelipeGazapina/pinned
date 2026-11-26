//! An account's fires wait on Postgres, and nothing else on that account's store may wait with
//! them. On 2026-11-24 the hub froze this way: a mirror held the store while it read the fires,
//! three machines' mirrors blocked their tokio workers on the store, no worker was left to read
//! the database's answer, and the hub stopped answering anything until it was restarted.
use super::*;
use pinned_server::hub::copies;
use std::sync::mpsc;

/// How long the hub has to answer while the fires read waits.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// The connections that wait for a lock on `trigger_fires` in `database`.
fn waiting_on_fires(runtime: &tokio::runtime::Runtime, database: &str) -> i64 {
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{database}", postgres()), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        let row = client
            .query_one("select count(*) from pg_stat_activity where datname = $1 and wait_event_type = 'Lock' and query like '%trigger_fires%'", &[&database])
            .await
            .unwrap();
        row.get(0)
    })
}

/// Runs `call` on a thread of its own and answers whether it returned within `within`. A call
/// that never returns leaves its thread blocked, not the test.
fn returns_within(within: Duration, call: impl FnOnce() + Send + 'static) -> bool {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        call();
        let _ = done.send(());
    });
    finished.recv_timeout(within).is_ok()
}

#[test]
fn a_mirror_waiting_on_the_fires_leaves_the_store_and_the_hub_answering() {
    let mut hub = TestHub::start("store_lock_freeze");
    // Two workers, so the two calls that wait with the mirror are every worker the hub has.
    let addr = hub.stop();
    hub.runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    hub.serve(&addr);
    let (token, view) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    let (machine, _) = account::register_machine(&hub.url, &token, "desk", "macos").unwrap();
    let account = hub.runtime.block_on(hub.hub().account(uuid::Uuid::parse_str(&view.id).unwrap())).unwrap();
    let connectors = hub.runtime.block_on(hub.hub().connectors(&account)).unwrap();

    // Another session holds the fires table, so reading the fires waits as on a slow database.
    let locker_runtime = tokio::runtime::Runtime::new().unwrap();
    let locker = locker_runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        client.batch_execute("begin; lock table trigger_fires in access exclusive mode").await.unwrap();
        client
    });

    let (done, finished) = mpsc::channel::<&str>();
    {
        let (running, account, machine, token, done) = (hub.hub().clone(), account.clone(), machine.id.clone(), token.clone(), done.clone());
        hub.runtime.spawn(async move {
            copies::mirror(&running, &account, &machine, "", &token, false).await.unwrap_or_else(|err| panic!("{err}"));
            let _ = done.send("mirror");
        });
    }
    wait_for("the mirror to wait on the fires table", || waiting_on_fires(&locker_runtime, &hub.database) > 0);
    // Two more snapshots of the same account, as two more machines connecting at once ask for.
    for _ in 0..2 {
        let (connectors, done) = (connectors.clone(), done.clone());
        hub.runtime.spawn(async move {
            connectors.copy_parts();
            let _ = done.send("copy_parts");
        });
    }
    std::thread::sleep(Duration::from_millis(500));

    let url = hub.url.clone();
    let version_answered = returns_within(ANSWER_WITHIN, move || {
        let client = reqwest::blocking::Client::builder().timeout(ANSWER_WITHIN * 2).build().unwrap();
        client.get(format!("{url}/api/version")).send().unwrap().error_for_status().unwrap();
    });
    let store_answered = {
        let connectors = connectors.clone();
        returns_within(ANSWER_WITHIN, move || {
            connectors.runner();
        })
    };

    // The database answers again: the mirror and both snapshots finish.
    locker_runtime.block_on(locker.batch_execute("rollback")).unwrap();
    let mut finished_calls = Vec::new();
    while let Ok(call) = finished.recv_timeout(WAIT) {
        finished_calls.push(call);
        if finished_calls.len() == 3 {
            break;
        }
    }
    // A frozen runtime is let go of, not waited for, so a failure ends the test.
    hub.stop();

    assert_eq!(
        (version_answered, store_answered, finished_calls.len()),
        (true, true, 3),
        "while a mirror waited on the fires: GET /api/version answered within {ANSWER_WITHIN:?}, the store answered within {ANSWER_WITHIN:?}; after the fires table was let go these finished: {finished_calls:?}"
    );
}
