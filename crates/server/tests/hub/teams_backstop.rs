//! pinned Teams, the reservation backstop: a run whose place is held (reserved) and whose session
//! never opens ends at the team policy's limit, counted on its bot, and frees its place; the
//! reaper never ends a reserved run that is still within it.

use super::teams_admission::{none, Crew, POLLS_HELD, THINKING};
use super::*;
use pinned_core::teams::TeamOp;
use pinned_server::teams;

impl Crew {
    /// The team's runs of bot `id` as a member lists them: (state, outcome) of each.
    fn listed(&self, id: &str) -> Vec<(String, Option<String>)> {
        let runs = self.call(&self.bob, TeamOp::Runs).unwrap();
        runs.as_array().unwrap().iter().filter(|r| r["workflow"] == id).map(|r| (r["state"].as_str().unwrap().to_owned(), r["outcome"].as_str().map(str::to_owned))).collect()
    }

    /// Moves back when bot `id`'s reserved runs got their place and when they started.
    fn age_reserved(&self, id: &str, reserved: &str, started: &str) {
        admin_on(
            &self.hub.database,
            &format!("update team_runs set reserved_at = now() - interval '{reserved}', started_at = now() - interval '{started}' where team = '{}' and workflow = '{id}' and reserved", self.team),
        );
    }

    /// One pass of the hub's calls tick, as `teams::spawn_tick` runs it.
    fn tick(&self) {
        self.hub.runtime.block_on(teams::calls::tick(self.hub.hub(), &self.hub.team_policy)).unwrap();
    }

