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
/// The tests whose model call about ENG-1 gets no answer while they are listed here, as a model
/// that is still thinking when its hub stops.
pub(crate) static THINKING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The tests whose Linear polls get no answer while they are listed here, so a check of the
/// team's triggers claims no granted event meanwhile.
pub(crate) static POLLS_HELD: Mutex<Vec<String>> = Mutex::new(Vec::new());

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
    CALLS.lock().unwrap().push(Call { service, key: key.clone(), path: path.clone(), body: body.clone() });
    while service == "linear" && body.contains("issues(") && POLLS_HELD.lock().unwrap().iter().any(|t| key.ends_with(t.as_str())) {
        std::thread::sleep(Duration::from_millis(50));
    }
    while service == "model" && body.contains("ENG-1:") && THINKING.lock().unwrap().iter().any(|t| key.ends_with(t.as_str())) {
        std::thread::sleep(Duration::from_millis(50));
    }
    // A model call about ENG-1 that stops its run (`ta_rel_stop`) or fails (`ta_rel_model`).
    let (status, special) = match (service, path.as_str()) {
        ("model", "/v1/chat/completions") if body.contains("ENG-1:") && key.ends_with("ta_rel_stop") => ("200 OK", Some(json!({ "choices": [{ "message": { "content": "{\"kind\":\"stop\"}" } }] }))),
        ("model", "/v1/chat/completions") if body.contains("ENG-1:") && key.ends_with("ta_cnt_stop") => ("200 OK", Some(json!({ "choices": [{ "message": { "content": "{\"kind\":\"stop\"}" } }] }))),
        ("model", "/v1/chat/completions") if body.contains("ENG-1:") && key.ends_with("ta_rel_model") => ("500 Internal Server Error", Some(json!({ "error": { "message": "the model is down" } }))),
        _ => ("200 OK", None),
    };
    let answer = special.unwrap_or_else(|| match (service, path.as_str()) {
        ("linear", _) if body.contains("viewer") => json!({ "data": { "viewer": { "name": "Acme bot", "email": "", "organization": { "name": "Acme" } } } }),
        ("linear", _) if body.contains("issues(") => json!({ "data": { "issues": { "nodes": [] } } }),
        ("linear", _) if body.contains("issueUpdate") => json!({ "data": { "issueUpdate": { "success": true } } }),
        ("linear", _) if body.contains("issue(id") => json!({ "data": { "issue": { "identifier": "ENG", "team": { "name": "Eng", "states": { "nodes": [{ "id": "state-1", "name": "In Progress" }] } } } } }),
        ("linear", _) => json!({ "errors": [{ "message": "the stand-in knows no such query" }] }),
        (_, p) if p.starts_with("/v1/models/") => json!({ "id": "acme/sorter", "type": "language" }),
        (_, "/v1/chat/completions") => json!({ "choices": [{ "message": { "content": "{\"kind\":\"bug\"}" } }] }),
        _ => json!({}),
    });
    let answer = answer.to_string();
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len());
}

