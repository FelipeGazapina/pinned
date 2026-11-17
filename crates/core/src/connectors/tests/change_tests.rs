//! Linear status and assignee triggers on a person's account: one check reads the account's
//! changes once, and each change a workflow matches is claimed, then starts one run.
use super::*;
use std::sync::Arc;

/// What the loopback Linear answers. It knows `{ viewer { id } }`, the changes read, and a move
/// (the issue's team states, then `issueUpdate`); any other request is kept in `unlisted`,
/// answered 500, and fails the test.
#[derive(Default)]
struct Fake {
    issues: Vec<Value>,
    /// The status the changes read answers; 0 is 200.
    status: u16,
    seen: Vec<Value>,
    unlisted: Vec<String>,
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while let Ok(n) = std::io::Read::read(stream, &mut chunk) {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
        let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
        let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
        if buf.len() >= end + 4 + length {
            return String::from_utf8_lossy(&buf[end + 4..]).into_owned();
        }
    }
    String::new()
}

/// Points Linear at a loopback stand-in that answers from `fake`.
fn serve_linear(fake: Arc<Mutex<Fake>>) {
    install_rustls();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    linear::set_api(Some(format!("http://{}/graphql", listener.local_addr().unwrap())));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let body: Value = serde_json::from_str(&read_request(&mut stream)).unwrap_or(Value::Null);
            let query = body["query"].as_str().unwrap_or_default().to_owned();
            let mut fake = fake.lock().unwrap();
            fake.seen.push(body.clone());
            let (status, reply) = if query.contains("viewer { id }") {
                (200, json!({ "data": { "viewer": { "id": "u-me" } } }))
            } else if query.contains("issues(") && fake.status != 0 {
                (fake.status, json!({}))
            } else if query.contains("issues(") {
                (200, json!({ "data": { "issues": { "nodes": fake.issues.clone(), "pageInfo": { "endCursor": "c-1" } } } }))
            } else if query.contains("issue(id: $id)") {
                let states = json!([{ "id": "s-todo", "name": "Todo" }, { "id": "s-progress", "name": "In Progress" }]);
                (200, json!({ "data": { "issue": { "identifier": "ENG-1", "team": { "name": "Engineering", "states": { "nodes": states } } } } }))
            } else if query.contains("issueUpdate(") {
                (200, json!({ "data": { "issueUpdate": { "success": true } } }))
            } else {
                fake.unlisted.push(query.chars().take(80).collect());
                (500, json!({ "errors": [{ "message": "the stand-in knows no such query" }] }))
            };
            let reply = reply.to_string();
            let head = format!("HTTP/1.1 {status} stand-in\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len());
            let _ = std::io::Write::write_all(&mut stream, format!("{head}{reply}").as_bytes());
        }
    });
}

