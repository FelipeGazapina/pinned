//! One run per trigger event: `trigger_fires` keeps each key an account's workflow claimed, so two
//! hubs on one database, each with its own `Connectors` of the account, that see one event start
//! one run (`two_hubs_that_see_one_linear_change_start_one_run` counts the runs; the claim test
//! below counts claims). The hub's hourly expiry drops claims older than 7 days; the key can be
//! claimed again. A claim whose insert timed out but was stored is won by its own claimer's retry.
use super::*;
use pinned_core::connectors::{Connectors, Keys};
use std::sync::atomic::AtomicUsize;

const KEY: &str = "linear:ISS-1#h-1";

/// The account's `Connectors` on the hub `hub`, as its check uses them.
fn connectors_of(runtime: &tokio::runtime::Runtime, hub: &Arc<pinned_server::hub::Hub>, account: uuid::Uuid) -> Arc<Connectors> {
    runtime.block_on(async {
        let account = hub.account(account).await.unwrap();
        hub.connectors(&account).await.unwrap()
    })
}

#[test]
fn a_trigger_fire_is_claimed_once_per_account_across_two_hubs_until_its_claim_expires() {
    let hub = TestHub::start("trigger_fires");
    let id = |email: &str| {
        let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
        uuid::Uuid::parse_str(&view.id).unwrap()
    };
    let (ana, bob) = (id("ana@acme.dev"), id("bob@acme.dev"));
    let (twin, running) = hub.twin();
    let ana_here = connectors_of(&hub.runtime, hub.hub(), ana);
    let ana_there = connectors_of(&twin, &running.hub, ana);
    let bob_here = connectors_of(&hub.runtime, hub.hub(), bob);

    // Another account with the same workflow id and key claims its own fire.
    assert_eq!(bob_here.claim_fire("wf-1", KEY), Ok(true), "the first claim of an account wins");

    // Both hubs claim one event of one account at the same moment, from inside their checks' runtimes.
    let gate = Arc::new(tokio::sync::Barrier::new(2));
    let claim = |runtime: &tokio::runtime::Runtime, connectors: &Arc<Connectors>| {
        let (connectors, gate) = (connectors.clone(), gate.clone());
        runtime.spawn(async move {
            gate.wait().await;
            connectors.claim_fire("wf-1", KEY)
        })
    };
    let (here, there) = (claim(&hub.runtime, &ana_here), claim(&twin, &ana_there));
    let mut claims = vec![hub.runtime.block_on(here).unwrap(), twin.block_on(there).unwrap()];
    claims.sort();
    assert_eq!(claims, [Ok(false), Ok(true)], "exactly one of the two hubs claims the fire, despite the other account's claim");
    assert_eq!(ana_here.claim_fire("wf-1", KEY), Ok(false), "a claimed key stays claimed");
    assert_eq!(ana_there.claim_fire("wf-1", KEY), Ok(false), "on either hub");
    assert_eq!(bob_here.claim_fire("wf-1", KEY), Ok(false), "for the other account too");

    // The hourly expiry drops a claim older than 7 days, and keeps a younger one.
    admin_on(&hub.database, &format!(
        "update trigger_fires set at = now() - interval '8 days' where account = '{ana}';
         update trigger_fires set at = now() - interval '6 days' where account = '{bob}';"
    ));
    hub.runtime.block_on(hub.hub().expire_held_ends());
    assert_eq!(ana_there.claim_fire("wf-1", KEY), Ok(true), "an expired claim's key can be claimed again");
    assert_eq!(ana_here.claim_fire("wf-1", KEY), Ok(false), "once");
    assert_eq!(bob_here.claim_fire("wf-1", KEY), Ok(false), "a claim of 6 days stays");
}