/// The bodies of the model questions asked with keys of test `name`.
pub(crate) fn model_bodies(name: &str) -> Vec<String> {
    CALLS.lock().unwrap().iter().filter(|c| c.service == "model" && c.path == "/v1/chat/completions" && c.key.ends_with(name)).map(|c| c.body.clone()).collect()
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
pub(crate) struct Crew {
    pub(crate) hub: TestHub,
    pub(crate) team: Uuid,
    pub(crate) ana: Arc<Account>,
    pub(crate) bob: Arc<Account>,
    folder: PathBuf,
    name: String,
}

impl Crew {
    pub(crate) fn new(name: &str) -> Crew {
        standins();
        let patient = TeamPolicy { escalate_after_secs: 3600, give_up_after_secs: 7200, ..TeamPolicy::default() };
        // A box that fails to boot cools its bot down for an hour; these tests start the next event right after.
        let hub = TestHub::start_with_team_policy(name, Policy { cooldown: Duration::ZERO, ..fast() }, patient);
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

    pub(crate) fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        let op = teams::Op::read(op)?;
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), op))
    }

    /// A Linear label bot whose Ask step asks the model what kind of issue it is, and whose
    /// session moves the issue to "In Progress"; bob saved it and ana turned it on.
    pub(crate) fn on_bot(&self, name: &str) -> String {
        self.ask_bot(name, &[])
    }

    /// `on_bot` whose Ask step ends the run, without a session, on any of the answers `stop_on`.
    fn ask_bot(&self, name: &str, stop_on: &[&str]) -> String {
        let id = save_team_workflow(&self.hub, &self.bob, self.team, json!({
            "name": name, "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
            "columns": { "picked": "In Progress", "coding": "", "review": "" },
            "ask": { "model": "acme/sorter", "instructions": "Sort the issue.", "questions": [{ "name": "kind", "ask": "What kind of issue is it?", "stopOn": stop_on }] },
        }))
        .unwrap();
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot, connector: None }).unwrap();
        self.turn_on(&id, true);
        id
    }

    fn turn_on(&self, id: &str, on: bool) {
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.to_owned(), on }).unwrap();
    }

    /// A Linear label bot whose graph moves the issue to "In Progress", asks the model what kind
    /// of issue it is, then opens its session; bob saved it and ana turned it on.
    pub(crate) fn graph_bot(&self, name: &str) -> String {
        self.graph_of(
            name,
            vec![
                json!({ "type": "stage-picked" }),
                json!({ "type": "classify", "model": "acme/sorter", "instructions": "Sort the issue.", "questions": [{ "name": "kind", "ask": "What kind of issue is it?" }] }),
                json!({ "type": "session", "prompt": "Fix the issue." }),
            ],
        )
    }

    /// A Linear label bot whose graph only moves the issue to "In Progress": it ends before any session.
    fn sessionless_graph_bot(&self, name: &str) -> String {
        self.graph_of(name, vec![json!({ "type": "stage-picked" })])
    }

    pub(crate) fn graph_of(&self, name: &str, steps: Vec<Value>) -> String {
        let watch = json!({ "kind": "linear-label", "label": "agent" });
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
        let id = save_team_workflow(&self.hub, &self.bob, self.team, json!({
            "name": name, "enabled": false, "trigger": watch,
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
            "columns": { "picked": "In Progress", "coding": "", "review": "" },
            "graph": { "nodes": nodes, "edges": edges },
        }))
        .unwrap();
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot, connector: None }).unwrap();
        self.turn_on(&id, true);
        id
    }

    /// What the graph bot's Linear poller finds for issue `ENG-n`.
    pub(crate) fn graph_labeled(&self, id: &str, n: u32) -> Event {
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
    pub(crate) fn live_of(&self, id: &str) -> Vec<String> {
        let runs = self.call(&self.bob, TeamOp::Runs).unwrap();
        runs.as_array().unwrap().iter().filter(|r| r["workflow"] == id && r["state"] == "working").map(|r| r["id"].as_str().unwrap().to_owned()).collect()
    }

    pub(crate) fn stop(&self, run: &str) {
        self.call(&self.ana, serde_json::from_value(json!({ "op": "stop_run", "run": run })).unwrap()).unwrap();
    }

    /// A Slack mention bot of channel `C0ACME001`; bob saved it and ana turned it on.
    fn slack_bot(&self, name: &str) -> String {
        let id = save_team_workflow(&self.hub, &self.bob, self.team, json!({
            "name": name, "enabled": false, "trigger": { "kind": "slack-mention", "channel": "C0ACME001" },
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
        }))
        .unwrap();
        let bot = Bot { workflow: id.clone(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot, connector: None }).unwrap();
        self.turn_on(&id, true);
        id
    }

    /// The event the team's Slack poller finds for a mention that starts thread `thread`.
    fn mentioned(&self, id: &str, thread: &str) -> Event {
        Event::Run(Run {
            cwd: self.folder.clone(),
            repo: None,
            args: vec!["--name".into(), format!("Thread {thread}"), "Answer the thread.\n\nSlack thread in #acme, oldest first:\nENG-0: a thread".into()],
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
    pub(crate) fn labeled(&self, id: &str, n: u32) -> Event {
        Event::Run(Run {
            cwd: self.folder.clone(),
            repo: None,
            args: vec!["--name".into(), format!("ENG-{n} Fix page {n}"), format!("Fix it.\n\nLinear issue ENG-{n}: Fix page {n}\nhttps://linear.example.test/acme/issue/ENG-{n}\n\n(no description)")],
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
    pub(crate) fn found(&self, event: Event) -> Result<(), String> {
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
    pub(crate) fn waiting(&self, id: &str) -> i64 {
        self.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), self.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.query_one("select count(*) from team_run_queue where team = $1 and workflow = $2", &[&self.team, &id]).await.unwrap().get::<_, i64>(0)
        })
    }

    /// The state of every run row of the team that was reserved and whose session never opened.
    fn unopened(&self) -> Vec<String> {
        self.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), self.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            let rows = db.query("select state from team_runs where team = $1 and reserved order by state", &[&self.team]).await.unwrap();
            rows.iter().map(|r| r.get::<_, String>(0)).collect()
        })
    }

    /// The skipped counts of bot `id` as the bot page lists them.
    pub(crate) fn skipped(&self, id: &str) -> Value {
        let bots = self.call(&self.bob, TeamOp::Bots).unwrap();
        bots.as_array().unwrap().iter().find(|b| b["workflow"] == id).unwrap()["skipped"].clone()
    }

    pub(crate) fn outside(&self, n: u32) -> Vec<String> {
        outside(&self.name, n)
    }
}

pub(crate) fn none() -> Vec<String> {
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
        format!("Answer the thread.\n\nSlack thread in #acme, oldest first:\n{}\n\nReplies the thread got while this run waited to start:\n\n5 earlier replies not shown\n\n{}", pinned_core::untrusted::frame("a Slack thread", "ENG-0: a thread"), framed.join("\n\n"))
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

#[test]
fn a_waiting_run_whose_start_fails_is_counted_on_its_bot() {
    let crew = Crew::new("ta_start_failed");
    let id = crew.on_bot("Pages");
    assert_eq!(crew.found(crew.labeled(&id, 1)), Ok(()));
    let reply = crew.call(&crew.bob, serde_json::from_value(json!({ "op": "run_now", "workflow": id })).unwrap()).unwrap();
    assert_eq!(reply["waiting"], json!(true), "bob's run waits behind the first");
    assert_eq!(crew.skipped(&id), Value::Null);

    crew.hub.boxes.state.lock().unwrap().failing = Some(Exec { exit_code: 22, output: "Downloading pinned-server\n".into(), error: "curl: (22) The requested URL returned error: 404\n".into() });
    let first = crew.live_of(&id);
    crew.stop(&first[0]);
    wait_for("the failed start to be counted", || !crew.skipped(&id).is_null());
    assert_eq!(crew.skipped(&id), json!([{ "reason": "start_failed", "count": 1 }]));
    assert_eq!(crew.waiting(&id), 0, "it is dropped, not kept");
    assert_eq!(crew.live(), 0, "no run started");
}

/// A team that may run one at a time, whose only place a run of bot `holder` takes; two events of
/// `bot` (ENG-1, ENG-2) wait behind it. Stopping the holder gives ENG-1 the place first.
fn two_wait_behind(crew: &Crew, holder: &str, bot: &str, graph: bool) {
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    assert_eq!(crew.found(crew.labeled(holder, 9)), Ok(()));
    for n in 1..=2 {
        let event = if graph { crew.graph_labeled(bot, n) } else { crew.labeled(bot, n) };
        assert_eq!(crew.found(event), Ok(()), "ENG-{n}");
    }
    assert_eq!((crew.live(), crew.waiting(bot)), (1, 2), "the holder's run is the only one, both events wait");
    assert_eq!((crew.outside(1), crew.outside(2)), (none(), none()), "nothing reached outside pinned while they wait");
    let held = crew.live_of(holder);
    crew.stop(&held[0]);
}

/// What a freed place does after ENG-1 was admitted and its run ended or never opened: its
/// reserved row is gone, and ENG-2 got the place.
fn assert_place_freed_for_eng_2(crew: &Crew, bot: &str, unopened: &[&str]) {
    wait_for("ENG-2 to take the place", || crew.live_of(bot).len() == 1);
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(crew.unopened(), unopened, "ENG-1's reserved row ended, none is left holding the place");
    assert_eq!(crew.live(), 1, "the place ENG-1 held is back: only ENG-2's run lives");
    assert_eq!(crew.waiting(bot), 0);
    assert_eq!(crew.outside(2), vec!["model".to_owned(), "linear move".to_owned()], "ENG-2 asked once and moved its issue once");
}

#[test]
fn an_ask_step_that_stops_the_run_frees_its_reserved_place() {
    let crew = Crew::new("ta_rel_stop");
    let (holder, pages) = (crew.on_bot("Docs"), crew.ask_bot("Pages", &["stop"]));
    two_wait_behind(&crew, &holder, &pages, false);

    assert_place_freed_for_eng_2(&crew, &pages, &["done"]);
    assert_eq!(crew.outside(1), vec!["model".to_owned()], "ENG-1 asked the model, was told to stop, and its issue stayed where it was");
}

#[test]
fn a_failed_model_call_frees_its_reserved_place() {
    let crew = Crew::new("ta_rel_model");
    let (holder, pages) = (crew.on_bot("Docs"), crew.on_bot("Pages"));
    two_wait_behind(&crew, &holder, &pages, false);

    assert_place_freed_for_eng_2(&crew, &pages, &["done"]);
    assert_eq!(crew.outside(1), vec!["model".to_owned()], "ENG-1's model call failed and its issue stayed where it was");
}

#[test]
fn a_session_that_does_not_open_frees_its_reserved_place() {
    let crew = Crew::new("ta_rel_open");
    let (holder, pages) = (crew.on_bot("Docs"), crew.on_bot("Pages"));
    two_wait_behind(&crew, &holder, &pages, false);
    crew.hub.boxes.state.lock().unwrap().failing = Some(Exec { exit_code: 22, output: "Downloading pinned-server\n".into(), error: "curl: (22) The requested URL returned error: 404\n".into() });

    assert_place_freed_for_eng_2(&crew, &pages, &["failed"]);
    assert_eq!(crew.outside(1), vec!["model".to_owned()], "ENG-1 asked the model, its box did not boot, and its issue stayed where it was");
}

#[test]
fn a_graph_that_ends_before_any_session_frees_its_reserved_place() {
    let crew = Crew::new("ta_rel_graph");
    let (holder, pages) = (crew.on_bot("Docs"), crew.sessionless_graph_bot("Triage"));
    two_wait_behind(&crew, &holder, &pages, true);

    wait_for("both graphs to run", || crew.outside(1).len() == 1 && crew.outside(2).len() == 1);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(crew.unopened(), vec!["done", "done"], "both reserved rows ended, none is left holding a place");
    assert_eq!((crew.live(), crew.waiting(&pages)), (0, 0), "both graphs ended, neither holds a place");
    assert_eq!((crew.outside(1), crew.outside(2)), (vec!["linear move".to_owned()], vec!["linear move".to_owned()]), "each graph moved its issue once");
}

/// A lock the test holds on the team's waiting rows: a grant (an update) goes through, a claim (a
/// delete) waits, as a poller that has not claimed yet. It lives on a runtime of its own, so a
/// stopped hub does not drop it. A hub's start changes the tables, so it waits for the release.
struct ClaimGate {
    runtime: tokio::runtime::Runtime,
    db: tokio_postgres::Client,
    team: Uuid,
}

impl ClaimGate {
    fn hold(crew: &Crew) -> ClaimGate {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let url = format!("{}/{}", postgres(), crew.hub.database);
        let db = runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.batch_execute("begin").await.unwrap();
            db.query("select id from team_run_queue where team = $1 for key share", &[&crew.team]).await.unwrap();
            db
        });
        ClaimGate { runtime, db, team: crew.team }
    }

    /// How many of the team's waiting rows are granted.
    fn granted(&self) -> i64 {
        self.runtime.block_on(self.db.query_one("select count(*) from team_run_queue where team = $1 and granted_at is not null", &[&self.team])).unwrap().get(0)
    }

    /// The server processes of the claims that wait on this lock.
    fn claims(&self) -> Vec<i32> {
        // Inside a transaction, Postgres keeps the first read of its activity view unless told not to.
        self.runtime.block_on(self.db.query("select pg_stat_clear_snapshot()", &[])).unwrap();
        let rows = self.runtime.block_on(self.db.query(
            "select pid from pg_stat_activity where datname = current_database() and wait_event_type = 'Lock' and query like '% team_run_queue where team = $1 and id = $2%'",
            &[],
        ));
        rows.unwrap().iter().map(|r| r.get(0)).collect()
    }

    /// Ends the server process of a stopped hub's claim before it took the row, as Postgres does
    /// when it finds that hub gone.
    fn end(&self, claims: &[i32]) {
        for pid in claims {
            self.runtime.block_on(self.db.query("select pg_terminate_backend($1)", &[pid])).unwrap();
        }
        wait_for("the stopped hub's claim to end", || self.claims().is_empty());
    }

    fn release(self) {
        self.runtime.block_on(self.db.batch_execute("commit")).unwrap();
    }
}