fn ago(minutes: i64) -> DateTime<Utc> {
    Utc::now() - chrono::Duration::minutes(minutes)
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A history entry that moves an issue from `from` to `to`, made by Ana at `at`.
fn moved(history: &str, at: DateTime<Utc>, from: &str, to: &str) -> Value {
    json!({ "id": history, "createdAt": stamp(at), "actor": { "id": "u-ana", "name": "Ana" },
        "fromState": { "name": from }, "toState": { "name": to }, "fromAssignee": null, "toAssignee": null })
}

/// ENG-`n`, updated at `updated`, with `history` oldest first. Its title and description are text
/// people write in Linear.
fn issue(n: u32, updated: DateTime<Utc>, history: Vec<Value>) -> Value {
    json!({ "id": format!("i-{n}"), "identifier": format!("ENG-{n}"), "title": format!("Export 500s {n}"),
        "description": "Steps: open the export page", "url": format!("https://linear.example/ENG-{n}"),
        "updatedAt": stamp(updated), "team": { "key": "ENG", "name": "Engineering" }, "project": null,
        "history": { "nodes": history } })
}

fn ready_to_dev(id: &str, since: DateTime<Utc>) -> Connector {
    Connector {
        id: id.into(),
        name: format!("Pick up {id}"),
        trigger: Trigger::LinearStatus { from: String::new(), to: "Ready to Dev".into(), scope: LinearScope::default(), by: By::Anyone },
        columns: Columns::default(),
        since: since.to_rfc3339(),
        ..linear_connector()
    }
}

/// A store that keeps claims as a hub's `trigger_fires` does, by workflow and key with the
/// claimer, and can be told to fail claims, store a failed one, wait, or fail saves.
#[derive(Clone, Default)]
struct Claims {
    inner: MemStorage,
    rows: Arc<Mutex<BTreeMap<(String, String), String>>>,
    /// One entry per coming claim: true fails it. Claims after the script succeed.
    fail: Arc<Mutex<Vec<bool>>>,
    /// A failed claim is stored all the same, as an insert that timed out but committed.
    commit_failed: Arc<AtomicBool>,
    wait: Arc<Mutex<Duration>>,
    calls: Arc<Mutex<Vec<String>>>,
    released: Arc<Mutex<Vec<String>>>,
    fail_save: Arc<AtomicBool>,
}

impl Storage for Claims {
    fn load(&self) -> Option<Vec<u8>> {
        self.inner.load()
    }
    fn save(&self, store: &[u8]) -> Result<(), String> {
        if self.fail_save.load(Ordering::SeqCst) {
            return Err("the disk is full".into());
        }
        self.inner.save(store)
    }
    fn key(&self, name: &str) -> Option<String> {
        self.inner.key(name)
    }
    fn write_key(&self, name: &str, key: &str) -> Result<(), String> {
        self.inner.write_key(name, key)
    }
    fn remove_key(&self, name: &str) {
        self.inner.remove_key(name)
    }
    fn load_runs(&self) -> Option<Vec<u8>> {
        None
    }
    fn save_runs(&self, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn claim_fire(&self, workflow: &str, key: &str, claimer: &str, retry: bool) -> Result<bool, String> {
        self.calls.lock().unwrap().push(key.to_owned());
        std::thread::sleep(*self.wait.lock().unwrap());
        let fail = {
            let mut script = self.fail.lock().unwrap();
            if script.is_empty() {
                false
            } else {
                script.remove(0)
            }
        };
        let mut rows = self.rows.lock().unwrap();
        let row = (workflow.to_owned(), key.to_owned());
        if fail {
            if self.commit_failed.load(Ordering::SeqCst) {
                rows.entry(row).or_insert_with(|| claimer.to_owned());
            }
            return Err("could not claim the fire: the database did not answer".into());
        }
        match rows.get(&row) {
            Some(stored) => Ok(retry && stored == claimer),
            None => {
                rows.insert(row, claimer.to_owned());
                Ok(true)
            }
        }
    }
    fn release_fire(&self, workflow: &str, key: &str, claimer: &str) -> Result<(), String> {
        self.released.lock().unwrap().push(key.to_owned());
        let mut rows = self.rows.lock().unwrap();
        if rows.get(&(workflow.to_owned(), key.to_owned())).is_some_and(|c| c == claimer) {
            rows.remove(&(workflow.to_owned(), key.to_owned()));
        }
        Ok(())
    }
}

/// The account's connectors with `workflows` and the Linear key `key` (each test its own, as the
/// viewer id is kept per key).
fn on(claims: &Claims, workflows: Vec<Connector>, key: &str) -> Connectors {
    claims.inner.save(&serde_json::to_vec(&Store { connectors: workflows, ..Store::default() }).unwrap()).unwrap();
    claims.inner.write_key(LINEAR_KEY, key).unwrap();
    Connectors::new(claims.clone(), "/bin/pinned".into())
}

/// One check: the runs it handed to `open` (each answered `answer`), and its error.
fn check_answering(connectors: &Connectors, answer: Result<(), String>) -> (Vec<Run>, Option<String>) {
    let opened = Mutex::new(vec![]);
    let open = |work: Work| {
        if let Work::Run(run) = &work {
            opened.lock().unwrap().push(run.clone());
        }
        let answer = answer.clone();
        async move { answer }
    };
    let check = block_on(connectors.check_once(&open));
    (opened.into_inner().unwrap(), check.error)
}

fn check(connectors: &Connectors) -> (Vec<Run>, Option<String>) {
    check_answering(connectors, Ok(()))
}

fn since_of(connectors: &Connectors, id: &str) -> DateTime<Utc> {
    let store = connectors.store.lock().unwrap();
    let since = &store.connectors.iter().find(|c| c.id == id).unwrap().since;
    DateTime::parse_from_rfc3339(since).unwrap().with_timezone(&Utc)
}

fn fake_with(issues: Vec<Value>) -> Arc<Mutex<Fake>> {
    let fake = Arc::new(Mutex::new(Fake { issues, ..Fake::default() }));
    serve_linear(fake.clone());
    fake
}

fn no_unlisted(fake: &Arc<Mutex<Fake>>) {
    assert_eq!(fake.lock().unwrap().unlisted, Vec::<String>::new(), "a request the stand-in does not list");
}

/// Design unit 4 and C4: a move into Ready to Dev starts one run that says what changed, with
/// everything Linear wrote framed as outside data, on a person's account too. The next check
/// starts none.
#[test]
fn a_move_into_ready_to_dev_starts_one_framed_run_and_the_next_check_none() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(2);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(10))], "lin-e1-move");

    let (runs, error) = check(&connectors);
    assert_eq!(error, None);
    assert_eq!(runs.len(), 1, "one run for one move");
    let run = &runs[0];
    assert_eq!(run.subject, Subject::LinearIssue { issue: "i-1".into() });
    assert!(run.input.contains("Moved from Backlog to Ready to Dev by Ana."), "{}", run.input);
    let prompt = run.args.last().unwrap();
    assert!(prompt.contains(&crate::untrusted::frame("a Linear issue", &run.input)), "the change is framed: {prompt}");
    let outside = outside_frames(prompt);
    for text in ["Ana", "Backlog", "Export 500s", "Steps: open"] {
        assert!(!outside.contains(text), "{text:?} outside a frame: {outside}");
    }
    for arg in &run.args[..run.args.len() - 1] {
        assert!(!arg.contains("Export 500s"), "the title reaches an argument: {arg}");
    }
    assert_eq!(*claims.calls.lock().unwrap(), ["linear:i-1#h-1:state"]);

    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None), "the next check starts none");
    no_unlisted(&fake);
}

