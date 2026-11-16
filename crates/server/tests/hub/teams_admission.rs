//! pinned Teams, admission: the hub decides once whether a trigger's event runs now, waits, or is
//! dropped or skipped, before anything outside pinned happens for it. Each test hands the team's
//! own connectors an event the way its poller would (`Connectors::found`), against stand-ins for
//! Linear, Slack and the model gateway that record every call, and checks which ones the event
//! made.

use super::teams_slack_notice::{seen, slack_standin};
use super::*;
use pinned_core::connectors::Event;
use pinned_core::teams::{Bot, BotLimits, CallRules, TeamOp, TeamPolicy};
use pinned_server::hub::Account;
use pinned_server::teams;
use uuid::Uuid;

/// A call the Linear or model stand-in got: which, with what key, and its body.
#[derive(Clone, Debug)]
struct Call {
    service: &'static str,
    key: String,
    path: String,
    body: String,
}

static CALLS: Mutex<Vec<Call>> = Mutex::new(Vec::new());

/// Starts the Linear and model stand-ins once, and the Slack one. Each test names its own keys,
/// so they share the servers.
fn standins() {
    static SERVER: OnceLock<()> = OnceLock::new();
    SERVER.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        std::env::set_var(pinned_core::linear::API_ENV, format!("{root}/graphql"));
        std::env::set_var(pinned_core::vercel::API_ROOT_ENV, format!("{root}/v1"));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || answer(stream));
            }
        });
    });
    slack_standin();
}

fn answer(mut stream: TcpStream) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut head = vec![];
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let header = |name: &str| head.lines().find_map(|l| l.to_ascii_lowercase().starts_with(&format!("{name}:")).then(|| l.split_once(':').unwrap().1.trim().to_owned())).unwrap_or_default();
    let mut body = vec![0u8; header("content-length").parse().unwrap_or(0)];
    let _ = stream.read_exact(&mut body);
    let body = String::from_utf8_lossy(&body).to_string();
    let path = head.split_whitespace().nth(1).unwrap_or_default().to_owned();
    let key = header("authorization").trim_start_matches("Bearer ").to_owned();
    let service = if path.starts_with("/graphql") { "linear" } else { "model" };
    CALLS.lock().unwrap().push(Call { service, key, path: path.clone(), body: body.clone() });
    let answer = match (service, path.as_str()) {
        ("linear", _) if body.contains("viewer") => json!({ "data": { "viewer": { "name": "Acme bot", "email": "", "organization": { "name": "Acme" } } } }),
        ("linear", _) if body.contains("issues(") => json!({ "data": { "issues": { "nodes": [] } } }),
        ("linear", _) if body.contains("issueUpdate") => json!({ "data": { "issueUpdate": { "success": true } } }),
        ("linear", _) if body.contains("issue(id") => json!({ "data": { "issue": { "identifier": "ENG", "team": { "name": "Eng", "states": { "nodes": [{ "id": "state-1", "name": "In Progress" }] } } } } }),
        ("linear", _) => json!({ "errors": [{ "message": "the stand-in knows no such query" }] }),
        (_, p) if p.starts_with("/v1/models/") => json!({ "id": "acme/sorter", "type": "language" }),
        (_, "/v1/chat/completions") => json!({ "choices": [{ "message": { "content": "{\"kind\":\"bug\"}" } }] }),
        _ => json!({}),
    };
    let answer = answer.to_string();
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len());
}

/// What reached outside pinned for issue `ENG-n` with keys of test `name`: each model question
/// about it, each Linear move of it, and each Slack reaction or post.
fn outside(name: &str, n: u32) -> Vec<String> {
    let issue = format!("\"eng-{n}\"");
    let text = format!("ENG-{n}:");
    let mut seen_calls: Vec<String> = CALLS
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.key.ends_with(name))
        .filter_map(|c| match c.service {
            "model" if c.path == "/v1/chat/completions" && c.body.contains(&text) => Some("model".to_owned()),
            "linear" if c.body.contains("issueUpdate") && c.body.contains(&issue) => Some("linear move".to_owned()),
            _ => None,
        })
        .collect();
    seen_calls.extend(seen(&format!("xoxb-dev-{name}")).into_iter().filter(|s| matches!(s.method.as_str(), "reactions.add" | "chat.postMessage")).filter(|s| s.text.contains(&text) || s.method == "reactions.add").map(|s| format!("slack {}", s.method)));
    seen_calls
}