/// Two hubs, each with its own `Connectors` of a person's account, check at the same moment and
/// both read one Linear move into Ready to Dev: exactly one run starts, and the next checks start
/// none.
#[test]
fn two_hubs_that_see_one_linear_change_start_one_run() {
    super::teams_admission::standins();
    let at = super::teams_admission::CHANGE_AT.get_or_init(|| (chrono::Utc::now() - chrono::Duration::minutes(2)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let at = chrono::DateTime::parse_from_rfc3339(at).unwrap();
    let hub = TestHub::start("trigger_fires_runs");
    let (_, view) = account::signup(&hub.url, "cara@acme.dev", "correct horse").unwrap();
    let cara = uuid::Uuid::parse_str(&view.id).unwrap();
    let (twin, running) = hub.twin();
    let here = connectors_of(&hub.runtime, hub.hub(), cara);
    let there = connectors_of(&twin, &running.hub, cara);
    let since = (at - chrono::Duration::minutes(8)).to_rfc3339();
    let store = json!({ "connectors": [{
        "id": "wf-change", "name": "Pick up", "enabled": true, "trigger": { "kind": "linear-status", "to": "Ready to Dev" },
        "folder": "/repo", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "", "since": since,
    }] });
    for connectors in [&here, &there] {
        connectors.import(store.clone(), Keys { linear: Some("lin_tf_change".into()), ..Keys::default() }).unwrap();
    }

    let opened = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(tokio::sync::Barrier::new(2));
    let check = |runtime: &tokio::runtime::Runtime, connectors: &Arc<Connectors>| {
        let (connectors, opened, gate) = (connectors.clone(), opened.clone(), gate.clone());
        runtime.spawn(async move {
            gate.wait().await;
            let open = move |work: Work| {
                if matches!(work, Work::Run(_)) {
                    opened.fetch_add(1, Ordering::SeqCst);
                }
                async { Ok::<(), String>(()) }
            };
            connectors.check_once(&open).await.error
        })
    };
    let (a, b) = (check(&hub.runtime, &here), check(&twin, &there));
    assert_eq!((hub.runtime.block_on(a).unwrap(), twin.block_on(b).unwrap()), (None, None));
    assert_eq!(opened.load(Ordering::SeqCst), 1, "exactly one run starts for one change seen by two hubs");

    let (a, b) = (check(&hub.runtime, &here), check(&twin, &there));
    assert_eq!((hub.runtime.block_on(a).unwrap(), twin.block_on(b).unwrap()), (None, None));
    assert_eq!(opened.load(Ordering::SeqCst), 1, "the next checks start none");
}

/// A claim whose insert timed out may still be stored. Its claimer's retry wins it; another
/// claimer's retry, and a claim that is not a retry, lose it.
#[test]
fn a_claim_stored_by_a_timed_out_insert_is_won_by_its_claimers_retry_only() {
    let hub = TestHub::start("trigger_fires_retry");
    let (_, view) = account::signup(&hub.url, "dan@acme.dev", "correct horse").unwrap();
    let dan = uuid::Uuid::parse_str(&view.id).unwrap();
    let (twin, running) = hub.twin();
    let here = connectors_of(&hub.runtime, hub.hub(), dan);
    let there = connectors_of(&twin, &running.hub, dan);
    assert_ne!(here.claimer(), there.claimer());
    admin_on(&hub.database, &format!(
        "insert into trigger_fires (account, workflow, key, claimer) values ('{dan}', 'wf-1', '{KEY}', '{}');",
        here.claimer()
    ));
    let retry = |runtime: &tokio::runtime::Runtime, connectors: &Arc<Connectors>, again: bool| {
        let connectors = connectors.clone();
        runtime.block_on(runtime.spawn(async move {
            match again {
                true => connectors.claim_fire_retry("wf-1", KEY),
                false => connectors.claim_fire("wf-1", KEY),
            }
        })).unwrap()
    };
    assert_eq!(retry(&twin, &there, true), Ok(false), "another claimer's retry loses");
    assert_eq!(retry(&hub.runtime, &here, true), Ok(true), "its claimer's retry wins");
    assert_eq!(retry(&hub.runtime, &here, false), Ok(false), "a claim that is not a retry loses");
}