/// Design unit 4: a move made while the workflow was off is dropped once it is turned on.
#[test]
fn a_move_made_while_the_workflow_was_off_starts_nothing_once_it_is_on() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(2);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let off = Connector { enabled: false, ..ready_to_dev("w1", ago(10)) };
    let connectors = on(&claims, vec![off.clone()], "lin-e1-off");
    connectors.save_connector(Connector { enabled: true, ..off }).unwrap();
    assert!(since_of(&connectors, "w1") > at, "turning it on moves since to now");

    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None));
    assert!(claims.calls.lock().unwrap().is_empty(), "nothing is claimed");
    no_unlisted(&fake);
}

/// A workflow that changes its trigger to a change kind drops what happened before, too.
#[test]
fn a_workflow_whose_trigger_becomes_a_change_kind_starts_from_now() {
    let claims = Claims::default();
    let label = Connector { id: "w1".into(), since: ago(10).to_rfc3339(), ..linear_connector() };
    let connectors = on(&claims, vec![label.clone()], "lin-e1-kind");
    connectors.save_connector(Connector { trigger: ready_to_dev("w1", ago(10)).trigger, ..label.clone() }).unwrap();
    assert!(since_of(&connectors, "w1") > ago(1), "a change kind starts now");
    let label = Connector { since: ago(10).to_rfc3339(), ..label };
    let connectors = on(&claims, vec![label.clone()], "lin-e1-kind");
    connectors.save_connector(Connector { instructions: "Other.".into(), ..label }).unwrap();
    assert!(since_of(&connectors, "w1") < ago(9), "linear-label keeps its since");
}

/// C2: after a restart a change 2 h old is skipped and counted, once, and the read starts at
/// `now - CATCH_UP`.
#[test]
fn a_change_older_than_the_catch_up_is_skipped_and_counted_once() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(120);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(180))], "lin-e1-away");

    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None));
    assert!(claims.calls.lock().unwrap().is_empty(), "a skipped change is not claimed");
    assert_eq!(connectors.change_counts()["w1"].skipped_away, 1);
    let floor = Utc::now() - chrono::Duration::from_std(CATCH_UP).unwrap() - chrono::Duration::seconds(30);
    let read = fake.lock().unwrap().seen.iter().find(|b| b["query"].as_str().unwrap_or_default().contains("issues(")).cloned().unwrap();
    let from = read["variables"]["filter"]["updatedAt"]["gte"].as_str().expect("the read starts at the catch-up floor");
    assert!(DateTime::parse_from_rfc3339(from).unwrap() > floor, "the read starts at now - CATCH_UP: {from}");
    assert!(since_of(&connectors, "w1") > floor);

    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None));
    assert_eq!(connectors.change_counts()["w1"].skipped_away, 1, "counted once");
    no_unlisted(&fake);
}

/// C1: a Linear error skips the pass and moves neither `since` nor the cursor.
#[test]
fn a_linear_error_moves_neither_since_nor_the_cursor() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let fake = fake_with(vec![]);
    fake.lock().unwrap().status = 429;
    let claims = Claims::default();
    let since = ago(10);
    let cursor = stamp(ago(5));
    let store = Store { connectors: vec![ready_to_dev("w1", since)], linear_changes_from: Some(cursor.clone()), ..Store::default() };
    claims.inner.save(&serde_json::to_vec(&store).unwrap()).unwrap();
    claims.inner.write_key(LINEAR_KEY, "lin-e1-error").unwrap();
    let connectors = Connectors::new(claims.clone(), "/bin/pinned".into());

    let (runs, error) = check(&connectors);
    assert_eq!(runs.len(), 0);
    assert!(error.as_deref().is_some_and(|e| e.contains("Linear answered 429")), "{error:?}");
    assert!(fake.lock().unwrap().seen.iter().any(|b| b["query"].as_str().unwrap_or_default().contains("issues(")), "the read was made");
    assert_eq!(since_of(&connectors, "w1"), DateTime::parse_from_rfc3339(&since.to_rfc3339()).unwrap().with_timezone(&Utc));
    assert_eq!(connectors.store.lock().unwrap().linear_changes_from.as_deref(), Some(cursor.as_str()));
    no_unlisted(&fake);
}

/// A hub's team keeps the change triggers inert: no Linear read, no claim, no run.
#[test]
fn a_team_keeps_the_change_triggers_inert() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(2);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let since = ago(10);
    let connectors = on(&claims, vec![ready_to_dev("w1", since)], "lin-e1-team");
    connectors.set_admission(Arc::new(TeamAdmission));

    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None));
    assert!(fake.lock().unwrap().seen.is_empty(), "a team reads no Linear changes");
    assert!(claims.calls.lock().unwrap().is_empty());
    assert_eq!(connectors.store.lock().unwrap().connectors[0].since, since.to_rfc3339(), "its since stays");
}