/// A hub with a team that keeps the Upstash key and Claude token, and has Linear, Slack and the
/// model gateway connected with keys ending in `name`: ana owns it, bob is a member.
struct Crew {
    hub: TestHub,
    team: Uuid,
    ana: Arc<Account>,
    bob: Arc<Account>,
    folder: PathBuf,
    name: String,
}

impl Crew {
    fn new(name: &str) -> Crew {
        standins();
        let patient = TeamPolicy { escalate_after_secs: 3600, give_up_after_secs: 7200, ..TeamPolicy::default() };
        let hub = TestHub::start_with_team_policy(name, fast(), patient);
        let sign_up = |email: &str| {
            let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
            hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
        };
        let (ana, bob) = (sign_up("ana@example.test"), sign_up("bob@example.test"));
        let made = hub.runtime.block_on(teams::call(hub.hub(), &ana, None, teams::Op::Team(TeamOp::Create { name: "Acme".into() }))).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", bob.id));
        let folder = temp(&format!("teams-admission-{name}"));
        let crew = Crew { hub, team, ana, bob, folder, name: name.to_owned() };
        crew.hub_call(json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN }));
        // A team takes no key from a member's app yet: the stand-ins' keys go straight in.
        let hub = crew.hub.hub().clone();
        crew.hub.runtime.block_on(async {
            let connectors = hub.connectors(&hub.account(team).await.unwrap()).await.unwrap();
            connectors.connect_linear(&format!("lin-{name}")).await.unwrap();
            connectors.connect_vercel(&format!("ver-{name}")).await.unwrap();
            connectors.connect_slack(&format!("xoxb-dev-{name}")).await.unwrap();
        });
        crew
    }

    fn hub_call(&self, call: Value) -> Value {
        self.call(&self.ana, TeamOp::Hub { call }).unwrap()
    }

    fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        let op = teams::Op::read(op)?;
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), op))
    }

    /// A Linear label bot whose Ask step asks the model what kind of issue it is, and whose
    /// session moves the issue to "In Progress"; bob saved it and ana turned it on.
    fn on_bot(&self, name: &str) -> String {
        let saved = self.hub_call(json!({ "cmd": "save_connector", "connector": {
            "name": name, "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
            "columns": { "picked": "In Progress", "coding": "", "review": "" },
            "ask": { "model": "acme/sorter", "instructions": "Sort the issue.", "questions": [{ "name": "kind", "ask": "What kind of issue is it?" }] },
        } }));
        let id = saved["id"].as_str().unwrap().to_owned();
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot }).unwrap();
        self.turn_on(&id, true);
        id
    }

    fn turn_on(&self, id: &str, on: bool) {
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.to_owned(), on }).unwrap();
    }

    /// A Linear label bot whose graph moves the issue to "In Progress", asks the model what kind
    /// of issue it is, then opens its session; bob saved it and ana turned it on.
    fn graph_bot(&self, name: &str) -> String {
        let watch = json!({ "kind": "linear-label", "label": "agent" });
        let steps = [
            json!({ "type": "stage-picked" }),
            json!({ "type": "classify", "model": "acme/sorter", "instructions": "Sort the issue.", "questions": [{ "name": "kind", "ask": "What kind of issue is it?" }] }),
            json!({ "type": "session", "prompt": "Fix the issue." }),
        ];
        let mut nodes = vec![json!({ "id": "t", "name": "Trigger", "position": [0, 0], "type": "trigger", "watch": watch })];
        let mut edges = vec![];
        let mut from = "t".to_owned();
        for (i, mut step) in steps.into_iter().enumerate() {
            let id = format!("s{i}");
            step["id"] = json!(id);
            step["name"] = json!(format!("Step {i}"));
            step["position"] = json!([i + 1, 0]);
            nodes.push(step);
            edges.push(json!({ "from": from, "port": "out", "to": id }));
            from = id;
        }
        let saved = self.hub_call(json!({ "cmd": "save_connector", "connector": {
            "name": name, "enabled": false, "trigger": watch,
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
            "columns": { "picked": "In Progress", "coding": "", "review": "" },
            "graph": { "nodes": nodes, "edges": edges },
        } }));
        let id = saved["id"].as_str().unwrap().to_owned();
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot }).unwrap();
        self.turn_on(&id, true);
        id
    }

    /// What the graph bot's Linear poller finds for issue `ENG-n`.
    fn graph_labeled(&self, id: &str, n: u32) -> Event {
        let input = pinned_core::connectors::TriggerInput::Issue {
            id: format!("eng-{n}"),
            identifier: format!("ENG-{n}"),
            title: format!("Fix page {n}"),
            description: None,
            url: format!("https://linear.example.test/acme/issue/ENG-{n}"),
        };
        Event::Graph { connector: id.to_owned(), subject: super::issue(&format!("eng-{n}")), input, at: "2026-11-16T10:00:00Z".into() }
    }

    /// The ids of the team's live runs of bot `id`.
    fn live_of(&self, id: &str) -> Vec<String> {
        let runs = self.call(&self.bob, TeamOp::Runs).unwrap();
        runs.as_array().unwrap().iter().filter(|r| r["workflow"] == id && r["state"] == "working").map(|r| r["id"].as_str().unwrap().to_owned()).collect()
    }

    fn stop(&self, run: &str) {
        self.call(&self.ana, serde_json::from_value(json!({ "op": "stop_run", "run": run })).unwrap()).unwrap();
    }

    /// A Slack mention bot of channel `C0ACME001`; bob saved it and ana turned it on.
    fn slack_bot(&self, name: &str) -> String {
        let saved = self.hub_call(json!({ "cmd": "save_connector", "connector": {
            "name": name, "enabled": false, "trigger": { "kind": "slack-mention", "channel": "C0ACME001" },
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
        } }));
        let id = saved["id"].as_str().unwrap().to_owned();
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot }).unwrap();
        self.turn_on(&id, true);
        id
    }

    /// The event the team's Slack poller finds for a mention that starts thread `thread`.
    fn mentioned(&self, id: &str, thread: &str) -> Event {
        Event::Run(Run {
            cwd: self.folder.clone(),
            repo: None,
            args: vec!["--name".into(), format!("Thread {thread}"), "Answer the thread.".into()],
            connector: id.to_owned(),
            subject: Subject::SlackThread { channel: "C0ACME001".into(), thread: thread.into() },
            mentioned: vec![],
            account: None,
            runs_on: RunsOn::FreshBox(BoxStep { size: Some(BoxSize::Small) }),
            input: "ENG-0: a thread".into(),
        })
    }

    /// Hands the team's connectors `work` as a check would, with the hub's own `open`.
    fn handed(&self, work: Work) -> Result<(), String> {
        let hub = self.hub.hub().clone();
        let team = self.team;
        self.hub.runtime.block_on(async move {
            let account = hub.account(team).await?;
            let connectors = hub.connectors(&account).await?;
            let open = |work| {
                let hub = hub.clone();
                async move { hub.open_found(team, work).await }
            };
            connectors.handed(work, &open).await
        })
    }

    /// The prompt the box of thread `thread` was opened with, once one was.
    fn box_prompt(&self, thread: &str) -> Option<String> {
        let subject = serde_json::to_value(Subject::SlackThread { channel: "C0ACME001".into(), thread: thread.into() }).unwrap();
        self.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), self.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            let row = db.query_opt("select spec from boxes where account = $1 and subject = $2", &[&self.team, &subject]).await.unwrap()?;
            let spec: Value = row.get(0);
            spec["args"].as_array()?.last()?.as_str().map(str::to_owned)
        })
    }

    /// What the waiting event of bot `id` keeps: its replies and how many older ones it dropped.
    fn kept(&self, id: &str) -> (Vec<String>, u64) {
        self.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), self.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            let start: Value = db.query_one("select start from team_run_queue where team = $1 and workflow = $2", &[&self.team, &id]).await.unwrap().get(0);
            let replies = start["replies"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_owned()).collect();
            (replies, start["earlier"].as_u64().unwrap())
        })
    }

    /// The event the team's Linear poller finds for label `agent` on issue `ENG-n`.
    fn labeled(&self, id: &str, n: u32) -> Event {
        Event::Run(Run {
            cwd: self.folder.clone(),
            repo: None,
            args: vec!["--name".into(), format!("ENG-{n} Fix page {n}")],
            connector: id.to_owned(),
            subject: super::issue(&format!("eng-{n}")),
            mentioned: vec![],
            account: None,
            runs_on: RunsOn::FreshBox(BoxStep { size: Some(BoxSize::Small) }),
            input: format!("ENG-{n}: Fix page {n}\nhttps://linear.example.test/acme/issue/ENG-{n}\n\n(no description)"),
        })
    }

    /// Hands the team's connectors `event` as a check that found it would, with the hub's own
    /// `open`; answers what the check would report.
    fn found(&self, event: Event) -> Result<(), String> {
        let hub = self.hub.hub().clone();
        let team = self.team;
        self.hub.runtime.block_on(async move {
            let account = hub.account(team).await?;
            let connectors = hub.connectors(&account).await?;
            let open = |work| {
                let hub = hub.clone();
                async move { hub.open_found(team, work).await }
            };
            connectors.found(event, &open).await
        })
    }

    /// The team's runs as a member lists them: (state, started_by) of each, newest first.
    fn runs(&self) -> Vec<(String, String)> {
        let runs = self.call(&self.bob, TeamOp::Runs).unwrap();
        runs.as_array().unwrap().iter().map(|r| (r["state"].as_str().unwrap().to_owned(), r["started_by"].as_str().unwrap().to_owned())).collect()
    }

    fn live(&self) -> usize {
        self.runs().iter().filter(|(state, _)| state == "working").count()
    }

    /// How many events wait in bot `id`'s line.
    fn waiting(&self, id: &str) -> i64 {
        self.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), self.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.query_one("select count(*) from team_run_queue where team = $1 and workflow = $2", &[&self.team, &id]).await.unwrap().get::<_, i64>(0)
        })
    }

    /// The skipped counts of bot `id` as the bot page lists them.
    fn skipped(&self, id: &str) -> Value {
        let bots = self.call(&self.bob, TeamOp::Bots).unwrap();
        bots.as_array().unwrap().iter().find(|b| b["workflow"] == id).unwrap()["skipped"].clone()
    }

    fn outside(&self, n: u32) -> Vec<String> {
        outside(&self.name, n)
    }
}

