//! Reporting an issue from the app: the dialog, an agent with the person's confirmation, the daily limit, drafts and
//! a hub with no token. GitHub is `fake_github`, in memory on 127.0.0.1.

use super::*;
use pinned_core::reports::{self, Context, Draft};
use pinned_server::reports::fake_github::{self, FakeGithub};
use pinned_server::reports::{Config, NOT_AVAILABLE};

const PASSWORD: &str = "correct horse";

/// A fake GitHub and the runtime that serves it.
struct Fake {
    _runtime: tokio::runtime::Runtime,
    github: FakeGithub,
}

impl Fake {
    fn start() -> Fake {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let github = runtime.block_on(fake_github::start());
        Fake { _runtime: runtime, github }
    }

    fn config(&self) -> Config {
        Config { api: self.github.url.clone(), repo: "acme/app".into(), token: "fake-hub-token".into(), timeout: pinned_core::github::CALL_TIMEOUT }
    }

    /// `config` with calls to GitHub that give up after one second.
    fn impatient(&self) -> Config {
        Config { timeout: Duration::from_secs(1), ..self.config() }
    }

    /// The issues the hub made, and the comments it added.
    fn posts(&self) -> (usize, usize) {
        let logged = self.github.requests();
        let issues = logged.iter().filter(|r| r.method == "POST" && r.path.ends_with("/issues")).count();
        let comments = logged.iter().filter(|r| r.method == "POST" && r.path.ends_with("/comments")).count();
        (issues, comments)
    }
}

fn reporting(name: &str, fake: Option<&Fake>) -> TestHub {
    TestHub::start_reports(name, Signup::Open, Policy::default(), false, Default::default(), "127.0.0.1:0", fake.map(Fake::config))
}

fn problem(title: &str, happened: &str) -> Value {
    json!({ "kind": "problem", "title": title, "happened": happened, "expected": "It works.", "repeat": "1. Open it" })
}

fn ask(client: &mut Client, ask: Value) -> Result<Value, String> {
    client.hub(json!({ "cmd": "reports", "ask": ask }))
}

fn report_issue(client: &mut Client, draft: Value, extra: Value) -> Result<Value, String> {
    let mut call = json!({ "cmd": "report_issue", "draft": draft });
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        call[key] = value;
    }
    client.hub(call)
}

/// The app's way to send: preview `draft`, then send it with the hash the preview showed.
fn reviewed_send(client: &mut Client, draft: Value, extra: Value) -> Result<Value, String> {
    let extra = reviewed(client, &draft, extra)?;
    report_issue(client, draft, extra)
}

/// `extra` with the hash a preview of `draft` showed, for a send made later.
fn reviewed(client: &mut Client, draft: &Value, mut extra: Value) -> Result<Value, String> {
    let preview = ask(client, json!({ "op": "preview", "draft": draft.clone() }))?;
    extra["shown"] = preview["shown"].clone();
    Ok(extra)
}

fn listed(client: &mut Client) -> Vec<Value> {
    ask(client, json!({ "op": "list" })).unwrap().as_array().unwrap().clone()
}

fn signed_up(hub: &TestHub, email: &str) -> String {
    account::signup(&hub.url, email, PASSWORD).unwrap().0
}

