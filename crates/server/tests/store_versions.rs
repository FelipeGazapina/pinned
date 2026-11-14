//! The account row's store and connector keys against a real Postgres: a write carries a version
//! and never replaces a newer one. Needs the test Postgres that `hub.rs` uses.
use pinned_server::db::{Db, Sealer};
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
    client.batch_execute(&format!("drop database if exists {database} with (force); create database {database}")).await.unwrap();
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