/// Item 7: one history entry that moves the status and changes the assignee is two events, each
/// claimed under its own kind.
#[test]
fn one_history_entry_claims_its_status_and_assignee_changes_under_two_keys() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(2);
    let mut both = moved("h-1", at, "Backlog", "Ready to Dev");
    both["toAssignee"] = json!({ "id": "u-bia", "name": "Bia" });
    let fake = fake_with(vec![issue(1, at, vec![both])]);
    let claims = Claims::default();
    let assigned = Connector {
        id: "w2".into(),
        trigger: Trigger::LinearAssigned { to: Assignee::Anyone, scope: LinearScope::default(), by: By::Anyone },
        ..ready_to_dev("w2", ago(10))
    };
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(10)), assigned], "lin-e1-both");

    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (2, None));
    let mut keys = claims.calls.lock().unwrap().clone();
    keys.sort();
    assert_eq!(keys, ["linear:i-1#h-1:assignee", "linear:i-1#h-1:state"]);
    let assigned_run = runs.iter().find(|r| r.connector == "w2").unwrap();
    assert!(assigned_run.input.contains("Assigned to Bia by Ana."), "{}", assigned_run.input);
    no_unlisted(&fake);
}

/// Item 3: a claim that answers an error starts nothing and holds the watermark; the next check
/// claims again and starts the run once.
#[test]
fn a_claim_error_starts_nothing_holds_the_watermark_and_the_next_check_fires_once() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(2);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    claims.fail.lock().unwrap().push(true);
    let since = ago(10);
    let connectors = on(&claims, vec![ready_to_dev("w1", since)], "lin-e1-claim-error");

    let (runs, error) = check(&connectors);
    assert_eq!(runs.len(), 0, "a claim error starts nothing");
    assert!(error.as_deref().is_some_and(|e| e.contains("could not claim")), "{error:?}");
    assert!(since_of(&connectors, "w1") < at, "the watermark stays before the change");
    assert_eq!(connectors.change_counts()["w1"].claim_errors, 1);

    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (1, None), "the next check fires it");
    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None), "once");
    no_unlisted(&fake);
}

/// Item 3: a claim that timed out but was stored fires once on its claimer's retry; another
/// claimer loses it.
#[test]
fn a_claim_that_timed_out_but_was_stored_fires_once_on_its_claimers_retry() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(2);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    claims.fail.lock().unwrap().push(true);
    claims.commit_failed.store(true, Ordering::SeqCst);
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(10))], "lin-e1-timeout");
    let other = Connectors::new(claims.clone(), "/bin/pinned".into());

    let (runs, _) = check(&connectors);
    assert_eq!(runs.len(), 0);
    assert_eq!(claims.rows.lock().unwrap().len(), 1, "the timed-out claim was stored");
    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (1, None), "its claimer's retry fires it");
    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None), "once");
    let (runs, error) = check(&other);
    assert_eq!((runs.len(), error), (0, None), "another claimer loses it");
    assert_eq!(claims.claim_fire("w1", "linear:i-1#h-1:state", other.claimer(), true), Ok(false), "even as a retry");
    no_unlisted(&fake);
}

/// Item 4: the watermark stops before the first change whose claim failed: the change before it
/// starts now, and the next check starts the failed one alone.
#[test]
fn the_watermark_stops_before_the_first_change_whose_claim_failed() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let (first, second) = (ago(3), ago(2));
    let fake = fake_with(vec![
        issue(2, second, vec![moved("h-2", second, "Backlog", "Ready to Dev")]),
        issue(1, first, vec![moved("h-1", first, "Backlog", "Ready to Dev")]),
    ]);
    let claims = Claims::default();
    claims.fail.lock().unwrap().extend([false, true]);
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(10))], "lin-e1-order");

    let (runs, error) = check(&connectors);
    assert_eq!(runs.iter().map(|r| r.subject.clone()).collect::<Vec<_>>(), [Subject::LinearIssue { issue: "i-1".into() }]);
    assert!(error.is_some());
    let since = since_of(&connectors, "w1");
    assert!(since >= first && since < second, "the watermark stops before the failed change: {since}");

    let (runs, error) = check(&connectors);
    assert_eq!(error, None);
    assert_eq!(runs.iter().map(|r| r.subject.clone()).collect::<Vec<_>>(), [Subject::LinearIssue { issue: "i-2".into() }]);
    no_unlisted(&fake);
}

/// Item 5: after the first claim error a check claims nothing more, so failing claims with four
/// changes pending end the check well within the call timeout.
#[test]
fn failing_claims_end_the_check_well_within_the_call_timeout() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let issues = (1..=4).rev().map(|n| issue(n, ago(6 - n as i64), vec![moved(&format!("h-{n}"), ago(6 - n as i64), "Backlog", "Ready to Dev")])).collect();
    let fake = fake_with(issues);
    let claims = Claims::default();
    claims.fail.lock().unwrap().extend([true; 4]);
    *claims.wait.lock().unwrap() = Duration::from_secs(2);
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(10))], "lin-e1-stall");

    let started = std::time::Instant::now();
    let (runs, error) = check(&connectors);
    let took = started.elapsed();
    assert_eq!(runs.len(), 0);
    assert!(error.is_some());
    assert_eq!(claims.calls.lock().unwrap().len(), 1, "one failed claim ends the claims of the check");
    assert!(took < CALL_TIMEOUT / 6, "the check took {took:?}");
    no_unlisted(&fake);
}