impl Crew {
    /// How many boxes were created for sessions so far.
    pub(crate) fn boxes_created(&self) -> usize {
        self.hub.boxes.state.lock().unwrap().created.len()
    }

    /// When each reserved run of bot `id` got its place (`reserved_at`), as `set` or `null`.
    pub(crate) fn reserved_at(&self, id: &str) -> Vec<String> {
        self.hub.runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), self.hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            let rows = db.query("select case when reserved_at is null then 'null' else 'set' end from team_runs where team = $1 and workflow = $2 and reserved", &[&self.team, &id]).await.unwrap();
            rows.iter().map(|r| r.get::<_, String>(0)).collect()
        })
    }
}

/// A team that may run one at a time: bot `holder`'s run takes the place, the graph bot's ENG-1
/// waits behind it, and the test holds every claim (`ClaimGate`). Stopping the holder grants ENG-1.
fn one_granted(crew: &Crew, holder: &str, bot: &str) -> ClaimGate {
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    assert_eq!(crew.found(crew.labeled(holder, 9)), Ok(()));
    assert_eq!(crew.found(crew.graph_labeled(bot, 1)), Ok(()));
    assert_eq!((crew.live(), crew.waiting(bot)), (1, 1), "the holder's run is the only one, ENG-1 waits");
    let gate = ClaimGate::hold(crew);
    let held = crew.live_of(holder);
    crew.stop(&held[0]);
    wait_for("ENG-1's grant and this hub's claim of it", || gate.granted() == 1 && gate.claims().len() == 1);
    assert_eq!(crew.outside(1), none(), "nothing of ENG-1 ran before its claim");
    assert_eq!(crew.reserved_at(bot), vec!["set".to_owned()], "its place is held in Postgres");
    gate
}

