//! "My reports" follows GitHub: the poller turns a maintainer's answer, a close, a fix in a release and a reopening
//! into the state a report's list shows. GitHub is `fake_github`, in memory on 127.0.0.1, and its clock starts
//! when the test does, so what happens there is dated like what the hub does.

use super::*;
use pinned_server::reports::fake_github::{self, iso, FakeGithub};
use pinned_server::reports::Config;

const PASSWORD: &str = "correct horse";
/// How often the poller looks, in these tests.
const POLL: Duration = Duration::from_secs(1);
const SEARCH_PREFIX: &str = "/search/issues?";

struct Fake {
    _runtime: tokio::runtime::Runtime,
    github: FakeGithub,
}

impl Fake {
    /// A fake GitHub whose token is the owner's, as the hub's is.
    fn start() -> Fake {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let github = runtime.block_on(fake_github::start());
        github.set_clock(now());
        github.set_hub_association("OWNER");
        Fake { _runtime: runtime, github }
    }

    fn config(&self) -> Config {
        Config { api: self.github.url.clone(), repo: "acme/app".into(), token: "fake-hub-token".into(), poll: POLL }
    }

    /// The `q` of every search the hub made, oldest first.
    fn searches(&self) -> Vec<String> {
        self.github
            .requests()
            .iter()
            .filter(|r| r.path.starts_with(SEARCH_PREFIX))
            .map(|r| url::form_urlencoded::parse(r.path[SEARCH_PREFIX.len()..].as_bytes()).find(|(k, _)| k == "q").map(|(_, v)| v.into_owned()).unwrap_or_default())
            .collect()
    }

    fn comment_reads(&self) -> usize {
        self.github.requests().iter().filter(|r| r.method == "GET" && r.path.contains("/comments?")).count()
    }