    /// The team may run one at a time, and a run of bot `holder` takes that place.
    fn take_one(&self, holder: &str) {
        admin_on(&self.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", self.team));
        assert_eq!(self.found(self.labeled(holder, 9)), Ok(()));
        assert_eq!(self.live_of(holder).len(), 1, "the holder's run takes the only place");
    }

    /// Hands the team event ENG-`n` of bot `id` in a task of its own, with its box's boot held
    /// until the answered gate is told; answers once the box's bootstrap started.
    fn boot_held(&self, id: &str, n: u32) -> (Arc<tokio::sync::Notify>, tokio::task::JoinHandle<Result<(), String>>) {
        let gate = Arc::new(tokio::sync::Notify::new());
        self.hub.boxes.state.lock().unwrap().gate = Some(gate.clone());
        let (hub, team, event) = (self.hub.hub().clone(), self.team, self.labeled(id, n));
        let found = self.hub.runtime.spawn(async move {
            let account = hub.account(team).await?;
            let connectors = hub.connectors(&account).await?;
            let open = |work| {
                let hub = hub.clone();
                async move { hub.open_found(team, work).await }
            };
            connectors.found(event, &open).await
        });
        wait_for("the box's bootstrap to start", || self.hub.boxes.state.lock().unwrap().scripts.iter().any(|s| s.contains("start.sh")));
        assert_eq!(self.reserved_at(id), vec!["set".to_owned()], "its place is held while its box boots");
        (gate, found)
    }
}

fn failed_never_started() -> Vec<(String, Option<String>)> {
    vec![("failed".to_owned(), Some("it never started".to_owned()))]
}

#[test]
fn a_reservation_that_never_opens_is_ended_and_counted() {
    let mut crew = Crew::new("tb_never_opens");
    THINKING.lock().unwrap().push("tb_never_opens".into());
    let (holder, pages, docs) = (crew.on_bot("Holder"), crew.on_bot("Pages"), crew.on_bot("Docs"));
    crew.take_one(&holder);
    assert_eq!(crew.found(crew.labeled(&pages, 1)), Ok(()));
    let held = crew.live_of(&holder);
    crew.stop(&held[0]);
    // Claimed: its Ask step asked the model, which is still thinking when the hub stops.
    wait_for("ENG-1's claim and its model call", || crew.outside(1) == vec!["model".to_owned()]);
    crew.hub.restart();
    THINKING.lock().unwrap().retain(|t| t != "tb_never_opens");
    assert_eq!(crew.reserved_at(&pages), vec!["set".to_owned()], "its reserved run holds the place");
    assert_eq!(crew.found(crew.labeled(&docs, 2)), Ok(()));
    assert_eq!((crew.waiting(&docs), crew.live_of(&docs).len()), (1, 0), "Docs' ENG-2 waits behind the reservation");

    crew.age_reserved(&pages, "16 minutes", "16 minutes");
    let boxes_before = crew.boxes_created();
    crew.tick();
    assert_eq!(crew.listed(&pages), failed_never_started(), "listed as failed: it never started");
    assert_eq!(crew.skipped(&pages), json!([{ "reason": "never_started", "count": 1 }]));
    wait_for("Docs' ENG-2 to take the freed place", || crew.live_of(&docs).len() == 1 && crew.waiting(&docs) == 0);
    // A few checks pass meanwhile: nothing of ENG-1 runs after its end.
    std::thread::sleep(Duration::from_secs(3));
    crew.tick();
    assert_eq!(crew.outside(1), vec!["model".to_owned()], "no second model call and no Linear move for ENG-1");
    assert_eq!(crew.skipped(&pages), json!([{ "reason": "never_started", "count": 1 }]), "counted once");
    assert_eq!(crew.listed(&pages), failed_never_started());
    assert_eq!(crew.boxes_created() - boxes_before, 1, "only Docs' ENG-2 got a box");
}

#[test]
fn an_unclaimed_grant_past_the_limit_is_dropped_with_its_reservation() {
    let crew = Crew::new("tb_unclaimed");
    let (holder, triage) = (crew.on_bot("Holder"), crew.graph_bot("Triage"));
    crew.take_one(&holder);
    assert_eq!(crew.found(crew.graph_labeled(&triage, 1)), Ok(()));
    assert_eq!(crew.waiting(&triage), 1, "ENG-1 waits");
    // No check of the team's triggers gets past its Linear poll, so none claims the grant.
    POLLS_HELD.lock().unwrap().push("tb_unclaimed".into());
    let held = crew.live_of(&holder);
    crew.stop(&held[0]);
    wait_for("ENG-1's grant", || crew.reserved_at(&triage) == vec!["set".to_owned()]);
    assert_eq!(crew.waiting(&triage), 1, "granted, its row waits for a claim");
    let boxes_before = crew.boxes_created();

    crew.age_reserved(&triage, "16 minutes", "16 minutes");
    crew.tick();
    assert_eq!(crew.listed(&triage), failed_never_started(), "listed as failed: it never started");
    assert_eq!(crew.skipped(&triage), json!([{ "reason": "never_started", "count": 1 }]));
    assert_eq!(crew.waiting(&triage), 0, "its granted row is gone");

    POLLS_HELD.lock().unwrap().retain(|t| t != "tb_unclaimed");
    // The held check goes on, and a few more pass: the dropped event never runs.
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(crew.outside(1), none(), "nothing of ENG-1 reached outside pinned");
    assert_eq!(crew.boxes_created() - boxes_before, 0, "no session opened");
    assert_eq!(crew.listed(&triage), failed_never_started());
    assert_eq!(crew.skipped(&triage), json!([{ "reason": "never_started", "count": 1 }]), "counted once");
}

#[test]
fn a_reservation_older_than_the_boot_wait_is_left_while_its_graph_runs() {
    let crew = Crew::new("tb_left");
    let pages = crew.on_bot("Pages");
    let (gate, found) = crew.boot_held(&pages, 1);
    // Started more than the box boot wait and RUN_WAIT ago, its place held for less than the limit.
    crew.age_reserved(&pages, "5 minutes", "45 minutes");

    crew.tick();
    assert_eq!(crew.listed(&pages), vec![("working".to_owned(), None)], "still live");
    assert_eq!(crew.reserved_at(&pages), vec!["set".to_owned()], "still reserved");
    assert_eq!(crew.skipped(&pages), Value::Null);

    gate.notify_one();
    assert_eq!(crew.hub.runtime.block_on(found).unwrap(), Ok(()));
    wait_for("ENG-1's session to move its issue", || crew.outside(1).len() == 2);
    // A few checks pass meanwhile: nothing runs a second time.
    std::thread::sleep(Duration::from_secs(3));
    crew.tick();
    assert_eq!(crew.outside(1), vec!["model".to_owned(), "linear move".to_owned()], "each outside step exactly once");
    assert_eq!(crew.reserved_at(&pages), none(), "its session opened in the place it held");
    assert_eq!(crew.listed(&pages), vec![("working".to_owned(), None)], "its session runs, untouched by the reaper");
}

#[test]
fn a_reserved_run_stopped_while_its_box_boots_stays_listed_stopped() {
    let crew = Crew::new("tb_stop_booting");
    let (holder, pages, docs) = (crew.on_bot("Holder"), crew.on_bot("Pages"), crew.on_bot("Docs"));
    crew.take_one(&holder);
    assert_eq!(crew.found(crew.labeled(&pages, 1)), Ok(()));
    assert_eq!(crew.found(crew.labeled(&docs, 2)), Ok(()));
    assert_eq!((crew.waiting(&pages), crew.waiting(&docs)), (1, 1), "ENG-1 and ENG-2 wait");
    let gate = Arc::new(tokio::sync::Notify::new());
    crew.hub.boxes.state.lock().unwrap().gate = Some(gate.clone());
    let boots = || crew.hub.boxes.state.lock().unwrap().scripts.iter().filter(|s| s.contains("start.sh")).count();
    let booted = boots();
    let held = crew.live_of(&holder);
    crew.stop(&held[0]);
    wait_for("ENG-1's box's bootstrap to start", || boots() > booted);
    assert_eq!(crew.reserved_at(&pages), vec!["set".to_owned()], "its place is held while its box boots");

    let live = crew.live_of(&pages);
    let stopped = crew.call(&crew.ana, serde_json::from_value(json!({ "op": "stop_run", "run": live[0] })).unwrap()).unwrap();
    assert_eq!((&stopped["state"], &stopped["outcome"]), (&json!("stopped"), &json!("stopped by ana")));
    gate.notify_one();

    wait_for("Docs' ENG-2 to take the freed place", || crew.live_of(&docs).len() == 1 && crew.waiting(&docs) == 0);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(crew.listed(&pages), vec![("stopped".to_owned(), Some("stopped by ana".to_owned()))], "listed as it ended");
    assert_eq!(crew.skipped(&pages), Value::Null, "a stop is not a failed start");
    assert_eq!(crew.outside(1), vec!["model".to_owned()], "ENG-1 asked the model, and no session moved its issue");
}