/// ENG-1 of graph bot `bot` ran once, each outside step once, in one new box, and nothing waits.
fn ran_once(crew: &Crew, bot: &str, boxes_before: usize) {
    wait_for("ENG-1's graph to run", || crew.outside(1).len() >= 2 && crew.reserved_at(bot).is_empty());
    // A few checks pass meanwhile: nothing runs a second time.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(crew.outside(1), vec!["linear move".to_owned(), "model".to_owned()], "each outside step exactly once");
    assert_eq!(crew.boxes_created() - boxes_before, 1, "one session opened, in one box");
    assert_eq!(crew.live_of(bot).len(), 1, "one run of ENG-1 lives");
    assert_eq!(crew.reserved_at(bot), none(), "its session opened in the place it was granted");
    assert_eq!(crew.waiting(bot), 0);
}

#[test]
fn a_granted_event_survives_a_hub_restart_and_runs_once() {
    let mut crew = Crew::new("ta_dur_restart");
    let (holder, triage) = (crew.on_bot("Docs"), crew.graph_bot("Triage"));
    let gate = one_granted(&crew, &holder, &triage);
    let boxes_before = crew.boxes_created();
    let stopped = gate.claims();

    let addr = crew.hub.stop();
    gate.end(&stopped);
    assert_eq!(gate.granted(), 1, "the stopped hub never claimed it");
    gate.release();
    crew.hub.serve(&addr);
    ran_once(&crew, &triage, boxes_before);
}