/// Item 6: a won claim whose run does not start is given back and counted, for a refused start
/// and for a store that cannot save.
#[test]
fn a_won_claim_whose_run_does_not_start_is_given_back_and_counted() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(2);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(10))], "lin-e1-refused");
    let (runs, error) = check_answering(&connectors, Err("the machine refused the run".into()));
    assert_eq!(runs.len(), 1, "the start was tried");
    assert!(error.is_some());
    assert_eq!(*claims.released.lock().unwrap(), ["linear:i-1#h-1:state"], "the claim is given back");
    assert!(claims.rows.lock().unwrap().is_empty());
    let counts = connectors.change_counts()["w1"].clone();
    assert_eq!((counts.released, counts.not_started), (1, 1));

    let claims = Claims::default();
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(10))], "lin-e1-refused");
    claims.fail_save.store(true, Ordering::SeqCst);
    let (runs, error) = check(&connectors);
    assert_eq!(runs.len(), 0, "a check that cannot save its watermark starts nothing");
    assert!(error.is_some());
    assert_eq!(*claims.released.lock().unwrap(), ["linear:i-1#h-1:state"], "and gives the claim back");
    no_unlisted(&fake);
}

/// Item 6: an Ask step that stops a change's run already asked its model, so the claim is kept:
/// the next check, whose overlap reads the change again, neither asks again nor starts it.
#[test]
fn an_ask_step_that_stops_a_change_run_keeps_its_claim() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = Utc::now() - chrono::Duration::seconds(10);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let ask = Ask {
        model: "google/gemini-3.5-flash".into(),
        instructions: "Classify.".into(),
        questions: vec![Question { name: "agent".into(), ask: "Can a coding agent handle it?".into(), choices: vec!["yes".into(), "no".into()], stop_on: vec!["no".into()] }],
    };
    let connectors = on(&claims, vec![Connector { ask: Some(ask), ..ready_to_dev("w1", ago(10)) }], "lin-e1-ask-stop");
    claims.inner.write_key(VERCEL_KEY, "vck_teststop").unwrap();
    let _reply = TestReply::set(r#"{"agent":"no"}"#.into());

    for _ in 0..2 {
        let (runs, error) = check(&connectors);
        assert_eq!((runs.len(), error), (0, None), "the Ask step stops the run");
    }
    assert!(claims.released.lock().unwrap().is_empty(), "the stopped run keeps its claim");
    assert_eq!(connectors.change_counts()["w1"].not_started, 1, "the model is asked once");
    no_unlisted(&fake);
}

/// Item 2: a change older than the catch-up floor is never claimed, so once its claim expired
/// (after `FIRES_KEPT`, longer than `CATCH_UP`) it does not fire again.
#[test]
fn a_change_older_than_the_catch_up_is_never_claimed_even_after_its_claim_expired() {
    assert!(CATCH_UP < FIRES_KEPT);
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = Utc::now() - chrono::Duration::days(8);
    let fake = fake_with(vec![issue(1, ago(1), vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let connectors = on(&claims, vec![ready_to_dev("w1", Utc::now() - chrono::Duration::days(10))], "lin-e1-expired");
    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None));
    assert!(claims.calls.lock().unwrap().is_empty(), "no claim for a change past the floor");
    no_unlisted(&fake);
}

/// Item 8: a node without hub keeps its claimed fires in its folder for `FIRES_KEPT`, so a
/// restart within the check's overlap fires nothing again.
#[test]
fn fired_keys_outlive_a_restart_of_a_node_without_hub() {
    let dir = temp_dir("fires-restart");
    let key = "linear:i-1#h-1:state";
    assert_eq!(FileStorage::new(dir.clone()).claim_fire("w1", key, "a", false), Ok(true));
    assert_eq!(FileStorage::new(dir.clone()).claim_fire("w1", key, "b", false), Ok(false), "a restart keeps the claim");
    let file = dir.join("fires.json");
    let mut fires: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    fires[0]["at"] = json!(Utc::now().timestamp() - 8 * 24 * 60 * 60);
    std::fs::write(&file, fires.to_string()).unwrap();
    assert_eq!(FileStorage::new(dir.clone()).claim_fire("w1", key, "c", false), Ok(true), "a claim older than FIRES_KEPT is dropped");
    let _ = std::fs::remove_dir_all(&dir);

    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = Utc::now() - chrono::Duration::seconds(20);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let (connectors, dir) = stored("fires-restart-check", vec![ready_to_dev("w1", ago(10))]);
    std::fs::write(dir.join(LINEAR_KEY), "lin-e1-restart").unwrap();
    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (1, None));
    drop(connectors);
    let connectors = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into());
    assert!(since_of(&connectors, "w1") < at, "the saved watermark overlaps the change");
    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None), "a restart does not fire it again");
    no_unlisted(&fake);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A graph workflow's run keeps the change in its trigger, which its templates read.
#[test]
fn a_graph_workflow_keeps_the_change_in_its_trigger() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = ago(2);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let mut workflow = ready_to_dev("w1", ago(10));
    workflow.graph = Some(migrate(&workflow));
    let connectors = on(&claims, vec![workflow], "lin-e1-graph");
    let (_, error) = check(&connectors);
    assert_eq!(error, None);
    let runs = connectors.graph_runs();
    assert_eq!(runs.len(), 1, "one graph run");
    let change = &runs[0].context["trigger"]["change"];
    assert_eq!((change["kind"].as_str(), change["from"].as_str(), change["to"].as_str(), change["by"].as_str()), (Some("status"), Some("Backlog"), Some("Ready to Dev"), Some("Ana")));
    let (_, error) = check(&connectors);
    assert_eq!(error, None);
    assert_eq!(connectors.graph_runs().len(), 1, "the next check starts none");
    no_unlisted(&fake);
}