#[test]
fn the_dialog_previews_then_files_the_exact_shown_text_and_a_changed_draft_is_refused() {
    let fake = Fake::start();
    let hub = reporting("reports_dialog", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut app = Client::app(&hub.url, &token);

    assert_eq!(ask(&mut app, json!({ "op": "status" })).unwrap(), json!({ "available": true, "left_today": 5 }));
    let draft = problem("Upload stops at 90%", "It stops at 90% for ana@example.test.");
    let preview = ask(&mut app, json!({ "op": "preview", "draft": draft })).unwrap();
    assert_eq!(preview["labels"], json!(["bug", "from-app", "needs-triage"]));
    assert_eq!(preview["left_today"], 5);
    let body = preview["body"].as_str().unwrap();
    assert!(body.contains("### What happened") && body.contains("[email]") && !body.contains("ana@example.test"), "{body}");
    let shown = preview["shown"].as_str().unwrap().to_owned();

    // Sending is a confirm action: a locked app login unlocks first, and the dialog is no exemption.
    assert_eq!(report_issue(&mut app, draft.clone(), json!({ "shown": shown })), Err(pinned_core::surface::LOCKED.into()));
    assert_eq!(fake.posts(), (0, 0));
    app.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();

    let stale = problem("Upload stops at 91%", "It stops at 90% for ana@example.test.");
    assert_eq!(report_issue(&mut app, stale, json!({ "shown": shown })), Err("The report changed since you reviewed it. Review it again.".into()));
    assert_eq!(fake.posts(), (0, 0), "a stale hash posts nothing");

    let sent = report_issue(&mut app, draft, json!({ "shown": shown })).unwrap();
    assert_eq!((sent["state"].clone(), sent["issue"].clone(), sent["by_agent"].clone()), (json!({ "state": "open" }), json!(1), json!(false)), "{sent}");
    assert_eq!(fake.posts(), (1, 0));
    assert_eq!(fake.github.labels(1), ["bug", "from-app", "needs-triage"]);
    let posted = fake.github.body(1);
    let id = sent["id"].as_str().unwrap();
    for heading in ["### What happened", "### Expected", "### How to repeat"] {
        assert!(posted.contains(heading), "{posted}");
    }
    assert!(posted.ends_with(&format!("---\n\nSent from the pinned app · report {}", reports::ref_of(id))), "{posted}");
    assert!(!posted.contains("ana@example.test") && !posted.contains("fake-hub-token"), "{posted}");

    let list = listed(&mut app);
    assert_eq!(list.len(), 1);
    assert_eq!((list[0]["state"].clone(), list[0]["issue"].clone(), list[0]["url"].clone()), (json!({ "state": "open" }), json!(1), json!("https://github.com/acme/app/issues/1")));
    assert_eq!(ask(&mut app, json!({ "op": "status" })).unwrap()["left_today"], 4);
}

#[test]
fn an_agents_report_waits_scrubbed_and_the_issue_is_its_held_text_once_the_owner_approves() {
    let fake = Fake::start();
    let hub = reporting("reports_agent", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = agent_of(&hub.url, &token, "ana@example.test", "ana-box-mac");
    let folder = ana.dir.join("site").display().to_string();
    let draft = json!({ "kind": "problem", "title": "Crash in site", "happened": format!("The site folder at {folder}/src breaks for ana@example.test on ana-box-mac."), "expected": "No crash.", "repeat": "Open site" });

    let held = act(&ana.dir, &ana.tab, "report_issue", json!({ "draft": draft })).unwrap();
    assert_eq!(held["outcome"], "pending", "{held}");
    let id = held["id"].as_str().unwrap().to_owned();
    assert_eq!(fake.posts(), (0, 0), "nothing is sent before the owner approves");
    let waiting = ana.client.hub(json!({ "cmd": "confirmations" })).unwrap();
    let item = waiting.as_array().unwrap().iter().find(|c| c["id"] == id.as_str()).unwrap().clone();
    let public = item["args"].to_string();
    for private in ["site", folder.as_str(), "ana@example.test", "ana-box-mac"] {
        assert!(!public.contains(private), "the held text still holds {private}: {public}");
    }
    assert!(public.contains("[project]") && public.contains("[email]") && public.contains("[name]"), "{public}");

    ana.client.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    let decided = ana.client.hub(json!({ "cmd": "decide_confirmation", "id": id, "approve": true })).unwrap();
    assert_eq!((&decided["decision"], &decided["outcome"]), (&json!("approved"), &json!("done")), "{decided}");
    assert_eq!(decided["result"], json!({ "number": 1, "url": "https://github.com/acme/app/issues/1" }));
    assert_eq!(fake.posts(), (1, 0));

    let rows = listed(&mut ana.client);
    assert_eq!((rows.len(), rows[0]["by_agent"].clone()), (1, json!(true)));
    let held_draft: Draft = serde_json::from_value(item["args"]["draft"].clone()).unwrap();
    let (title, body, labels) = reports::render(&held_draft, &Context::default(), None, &reports::ref_of(rows[0]["id"].as_str().unwrap())).unwrap();
    assert_eq!((fake.github.body(1), fake.github.labels(1)), (body, labels));
    assert_eq!(title, "Crash in [project]");
    wait_for("the notice in the agent's queue", || ana.node.machine.view(&ana.tab).unwrap().queued >= 1);
    let told = turn_end_context(&ana.dir, &ana.tab);
    assert!(told.contains("https://github.com/acme/app/issues/1") && told.contains("\"number\":1"), "{told}");

    let denied = act(&ana.dir, &ana.tab, "report_issue", json!({ "draft": problem("Another crash", "It closes.") })).unwrap();
    let decided = ana.client.hub(json!({ "cmd": "decide_confirmation", "id": denied["id"], "approve": false })).unwrap();
    assert_eq!(decided["decision"], "denied");
    assert_eq!(fake.posts(), (1, 0), "a denial posts nothing");

    let mut with_errors = problem("Crash", "It closes.");
    with_errors["attach"] = json!({ "environment": false, "errors": ["panic at main.rs:1"] });
    let refused = act(&ana.dir, &ana.tab, "report_issue", json!({ "draft": with_errors })).unwrap_err();
    assert!(refused.contains("no error lines"), "{refused}");
    let refused = act(&ana.dir, &ana.tab, "report_issue", json!({ "draft": problem("Crash", "It closes."), "shown": "abc" })).unwrap_err();
    assert!(refused.contains("send only draft"), "{refused}");
    let refused = act(&ana.dir, &ana.tab, "reports", json!({ "ask": { "op": "preview", "draft": problem("x", "y") } })).unwrap_err();
    assert!(refused.contains("preview is the app's"), "{refused}");
    assert_eq!(fake.posts(), (1, 0));
}

#[test]
fn a_report_can_join_a_similar_open_issue_as_a_comment_and_a_closed_one_is_refused() {
    let fake = Fake::start();
    fake.github.add_issue_numbered(318, "Diff slow on files over 5000 lines", "The diff takes a minute.");
    fake.github.add_issue_numbered(320, "Old diff crash", "Fixed since.");
    fake.github.close(320, "completed", None);
    let hub = reporting("reports_similar", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut app = Client::connect(&hub.url, &token);

    let found = ask(&mut app, json!({ "op": "similar", "title": "The diff is slow for files over 5000 lines" })).unwrap();
    assert_eq!(found.as_array().unwrap().iter().map(|s| s["number"].clone()).collect::<Vec<_>>(), [json!(318)], "{found}");
    assert_eq!(found[0]["title"], "Diff slow on files over 5000 lines");
    let preview = ask(&mut app, json!({ "op": "preview", "draft": problem("The diff is slow for files over 5000 lines", "Same here.") })).unwrap();
    assert_eq!(preview["similar"][0]["number"], 318);

    let mut closed = problem("Old diff crash again", "It closes.");
    closed["to_issue"] = json!(320);
    let refused = reviewed_send(&mut app, closed, json!({})).unwrap_err();
    assert!(refused.contains("Issue #320 is closed"), "{refused}");
    assert_eq!(ask(&mut app, json!({ "op": "status" })).unwrap()["left_today"], 5, "a refused report does not count");

    let mut added = problem("The diff is slow for files over 5000 lines", "Same here, with 8000 lines.");
    added["to_issue"] = json!(318);
    let sent = reviewed_send(&mut app, added, json!({})).unwrap();
    assert_eq!((sent["issue"].clone(), sent["followed"].clone(), sent["state"].clone()), (json!(318), json!(true), json!({ "state": "open" })), "{sent}");
    assert_eq!(fake.posts(), (0, 1), "one comment and no new issue");
    let comment = &fake.github.comment_bodies(318)[0];
    assert!(comment.starts_with("### Another case from the pinned app") && comment.contains("Same here, with 8000 lines."), "{comment}");
    let text = hub.runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.query_one("select own_comments::text from reports", &[]).await.unwrap().get::<_, String>(0)
    });
    assert_eq!(text, "{1}", "the comment's id is kept");
    assert_eq!(ask(&mut app, json!({ "op": "status" })).unwrap()["left_today"], 4, "an added case counts");
}

#[test]
fn drafts_are_saved_listed_sent_by_id_and_deleted_and_only_their_account_sees_them() {
    let fake = Fake::start();
    let hub = reporting("reports_drafts", Some(&fake));
    let (ana_token, bob_token) = (signed_up(&hub, "ana@example.test"), signed_up(&hub, "bob@example.test"));
    let mut ana = Client::connect(&hub.url, &ana_token);
    let mut bob = Client::connect(&hub.url, &bob_token);

    let first = ask(&mut ana, json!({ "op": "save_draft", "draft": problem("Half written", "It") })).unwrap();
    let id = first["id"].as_str().unwrap().to_owned();
    assert_eq!(first["state"], json!({ "state": "draft" }));
    let again = ask(&mut ana, json!({ "op": "save_draft", "id": id, "draft": problem("Half written", "It closes.") })).unwrap();
    assert_eq!((again["id"].clone(), again["draft"]["happened"].clone()), (json!(id), json!("It closes.")));
    let list = listed(&mut ana);
    assert_eq!((list.len(), list[0]["state"].clone(), list[0]["title"].clone()), (1, json!({ "state": "draft" }), json!("Half written")));
    assert_eq!(fake.posts(), (0, 0));

    let no_draft = |id: &str| Err(format!("this account has no draft {id}"));
    assert_eq!(listed(&mut bob), Vec::<Value>::new(), "bob lists none of ana's");
    assert_eq!(ask(&mut bob, json!({ "op": "delete_draft", "id": id })), no_draft(&id));
    assert_eq!(ask(&mut bob, json!({ "op": "save_draft", "id": id, "draft": problem("Bob's now", "x") })), no_draft(&id));
    assert_eq!(reviewed_send(&mut bob, problem("Half written", "It closes."), json!({ "draft_id": id })), no_draft(&id));
    assert_eq!(listed(&mut ana)[0]["title"], "Half written", "ana's draft is untouched");
    assert_eq!(fake.posts(), (0, 0));

    let sent = reviewed_send(&mut ana, problem("Half written", "It closes."), json!({ "draft_id": id })).unwrap();
    assert_eq!((sent["id"].clone(), sent["state"].clone(), sent["draft"].clone()), (json!(id), json!({ "state": "open" }), Value::Null), "{sent}");
    let list = listed(&mut ana);
    assert_eq!(list.len(), 1, "the draft became the report: {list:?}");
    assert_eq!(ask(&mut ana, json!({ "op": "delete_draft", "id": id })), no_draft(&id), "a sent report is no draft");

    let second = ask(&mut ana, json!({ "op": "save_draft", "draft": problem("Another", "x") })).unwrap();
    assert_eq!(ask(&mut ana, json!({ "op": "delete_draft", "id": second["id"] })).unwrap(), Value::Null);
    assert_eq!(listed(&mut ana).len(), 1);
    assert_eq!(fake.posts(), (1, 0));
}

#[test]
fn the_sixth_report_of_a_day_is_refused_before_any_request_and_an_added_case_and_concurrent_sends_count() {
    let fake = Fake::start();
    fake.github.add_issue_numbered(7, "Diff slow", "The diff takes a minute.");
    let hub = reporting("reports_limit", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = Client::connect(&hub.url, &token);
    let before = fake.github.requests().len();

    for n in 0..3 {
        reviewed_send(&mut ana, problem(&format!("Crash number {n}"), "It closes."), json!({})).unwrap();
    }
    let mut added = problem("Diff slow again", "Same.");
    added["to_issue"] = json!(7);
    reviewed_send(&mut ana, added, json!({})).unwrap();
    assert_eq!(fake.posts(), (3, 1));
    assert_eq!(ask(&mut ana, json!({ "op": "status" })).unwrap()["left_today"], 1);

    // Two sends at once, with one left: one files, the other is refused. A slow create keeps the first one sending
    // while the second asks.
    fake.github.slow_creates(Duration::from_millis(400));
    let (url, answers) = (hub.url.clone(), Mutex::new(vec![]));
    std::thread::scope(|scope| {
        for n in 0..2 {
            let (url, token, answers) = (&url, &token, &answers);
            scope.spawn(move || {
                let mut client = Client::connect(url, token);
                let answer = reviewed_send(&mut client, problem(&format!("Race {n}"), "It closes."), json!({}));
                answers.lock().unwrap().push(answer);
            });
        }
    });
    let answers = answers.into_inner().unwrap();
    assert_eq!(answers.iter().filter(|a| a.is_ok()).count(), 1, "{answers:?}");
    let refused = answers.iter().find_map(|a| a.as_ref().err()).unwrap();
    assert!(refused.starts_with("You sent 5 reports in the last day. You can send again at ") && refused.ends_with(" UTC."), "{refused}");
    assert_eq!(fake.posts().0 + fake.posts().1, 5, "still 5");

    let sixth = problem("Sixth", "It closes.");
    let shown = ask(&mut ana, json!({ "op": "preview", "draft": sixth.clone() })).unwrap()["shown"].clone();
    let requests = fake.github.requests().len();
    let refused = report_issue(&mut ana, sixth, json!({ "shown": shown })).unwrap_err();
    assert!(refused.starts_with("You sent 5 reports in the last day."), "{refused}");
    assert_eq!(fake.github.requests().len(), requests, "no request left the hub for the sixth");
    assert!(before < requests);
    assert_eq!(ask(&mut ana, json!({ "op": "status" })).unwrap()["left_today"], 0);
    let mut bob = Client::connect(&hub.url, &signed_up(&hub, "bob@example.test"));
    assert!(reviewed_send(&mut bob, problem("Bob's own", "It closes."), json!({})).is_ok(), "the limit is per account");
}

#[test]
fn a_hub_without_a_token_says_so_sends_nothing_and_refuses_an_agents_report() {
    let fake = Fake::start();
    let hub = reporting("reports_off", None);
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = agent_of(&hub.url, &token, "ana@example.test", "ana-off-mac");

    assert_eq!(ask(&mut ana.client, json!({ "op": "status" })).unwrap(), json!({ "available": false }));
    let off: Result<Value, String> = Err(NOT_AVAILABLE.into());
    assert_eq!(NOT_AVAILABLE, "Reporting is not available on this hub");
    for op in [json!({ "op": "list" }), json!({ "op": "similar", "title": "Crash" }), json!({ "op": "preview", "draft": problem("x", "y") }), json!({ "op": "save_draft", "draft": problem("x", "y") }), json!({ "op": "delete_draft", "id": "d" })] {
        assert_eq!(ask(&mut ana.client, op.clone()), off, "{op}");
    }
    assert_eq!(report_issue(&mut ana.client, problem("Crash", "It closes."), json!({})), off);
    assert_eq!(act(&ana.dir, &ana.tab, "report_issue", json!({ "draft": problem("Crash", "It closes.") })), off);
    assert_eq!(act(&ana.dir, &ana.tab, "reports", json!({ "ask": { "op": "list" } })), off);
    assert_eq!(act(&ana.dir, &ana.tab, "reports", json!({ "ask": { "op": "status" } })).unwrap()["result"], json!({ "available": false }));
    assert!(ana.client.hub(json!({ "cmd": "confirmations" })).unwrap().as_array().unwrap().is_empty(), "nothing waits for an approval");
    assert!(fake.github.requests().is_empty(), "no request left the hub");
}

#[test]
fn an_agent_reads_only_where_a_report_is_never_its_text_or_what_github_said() {
    let fake = Fake::start();
    fake.github.add_issue_numbered(318, "Diff slow on files over 5000 lines in /Users/ana/shop", "Body text from GitHub.");
    fake.github.add_comment_as(318, "maintainer", "OWNER", "A comment nobody asked an agent to read.");
    fake.github.add_issue_numbered(319, &format!("Zigzag {}", "w".repeat(150)), "Long.");
    let hub = reporting("reports_agent_view", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = agent_of(&hub.url, &token, "ana@example.test", "ana-view-mac");
    reviewed_send(&mut ana.client, problem("Crash on launch", "It closes."), json!({})).unwrap();
    ask(&mut ana.client, json!({ "op": "save_draft", "draft": problem("A draft", "Private words.") })).unwrap();

    let keys = |value: &Value| {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    let list = act(&ana.dir, &ana.tab, "reports", json!({ "ask": { "op": "list" } })).unwrap()["result"].clone();
    assert_eq!(list.as_array().unwrap().len(), 2, "{list}");
    for row in list.as_array().unwrap() {
        assert_eq!(keys(row), ["number", "state", "url"], "{row}");
    }
    assert!(!list.to_string().contains("Private words") && !list.to_string().contains("Crash on launch"), "{list}");
    let similar = act(&ana.dir, &ana.tab, "reports", json!({ "ask": { "op": "similar", "title": "Diff slow on files over 5000 lines" } })).unwrap()["result"].clone();
    assert_eq!(similar.as_array().unwrap().len(), 1, "{similar}");
    assert_eq!(keys(&similar[0]), ["number", "public_title", "state", "url"], "{similar}");
    assert_eq!((similar[0]["number"].clone(), similar[0]["state"].clone()), (json!(318), json!("open")));
    assert!(similar[0]["public_title"].as_str().unwrap().contains("[folder]") && !similar.to_string().contains("/Users/ana"), "{similar}");
    assert!(!similar.to_string().contains("Body text") && !similar.to_string().contains("comment nobody"), "{similar}");
    let long = act(&ana.dir, &ana.tab, "reports", json!({ "ask": { "op": "similar", "title": "Zigzag" } })).unwrap()["result"].clone();
    let long = long.as_array().unwrap().iter().find(|s| s["number"] == 319).unwrap()["public_title"].as_str().unwrap().to_owned();
    assert_eq!(long.chars().count(), 120, "an agent reads at most 120 characters of a title: {long}");
}

#[test]
fn the_reports_environment_reads_a_token_alone_and_overrides_only_under_dev() {
    use pinned_server::reports::Config;
    let vars = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string());
    let overrides = vars(&[("PINNED_REPORTS_GITHUB_TOKEN", "fake-1"), ("PINNED_REPORTS_REPO", "acme/app"), ("PINNED_REPORTS_GITHUB_API", "http://127.0.0.1:4010")]);
    let outside = Config::from_env(false, &overrides).unwrap();
    assert_eq!((outside.api.as_str(), outside.repo.as_str()), ("https://api.github.com", "FelipeGazapina/pinned"), "outside --dev the overrides are ignored");
    let dev = Config::from_env(true, &overrides).unwrap();
    assert_eq!((dev.api.as_str(), dev.repo.as_str()), ("http://127.0.0.1:4010", "acme/app"));
    let remote = vars(&[("PINNED_REPORTS_GITHUB_TOKEN", "fake-1"), ("PINNED_REPORTS_GITHUB_API", "https://api.example.test")]);
    assert_eq!(Config::from_env(true, &remote), None, "a token never goes to a host that is not this machine");
    assert_eq!(Config::from_env(true, &vars(&[])), None);
}

#[test]
fn a_search_is_reused_for_a_minute_and_the_hub_makes_at_most_twenty_a_minute() {
    let fake = Fake::start();
    let hub = reporting("reports_search", Some(&fake));
    let mut ana = Client::connect(&hub.url, &signed_up(&hub, "ana@example.test"));
    let searches = || fake.github.requests().iter().filter(|r| r.path.starts_with("/search/issues")).count();

    ask(&mut ana, json!({ "op": "similar", "title": "Diff is slow" })).unwrap();
    ask(&mut ana, json!({ "op": "similar", "title": "Diff is slow" })).unwrap();
    assert_eq!(searches(), 1, "the second ask is answered from the first");
    // Each account makes at most five a minute, so four more make up the twenty.
    for n in 1..5 {
        ask(&mut ana, json!({ "op": "similar", "title": format!("Different words number{n}x") })).unwrap();
    }
    for (k, who) in ["bob", "cy", "dee"].into_iter().enumerate() {
        let mut other = Client::connect(&hub.url, &signed_up(&hub, &format!("{who}@example.test")));
        for n in 0..5 {
            ask(&mut other, json!({ "op": "similar", "title": format!("Other words team{k} number{n}x") })).unwrap();
        }
    }
    assert_eq!(searches(), 20);
    let mut eve = Client::connect(&hub.url, &signed_up(&hub, "eve@example.test"));
    let busy = ask(&mut eve, json!({ "op": "similar", "title": "One more topic entirely" })).unwrap_err();
    assert!(busy.contains("busy"), "{busy}");
    assert_eq!(searches(), 20, "the twenty-first search never left the hub");
    ask(&mut ana, json!({ "op": "similar", "title": "Diff is slow" })).unwrap();
    let preview = ask(&mut ana, json!({ "op": "preview", "draft": problem("Yet another topic", "It closes.") })).unwrap();
    assert_eq!(preview["similar"], json!([]), "a preview goes on without suggestions while searching is held back");
    assert_eq!(searches(), 20);
}

/// An account's app and its machine `name`, with an agent session in each of `folders`, made under the node's own
/// folder. The tabs come back in the order of `folders`.
fn agents_in(url: &str, token: &str, email: &str, name: &str, folders: &[&str]) -> (Agent, Vec<String>) {
    let mut client = Client::connect(url, token);
    client.wait("hello", |f| f["type"] == "hello");
    let dir = node_dir(&format!("reports-{name}"));
    let (machine, machine_token) = account::register_machine(url, token, name, "macos").unwrap();
    let creds = Credentials { hub: url.to_owned(), email: email.to_owned(), user_token: token.to_owned(), machine_id: machine.id.clone(), machine_token };
    creds.save(&dir).unwrap();
    let node = Node::start(&dir, creds);
    node.machine.listen(|_| Err("unexpected".into())).unwrap();
    client.wait_machines(&[(name, true)]);
    let mut tabs = vec![];
    for folder in folders {
        let path = dir.join(folder);
        std::fs::create_dir_all(&path).unwrap();
        let tab = node.machine.open_tab(Some(path.display().to_string()), false).unwrap();
        client.wait("the agent's tab at the hub", |f| f["type"] == "event" && f["event"]["kind"] == "tabs" && f["event"]["tabs"].as_array().unwrap().iter().any(|t| t["id"] == tab));
        tabs.push(tab);
    }
    let tab = tabs[0].clone();
    (Agent { client, node, dir, machine: machine.id, tab }, tabs)
}

/// What an agent's `report_issue` from `tab` holds for the owner: the confirmation's id and its draft.
fn held_report(agent: &mut Agent, tab: &str, draft: Value) -> (String, Value) {
    let held = act(&agent.dir, tab, "report_issue", json!({ "draft": draft })).unwrap();
    assert_eq!(held["outcome"], "pending", "{held}");
    let id = held["id"].as_str().unwrap().to_owned();
    let waiting = agent.client.hub(json!({ "cmd": "confirmations" })).unwrap();
    let item = waiting.as_array().unwrap().iter().find(|c| c["id"] == id.as_str()).unwrap().clone();
    (id, item["args"]["draft"].clone())
}

/// The title of the `n`th issue the hub posted, as GitHub got it.
fn posted_title(fake: &Fake, n: usize) -> String {
    let posts: Vec<_> = fake.github.requests().into_iter().filter(|r| r.method == "POST" && r.path.ends_with("/issues")).collect();
    serde_json::from_str::<Value>(&posts[n - 1].body).unwrap()["title"].as_str().unwrap().to_owned()
}

/// `<` n times, then `b>` n times: removing the innermost tag leaves the next one whole.
fn nested_tags(n: usize) -> String {
    format!("{}{}", "<".repeat(n), "b>".repeat(n))
}

#[test]
fn an_agent_in_a_worktree_or_a_windows_folder_reports_without_its_projects_name_and_with_its_machines_environment() {
    let fake = Fake::start();
    let hub = reporting("reports_cwd", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let (mut ana, tabs) = agents_in(&hub.url, &token, "ana@example.test", "ana-cwd-mac", &["x/projetos/loja-web/.worktrees/fix-1", "C:\\Users\\x\\proj\\caixa-pdv"]);
    let (worktree, windows) = (tabs[0].clone(), tabs[1].clone());
    ana.client.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();

    // The worktree's project, though its tab's folder is the worktree's, and an agent's own environment words.
    let mut draft = problem("loja-web crashes on save", "It closes.");
    draft["attach"] = json!({ "environment": "pinned 9.9.9 on ana-cwd-mac in loja-web" });
    let (id, held) = held_report(&mut ana, &worktree, draft);
    assert_eq!(held["title"], "[project] crashes on save", "{held}");
    assert_eq!(held["attach"]["environment"], "pinned 0.0.0-test · linux", "the machine's, never the agent's words: {held}");
    let decided = ana.client.hub(json!({ "cmd": "decide_confirmation", "id": id, "approve": true })).unwrap();
    assert_eq!(decided["outcome"], "done", "{decided}");
    assert_eq!(posted_title(&fake, 1), "[project] crashes on save");
    let hub_line = format!("hub {}", env!("CARGO_PKG_VERSION"));
    assert!(fake.github.body(1).contains(&format!("### Environment\n\npinned 0.0.0-test · linux · {hub_line}\n\n---")), "{}", fake.github.body(1));

    // A Windows folder's name, after its last backslash.
    let (id, held) = held_report(&mut ana, &windows, problem("caixa-pdv crashes on save", "It closes."));
    assert_eq!(held["title"], "[project] crashes on save", "{held}");
    ana.client.hub(json!({ "cmd": "decide_confirmation", "id": id, "approve": true })).unwrap();
    assert_eq!(posted_title(&fake, 2), "[project] crashes on save");
    assert!(!fake.github.body(2).contains("### Environment"), "no environment unless asked: {}", fake.github.body(2));

    // A word split by markup, however deep, is found once the markup is gone.
    let (_, held) = held_report(&mut ana, &worktree, problem(&format!("lo{}ja-web crashes", nested_tags(32)), "It closes."));
    assert_eq!(held["title"], "[project] crashes", "{held}");
}

#[test]
fn a_project_known_only_through_a_machines_list_is_scrubbed() {
    let fake = Fake::start();
    let hub = reporting("reports_listed", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = agent_of(&hub.url, &token, "ana@example.test", "ana-listed-mac");
    // A project the machine lists, with no tab in it, and no overview, lookup or box open ever asked for the list.
    let books = ana.dir.join("kestrel-books");
    std::fs::create_dir_all(&books).unwrap();
    ana.node.machine.open_project(books);
    let tab = ana.tab.clone();
    let (_, held) = held_report(&mut ana, &tab, problem("kestrel-books export fails", "It closes."));
    assert_eq!(held["title"], "[project] export fails", "{held}");
}

#[test]
fn a_listed_projects_repository_owner_and_name_are_scrubbed_on_their_own() {
    let fake = Fake::start();
    let hub = reporting("reports_listed_repo", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = agent_of(&hub.url, &token, "ana@example.test", "ana-repo-mac");
    let books = ana.dir.join("books-checkout");
    std::fs::create_dir_all(&books).unwrap();
    for args in [&["init", "-q"][..], &["remote", "add", "origin", "https://github.com/ozzy-lab/kestrel-books.git"][..]] {
        assert!(std::process::Command::new("git").args(args).current_dir(&books).status().unwrap().success());
    }
    ana.node.machine.open_project(books);
    let tab = ana.tab.clone();
    let (_, held) = held_report(&mut ana, &tab, problem("ozzy-lab said kestrel-books and ozzy-lab/kestrel-books fail", "It closes."));
    assert_eq!(held["title"], "[project] said [project] and [project] fail", "{held}");
}

#[test]
fn an_agents_environment_line_is_scrubbed_with_the_accounts_words() {
    let fake = Fake::start();
    let hub = reporting("reports_env_scrub", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    // A checkout named like the machine's system: the held line must not say it.
    let (mut ana, tabs) = agents_in(&hub.url, &token, "ana@example.test", "ana-env-mac", &["linux"]);
    let mut draft = problem("It crashes", "It closes.");
    draft["attach"] = json!({ "environment": true });
    let (id, held) = held_report(&mut ana, &tabs[0], draft);
    assert_eq!(held["attach"]["environment"], "pinned 0.0.0-test · [project]", "{held}");
    ana.client.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    ana.client.hub(json!({ "cmd": "decide_confirmation", "id": id, "approve": true })).unwrap();
    let hub_line = format!("hub {}", env!("CARGO_PKG_VERSION"));
    assert!(fake.github.body(1).contains(&format!("### Environment\n\npinned 0.0.0-test · [project] · {hub_line}\n\n---")), "{}", fake.github.body(1));
}

#[test]
fn a_machines_name_is_scrubbed_on_its_own() {
    let fake = Fake::start();
    let hub = reporting("reports_machine_name", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    account::register_machine(&hub.url, &token, "kestrel-7", "macos").unwrap();
    let mut app = Client::connect(&hub.url, &token);
    let preview = ask(&mut app, json!({ "op": "preview", "draft": problem("Fails on kestrel-7", "It closes.") })).unwrap();
    assert_eq!(preview["title"], "Fails on [name]", "{preview}");
}

#[test]
fn the_e_mail_local_part_is_scrubbed_on_its_own() {
    let fake = Fake::start();
    let hub = reporting("reports_local_part", Some(&fake));
    let token = signed_up(&hub, "corvo.lima@example.test");
    let mut app = Client::connect(&hub.url, &token);
    let preview = ask(&mut app, json!({ "op": "preview", "draft": problem("Signed in as corvo.lima it fails", "It closes.") })).unwrap();
    assert_eq!(preview["title"], "Signed in as [name] it fails", "{preview}");
}

#[test]
fn the_apps_words_scrub_the_preview_and_the_send_and_a_send_needs_a_review() {
    let fake = Fake::start();
    let hub = reporting("reports_app_context", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut app = Client::connect(&hub.url, &token);
    let mut draft = problem("zebra-corp fails on export", "It closes.");
    draft["attach"] = json!({ "environment": "pinned 0.18.6 · macOS 15.6 · arm64" });

    assert_eq!(report_issue(&mut app, draft.clone(), json!({})), Err("Review the report before sending it.".into()));
    assert_eq!(fake.posts(), (0, 0));

    let context = json!({ "names": ["zebra-corp"] });
    let preview = ask(&mut app, json!({ "op": "preview", "draft": draft.clone(), "context": context })).unwrap();
    assert_eq!(preview["title"], "[name] fails on export", "the word the app alone knows: {preview}");
    let sent = report_issue(&mut app, draft, json!({ "shown": preview["shown"] })).unwrap();
    assert_eq!(sent["issue"], 1, "the send scrubs with the preview's words, so its hash matches: {sent}");
    assert_eq!(posted_title(&fake, 1), "[name] fails on export");
    let hub_line = format!("hub {}", env!("CARGO_PKG_VERSION"));
    assert!(fake.github.body(1).contains(&format!("### Environment\n\npinned 0.18.6 · macOS 15.6 · arm64 · {hub_line}\n\n---")), "{}", fake.github.body(1));

    let large = json!({ "names": (0..501).map(|n| format!("w{n}")).collect::<Vec<_>>() });
    assert_eq!(ask(&mut app, json!({ "op": "preview", "draft": problem("x", "y"), "context": large })), Err("The report's context is too large.".into()));
    assert_eq!(ask(&mut app, json!({ "op": "similar", "title": "x", "context": { "files": ["f".repeat(201)] } })), Err("The report's context is too large.".into()));

    // Markup that splits a word is gone before the word is looked for, and markup that never settles is refused.
    let split = ask(&mut app, json!({ "op": "preview", "draft": problem(&format!("zebra{}-corp fails", nested_tags(32)), "x"), "context": context })).unwrap();
    assert_eq!(split["title"], "[name] fails", "{split}");
    let mut deep = "<b>".to_owned();
    for _ in 0..20 {
        deep = format!("<!{deep}[a](x)b>");
    }
    let refused = ask(&mut app, json!({ "op": "preview", "draft": problem("Crash", &format!("zebra{deep}-corp")), "context": context }));
    assert_eq!(refused, Err("The report has markup the hub cannot clean. Remove the HTML and try again.".into()));
}

#[test]
fn a_search_for_similar_issues_holds_only_scrubbed_words() {
    let fake = Fake::start();
    let config = Config { repo: "octo-labs/tracker".into(), ..fake.config() };
    let hub = TestHub::start_reports("reports_similar_scrub", Signup::Open, Policy::default(), false, Default::default(), "127.0.0.1:0", Some(config));
    let token = signed_up(&hub, "ana@example.test");
    let (ana, tabs) = agents_in(&hub.url, &token, "ana@example.test", "ana-similar-mac", &["acme-billing"]);
    let mut app = Client::connect(&hub.url, &token);
    let searched = || fake.github.requests().into_iter().filter(|r| r.path.starts_with("/search/issues")).map(|r| format!("{} {}", r.path, r.body)).collect::<Vec<_>>();

    ask(&mut app, json!({ "op": "similar", "title": "zebra-ledger export fails", "context": { "projects": ["zebra-ledger"] } })).unwrap();
    act(&ana.dir, &tabs[0], "reports", json!({ "ask": { "op": "similar", "title": "acme-billing export fails" } })).unwrap();
    let log = searched();
    assert_eq!(log.len(), 1, "both searches are the same scrubbed words: {log:?}");
    for private in ["zebra", "ledger", "acme", "billing"] {
        assert!(!log.iter().any(|r| r.to_lowercase().contains(private)), "the search holds {private}: {log:?}");
    }
    assert!(log[0].contains("q=export+fails+repo"), "{log:?}");
}

#[test]
fn another_tabs_folder_that_no_project_list_names_is_scrubbed() {
    let fake = Fake::start();
    let hub = reporting("reports_tabs", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let (mut ana, tabs) = agents_in(&hub.url, &token, "ana@example.test", "ana-tabs-mac", &["notes", "pardal-shop"]);
    // Gone from disk, so the machine no longer lists it as a project; its tab stays open.
    std::fs::remove_dir_all(ana.dir.join("pardal-shop")).unwrap();
    let (_, held) = held_report(&mut ana, &tabs[0], problem("pardal-shop broke", "It closes."));
    assert_eq!(held["title"], "[project] broke", "{held}");
}

/// One value, as text, of `sql` on the hub's database.
fn sql_text(hub: &TestHub, sql: &str) -> Option<String> {
    hub.runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.query_one(sql, &[]).await.unwrap().get::<_, Option<String>>(0)
    })
}

fn left_today(client: &mut Client) -> Value {
    ask(client, json!({ "op": "status" })).unwrap()["left_today"].clone()
}

fn could_not_send(why: &str) -> Result<Value, String> {
    Err(format!("Could not send the report: {why}"))
}

/// The report of `client`'s account whose id is `id`, as `list` gives it.
fn listed_row(client: &mut Client, id: &str) -> Value {
    listed(client).into_iter().find(|r| r["id"] == id).unwrap_or(Value::Null)
}

#[test]
fn a_send_a_stopped_hub_left_reads_failed_after_the_restart_and_still_counts() {
    let fake = Fake::start();
    let mut hub = reporting("reports_restart", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = Client::connect(&hub.url, &token);
    reviewed_send(&mut ana, problem("Crash on launch", "It closes."), json!({})).unwrap();
    // A send the hub was making when it stopped.
    admin_on(
        &hub.database,
        "insert into reports (id, account, kind, title, state, sent_at, created_at, updated_at)
         select 'aaaaaaaa-0000-4000-8000-000000000001', id, 'problem', 'Halfway', '{\"state\": \"sending\"}', extract(epoch from now())::bigint, 1, 1 from accounts where email = 'ana@example.test'",
    );
    assert_eq!(left_today(&mut ana), 3);

    hub.restart();
    let mut ana = Client::connect(&hub.url, &token);
    let row = listed_row(&mut ana, "aaaaaaaa-0000-4000-8000-000000000001");
    assert_eq!(row["state"], json!({ "state": "failed", "error": "the hub restarted while sending; check My reports before trying again" }), "{row}");
    assert_eq!(left_today(&mut ana), 3, "it may have reached GitHub, so it still counts");
    assert_eq!(sql_text(&hub, "select count(*)::text from reports where state->>'state' = 'sending'"), Some("0".into()));
}

#[test]
fn try_again_reuses_the_failed_row_finds_what_a_timed_out_send_filed_and_counts_once() {
    let fake = Fake::start();
    fake.github.add_issue_numbered(7, "Diff slow", "The diff takes a minute.");
    let hub = TestHub::start_reports("reports_again", Signup::Open, Policy::default(), false, Default::default(), "127.0.0.1:0", Some(fake.impatient()));
    let mut ana = Client::connect(&hub.url, &signed_up(&hub, "ana@example.test"));
    let issues = || fake.github.requests().iter().filter(|r| r.method == "POST" && r.path.ends_with("/issues")).count();

    // GitHub files it but answers too late.
    fake.github.hang_next_post(Duration::from_secs(3));
    let draft = problem("Crash on launch", "It closes.");
    assert_eq!(reviewed_send(&mut ana, draft.clone(), json!({ "request_id": "try-1" })), could_not_send("GitHub took too long to answer."));
    assert_eq!(fake.github.issue_count(), 8, "GitHub made it");
    let failed = listed(&mut ana).into_iter().find(|r| r["title"] == "Crash on launch").unwrap();
    let id = failed["id"].as_str().unwrap().to_owned();
    assert_eq!(failed["state"], json!({ "state": "failed", "error": "GitHub took too long to answer." }));
    assert_eq!(left_today(&mut ana), 4, "it may have reached GitHub");

    let again = reviewed_send(&mut ana, draft.clone(), json!({ "request_id": "try-1" })).unwrap();
    assert_eq!((again["id"].clone(), again["state"].clone(), again["issue"].clone()), (json!(id), json!({ "state": "open" }), json!(8)), "{again}");
    assert_eq!(issues(), 1, "the issue the timed-out try filed is the report, and nothing is posted again");
    assert!(fake.github.body(8).ends_with(&format!("report {}", reports::ref_of(&id))));
    assert_eq!(left_today(&mut ana), 4, "the row counts once");
    // The same request once more is the same report.
    assert_eq!(reviewed_send(&mut ana, draft, json!({ "request_id": "try-1" })).unwrap()["id"], json!(id));
    assert_eq!((issues(), listed(&mut ana).len()), (1, 1));

    // A 500 did nothing: trying again posts it, in the same row, under the same reference.
    let draft = problem("Slow sync", "It takes minutes.");
    let extra = reviewed(&mut ana, &draft, json!({ "request_id": "try-2" })).unwrap();
    fake.github.fail_next(500, "Server Error", None);
    assert_eq!(report_issue(&mut ana, draft.clone(), extra), could_not_send("GitHub is busy. Try again later."));
    assert_eq!(left_today(&mut ana), 4, "a 500 does not count");
    let again = reviewed_send(&mut ana, draft, json!({ "request_id": "try-2" })).unwrap();
    let id = again["id"].as_str().unwrap().to_owned();
    assert_eq!((again["state"].clone(), again["issue"].clone()), (json!({ "state": "open" }), json!(9)), "{again}");
    assert!(fake.github.body(9).ends_with(&format!("report {}", reports::ref_of(&id))));
    assert_eq!((fake.github.issue_count(), issues(), listed(&mut ana).len(), left_today(&mut ana)), (9, 3, 2, json!(3)), "the refused post, then the one that filed");

    // A case added to issue #7 that timed out is found among #7's comments.
    fake.github.hang_next_post(Duration::from_secs(3));
    let mut case = problem("Diff slow again", "Same.");
    case["to_issue"] = json!(7);
    assert_eq!(reviewed_send(&mut ana, case.clone(), json!({ "request_id": "try-3" })), could_not_send("GitHub took too long to answer."));
    let again = reviewed_send(&mut ana, case, json!({ "request_id": "try-3" })).unwrap();
    assert_eq!((again["issue"].clone(), again["followed"].clone(), again["state"].clone()), (json!(7), json!(true), json!({ "state": "open" })), "{again}");
    assert_eq!(fake.github.comment_bodies(7).len(), 1, "one comment");
    assert_eq!(left_today(&mut ana), 2);
}

#[test]
fn a_second_send_of_a_request_still_sending_waits_and_answers_its_outcome() {
    let fake = Fake::start();
    let hub = reporting("reports_inflight", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    fake.github.slow_creates(Duration::from_millis(800));
    let answers = Mutex::new(vec![]);
    std::thread::scope(|scope| {
        let (url, token, answers) = (&hub.url, &token, &answers);
        scope.spawn(move || {
            let mut client = Client::connect(url, token);
            answers.lock().unwrap().push(reviewed_send(&mut client, problem("Crash on launch", "It closes."), json!({ "request_id": "same" })).unwrap());
        });
        wait_for("the first send to be sending", || sql_text(&hub, "select string_agg(state->>'state', ',') from reports").as_deref() == Some("sending"));
        let mut client = Client::connect(url, token);
        answers.lock().unwrap().push(reviewed_send(&mut client, problem("Crash on launch", "It closes."), json!({ "request_id": "same" })).unwrap());
    });
    let answers = answers.into_inner().unwrap();
    assert_eq!((answers[0]["id"] == answers[1]["id"], answers[1]["issue"].clone(), answers[1]["state"].clone()), (true, json!(1), json!({ "state": "open" })), "{answers:?}");
    assert_eq!(fake.posts(), (1, 0));
}

#[test]
fn a_failed_send_counts_only_when_it_may_have_reached_github_and_never_says_what_github_said() {
    let fake = Fake::start();
    let hub = TestHub::start_reports("reports_counts", Signup::Open, Policy::default(), false, Default::default(), "127.0.0.1:0", Some(fake.impatient()));
    let mut ana = Client::connect(&hub.url, &signed_up(&hub, "ana@example.test"));
    let cases: [(u16, &str, Option<&str>, &str); 4] = [
        (401, "Bad credentials from GitHub", None, "GitHub refused the report."),
        (403, "Resource not accessible by integration", Some("4999"), "GitHub refused the report."),
        (403, "API rate limit exceeded for installation", Some("0"), "GitHub is busy. Try again later."),
        (500, "Internal trouble at GitHub", None, "GitHub is busy. Try again later."),
    ];
    for (n, (status, message, remaining, said)) in cases.into_iter().enumerate() {
        let draft = problem(&format!("Crash {n}"), "It closes.");
        let extra = reviewed(&mut ana, &draft, json!({})).unwrap();
        fake.github.fail_next(status, message, remaining);
        assert_eq!(report_issue(&mut ana, draft, extra), could_not_send(said), "{status} {message}");
        assert_eq!(left_today(&mut ana), 5, "a {status} answer did nothing, so it does not count");
    }
    fake.github.hang_next_post(Duration::from_secs(3));
    assert_eq!(reviewed_send(&mut ana, problem("Crash slow", "It closes."), json!({})), could_not_send("GitHub took too long to answer."));
    assert_eq!(left_today(&mut ana), 4, "a timeout may have filed it");
    let list = serde_json::to_string(&listed(&mut ana)).unwrap();
    for raw in ["Bad credentials from GitHub", "Resource not accessible", "API rate limit exceeded", "Internal trouble", "fake-hub-token"] {
        assert!(!list.contains(raw), "{raw} reached the app: {list}");
    }

    // A GitHub whose port nobody listens on: the request never left.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let config = Config { api, ..fake.config() };
    let hub = TestHub::start_reports("reports_unreachable", Signup::Open, Policy::default(), false, Default::default(), "127.0.0.1:0", Some(config));
    let mut bob = Client::connect(&hub.url, &signed_up(&hub, "bob@example.test"));
    assert_eq!(reviewed_send(&mut bob, problem("Crash", "It closes."), json!({})), could_not_send("Could not reach GitHub."));
    assert_eq!(left_today(&mut bob), 5, "a send that never left does not count");
    assert_eq!(listed(&mut bob)[0]["state"], json!({ "state": "failed", "error": "Could not reach GitHub." }));
}

#[test]
fn a_failed_draft_is_tried_again_by_id_and_a_failed_report_is_deleted_by_its_account_only() {
    let fake = Fake::start();
    let hub = TestHub::start_reports("reports_failed_drafts", Signup::Open, Policy::default(), false, Default::default(), "127.0.0.1:0", Some(fake.impatient()));
    let mut ana = Client::connect(&hub.url, &signed_up(&hub, "ana@example.test"));
    let mut bob = Client::connect(&hub.url, &signed_up(&hub, "bob@example.test"));
    let draft = problem("Half written", "It closes.");
    let id = ask(&mut ana, json!({ "op": "save_draft", "draft": draft })).unwrap()["id"].as_str().unwrap().to_owned();

    let extra = reviewed(&mut ana, &draft, json!({ "draft_id": id })).unwrap();
    fake.github.fail_next(502, "Bad gateway", None);
    assert_eq!(report_issue(&mut ana, draft.clone(), extra), could_not_send("GitHub is busy. Try again later."));
    let row = listed_row(&mut ana, &id);
    assert_eq!((row["state"].clone(), row["draft"]["happened"].clone()), (json!({ "state": "failed", "error": "GitHub is busy. Try again later." }), json!("It closes.")));
    let sent = reviewed_send(&mut ana, draft, json!({ "draft_id": id })).unwrap();
    assert_eq!((sent["id"].clone(), sent["state"].clone(), sent["issue"].clone()), (json!(id), json!({ "state": "open" }), json!(1)), "{sent}");
    assert_eq!((fake.posts(), listed(&mut ana).len(), left_today(&mut ana)), ((2, 0), 1, json!(4)));

    // A failed report that did not count is deleted; only by its own account.
    let extra = reviewed(&mut ana, &problem("Gone soon", "It closes."), json!({})).unwrap();
    fake.github.fail_next(500, "Server Error", None);
    report_issue(&mut ana, problem("Gone soon", "It closes."), extra).unwrap_err();
    let failed = listed(&mut ana).into_iter().find(|r| r["title"] == "Gone soon").unwrap()["id"].as_str().unwrap().to_owned();
    assert_eq!(ask(&mut bob, json!({ "op": "delete_draft", "id": failed })), Err(format!("this account has no draft {failed}")));
    assert_eq!(ask(&mut ana, json!({ "op": "delete_draft", "id": failed })), Ok(Value::Null));
    assert_eq!(listed(&mut ana).len(), 1);
    assert_eq!(ask(&mut ana, json!({ "op": "delete_draft", "id": id })), Err(format!("this account has no draft {id}")), "an open report is no draft");

    // One that counts leaves the list, and deleting it frees no send.
    fake.github.hang_next_post(Duration::from_secs(3));
    reviewed_send(&mut ana, problem("Timed out", "It closes."), json!({})).unwrap_err();
    assert_eq!(left_today(&mut ana), 3);
    let counted = listed(&mut ana).into_iter().find(|r| r["title"] == "Timed out").unwrap()["id"].as_str().unwrap().to_owned();
    assert_eq!(ask(&mut ana, json!({ "op": "delete_draft", "id": counted })), Ok(Value::Null));
    assert_eq!((listed(&mut ana).len(), left_today(&mut ana)), (1, json!(3)));
}

#[test]
fn a_denied_agent_report_is_kept_as_the_agents_draft_and_the_agent_hears_so() {
    let fake = Fake::start();
    let hub = reporting("reports_denied", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = agent_of(&hub.url, &token, "ana@example.test", "ana-deny-mac");
    let draft = json!({ "kind": "problem", "title": "Crash in site", "happened": "It closes for ana@example.test.", "expected": "No crash.", "repeat": "Open site" });
    let held = act(&ana.dir, &ana.tab, "report_issue", json!({ "draft": draft })).unwrap();
    let id = held["id"].as_str().unwrap().to_owned();
    ana.client.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    let decided = ana.client.hub(json!({ "cmd": "decide_confirmation", "id": id, "approve": false })).unwrap();
    assert_eq!((decided["decision"].clone(), decided["result"].clone()), (json!("denied"), json!("Not sent. Saved as a draft in My reports.")), "{decided}");

    let rows = listed(&mut ana.client);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0]["state"].clone(), rows[0]["title"].clone(), rows[0]["by_agent"].clone()), (json!({ "state": "draft" }), json!("Crash in [project]"), json!(true)));
    assert_eq!(rows[0]["draft"]["happened"], "It closes for [email].", "the held, scrubbed text is the draft");
    assert_eq!((fake.github.issue_count(), fake.posts()), (0, (0, 0)));
    wait_for("the notice in the agent's queue", || ana.node.machine.view(&ana.tab).unwrap().queued >= 1);
    let told = turn_end_context(&ana.dir, &ana.tab);
    assert!(told.contains(&format!("The user denied your report_issue request ({id}). Not sent. Saved as a draft in My reports.")), "{told}");
}

#[test]
fn two_hubs_on_one_database_count_the_sixth_send_under_the_accounts_lock() {
    let fake = Fake::start();
    let first = reporting("reports_two_hubs", Some(&fake));
    let token = signed_up(&first, "ana@example.test");
    let mut second = TestHub { runtime: tokio::runtime::Runtime::new().unwrap(), running: None, url: String::new(), database: first.database.clone(), dir: first.dir.clone(), boxes: first.boxes.clone(), policy: Policy::default(), learning: None, signup: Signup::Open, access: Default::default(), reports: Some(fake.config()) };
    second.serve("127.0.0.1:0");
    let mut ana = Client::connect(&first.url, &token);
    for n in 0..4 {
        reviewed_send(&mut ana, problem(&format!("Crash number {n}"), "It closes."), json!({})).unwrap();
    }
    fake.github.slow_creates(Duration::from_millis(300));

    // The account's lock, held from outside: neither hub marks a send while it is held.
    let outside = first.runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), first.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.batch_execute("begin; select pg_advisory_xact_lock(hashtextextended('pinned-reports:' || id::text, 0)) from accounts where email = 'ana@example.test'").await.unwrap();
        db
    });
    let answers = Mutex::new(vec![]);
    std::thread::scope(|scope| {
        for (n, url) in [first.url.clone(), second.url.clone()].into_iter().enumerate() {
            let (token, answers) = (&token, &answers);
            scope.spawn(move || {
                let mut client = Client::connect(&url, token);
                answers.lock().unwrap().push(reviewed_send(&mut client, problem(&format!("Race {n}"), "It closes."), json!({})));
            });
        }
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(sql_text(&first, "select count(*)::text from reports"), Some("4".into()), "no send is marked while the lock is held");
        first.runtime.block_on(outside.batch_execute("rollback")).unwrap();
    });
    let answers = answers.into_inner().unwrap();
    assert_eq!(answers.iter().filter(|a| a.is_ok()).count(), 1, "{answers:?}");
    assert!(answers.iter().find_map(|a| a.as_ref().err()).unwrap().starts_with("You sent 5 reports in the last day."), "{answers:?}");
    assert_eq!((fake.posts(), left_today(&mut ana)), ((5, 0), json!(0)));
}

#[test]
fn a_case_for_a_pull_request_or_a_locked_issue_is_refused_before_it_counts() {
    let fake = Fake::start();
    fake.github.add_issue_numbered(5, "Add a button", "A pull request.");
    fake.github.make_pull(5);
    fake.github.add_issue_numbered(6, "Heated thread", "Locked.");
    fake.github.lock(6);
    let hub = reporting("reports_pull_locked", Some(&fake));
    let mut ana = Client::connect(&hub.url, &signed_up(&hub, "ana@example.test"));
    let mut case = problem("Same here", "It closes.");
    case["to_issue"] = json!(5);
    assert_eq!(reviewed_send(&mut ana, case.clone(), json!({})), Err("#5 is a pull request, not an issue.".into()));
    case["to_issue"] = json!(6);
    assert_eq!(reviewed_send(&mut ana, case.clone(), json!({})), Err("Issue #6 is locked. Send this as a new report instead.".into()));
    case["to_issue"] = json!(40);
    assert_eq!(reviewed_send(&mut ana, case, json!({})), Err("There is no issue #40.".into()));
    assert_eq!((fake.posts(), left_today(&mut ana), listed(&mut ana).len()), ((0, 0), json!(5), 0));
}

#[test]
fn agents_keep_at_most_ten_drafts_and_the_account_fifty() {
    let fake = Fake::start();
    let hub = reporting("reports_draft_caps", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let mut ana = agent_of(&hub.url, &token, "ana@example.test", "ana-caps-mac");
    let save = |n: usize| json!({ "ask": { "op": "save_draft", "draft": problem(&format!("Agent draft {n}"), "x") } });
    for n in 0..10 {
        act(&ana.dir, &ana.tab, "reports", save(n)).unwrap();
    }
    assert_eq!(act(&ana.dir, &ana.tab, "reports", save(10)), Err("There are 10 drafts from agents. Delete one first.".into()));
    let rows = listed(&mut ana.client);
    assert_eq!((rows.len(), rows.iter().all(|r| r["by_agent"] == true)), (10, true));
    for n in 0..40 {
        ask(&mut ana.client, json!({ "op": "save_draft", "draft": problem(&format!("My draft {n}"), "x") })).unwrap();
    }
    assert_eq!(ask(&mut ana.client, json!({ "op": "save_draft", "draft": problem("One more", "x") })), Err("You have 50 drafts. Delete one to save another.".into()));
    assert_eq!(listed(&mut ana.client).iter().filter(|r| r["by_agent"] == false).count(), 40);
}

#[test]
fn one_account_makes_at_most_five_searches_a_minute() {
    let fake = Fake::start();
    let hub = reporting("reports_search_account", Some(&fake));
    let mut ana = Client::connect(&hub.url, &signed_up(&hub, "ana@example.test"));
    let mut bob = Client::connect(&hub.url, &signed_up(&hub, "bob@example.test"));
    for n in 0..5 {
        ask(&mut ana, json!({ "op": "similar", "title": format!("Topic number{n}x") })).unwrap();
    }
    assert_eq!(ask(&mut ana, json!({ "op": "similar", "title": "Sixth topic" })), Err("Searching the issues is busy. Try again in a minute.".into()));
    ask(&mut ana, json!({ "op": "similar", "title": "Topic number0x" })).unwrap();
    ask(&mut bob, json!({ "op": "similar", "title": "Sixth topic" })).unwrap();
    assert_eq!(fake.github.requests().iter().filter(|r| r.path.starts_with("/search/issues")).count(), 6);
}

#[test]
fn a_hub_without_reports_makes_no_table_and_one_whose_table_fails_still_serves() {
    let fake = Fake::start();
    let mut hub = reporting("reports_no_table", None);
    assert_eq!(sql_text(&hub, "select to_regclass('reports')::text"), None);
    // A table in the way: readying reporting fails, the hub serves without it.
    admin_on(&hub.database, "create table reports (id int)");
    hub.reports = Some(fake.config());
    hub.restart();
    let mut ana = Client::connect(&hub.url, &signed_up(&hub, "ana@example.test"));
    assert_eq!(ask(&mut ana, json!({ "op": "status" })), Ok(json!({ "available": false })));
    assert_eq!(reviewed_send(&mut ana, problem("Crash", "It closes."), json!({})), Err(NOT_AVAILABLE.into()));
}

#[test]
fn a_locked_login_unlocks_before_it_sends_a_report_or_approves_an_agents() {
    let fake = Fake::start();
    let hub = reporting("reports_locked", Some(&fake));
    let token = signed_up(&hub, "ana@example.test");
    let ana = agent_of(&hub.url, &token, "ana@example.test", "ana-locked-mac");
    let mut app = Client::app(&hub.url, &token);
    let locked: Result<Value, String> = Err(pinned_core::surface::LOCKED.into());

    assert_eq!(report_issue(&mut app, problem("From the dialog", "It closes."), json!({})), locked);
    let held = act(&ana.dir, &ana.tab, "report_issue", json!({ "draft": problem("From the agent", "It closes.") })).unwrap();
    assert_eq!(app.hub(json!({ "cmd": "decide_confirmation", "id": held["id"], "approve": true })), locked);
    assert_eq!(fake.posts(), (0, 0));

    app.hub(json!({ "cmd": "unlock", "password": PASSWORD })).unwrap();
    assert_eq!(reviewed_send(&mut app, problem("From the dialog", "It closes."), json!({})).unwrap()["issue"], 1);
    let decided = app.hub(json!({ "cmd": "decide_confirmation", "id": held["id"], "approve": true })).unwrap();
    assert_eq!((decided["outcome"].clone(), decided["result"].clone()), (json!("done"), json!({ "number": 2, "url": "https://github.com/acme/app/issues/2" })), "{decided}");
    assert_eq!(fake.posts(), (2, 0));
}

#[test]
fn a_send_with_no_title_or_too_long_is_refused_and_a_sent_report_is_no_draft() {
    let fake = Fake::start();
    let hub = reporting("reports_refusals", Some(&fake));
    let mut ana = Client::connect(&hub.url, &signed_up(&hub, "ana@example.test"));
    assert_eq!(report_issue(&mut ana, problem("   ", "It closes."), json!({})), Err("Give the report a title.".into()));
    let long = problem("Long", &"x".repeat(64_001));
    let too_long: Result<Value, String> = Err("The report is too long: keep it under 64000 characters.".into());
    assert_eq!(reviewed_send(&mut ana, long.clone(), json!({})), too_long);
    assert_eq!(ask(&mut ana, json!({ "op": "save_draft", "draft": long })), too_long);
    assert_eq!((fake.github.requests().len(), listed(&mut ana).len(), left_today(&mut ana)), (0, 0, json!(5)));

    let sent = reviewed_send(&mut ana, problem("Sent", "It closes."), json!({})).unwrap();
    let id = sent["id"].as_str().unwrap().to_owned();
    let no_draft: Result<Value, String> = Err(format!("this account has no draft {id}"));
    assert_eq!(ask(&mut ana, json!({ "op": "save_draft", "id": id, "draft": problem("Rewritten", "x") })), no_draft, "a sent report is not saved over");
    assert_eq!(reviewed_send(&mut ana, problem("Again", "x"), json!({ "draft_id": id })), no_draft, "a sent report is not sent again by id");
    assert_eq!(listed_row(&mut ana, &id)["title"], "Sent");
    assert_eq!(fake.posts(), (1, 0));
}
