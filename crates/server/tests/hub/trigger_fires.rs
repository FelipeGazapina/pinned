//! One run per trigger event: `trigger_fires` keeps each key an account's workflow claimed, so two
//! hubs on one database, each with its own `Connectors` of the account, that see one event start
//! one run. The hub's hourly expiry drops claims older than 7 days; the key can be claimed again.
use super::*;
use pinned_core::connectors::Connectors;

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