/// The change line, and an issue trigger with no change that reads as before.
#[test]
fn a_change_trigger_says_what_changed() {
    let base = |change: Option<IssueChange>| TriggerInput::Issue {
        id: "i-1".into(),
        identifier: "ENG-1".into(),
        title: "Export 500s".into(),
        description: None,
        url: "https://linear.example/ENG-1".into(),
        change,
    };
    let change = |kind, from: Option<&str>, to: Option<&str>| IssueChange { kind, from: from.map(Into::into), to: to.map(Into::into), by: "Ana".into(), at: "2026-11-17T10:00:00.000Z".into() };
    assert_eq!(trigger_text(&base(None)), "ENG-1: Export 500s\nhttps://linear.example/ENG-1\n\n(no description)");
    assert_eq!(
        trigger_text(&base(Some(change(IssueChangeKind::Status, Some("Backlog"), Some("Ready to Dev"))))),
        "ENG-1: Export 500s\nMoved from Backlog to Ready to Dev by Ana.\nhttps://linear.example/ENG-1\n\n(no description)"
    );
    assert!(trigger_text(&base(Some(change(IssueChangeKind::Assignee, None, Some("Bia"))))).contains("\nAssigned to Bia by Ana.\n"));
    assert!(trigger_text(&base(Some(change(IssueChangeKind::Assignee, Some("Bia"), None)))).contains("\nUnassigned by Ana.\n"));
    assert!(!serde_json::to_string(&base(None)).unwrap().contains("change"), "older apps see no change field");
}

/// A workflow that fires on a move into `to`.
fn into(id: &str, to: &str, since: DateTime<Utc>) -> Connector {
    Connector { trigger: Trigger::LinearStatus { from: String::new(), to: to.into(), scope: LinearScope::default(), by: By::Anyone }, ..ready_to_dev(id, since) }
}

/// E2 item 1: a move pinned made itself (its stage step) comes back as a change; that echo starts
/// no workflow and is counted. The same move made by someone else, with no write of pinned's
/// within `ECHO_WINDOW` of it, starts one run.
#[test]
fn pinneds_own_move_starts_nothing_and_the_same_move_by_someone_else_starts_one_run() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let fake = fake_with(vec![]);
    let claims = Claims::default();
    let workflow = Connector { columns: Columns { picked: "In Progress".into(), ..Columns::default() }, ..into("w1", "In Progress", ago(10)) };
    let connectors = on(&claims, vec![workflow], "lin-e2-echo");
    // Its stage step moves ENG-1 and ENG-3 into In Progress; ENG-2 nobody in pinned moved.
    for issue in ["i-1", "i-3"] {
        let moved_to = block_on(connectors.stage("w1", &Subject::LinearIssue { issue: issue.into() }, Stage::Picked));
        assert_eq!(moved_to, Ok("moved the issue to In Progress".into()));
    }
    let now = Utc::now();
    fake.lock().unwrap().issues = vec![
        issue(1, now, vec![moved("h-1", now, "Todo", "In Progress")]),
        issue(2, ago(1), vec![moved("h-2", ago(1), "Todo", "In Progress")]),
        // Three minutes before pinned's write: not its echo.
        issue(3, ago(3), vec![moved("h-3", ago(3), "Todo", "In Progress")]),
    ];

    let (runs, error) = check(&connectors);
    assert_eq!(error, None);
    let mut started: Vec<String> = runs.iter().map(|run| subject_key(&run.subject)).collect();
    started.sort();
    assert_eq!(started, ["i-2", "i-3"], "the echo starts nothing; the others start one run each");
    assert_eq!(connectors.change_counts()["w1"].echoes, 1);
    let (runs, error) = check(&connectors);
    assert_eq!((runs.len(), error), (0, None), "the next check, which reads the echo again, starts none");
    assert_eq!(connectors.change_counts()["w1"].echoes, 1, "and counts it once");
    no_unlisted(&fake);
}

/// E2 item 2: two moves into the same status on one issue 5 minutes apart start the workflow
/// once, the second counted as cooled; 16 minutes apart, twice, the second on the next check
/// (#346 E3 item 1: a check starts one run of a workflow on an issue).
#[test]
fn a_second_move_within_the_cooldown_starts_nothing_and_after_it_starts_again() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let fake = fake_with(vec![
        issue(1, ago(7), vec![moved("h-1", ago(12), "Backlog", "Ready to Dev"), moved("h-2", ago(7), "Todo", "Ready to Dev")]),
        issue(2, ago(4), vec![moved("h-3", ago(20), "Backlog", "Ready to Dev"), moved("h-4", ago(4), "Todo", "Ready to Dev")]),
    ]);
    let claims = Claims::default();
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(30))], "lin-e2-cooldown");

    let (runs, error) = check(&connectors);
    assert_eq!(error, None);
    let on_issue = |runs: &[Run], issue: &str| runs.iter().filter(|run| run.subject == Subject::LinearIssue { issue: issue.into() }).count();
    assert_eq!(on_issue(&runs, "i-1"), 1, "5 minutes apart: one start");
    assert_eq!(on_issue(&runs, "i-2"), 1, "16 minutes apart: one start this check");
    assert_eq!(connectors.change_counts()["w1"].cooled, 1);
    let (later, error) = check(&connectors);
    assert_eq!(error, None);
    assert_eq!((on_issue(&later, "i-1"), on_issue(&later, "i-2")), (0, 1), "and the second start on the next check");
    assert_eq!(connectors.change_counts()["w1"].cooled, 1);
    no_unlisted(&fake);
}