fn none() -> Vec<String> {
    vec![]
}

#[test]
fn an_admitted_event_asks_the_model_once_and_moves_its_issue_once() {
    let crew = Crew::new("ta_admitted");
    let id = crew.on_bot("Pages");

    assert_eq!(crew.found(crew.labeled(&id, 1)), Ok(()));
    assert_eq!(crew.runs(), vec![("working".to_owned(), "trigger:linear_label".to_owned())]);
    assert_eq!(crew.outside(1), vec!["model".to_owned(), "linear move".to_owned()]);
}

#[test]
fn an_event_queued_behind_a_live_run_reaches_nothing_outside_pinned() {
    let crew = Crew::new("ta_queued");
    let id = crew.on_bot("Pages");
    let _ = crew.found(crew.labeled(&id, 1));

    assert_eq!(crew.found(crew.labeled(&id, 2)), Ok(()));
    assert_eq!(crew.waiting(&id), 1, "it waits in the bot's line");
    assert_eq!(crew.live(), 1);
    assert_eq!(crew.outside(2), none(), "no model call, no Linear move, no Slack reaction or post");
}

#[test]
fn the_21st_waiting_event_drops_the_oldest_and_neither_reaches_outside_pinned() {
    let crew = Crew::new("ta_dropped");
    let id = crew.on_bot("Pages");
    let _ = crew.found(crew.labeled(&id, 1));

    for n in 2..=22 {
        assert_eq!(crew.found(crew.labeled(&id, n)), Ok(()), "event ENG-{n}");
    }
    assert_eq!(crew.waiting(&id), 20);
    assert_eq!(crew.skipped(&id), json!([{ "reason": "queue_full", "count": 1 }]));
    for n in 2..=22 {
        assert_eq!(crew.outside(n), none(), "ENG-{n}, ENG-2 the dropped one");
    }
}

