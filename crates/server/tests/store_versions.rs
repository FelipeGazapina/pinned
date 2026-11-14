//! The account row's store and connector keys against a real Postgres: a write carries a version
//! and never replaces a newer one. Needs the test Postgres that `hub.rs` uses.
use pinned_server::db::{Db, Sealer, WriteFailure};
use std::collections::BTreeMap;
use uuid::Uuid;

/// The test Postgres, where the role `pinned` owns a database `pinned`. Override with PINNED_TEST_POSTGRES.
fn postgres() -> String {
    std::env::var("PINNED_TEST_POSTGRES").unwrap_or_else(|_| "postgres://pinned@127.0.0.1:5499".into())
}

/// A fresh database, its Db and one account in it.
async fn fresh(name: &str, dir: &std::path::Path) -> (String, Db, Uuid) {
    let database = format!("pinned_test_{name}_{}", std::process::id());
    let (client, connection) = tokio_postgres::connect(&format!("{}/pinned", postgres()), tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    client.batch_execute(&format!("drop database if exists {database} with (force)")).await.unwrap();
    client.batch_execute(&format!("create database {database}")).await.unwrap();
    let url = format!("{}/{database}", postgres());
    let db = Db::connect(&url, Sealer::load(&dir.join("hub.key")).unwrap()).await.unwrap();
    let account = db.create_account(&format!("{name}@example.test"), "x").await.unwrap().unwrap();
    (url, db, account.id.parse().unwrap())
}

#[tokio::test]
async fn an_older_version_never_replaces_a_newer_one_and_a_reopened_hub_writes_above_the_stored_version() {
    let dir = tempfile::tempdir().unwrap();
    let (url, db, account) = fresh("versions", dir.path()).await;
    assert!(db.save_store(account, 2, Some(r#"{"n":2}"#), Some(r#"[{"run":2}]"#)).await.unwrap(), "version 2 lands");
    assert!(!db.save_store(account, 1, Some(r#"{"n":1}"#), None).await.unwrap(), "version 1 comes too late and is not written");
    assert!(!db.save_store(account, 2, Some(r#"{"n":"again"}"#), None).await.unwrap(), "nor is a second version 2");
    let keys = |v: &str| BTreeMap::from([("linear".to_owned(), v.to_owned())]);
    assert!(db.save_secrets(account, 3, &keys("three")).await.unwrap());
    assert!(!db.save_secrets(account, 1, &keys("one")).await.unwrap());
    let stored = db.connectors(account).await.unwrap();
    assert_eq!(stored.store, serde_json::json!({ "n": 2, "runs": [{ "run": 2 }] }), "the row keeps version 2's document");
    assert_eq!(stored.secrets, keys("three"));
    assert_eq!(stored.version, 3, "the newest version of either kind");

    // A new hub process reads the versions back and writes above them.
    drop(db);
    let reopened = Db::connect(&url, Sealer::load(&dir.path().join("hub.key")).unwrap()).await.unwrap();
    let stored = reopened.connectors(account).await.unwrap();
    assert_eq!(stored.version, 3);
    assert!(reopened.save_store(account, stored.version + 1, Some(r#"{"n":4}"#), None).await.unwrap(), "a write after reopening is accepted");
    assert_eq!(reopened.connectors(account).await.unwrap().store, serde_json::json!({ "n": 4 }), "runs absent means no runs in the row");
}


#[tokio::test]
async fn a_document_as_deep_as_the_hub_accepts_lands_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let (_, db, account) = fresh("deep", dir.path()).await;
    let depth = pinned_server::hub::DOCUMENT_DEPTH;
    // The store at the deepest a save accepts, and runs one less, under `runs` in the same row.
    let store = format!(r#"{{"deep":{}0{}}}"#, "[".repeat(depth - 1), "]".repeat(depth - 1));
    let runs = format!("{}0{}", "[".repeat(depth - 1), "]".repeat(depth - 1));
    assert!(db.save_store(account, 1, Some(&store), Some(&runs)).await.unwrap());
    let stored = db.connectors(account).await.unwrap();
    assert!(stored.store["deep"].is_array() && stored.store["runs"].is_array(), "the row reads back as a document");
}

#[tokio::test]
async fn postgres_refusing_a_document_is_a_permanent_failure_and_a_later_document_still_lands() {
    let dir = tempfile::tempdir().unwrap();
    let (_, db, account) = fresh("refused", dir.path()).await;
    // jsonb has no \u0000; serde reads it as a plain string.
    match db.save_store(account, 1, Some(r#"{"n":"a\u0000b"}"#), None).await {
        Err(WriteFailure::Refused { sqlstate }) => assert!(sqlstate.starts_with("22"), "a data exception: {sqlstate}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(db.save_store(account, 2, Some(r#"{"n":2}"#), None).await.unwrap());
    assert_eq!(db.connectors(account).await.unwrap().store, serde_json::json!({ "n": 2 }));
}

/// A write that gives its connection up while Postgres runs its statement, which waits for the
/// row: a newer document lands meanwhile, and the given-up statement, which Postgres still runs
/// once the row is free, does not replace it. The hub then writes on a new connection.
#[tokio::test]
async fn a_write_given_up_that_postgres_runs_late_does_not_replace_the_newer_document() {
    let dir = tempfile::tempdir().unwrap();
    let (url, db, account) = fresh("given_up", dir.path()).await;
    db.shorten_write_wait(std::time::Duration::from_secs(2));
    let (other, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    // Another writer holds the row, as another hub process or a slow statement would.
    other.batch_execute("begin").await.unwrap();
    other.execute("select 1 from accounts where id = $1 for update", &[&account]).await.unwrap();

    match db.save_store(account, 1, Some(r#"{"n":1}"#), None).await {
        Err(WriteFailure::Failed(e)) => assert!(e.contains("given up"), "{e}"),
        other => panic!("expected the write to give up, got {other:?}"),
    }
    let waiting = "select count(*) from pg_stat_activity where datname = current_database() and wait_event_type = 'Lock'";
    assert_eq!(other.query_one(waiting, &[]).await.unwrap().get::<_, i64>(0), 1, "the given-up statement still waits in Postgres");

    // The newer document lands, then the row is free and the late statement runs.
    other.execute("update accounts set store = '{\"n\":2}', store_version = 2 where id = $1", &[&account]).await.unwrap();
    other.batch_execute("commit").await.unwrap();
    for _ in 0..250 {
        if other.query_one(waiting, &[]).await.unwrap().get::<_, i64>(0) == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let row = other.query_one("select store, store_version from accounts where id = $1", &[&account]).await.unwrap();
    assert_eq!((row.get::<_, serde_json::Value>(0), row.get::<_, i64>(1)), (serde_json::json!({ "n": 2 }), 2), "the late write of version 1 did not land");

    assert!(db.save_store(account, 3, Some(r#"{"n":3}"#), None).await.unwrap(), "the hub writes again on a new connection");
    assert_eq!(db.connectors(account).await.unwrap().store, serde_json::json!({ "n": 3 }));
}

/// Two hub processes, as every deploy runs them for a while, each with its own connection and its
/// own copy of the account, built as the hub builds it. Both load the account at the same
/// version; the first saves many times, then the second, numbered lower, saves once. The row ends
/// with the second's document and keys: the write that lands last wins.
#[tokio::test]
async fn the_later_save_of_the_lower_numbered_of_two_hub_processes_lands_in_the_row() {
    use pinned_server::hub::AccountStore;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let (url, db, account) = fresh("two_processes", dir.path()).await;
    assert!(db.save_store(account, 5, Some(r#"{"n":0}"#), None).await.unwrap());
    let process = || async { Db::connect(&url, Sealer::load(&dir.path().join("hub.key")).unwrap()).await.unwrap() };
    let first = AccountStore::load(process().await, account).await.unwrap();
    let second = AccountStore::load(process().await, account).await.unwrap();

    for n in 1..=20 {
        first.save(format!(r#"{{"by":"first","n":{n}}}"#).as_bytes()).unwrap();
        first.write_key("linear", &format!("first {n}")).unwrap();
        assert!(first.flush(Duration::from_secs(10)).await);
    }
    let stored = db.connectors(account).await.unwrap();
    assert_eq!(stored.store, serde_json::json!({ "by": "first", "n": 20 }));
    assert_eq!(stored.version, 45, "the first process numbered from 6 to 45");

    second.save(br#"{"by":"second","n":1}"#).unwrap();
    second.write_key("linear", "second").unwrap();
    assert!(second.flush(Duration::from_secs(10)).await);
    let stored = db.connectors(account).await.unwrap();
    assert_eq!(stored.store, serde_json::json!({ "by": "second", "n": 1 }), "the second process's later store is in the row");
    assert_eq!(stored.secrets, BTreeMap::from([("linear".to_owned(), "second".to_owned())]), "and its later keys");
    assert!(stored.version > 45, "written above what the first process wrote: {}", stored.version);

    // The first saves again, and that later save lands in its turn.
    first.save(br#"{"by":"first","n":21}"#).unwrap();
    assert!(first.flush(Duration::from_secs(10)).await);
    assert_eq!(db.connectors(account).await.unwrap().store, serde_json::json!({ "by": "first", "n": 21 }));
}

/// What `save` takes, jsonb holds: an escaped backslash before the text u0000 and a surrogate
/// pair land in the row as saved. What jsonb would refuse, `save` refuses first.
#[tokio::test]
async fn a_document_save_accepts_lands_in_jsonb_and_one_jsonb_refuses_is_refused_at_save() {
    use pinned_server::hub::AccountStore;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let (_, db, account) = fresh("escapes", dir.path()).await;
    let storage = AccountStore::load(db.clone(), account).await.unwrap();
    for refused in [r#"{"n":"a\u0000b"}"#, r#"{"n":{"\u0000":1}}"#, r#"{"n":"\ud800"}"#, r#"{"n":"\udc00\ud800"}"#] {
        assert!(storage.save(refused.as_bytes()).is_err(), "save refuses {refused}");
        assert!(matches!(db.save_store(account, 1, Some(refused), None).await, Err(WriteFailure::Refused { .. })), "as jsonb does: {refused}");
    }
    let store = r#"{"a":"x\\u0000y","b\\u0000":"\\\\u0000","c":"😀","d":"é"}"#;
    storage.save(store.as_bytes()).unwrap();
    assert!(storage.flush(Duration::from_secs(10)).await);
    let stored = db.connectors(account).await.unwrap();
    assert_eq!(stored.store, serde_json::from_str::<serde_json::Value>(store).unwrap(), "the row holds the document as saved");
    assert_eq!(stored.store["a"], "x\\u0000y");
    assert_eq!(stored.version, 1);
}

/// The account's row is deleted while the hub holds the account. A save finds no row: the writer
/// drops the document after one try instead of retrying it forever, and its slot empties.
#[tokio::test]
async fn a_save_for_an_account_whose_row_is_gone_is_dropped_after_one_try() {
    use pinned_server::hub::AccountStore;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let (url, db, account) = fresh("row_gone", dir.path()).await;
    let storage = AccountStore::load(db.clone(), account).await.unwrap();
    let (other, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    assert_eq!(other.execute("delete from accounts where id = $1", &[&account]).await.unwrap(), 1);
    assert_eq!(db.stored_versions(account).await.unwrap(), None, "no row, no versions");

    storage.save(br#"{"n":1}"#).unwrap();
    storage.write_key("linear", "k").unwrap();
    // A retry would wait out a backoff of 0.5 s at least; one try answers at once.
    let started = std::time::Instant::now();
    assert!(storage.flush(Duration::from_secs(5)).await, "the slots empty");
    assert!(started.elapsed() < Duration::from_millis(400), "after one try, no retry: {:?}", started.elapsed());
}

/// A statement Postgres cannot run against the schema (SQLSTATE class 42) is not the document's
/// fault: the write fails and is retried, it is not refused and dropped.
#[tokio::test]
async fn a_statement_the_schema_does_not_fit_is_a_failure_to_retry_not_a_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let (url, db, account) = fresh("class_42", dir.path()).await;
    let (other, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    other.batch_execute("alter table accounts rename column store_version to store_version_moved").await.unwrap();
    match db.save_store(account, 1, Some(r#"{"n":1}"#), None).await {
        Err(WriteFailure::Failed(_)) => {}
        other => panic!("expected a failure to retry, got {other:?}"),
    }
    other.batch_execute("alter table accounts rename column store_version_moved to store_version").await.unwrap();
    assert!(db.save_store(account, 1, Some(r#"{"n":1}"#), None).await.unwrap(), "the same write lands once the schema fits");
}

/// A row the hub cannot read, a number jsonb holds that serde_json can't (past f64), makes
/// loading the account an error, not a panic inside the hub.
#[tokio::test]
async fn loading_an_account_whose_store_cannot_be_read_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let (url, db, account) = fresh("unreadable", dir.path()).await;
    let (other, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    other.execute(r#"update accounts set store = '{"n":1e999}'::jsonb where id = $1"#, &[&account]).await.unwrap();
    let loaded = tokio::spawn(async move { db.connectors(account).await }).await.expect("connectors did not panic");
    let e = loaded.err().expect("an unreadable store is an error");
    assert!(e.contains("unreadable store"), "{e}");
}