    /// Waits until the poller has run a whole poll since now: the search after the next one is made only when the
    /// poll that made the next one has written everything it decided.
    fn settle(&self) {
        let base = self.searches().len();
        until("a whole poll", || self.searches().len() >= base + 2);
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn polling(name: &str, fake: Option<&Fake>) -> TestHub {
    TestHub::start_reports(name, Signup::Open, Policy::default(), false, Default::default(), "127.0.0.1:0", fake.map(Fake::config))
}

fn signed_up(hub: &TestHub, email: &str) -> Client {
    let token = account::signup(&hub.url, email, PASSWORD).unwrap().0;
    let mut app = Client::app(&hub.url, &token);
    app.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    app
}

/// Files a report, as the dialog does, new or as another case on `to_issue`; returns the issue it landed on.
fn send(app: &mut Client, title: &str, to_issue: Option<u64>) -> u64 {
    let mut draft = json!({ "kind": "problem", "title": title, "happened": "It stops.", "expected": "It works.", "repeat": "1. Open it" });
    if let Some(issue) = to_issue {
        draft["to_issue"] = json!(issue);
    }
    let sent = app.hub(json!({ "cmd": "report_issue", "draft": draft })).unwrap();
    sent["issue"].as_u64().unwrap()
}

fn rows(app: &mut Client) -> Vec<Value> {
    app.hub(json!({ "cmd": "reports", "ask": { "op": "list" } })).unwrap().as_array().unwrap().clone()
}

/// The state of the report on `issue`, as the app's list shows it.
fn state(app: &mut Client, issue: u64) -> Value {
    rows(app).iter().find(|r| r["issue"] == issue).map(|r| r["state"].clone()).unwrap_or(Value::Null)
}

fn wait_state(app: &mut Client, issue: u64, expected: Value) {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let found = state(app, issue);
        if found == expected {
            return;
        }
        assert!(Instant::now() < end, "issue {issue} never read {expected}, it reads {found}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn sent_at(app: &mut Client, issue: u64) -> i64 {
    rows(app).iter().find(|r| r["issue"] == issue).unwrap()["created_at"].as_i64().unwrap()
}

fn updated_at(app: &mut Client, issue: u64) -> i64 {
    rows(app).iter().find(|r| r["issue"] == issue).unwrap()["updated_at"].as_i64().unwrap()
}

/// Everything the `reports` table holds, as text.
fn stored(hub: &TestHub) -> String {
    hub.runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.query("select row_to_json(reports)::text from reports order by created_at", &[]).await.unwrap().iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n")
    })
}

fn closed(reason: &str) -> Value {
    json!({ "state": "closed", "reason": reason })
}

fn fixed(version: &str) -> Value {
    json!({ "state": "fixed", "version": version })
}

#[test]
fn a_maintainer_comment_answers_and_a_stranger_or_the_hubs_own_comment_never_does() {
    let fake = Fake::start();
    let hub = polling("poller_answer", Some(&fake));
    let (mut ana, mut bob) = (signed_up(&hub, "ana@example.test"), signed_up(&hub, "bob@example.test"));
    assert_eq!(send(&mut ana, "Upload stops at 90%", None), 1);
    // Bob adds a case to ana's issue: the hub posts it with the owner's token, so it reads as the owner's.
    assert_eq!(send(&mut bob, "Upload stops at 90% too", Some(1)), 1);
    let own = fake.github.comment_bodies(1);
    assert_eq!(own.len(), 1, "the hub's added case is the issue's one comment");

    // The hub's own comment, posted from bob's row, is not an answer on ana's row either.
    let reads = fake.comment_reads();
    fake.settle();
    assert!(fake.comment_reads() > reads, "the poller did read the comments");
    assert_eq!((state(&mut ana, 1), state(&mut bob, 1)), (json!({ "state": "open" }), json!({ "state": "open" })));

    // Somebody who is no maintainer writes.
    let later = sent_at(&mut bob, 1) + 3600;
    fake.github.add_comment_full(1, "a-stranger", "NONE", "Same here, please fix.", &iso(later));
    fake.github.add_comment_full(1, "a-contributor", "CONTRIBUTOR", "I can reproduce it.", &iso(later + 1));
    let reads = fake.comment_reads();
    fake.settle();
    assert!(fake.comment_reads() > reads, "the poller did read the new comments");
    assert_eq!((state(&mut ana, 1), state(&mut bob, 1)), (json!({ "state": "open" }), json!({ "state": "open" })));

    // The owner answers: both reports read Answered, at the comment's time.
    fake.github.add_comment_full(1, "the-maintainer", "OWNER", "We found it, a fix is coming.", &iso(later + 10));
    wait_state(&mut ana, 1, json!({ "state": "answered", "at": later + 10 }));
    wait_state(&mut bob, 1, json!({ "state": "answered", "at": later + 10 }));

    // Nothing but the state is kept, and a poll that changes nothing leaves `updated_at` alone.
    let kept = stored(&hub);
    for word in ["the-maintainer", "a-stranger", "We found it", "Same here, please fix", "OWNER"] {
        assert!(!kept.contains(word), "the table holds {word}: {kept}");
    }
    let (before_ana, before_bob) = (updated_at(&mut ana, 1), updated_at(&mut bob, 1));
    fake.settle();
    fake.settle();
    assert_eq!((updated_at(&mut ana, 1), updated_at(&mut bob, 1)), (before_ana, before_bob));
    assert!(before_ana >= sent_at(&mut ana, 1), "the state change dated the row");

    // Nothing the poller asked GitHub holds a word of the reports, and it only reads.
    for request in fake.github.requests().iter().filter(|r| r.method == "GET") {
        assert!(!request.path.contains("Upload") && !request.path.contains("fake-hub-token"), "{}", request.path);
    }
    for query in fake.searches() {
        assert!(query.starts_with("repo:acme/app is:issue updated:>=20") && query.split(' ').count() == 3, "{query}");
    }
}

#[test]
fn a_completed_issue_is_fixed_in_the_first_release_after_its_merged_pull_request_and_waits_for_one() {
    let fake = Fake::start();
    let hub = polling("poller_fixed", Some(&fake));
    let mut ana = signed_up(&hub, "ana@example.test");
    let merged = now() + 600;

    assert_eq!(send(&mut ana, "Upload stops at 90%", None), 1);
    fake.github.add_merged_pull(1, 42, &iso(merged));
    fake.github.close(1, "completed", None);
    fake.github.add_release("v0.18.6", &iso(merged - 100));
    fake.github.add_release("v0.18.8", &iso(merged + 200));
    fake.github.add_release("v0.18.7", &iso(merged + 100));
    fake.github.add_release_as("v0.19.0", &iso(merged + 50), true, false);
    fake.github.add_release_as("v0.18.9-rc1", &iso(merged + 60), false, true);
    wait_state(&mut ana, 1, fixed("0.18.7"));

    // A second one is merged after every release there is: closed, waiting.
    let later = merged + 10_000;
    assert_eq!(send(&mut ana, "Sync is slow", None), 2);
    fake.github.add_merged_pull(2, 43, &iso(later));
    fake.github.close(2, "completed", None);
    wait_state(&mut ana, 2, closed("completed"));
    fake.settle();
    assert_eq!(state(&mut ana, 2), closed("completed"), "no release yet, so it keeps waiting");
    assert_eq!(state(&mut ana, 1), fixed("0.18.7"));

    // The release comes while the issue itself does not change: GitHub's search says nothing changed.
    fake.github.set_search_blind(true);
    fake.github.add_release("v0.18.9", &iso(later + 100));
    wait_state(&mut ana, 2, fixed("0.18.9"));
    assert_eq!(state(&mut ana, 1), fixed("0.18.7"), "a fixed report stays where it is");
}

#[test]
fn an_issue_closed_as_not_planned_or_duplicate_says_so_and_any_other_reason_reads_closed() {
    let fake = Fake::start();
    let hub = polling("poller_closed", Some(&fake));
    let mut ana = signed_up(&hub, "ana@example.test");
    for (n, title) in ["Dark mode", "Upload stops", "Slow sync"].iter().enumerate() {
        assert_eq!(send(&mut ana, title, None), n as u64 + 1);
    }
    fake.github.close(1, "not_planned", None);
    fake.github.close(2, "duplicate", None);
    fake.github.close(3, "Please see https://example.test/secret: wontfix", None);
    wait_state(&mut ana, 1, closed("not_planned"));
    wait_state(&mut ana, 2, closed("duplicate"));
    wait_state(&mut ana, 3, closed("closed"));
    let kept = stored(&hub);
    assert!(!kept.contains("example.test/secret") && !kept.contains("wontfix"), "{kept}");
}

#[test]
fn a_fixed_in_label_set_by_hand_wins_and_a_label_or_tag_that_is_no_version_is_ignored() {
    let fake = Fake::start();
    let hub = polling("poller_label", Some(&fake));
    let mut ana = signed_up(&hub, "ana@example.test");
    let merged = now() + 600;
    for (n, title) in ["Upload stops", "Sync is slow", "Search is empty", "Login loops"].iter().enumerate() {
        assert_eq!(send(&mut ana, title, None), n as u64 + 1);
        fake.github.add_merged_pull(n as u64 + 1, 40 + n as u64, &iso(merged));
        fake.github.close(n as u64 + 1, "completed", None);
    }
    fake.github.add_release("nightly", &iso(merged + 10));
    fake.github.add_release("v0.18.7", &iso(merged + 20));
    fake.github.add_label(1, "fixed-in:0.18.9");
    fake.github.add_label(2, "fixed-in:latest");
    fake.github.add_label(3, "fixed-in:v0.20.1");
    fake.github.add_label(4, "fixed-in:1.2");
    wait_state(&mut ana, 1, fixed("0.18.9"));
    wait_state(&mut ana, 2, fixed("0.18.7"));
    wait_state(&mut ana, 3, fixed("0.20.1"));
    wait_state(&mut ana, 4, fixed("0.18.7"));
}

#[test]
fn a_reopened_issue_is_open_again_or_answered_if_it_had_been() {
    let fake = Fake::start();
    let hub = polling("poller_reopen", Some(&fake));
    let mut ana = signed_up(&hub, "ana@example.test");
    assert_eq!(send(&mut ana, "Upload stops", None), 1);
    assert_eq!(send(&mut ana, "Sync is slow", None), 2);

    // Closed as completed with nothing that says it was fixed: it waits, and a reopening ends the wait.
    fake.github.close(1, "completed", None);
    wait_state(&mut ana, 1, closed("completed"));
    fake.github.reopen(1);
    wait_state(&mut ana, 1, json!({ "state": "open" }));

    // Answered, then closed, then reopened: answered again, at the same time.
    let at = sent_at(&mut ana, 2) + 3600;
    fake.github.add_comment_full(2, "the-maintainer", "OWNER", "Looking.", &iso(at));
    wait_state(&mut ana, 2, json!({ "state": "answered", "at": at }));
    fake.github.close(2, "completed", None);
    wait_state(&mut ana, 2, closed("completed"));
    fake.github.reopen(2);
    wait_state(&mut ana, 2, json!({ "state": "answered", "at": at }));
    fake.settle();
    assert_eq!((state(&mut ana, 1), state(&mut ana, 2)), (json!({ "state": "open" }), json!({ "state": "answered", "at": at })));
}

#[test]
fn a_rate_limit_changes_no_row_and_the_next_poll_asks_from_the_same_time() {
    let fake = Fake::start();
    let hub = polling("poller_limit", Some(&fake));
    let mut ana = signed_up(&hub, "ana@example.test");
    assert_eq!(send(&mut ana, "Upload stops", None), 1);
    fake.settle();
    let sent = sent_at(&mut ana, 1);
    let made = fake.searches().len();

    // The next search is refused for the rate limit, and the maintainer answers meanwhile.
    fake.github.fail_next(403, "API rate limit exceeded for 203.0.113.9", Some("0"));
    fake.github.add_comment_full(1, "the-maintainer", "OWNER", "Looking.", &iso(sent + 3600));
    until("the refused search", || fake.searches().len() > made);
    assert_eq!(state(&mut ana, 1), json!({ "state": "open" }), "a poll that fails changes no row");
    until("the search after it", || fake.searches().len() > made + 1);
    let (refused, again) = (fake.searches()[made].clone(), fake.searches()[made + 1].clone());
    assert_eq!(refused, again, "the poll that failed did not move the time the next one asks from");
    assert!(refused.starts_with("repo:acme/app is:issue updated:>=20"), "{refused}");
    // The next poll after the limit clears catches up.
    wait_state(&mut ana, 1, json!({ "state": "answered", "at": sent + 3600 }));
}

#[test]
fn the_time_of_the_last_poll_survives_a_hub_restart() {
    let fake = Fake::start();
    let mut hub = polling("poller_restart", Some(&fake));
    let token = account::signup(&hub.url, "ana@example.test", PASSWORD).unwrap().0;
    let mut ana = Client::app(&hub.url, &token);
    ana.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    assert_eq!(send(&mut ana, "Upload stops", None), 1);
    let sent = sent_at(&mut ana, 1);
    until("three polls", || fake.searches().len() >= 3);
    let before = fake.searches();
    let last = before.last().unwrap().clone();
    assert!(last.as_str() > format!("repo:acme/app is:issue updated:>={}", iso(sent - 120)).as_str(), "the polls moved on from the report's own time: {last}");

    hub.restart();
    let made = fake.searches().len();
    until("a search after the restart", || fake.searches().len() > made);
    let first_after = fake.searches()[made].clone();
    assert!(first_after >= last, "the restarted hub asked from {first_after}, not from where the last one stopped ({last})");
    let mut ana = Client::app(&hub.url, &token);
    assert_eq!(rows(&mut ana).len(), 1, "the restarted hub still lists the report");
}

#[test]
fn a_hub_without_reporting_never_calls_github_and_a_hub_with_nothing_to_follow_asks_nothing() {
    let fake = Fake::start();
    let off = polling("poller_off", None);
    let mut ana = signed_up(&off, "ana@example.test");
    assert_eq!(ana.hub(json!({ "cmd": "reports", "ask": { "op": "status" } })).unwrap(), json!({ "available": false }));
    let idle = polling("poller_idle", Some(&fake));
    std::thread::sleep(POLL * 4);
    assert_eq!(fake.github.requests(), vec![], "no poller without a config, and no search when no report waits");
    drop((off, idle));
}

#[test]
fn two_accounts_on_one_issue_both_follow_it_and_neither_list_holds_the_others_reports() {
    let fake = Fake::start();
    let hub = polling("poller_two", Some(&fake));
    let (mut ana, mut bob, mut cy) = (signed_up(&hub, "ana@example.test"), signed_up(&hub, "bob@example.test"), signed_up(&hub, "cy@example.test"));
    assert_eq!(send(&mut ana, "Upload stops at 90%", None), 1);
    assert_eq!(send(&mut bob, "Upload stops at 90% for me", Some(1)), 1);
    assert_eq!(send(&mut cy, "Dark mode", None), 2);
    let (ana_rows, bob_rows, cy_rows) = (rows(&mut ana), rows(&mut bob), rows(&mut cy));
    assert_eq!((ana_rows.len(), bob_rows.len(), cy_rows.len()), (1, 1, 1));
    assert_ne!(ana_rows[0]["id"], bob_rows[0]["id"]);
    assert_eq!((ana_rows[0]["issue"].clone(), bob_rows[0]["issue"].clone(), cy_rows[0]["issue"].clone()), (json!(1), json!(1), json!(2)));

    let at = sent_at(&mut bob, 1) + 3600;
    fake.github.add_comment_full(1, "the-maintainer", "MEMBER", "Looking.", &iso(at));
    wait_state(&mut ana, 1, json!({ "state": "answered", "at": at }));
    wait_state(&mut bob, 1, json!({ "state": "answered", "at": at }));
    fake.github.close(1, "not_planned", None);
    wait_state(&mut ana, 1, closed("not_planned"));
    wait_state(&mut bob, 1, closed("not_planned"));
    fake.settle();
    assert_eq!(state(&mut cy, 2), json!({ "state": "open" }), "an issue nobody touched stays open");
    for (client, expected) in [(&mut ana, 1), (&mut bob, 1), (&mut cy, 1)] {
        let own = rows(client);
        assert_eq!(own.len(), expected);
    }
    assert_eq!(rows(&mut cy)[0]["title"], "Dark mode");
}