#[test]
fn an_event_past_the_team_cap_reaches_nothing_outside_pinned() {
    let crew = Crew::new("ta_cap");
    let bots: Vec<String> = (1..=4).map(|n| crew.on_bot(&format!("Bot {n}"))).collect();
    for (n, id) in (1..=3).zip(&bots) {
        let _ = crew.found(crew.labeled(id, n));
    }
    assert_eq!(crew.live(), 3);

    assert_eq!(crew.found(crew.labeled(&bots[3], 4)), Ok(()));
    assert_eq!(crew.waiting(&bots[3]), 1);
    assert_eq!(crew.live(), 3);
    assert_eq!(crew.outside(4), none());
}

#[test]
fn an_event_of_an_off_bot_reaches_nothing_outside_pinned() {
    let crew = Crew::new("ta_off");
    let id = crew.on_bot("Pages");
    crew.turn_on(&id, false);

    assert_eq!(crew.found(crew.labeled(&id, 1)), Ok(()));
    assert_eq!(crew.runs(), vec![]);
    assert_eq!(crew.waiting(&id), 0, "never queued");
    assert_eq!(crew.skipped(&id), json!([{ "reason": "off", "count": 1 }]));
    assert_eq!(crew.outside(1), none());
}

#[test]
fn a_waiting_graph_event_runs_every_step_once_when_its_turn_comes() {
    let crew = Crew::new("ta_graph");
    let id = crew.graph_bot("Triage");

    assert_eq!(crew.found(crew.graph_labeled(&id, 1)), Ok(()));
    assert_eq!(crew.outside(1), vec!["linear move".to_owned(), "model".to_owned()], "the first event's graph ran");
    assert_eq!(crew.live_of(&id).len(), 1);

    assert_eq!(crew.found(crew.graph_labeled(&id, 2)), Ok(()));
    assert_eq!(crew.waiting(&id), 1, "the raw event waits");
    assert_eq!(crew.outside(2), none(), "no step of the waiting event ran");

    let first = crew.live_of(&id);
    crew.stop(&first[0]);
    wait_for("the waiting event's graph to run", || crew.outside(2).len() >= 2 && crew.live_of(&id).len() == 1);
    // Another check or two pass meanwhile: nothing runs a second time.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(crew.outside(2), vec!["linear move".to_owned(), "model".to_owned()], "each outside step exactly once");
    assert_eq!(crew.waiting(&id), 0);
    assert_eq!(crew.live_of(&id).len(), 1, "its session opened in the place it was granted");
    assert_eq!(crew.outside(1), vec!["linear move".to_owned(), "model".to_owned()], "the first event's steps did not run again");
}