/// Five qualifying changes on ENG-`n` within the hour, between two workflows: into Ready to Dev
/// (`w1`) and into In Progress (`w2`), each 15 minutes or more after its last start there.
fn five_changes(n: u32, fifth: Value) -> Value {
    issue(n, ago(10), vec![
        moved("h-1", ago(50), "Backlog", "Ready to Dev"),
        moved("h-2", ago(45), "Ready to Dev", "In Progress"),
        moved("h-3", ago(30), "In Progress", "Ready to Dev"),
        moved("h-4", ago(25), "Ready to Dev", "In Progress"),
        fifth,
    ])
}

/// E2 item 3: at most `ISSUE_CAP` change-trigger starts per issue per hour across the account's
/// workflows: the fifth is skipped, counted, and logged once for the issue. A check starts one run
/// of a workflow on an issue (#346 E3 item 1), so the four starts take two checks.
#[test]
fn a_fifth_change_on_one_issue_within_the_hour_is_capped_and_logged_once() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let fake = fake_with(vec![five_changes(46, moved("h-5", ago(10), "In Progress", "Ready to Dev"))]);
    let claims = Claims::default();
    let connectors = on(&claims, vec![into("w-cap-1", "Ready to Dev", ago(55)), into("w-cap-2", "In Progress", ago(55))], "lin-e2-cap");

    let (runs, error) = check(&connectors);
    assert_eq!(error, None);
    let (later, error) = check(&connectors);
    assert_eq!(error, None);
    assert_eq!((runs.len(), later.len()), (2, 2), "exactly four starts");
    assert_eq!(connectors.change_counts()["w-cap-1"].capped, 1, "the fifth is capped");
    assert_eq!(connectors.change_counts().get("w-cap-2").map_or(0, |counts| counts.capped), 0);
    fake.lock().unwrap().issues = vec![five_changes(46, moved("h-5", ago(10), "In Progress", "Ready to Dev"))];
    fake.lock().unwrap().issues[0]["history"]["nodes"].as_array_mut().unwrap().push(moved("h-6", Utc::now(), "Ready to Dev", "In Progress"));
    let (runs, _) = check(&connectors);
    assert_eq!(runs.len(), 0, "a sixth within the hour is capped too");
    assert_eq!(connectors.change_counts()["w-cap-2"].capped, 1);
    let lines: Vec<String> = cap_lines().into_iter().filter(|line| line.contains("i-46")).collect();
    assert_eq!(lines.len(), 1, "one log line for the issue in the hour: {lines:?}");
    assert!(lines[0].contains("w-cap-1") && !lines[0].contains("Export"), "{}", lines[0]);
    no_unlisted(&fake);
}

/// E2 item 6: the guards run before the Ask step: a capped change asks no model.
#[test]
fn a_capped_change_asks_no_model() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let mut assigned = moved("h-5", ago(10), "Ready to Dev", "Ready to Dev");
    assigned["fromState"] = Value::Null;
    assigned["toState"] = Value::Null;
    assigned["toAssignee"] = json!({ "id": "u-bia", "name": "Bia" });
    let fake = fake_with(vec![five_changes(47, assigned)]);
    let claims = Claims::default();
    let ask = Ask {
        model: "google/gemini-3.5-flash".into(),
        instructions: "Classify.".into(),
        questions: vec![Question { name: "agent".into(), ask: "Can a coding agent handle it?".into(), choices: vec!["yes".into(), "no".into()], stop_on: vec!["no".into()] }],
    };
    let asking = Connector {
        ask: Some(ask),
        trigger: Trigger::LinearAssigned { to: Assignee::Anyone, scope: LinearScope::default(), by: By::Anyone },
        ..ready_to_dev("w-ask-3", ago(55))
    };
    let connectors = on(&claims, vec![into("w-ask-1", "Ready to Dev", ago(55)), into("w-ask-2", "In Progress", ago(55)), asking], "lin-e2-cap-ask");
    claims.inner.write_key(VERCEL_KEY, "vck_testcap").unwrap();
    let _reply = TestReply::set(r#"{"agent":"yes"}"#.into());
    crate::vercel::take_test_inputs();

    let (runs, error) = check(&connectors);
    assert_eq!(error, None);
    let (later, error) = check(&connectors);
    assert_eq!(error, None);
    assert_eq!((runs.len(), later.len()), (2, 2));
    assert_eq!(connectors.change_counts()["w-ask-3"].capped, 1);
    assert_eq!(crate::vercel::take_test_inputs(), Vec::<String>::new(), "no model was asked");
    no_unlisted(&fake);
}

/// E2 item 5: a store saved before the guards' counts reads them back as zeros.
#[test]
fn an_old_store_reads_the_guard_counts_as_zeros() {
    let old = r#"{"change_counts":{"w1":{"skippedAway":2,"claimErrors":1,"released":0,"notStarted":1}},"connectors":[]}"#;
    let store = serde_json::from_str::<Store>(old);
    assert!(store.is_ok(), "an old store reads: {:?}", store.as_ref().err());
    let counts = &store.unwrap().change_counts["w1"];
    assert_eq!((counts.skipped_away, counts.claim_errors, counts.not_started), (2, 1, 1));
    assert_eq!((counts.echoes, counts.cooled, counts.capped), (0, 0, 0));
}