#[test]
fn two_hubs_claim_a_granted_event_once() {
    let mut crew = Crew::new("ta_dur_twin");
    let (holder, triage) = (crew.on_bot("Docs"), crew.graph_bot("Triage"));
    let gate = one_granted(&crew, &holder, &triage);
    let boxes_before = crew.boxes_created();
    let stopped = gate.claims();
    let addr = crew.hub.stop();
    gate.end(&stopped);
    gate.release();

    // Two hubs start on the same database at once, and each one's first check finds the grant.
    let (twin, _running) = crew.hub.twin();
    crew.hub.serve(&addr);
    // Only one claim gets the event. Whether its box then reaches the hub that opened it is not
    // what this test checks: the claim is, by every step outside pinned and by the boxes asked for.
    wait_for("ENG-1's graph to run", || crew.outside(1).len() >= 2 && crew.boxes_created() > boxes_before);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(crew.outside(1), vec!["linear move".to_owned(), "model".to_owned()], "each outside step exactly once");
    assert_eq!(crew.boxes_created() - boxes_before, 1, "one session asked for, in one box");
    assert_eq!(crew.waiting(&triage), 0, "the claim took the row");
    twin.shutdown_background();
}

#[test]
fn a_claimed_event_is_not_run_again_after_a_restart() {
    let mut crew = Crew::new("ta_dur_claimed");
    THINKING.lock().unwrap().push("ta_dur_claimed".into());
    let (holder, pages) = (crew.on_bot("Docs"), crew.on_bot("Pages"));
    admin_on(&crew.hub.database, &format!("update teams set max_live_runs = 1 where id = '{}'", crew.team));
    assert_eq!(crew.found(crew.labeled(&holder, 9)), Ok(()));
    assert_eq!(crew.found(crew.labeled(&pages, 1)), Ok(()));
    assert_eq!(crew.waiting(&pages), 1, "ENG-1 waits");
    let boxes_before = crew.boxes_created();
    let held = crew.live_of(&holder);
    crew.stop(&held[0]);
    // Claimed: its Ask step asked the model, which is still thinking when the hub stops.
    wait_for("ENG-1's claim and its model call", || crew.outside(1) == vec!["model".to_owned()]);
    assert_eq!(crew.waiting(&pages), 0, "the claim took its row");

    crew.hub.restart();
    THINKING.lock().unwrap().retain(|t| t != "ta_dur_claimed");
    // A few checks of the new hub pass.
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(crew.outside(1), vec!["model".to_owned()], "no second model call and no Linear move");
    assert_eq!(crew.boxes_created() - boxes_before, 0, "no session opened");
    assert_eq!(crew.reserved_at(&pages), vec!["set".to_owned()], "its reserved run is left for the backstop");
    assert_eq!(crew.live_of(&pages).len(), 1);
}