#[test]
fn replies_to_a_waiting_thread_wait_with_it_and_reach_its_run_framed() {
    let crew = Crew::new("ta_replies");
    let id = crew.slack_bot("Footer");
    assert_eq!(crew.found(crew.mentioned(&id, "1792400050.000100")), Ok(()));
    assert_eq!(crew.found(crew.mentioned(&id, "1792400050.000200")), Ok(()));
    assert_eq!(crew.waiting(&id), 1, "the second thread's event waits");

    for n in 1..=25 {
        let ts = format!("1792400060.{n:06}");
        let delivery = Delivery {
            connector: id.clone(),
            channel: "C0ACME001".into(),
            thread: "1792400050.000200".into(),
            messages: vec![format!("[Slack reply from Dina] reply {n}")],
            cursor: ts.clone(),
            mentioned: vec![ts],
        };
        assert_eq!(crew.handed(Work::Delivery(delivery)), Ok(()), "reply {n}");
    }
    let reached: Vec<String> = seen("xoxb-dev-ta_replies").into_iter().map(|s| s.method).filter(|m| m == "reactions.add" || m == "chat.postMessage").collect();
    assert_eq!(reached, Vec::<String>::new(), "no reaction or comment while it waits");
    let (replies, earlier) = crew.kept(&id);
    assert_eq!(replies, (6..=25).map(|n| format!("[Slack reply from Dina] reply {n}")).collect::<Vec<_>>());
    assert_eq!(earlier, 5);

    let first = crew.live_of(&id);
    crew.stop(&first[0]);
    wait_for("the waiting thread's box", || crew.box_prompt("1792400050.000200").is_some());
    let framed: Vec<String> = (6..=25).map(|n| pinned_core::untrusted::frame("a Slack reply in this thread", &format!("[Slack reply from Dina] reply {n}"))).collect();
    assert_eq!(
        crew.box_prompt("1792400050.000200").unwrap(),
        format!("Answer the thread.\n\nReplies the thread got while this run waited to start:\n\n5 earlier replies not shown\n\n{}", framed.join("\n\n"))
    );
}

#[test]
fn another_hub_deciding_holds_the_last_place_and_this_event_waits() {
    let crew = Crew::new("ta_lock");
    let (a, b) = (crew.on_bot("Pages"), crew.on_bot("Docs"));
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    // Another hub on the same database, deciding for the team: it holds the team's lock.
    let other = crew.hub.runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), crew.hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.execute(&format!("select pg_advisory_lock({})", teams::queue::TURN_KEY), &[&crew.team]).await.unwrap();
        db
    });

    std::thread::scope(|scope| {
        let ours = scope.spawn(|| crew.found(crew.labeled(&a, 1)));
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(crew.live(), 0, "this hub waits for the other's decision");
        assert!(!ours.is_finished());
        // The other hub takes the last free place, then lets go.
        crew.hub.runtime.block_on(async {
            other
                .execute("insert into team_runs (id, team, workflow, started_by, state) values (gen_random_uuid(), $1, $2, '\"trigger:linear_label\"', 'working')", &[&crew.team, &b])
                .await
                .unwrap();
            other.execute(&format!("select pg_advisory_unlock({})", teams::queue::TURN_KEY), &[&crew.team]).await.unwrap();
        });
        assert_eq!(ours.join().unwrap(), Ok(()));
    });
    assert_eq!(crew.live(), 1, "exactly one run: the other hub's");
    assert_eq!(crew.waiting(&a), 1, "this event waits");
    assert_eq!(crew.outside(1), none());
}