/// #346 E3 item 4: a change whose run fails to start, read again in the next check's overlap,
/// starts there once. Its first try was no start: it is not cooled, and the cap counts one start.
#[test]
fn a_change_whose_first_run_failed_to_start_starts_on_the_retry_and_counts_one_start() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let at = Utc::now() - chrono::Duration::seconds(10);
    let fake = fake_with(vec![issue(1, at, vec![moved("h-1", at, "Backlog", "Ready to Dev")])]);
    let claims = Claims::default();
    let connectors = on(&claims, vec![ready_to_dev("w1", ago(10))], "lin-e3-started");

    let (runs, error) = check_answering(&connectors, Err("the machine refused the run".into()));
    assert_eq!(runs.len(), 1, "the start was tried");
    assert!(error.is_some());
    let mut started = 0;
    for _ in 0..2 {
        let (runs, error) = check(&connectors);
        assert_eq!(error, None);
        started += runs.len();
    }
    assert_eq!(connectors.change_counts()["w1"].cooled, 0, "a run that did not start cools nothing");
    assert_eq!(started, 1, "the retry starts it once");
    let starts = connectors.guards.lock().unwrap().starts.get("i-1").map_or(0, Vec::len);
    assert_eq!(starts, 1, "the cap counts one start");
    no_unlisted(&fake);
}

/// An Ask step that stops a run whose issue a coding agent cannot handle.
fn agent_ask() -> Ask {
    Ask {
        model: "google/gemini-3.5-flash".into(),
        instructions: "Classify.".into(),
        questions: vec![Question { name: "agent".into(), ask: "Can a coding agent handle it?".into(), choices: vec!["yes".into(), "no".into()], stop_on: vec!["no".into()] }],
    }
}

/// `reads` checks' reads of the account, each before any of their runs is handed over, then the
/// hand-over of every run they found, each answered `answer`: the runs each read found, and the
/// runs handed to `open`.
fn reads_then_hand_over(connectors: &Connectors, reads: usize, answer: Result<(), String>) -> (Vec<usize>, Vec<Run>) {
    let mut found = vec![];
    let mut work = vec![];
    for _ in 0..reads {
        let (read, _) = block_on(connectors.poll());
        found.push(read.iter().filter(|(work, _)| matches!(work, Work::Run(_))).count());
        work.extend(read);
    }
    let opened = Mutex::new(vec![]);
    let open = |work: Work| {
        if let Work::Run(run) = &work {
            opened.lock().unwrap().push(run.clone());
        }
        let answer = answer.clone();
        async move { answer }
    };
    for (work, fire) in work {
        let _ = block_on(connectors.hand_over_fired(work, fire, &open));
    }
    (found, opened.into_inner().unwrap())
}

/// #346 E3 item 1: two moves of one issue into Ready to Dev, 20 minutes apart (past the
/// cooldown), read in one check by a workflow with an Ask step. A check starts at most one run on
/// the issue, so the later move waits for the next check; each run's Ask step reads the issue and
/// the change (Ana, Todo) only inside the frame, whatever the order the runs are handed over in;
/// and each run that does not start gives back its own claim.
#[test]
fn two_changes_on_one_issue_start_one_run_a_check_each_framed_and_each_gives_back_its_own_claim() {
    let _api = slack::lock_api();
    let _restore = RestoreLinear;
    let fake = fake_with(vec![issue(1, ago(20), vec![moved("h-1", ago(40), "Backlog", "Ready to Dev"), moved("h-2", ago(20), "Todo", "Ready to Dev")])]);
    let claims = Claims::default();
    let connectors = on(&claims, vec![Connector { ask: Some(agent_ask()), ..ready_to_dev("w1", ago(50)) }], "lin-e3-two");
    claims.inner.write_key(VERCEL_KEY, "vck_testtwo").unwrap();
    let _reply = TestReply::set(r#"{"agent":"yes"}"#.into());
    crate::vercel::take_test_inputs();

    let (found, runs) = reads_then_hand_over(&connectors, 2, Err("the machine refused the run".into()));
    assert_eq!(found, [1, 1], "one run on the issue a check: the later move waits for the next");
    let inputs = crate::vercel::take_test_inputs();
    assert_eq!(inputs.len(), 2, "each run asks once");
    assert!(runs[0].input.contains("Moved from Backlog to Ready to Dev by Ana."), "{}", runs[0].input);
    assert!(runs[1].input.contains("Moved from Todo to Ready to Dev by Ana."), "{}", runs[1].input);
    for (input, run) in inputs.iter().zip(&runs) {
        assert_eq!(input, &crate::untrusted::frame("a Linear issue", &run.input), "the Ask step reads the change only framed");
    }
    assert_eq!(*claims.released.lock().unwrap(), ["linear:i-1#h-1:state", "linear:i-1#h-2:state"], "each run gives back its own claim");
    assert!(claims.rows.lock().unwrap().is_empty());
    no_unlisted(&fake);
}