#[test]
fn a_granted_event_whose_box_does_not_boot_is_counted_as_start_failed() {
    let crew = Crew::new("ta_cnt_open");
    let (holder, pages) = (crew.on_bot("Docs"), crew.on_bot("Pages"));
    two_wait_behind(&crew, &holder, &pages, false);
    crew.hub.boxes.state.lock().unwrap().failing = Some(Exec { exit_code: 22, output: "Downloading pinned-server\n".into(), error: "curl: (22) The requested URL returned error: 404\n".into() });

    assert_place_freed_for_eng_2(&crew, &pages, &["failed"]);
    assert_eq!(crew.skipped(&pages), json!([{ "reason": "start_failed", "count": 1 }]), "ENG-1's box did not boot: counted once");
    assert_eq!(crew.skipped(&holder), Value::Null);
}

#[test]
fn a_granted_event_its_ask_step_stops_is_not_counted() {
    let crew = Crew::new("ta_cnt_stop");
    let (holder, pages) = (crew.on_bot("Docs"), crew.ask_bot("Pages", &["stop"]));
    two_wait_behind(&crew, &holder, &pages, false);

    assert_place_freed_for_eng_2(&crew, &pages, &["done"]);
    assert_eq!(crew.outside(1), vec!["model".to_owned()], "ENG-1's Ask step stopped it");
    assert_eq!(crew.skipped(&pages), Value::Null, "a stop by its Ask step is not a failed start");
}
