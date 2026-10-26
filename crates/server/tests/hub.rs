use base64::Engine;
use pinned_core::account::{self, Credentials};
use pinned_core::agent::{self, AgentEvent, AgentStatus, Stage, Subject};
use pinned_core::checkpoint;
use pinned_core::connectors::{Delivery, Run, Work};
use pinned_core::link::{self, Link};
use pinned_core::machine::{Launch, Machine};
use pinned_core::sync;
use pinned_core::wire::HubCall;
use serde_json::{json, Value};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const POSTGRES: &str = "postgres://pinned@127.0.0.1:5499";
const WAIT: Duration = Duration::from_secs(10);

struct TestHub {
    runtime: tokio::runtime::Runtime,
    running: Option<pinned_server::Running>,
    url: String,
    database: String,
    dir: PathBuf,
}

impl TestHub {
    fn start(name: &str) -> TestHub {
        TestHub::start_with(name, true)
    }

    fn start_with(name: &str, open_signup: bool) -> TestHub {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let database = format!("pinned_test_{name}_{}", std::process::id());
        admin(&format!("drop database if exists {database} with (force)"));
        admin(&format!("create database {database}"));
        let dir = temp(&format!("hub-{name}"));
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::write(dir.join("ui/index.html"), "<title>pinned</title>").unwrap();
        std::fs::create_dir_all(dir.join("downloads")).unwrap();
        std::fs::write(dir.join("downloads/pinned-server-x86_64-unknown-linux-musl"), "ELF").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let running = runtime
            .block_on(pinned_server::start(pinned_server::Config {
                db: format!("{POSTGRES}/{database}"),
                listen: "127.0.0.1:0".into(),
                ui: dir.join("ui"),
                downloads: dir.join("downloads"),
                public_url: None,
                key_file: dir.join("hub.key"),
                open_signup,
                runs: pinned_core::runs::Timing { check: Duration::from_secs(1), grace: 3 },
            }))
            .unwrap();
        let url = format!("http://{}", running.addr);
        TestHub { runtime, running: Some(running), url, database, dir }
    }

    fn hub(&self) -> &Arc<pinned_server::hub::Hub> {
        &self.running.as_ref().unwrap().hub
    }
}

impl Drop for TestHub {
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            running.server.abort();
        }
        admin(&format!("drop database if exists {} with (force)", self.database));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn admin(sql: &str) {
    admin_on("pinned", sql);
}

fn admin_on(database: &str, sql: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{POSTGRES}/{database}"), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        client.batch_execute(sql).await.unwrap();
    });
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pinned-server-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct Client {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    frames: Vec<Value>,
    next: u64,
    closed: Option<u16>,
}

impl Client {
    fn connect(hub: &str, token: &str) -> Client {
        let (mut ws, _) = tungstenite::connect(format!("{}/ws/client", hub.replace("http://", "ws://"))).unwrap();
        if let MaybeTlsStream::Plain(s) = ws.get_mut() {
            s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        }
        ws.send(Message::text(json!({ "type": "auth", "token": token }).to_string())).unwrap();
        Client { ws, frames: vec![], next: 1, closed: None }
    }

    fn pump(&mut self) -> bool {
        loop {
            match self.ws.read() {
                Ok(Message::Text(text)) => self.frames.push(serde_json::from_str(&text).unwrap()),
                Ok(Message::Close(close)) => {
                    self.closed = Some(close.map_or(0, |c| c.code.into()));
                    return false;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return true,
                Err(_) => return false,
            }
        }
    }

    fn wait(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        loop {
            if let Some(found) = self.frames.iter().find(|f| pred(f)) {
                return found.clone();
            }
            assert!(self.pump(), "the socket closed while waiting for {what}");
            assert!(start.elapsed() < WAIT, "timed out waiting for {what}; got {:#?}", self.frames);
        }
    }

    fn send(&mut self, frame: Value) {
        self.ws.send(Message::text(frame.to_string())).unwrap();
    }

    fn request(&mut self, mut frame: Value) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        frame["id"] = json!(id);
        self.send(frame);
        let reply = self.wait(&format!("reply {id}"), |f| f["type"] == "reply" && f["id"] == id);
        match reply.get("err") {
            Some(e) => Err(e.as_str().unwrap().to_owned()),
            None => Ok(reply["ok"].clone()),
        }
    }

    fn hub(&mut self, call: Value) -> Result<Value, String> {
        self.request(json!({ "type": "hub", "call": call }))
    }

    fn call(&mut self, machine: &str, call: Value) -> Result<Value, String> {
        self.request(json!({ "type": "call", "machine": machine, "call": call }))
    }

    fn output(&mut self, machine: &str, tab: &str) -> String {
        self.pump();
        let bytes: Vec<u8> = self
            .frames
            .iter()
            .filter(|f| f["type"] == "event" && f["machine"] == machine && f["event"]["kind"] == "output" && f["event"]["tab"] == tab)
            .flat_map(|f| base64::engine::general_purpose::STANDARD.decode(f["event"]["data"].as_str().unwrap()).unwrap())
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn wait_output(&mut self, machine: &str, tab: &str, text: &str) -> String {
        let start = Instant::now();
        loop {
            let output = self.output(machine, tab);
            if output.contains(text) {
                return output;
            }
            assert!(start.elapsed() < WAIT, "timed out waiting for {text:?} in {output:?}");
        }
    }

    fn machines(&mut self) -> Vec<(String, bool)> {
        self.pump();
        let latest = self.frames.iter().rev().find(|f| f["type"] == "machines" || f["type"] == "hello").unwrap();
        latest["machines"].as_array().unwrap().iter().map(|m| (m["name"].as_str().unwrap().to_owned(), m["online"].as_bool().unwrap())).collect()
    }

    fn wait_machines(&mut self, expected: &[(&str, bool)]) {
        let start = Instant::now();
        let expected: Vec<(String, bool)> = expected.iter().map(|(n, o)| (n.to_string(), *o)).collect();
        while self.machines() != expected {
            assert!(start.elapsed() < WAIT, "timed out waiting for machines {expected:?}; have {:?}", self.machines());
        }
    }
}

struct Node {
    machine: Arc<Machine>,
    link: Option<Link>,
}

impl Node {
    fn start(dir: &Path, creds: Credentials) -> Node {
        let claude = dir.join("claude");
        std::fs::write(&claude, "#!/bin/sh\nexec cat\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let launch = Launch {
            exe: "/opt/pinned/bin/pinned-server".into(),
            claude,
            path_env: None,
            settings: dir.join("settings.json"),
            mcp_config: None,
            plugin_dir: dir.join("plugin"),
            tutorial_helper: dir.join("plugin/tutorial.mjs"),
            sock: dir.join("sock"),
            home: dir.join("home"),
        };
        let machine = Machine::new(launch, Some(dir.to_owned()), Some(dir.to_owned()), |_| {});
        let link = link::spawn(creds, machine.clone(), "0.0.0-test");
        Node { machine, link: Some(link) }
    }

    fn online(&self) -> bool {
        self.link.as_ref().is_some_and(Link::online)
    }

    fn kill(&mut self) {
        self.link.take().unwrap().stop();
        self.machine.quit();
    }

    /// Cuts the node off the hub, as a laptop closing its lid, with its sessions still running.
    fn disconnect(&mut self) {
        self.link.take().unwrap().stop();
    }

    fn reconnect(&mut self, creds: Credentials) {
        self.link = Some(link::spawn(creds, self.machine.clone(), "0.0.0-test"));
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        if let Some(link) = self.link.take() {
            link.stop();
        }
        self.machine.quit();
    }
}

fn pair(hub: &str, client: &mut Client, name: &str, dir: &Path) -> Credentials {
    let code = client.hub(json!({ "cmd": "pair_code" })).unwrap();
    let text = code["code"].as_str().unwrap();
    assert_eq!(code["command"], format!("curl -fsSL {hub}/install.sh | sh -s -- {text}"));
    let creds = account::pair(hub, &text.to_lowercase(), name, "linux").unwrap();
    assert_eq!((creds.hub.as_str(), creds.email.as_str(), creds.user_token.as_str()), (hub, "ana@acme.dev", ""));
    creds.save(dir).unwrap();
    assert_eq!(account::pair(hub, text, name, "linux"), Err("that pairing code is unknown, used or expired".into()));
    Credentials::load(dir).unwrap()
}

#[test]
fn a_client_drives_a_tab_on_a_paired_node_through_the_hub() {
    let hub = TestHub::start("e2e");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "Ana@Acme.dev", "correct horse").unwrap();
    assert_eq!(account.email, "ana@acme.dev");
    let mut client = Client::connect(&url, &token);
    let hello = client.wait("hello", |f| f["type"] == "hello");
    assert_eq!(hello["account"], json!({ "id": account.id, "email": "ana@acme.dev" }));
    assert_eq!((hello["machines"].clone(), hello["tabs"].clone()), (json!([]), json!({})));

    let (dir_a, dir_b) = (temp("node-a"), temp("node-b"));
    let creds_a = pair(&url, &mut client, "node-a", &dir_a);
    let creds_b = pair(&url, &mut client, "node-b", &dir_b);
    client.wait_machines(&[("node-a", false), ("node-b", false)]);
    let _node_a = Node::start(&dir_a, creds_a);
    let mut node_b = Node::start(&dir_b, creds_b.clone());
    client.wait_machines(&[("node-a", true), ("node-b", true)]);
    let listed = client.hub(json!({ "cmd": "machines" })).unwrap();
    let names: Vec<(&str, bool)> = listed.as_array().unwrap().iter().map(|m| (m["name"].as_str().unwrap(), m["online"].as_bool().unwrap())).collect();
    assert_eq!(names, [("node-a", true), ("node-b", true)]);
    let b = creds_b.machine_id.as_str();

    let tab = client.call(b, json!({ "cmd": "open_tab", "path": null, "bypass": false })).unwrap();
    assert_eq!(tab, "tab-1");
    let cwd = client.call(b, json!({ "cmd": "start_claude", "tab": "tab-1", "cols": 80, "rows": 24 })).unwrap();
    assert_eq!(cwd, dir_b.display().to_string());
    client.wait("node-b's running tab", |f| {
        f["type"] == "event" && f["machine"] == b && f["event"]["kind"] == "tabs" && f["event"]["tabs"][0]["running"] == true
    });
    client.send(json!({ "type": "attach", "machine": b, "tab": "tab-1" }));
    let typed: String = (0..300).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    for key in typed.chars().chain(['\r']) {
        client.send(json!({ "type": "call", "id": 1000, "machine": b, "call": { "cmd": "write_input", "tab": "tab-1", "data": key.to_string() } }));
    }
    let screen = format!("{typed}\r\n{typed}\r\n");
    let echoed = client.wait_output(b, "tab-1", "\r\n");
    assert_eq!(echoed.get(..typed.len()), Some(typed.as_str()), "keys echo in the order they were typed");
    assert_eq!(client.wait_output(b, "tab-1", &screen), screen);

    let mut late = Client::connect(&url, &token);
    let hello = late.wait("hello", |f| f["type"] == "hello");
    assert_eq!(hello["tabs"][b][0]["id"], "tab-1");
    late.send(json!({ "type": "attach", "machine": b, "tab": "tab-1" }));
    assert_eq!(late.wait_output(b, "tab-1", &screen), screen, "a late client gets the screen");
    client.call(b, json!({ "cmd": "write_input", "tab": "tab-1", "data": "again\r" })).unwrap();
    late.wait_output(b, "tab-1", "again\r\nagain\r\n");
    assert_eq!(client.wait_output(b, "tab-1", "again\r\nagain\r\n"), format!("{screen}again\r\nagain\r\n"), "the first client never sees the screen twice");

    let refused = client.call(b, json!({ "cmd": "format_disk" })).unwrap_err();
    let says = format!("this hub runs pinned {}, which can't answer that: unknown variant `format_disk`, expected one of `tabs`, `open_tab`", env!("CARGO_PKG_VERSION"));
    assert!(refused.starts_with(&says), "{refused}");
    assert!(client.hub(json!({ "cmd": "self_destruct" })).unwrap_err().contains("unknown variant `self_destruct`"));
    assert_eq!(client.call("not-a-machine", json!({ "cmd": "tabs" })), Err("this account has no machine not-a-machine".into()));

    client.call(b, json!({ "cmd": "close_tab", "tab": "tab-1" })).unwrap();
    client.wait("node-b without tabs", |f| f["type"] == "event" && f["machine"] == b && f["event"] == json!({ "kind": "tabs", "tabs": [] }));

    node_b.kill();
    client.wait_machines(&[("node-a", true), ("node-b", false)]);
    assert_eq!(client.call(b, json!({ "cmd": "tabs" })), Err("node-b is offline".into()));
    client.hub(json!({ "cmd": "rename_machine", "id": b, "name": "vps" })).unwrap();
    client.wait_machines(&[("node-a", true), ("vps", false)]);
    client.hub(json!({ "cmd": "remove_machine", "id": b })).unwrap();
    client.wait_machines(&[("node-a", true)]);
}

#[test]
fn the_hub_serves_the_install_script_binaries_and_the_web_app() {
    let hub = TestHub::start("files");
    let get = |path: &str| {
        let response = reqwest::blocking::get(format!("{}{path}", hub.url)).unwrap();
        (response.status().as_u16(), response.text().unwrap())
    };
    let (status, script) = get("/install.sh");
    assert_eq!(status, 200);
    assert!(script.starts_with("#!/bin/sh\n"));
    assert!(script.contains(&format!("HUB='{}'", hub.url)));
    assert_eq!(get("/download/pinned-server-x86_64-unknown-linux-musl"), (200, "ELF".into()));
    assert_eq!(get("/download/pinned-server-aarch64-apple-darwin").0, 404);
    assert_eq!(get("/download/..%2Fui%2Findex.html").0, 404);
    assert_eq!(get("/"), (200, "<title>pinned</title>".into()));
    assert_eq!(get("/api/version"), (200, env!("CARGO_PKG_VERSION").into()));
    let page = reqwest::blocking::get(format!("{}/index.html", hub.url)).unwrap();
    assert_eq!(page.headers()["cache-control"], "no-cache", "a browser revalidates the web app after each deploy");
}

#[test]
fn a_connector_run_goes_to_its_machine_else_another_online_one() {
    let hub = TestHub::start("dispatch");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let register = |name: &str, dir: &Path| {
        let (machine, machine_token) = account::register_machine(&url, &token, name, "macos").unwrap();
        assert_eq!((machine.name.as_str(), machine.online), (name, false));
        Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token }
            .save(dir)
            .unwrap();
        Credentials::load(dir).unwrap()
    };
    let (dir_a, dir_b, repo) = (temp("dispatch-a"), temp("dispatch-b"), temp("dispatch-repo"));
    let (creds_a, creds_b) = (register("mac-mini", &dir_a), register("vps", &dir_b));
    let (mac_mini, vps) = (Some(creds_a.machine_id.clone()), Some(creds_b.machine_id.clone()));
    let connector = client
        .hub(json!({ "cmd": "save_connector", "connector": {
            "name": "Bugs", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": repo, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": creds_a.machine_id,
        } }))
        .unwrap();
    let id = connector["id"].as_str().unwrap().to_owned();
    client.wait("the saved connector", |f| f["type"] == "settings" && f["settings"]["connectors"][0]["id"] == id);
    let account_id = uuid::Uuid::parse_str(&account.id).unwrap();
    let run = |subject: Subject| Work::Run(Run { cwd: repo.clone(), repo: None, args: vec![], connector: id.clone(), subject, mentioned: vec![], title: "ENG-1".into(), worktree: "eng-1".into(), resume: None });
    let open = |work| hub.runtime.block_on(hub.hub().open(account_id, work));
    let connector_tabs = |node: &Node| node.machine.tabs().into_iter().filter(|t| t.connector.as_deref() == Some(&id) && t.running).count();

    let mut node_b = Node::start(&dir_b, creds_b);
    wait_for("vps online", || node_b.online());
    client.wait_machines(&[("mac-mini", false), ("vps", true)]);
    assert_eq!(open(run(Subject::LinearIssue { issue: "uuid-1".into() })), vps, "the preferred machine is offline, so the other one runs it");
    assert_eq!(connector_tabs(&node_b), 1);
    assert_eq!(node_b.machine.tabs()[0].cwd, repo.display().to_string());

    let mut node_a = Node::start(&dir_a, creds_a);
    // The delivery below is a prompt, which the node holds until claude takes the terminal, as this stand-in does.
    std::fs::write(dir_a.join("claude"), "#!/bin/sh\nstty raw -echo; exec cat\n").unwrap();
    client.wait_machines(&[("mac-mini", true), ("vps", true)]);
    assert_eq!(open(run(Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() })), mac_mini);
    assert_eq!((connector_tabs(&node_a), connector_tabs(&node_b)), (1, 1), "the preferred machine runs it once online");

    let issue = Subject::LinearIssue { issue: "uuid-1".into() };
    assert_eq!(open(run(issue.clone())), vps, "a run a session already works on counts as taken by its machine");
    let Work::Run(again) = run(Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() }) else { unreachable!() };
    assert_eq!(node_a.machine.run(&again.spec()), Ok("tab-1".into()), "a machine answers a run sent again with the session it started");
    assert_eq!((connector_tabs(&node_a), connector_tabs(&node_b)), (1, 1));

    let forward = |node: &Node, subject: Subject| node.link.as_ref().unwrap().forward(agent::Message::Stage { connector: id.clone(), subject, stage: Stage::Coding });
    assert_eq!(
        forward(&node_b, issue.clone()),
        Ok("this workflow has no column for that stage, so the issue stays where it is".into()),
        "vps reports on the issue its session works on"
    );
    assert_eq!(
        forward(&node_a, issue),
        Err("this machine runs no session of that workflow on that subject".into()),
        "mac-mini can't report on vps's issue"
    );

    let delivery = |thread: &str| {
        Work::Delivery(Delivery { connector: id.clone(), channel: "C0123ABCD".into(), thread: thread.into(), messages: vec!["yes, file it".into()], cursor: "1792400090.000000".into(), mentioned: vec![] })
    };
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = seen.clone();
    node_a.machine.attach("tab-1", "test".into(), Box::new(move |b: &[u8]| sink.lock().unwrap().extend_from_slice(b))).unwrap();
    assert_eq!(open(delivery("1792400050.000200")), mac_mini, "the thread's live session takes its new messages");
    wait_for("the delivered prompt", || String::from_utf8_lossy(&seen.lock().unwrap()).contains("yes, file it"));
    assert_eq!(open(delivery("1792499999.000100")), None, "no session answers that thread");

    node_a.kill();
    node_b.kill();
    client.wait_machines(&[("mac-mini", false), ("vps", false)]);
    assert_eq!(open(run(Subject::LinearIssue { issue: "uuid-2".into() })), None, "with no machine online the run is unclaimed, for the next check");
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn a_run_whose_machine_goes_offline_resumes_on_another_machine_with_its_work() {
    let hub = TestHub::start("resume");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let register = |name: &str, dir: &Path| {
        let (machine, machine_token) = account::register_machine(&url, &token, name, "macos").unwrap();
        Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token }
            .save(dir)
            .unwrap();
        Credentials::load(dir).unwrap()
    };
    let (dir_a, dir_b, repo) = (temp("resume-a"), temp("resume-b"), temp("resume-repo"));
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("app.txt"), "v1\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
    let (creds_a, creds_b) = (register("mac-mini", &dir_a), register("vps", &dir_b));
    let vps = creds_b.machine_id.clone();
    let mut node_a = Node::start(&dir_a, creds_a.clone());
    let node_b = Node::start(&dir_b, creds_b);
    let args_file = dir_b.join("args");
    std::fs::write(dir_b.join("claude"), format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexec cat\n", args_file.display())).unwrap();
    client.wait_machines(&[("mac-mini", true), ("vps", true)]);

    let issue = Subject::LinearIssue { issue: "uuid-1".into() };
    let store = json!({
        "connectors": [{
            "id": "c1", "name": "Bugs", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": repo, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": creds_a.machine_id,
        }],
        "runs": { "c1/uuid-1": {
            "connector": "c1", "subject": { "issue": "uuid-1" }, "title": "ENG-1 Broken button", "state": "queued", "after": 0,
            "attempt": 0, "cwd": repo, "args": ["--name", "ENG-1", "Fix ENG-1"], "worktree": "eng-1", "created": 0, "updated": 0,
        } },
    });
    assert_eq!(client.hub(json!({ "cmd": "import", "store": store, "linear_key": null, "slack_key": null })), Ok(json!(true)));
    let tab_on = |node: &Node| node.machine.tabs().into_iter().find(|t| t.connector.as_deref() == Some("c1") && t.running);
    wait_for("mac-mini to start the queued run", || tab_on(&node_a).is_some());
    assert!(tab_on(&node_b).is_none());

    // The session names itself, then leaves work nobody committed.
    let tab = tab_on(&node_a).unwrap();
    node_a.machine.record_event(AgentEvent {
        status: AgentStatus::Working,
        cwd: Some(repo.display().to_string()),
        message: None,
        tab: Some(tab.id.clone()),
        session: Some("s1".into()),
        title: None,
        resume_at: None,
        prompt: None,
        snapshot: None,
    });
    std::fs::write(repo.join("wip.txt"), "half done\n").unwrap();
    std::fs::write(repo.join("app.txt"), "v2\n").unwrap();
    let line = format!("{}\n", json!({ "type": "user", "cwd": repo }));
    let transcripts = sync::claude_dir(&dir_a.join("home"), &repo);
    std::fs::create_dir_all(&transcripts).unwrap();
    std::fs::write(transcripts.join("s1.jsonl"), &line).unwrap();
    let checkpoint = checkpoint::capture(&dir_a.join("home"), "s1", 1 << 20).unwrap();
    let send = |node: &Node| node.link.as_ref().unwrap().call_hub(HubCall::Checkpoint { connector: "c1".into(), subject: issue.clone(), checkpoint: checkpoint.clone() });
    assert_eq!(send(&node_b), Err("this machine runs no session of that workflow on that subject".into()));
    assert_eq!(send(&node_a), Ok(Value::Null));

    node_a.disconnect();
    wait_for("vps to resume the run", || tab_on(&node_b).is_some());
    let resumed = tab_on(&node_b).unwrap();
    let worktree = repo.join(".claude/worktrees/eng-1");
    assert_eq!((resumed.cwd, resumed.session.as_deref()), (worktree.display().to_string(), Some("s1")));
    assert_eq!(std::fs::read_to_string(worktree.join("wip.txt")).unwrap(), "half done\n");
    assert_eq!(std::fs::read_to_string(worktree.join("app.txt")).unwrap(), "v2\n");
    assert_eq!(std::fs::read_to_string(sync::claude_dir(&dir_b.join("home"), &worktree).join("s1.jsonl")).unwrap(), line);
    wait_for("vps's claude to start", || args_file.exists());
    let args = std::fs::read_to_string(&args_file).unwrap();
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(args.windows(2).find(|w| w[0] == "--resume"), Some(&["--resume", "s1"][..]));
    assert!(args.last().unwrap().starts_with("pinned resumed this session because the machine running it went offline."), "{args:?}");

    // mac-mini comes back with the old session: its callbacks are refused and its tab closes.
    node_a.reconnect(creds_a);
    wait_for("mac-mini back online", || node_a.online());
    assert_eq!(
        node_a.link.as_ref().unwrap().forward(agent::Message::Stage { connector: "c1".into(), subject: issue.clone(), stage: Stage::Coding }),
        Err("this run moved to another machine after this one went offline: stop working on it here".into())
    );
    wait_for("mac-mini's stale tab to close", || tab_on(&node_a).is_none());
    assert!(tab_on(&node_b).is_some(), "vps keeps the run");
    let settings = client.hub(json!({ "cmd": "get_settings" })).unwrap();
    let run = &settings["runs"][0];
    assert_eq!((run["key"].as_str(), run["state"].as_str(), run["machine"].as_str(), run["attempt"].as_u64()), (Some("c1/uuid-1"), Some("running"), Some(vps.as_str()), Some(2)));
}

#[test]
fn an_apps_first_sign_in_moves_its_connectors_into_an_empty_account_only() {
    let hub = TestHub::start("import");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let dir = temp("import-mac");
    let (machine, machine_token) = account::register_machine(&url, &token, "macbook", "macos").unwrap();
    let creds = Credentials { hub: url.clone(), email: account.email, user_token: token.clone(), machine_id: machine.id, machine_token };
    creds.save(&dir).unwrap();
    let mut node = Node::start(&dir, creds);
    wait_for("macbook online", || node.online());
    let link = node.link.as_ref().unwrap();
    let local = json!({ "connectors": [{
        "id": "c1", "name": "Bugs", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
        "folder": "/repo", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "",
    }] });
    assert_eq!(link.call_hub(HubCall::Import { store: local, linear_key: Some("lin_api_1".into()), slack_key: None, github_key: None }), Ok(json!(true)));
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    assert_eq!(client.hub(json!({ "cmd": "get_settings" })).unwrap()["connectors"][0]["name"], "Bugs");

    let other = HubCall::Import { store: json!({ "connectors": [] }), linear_key: None, slack_key: Some("xoxb-2".into()), github_key: None };
    assert_eq!(link.call_hub(other), Ok(json!(false)), "a later machine can't replace the account's connectors");
    assert_eq!(client.hub(json!({ "cmd": "get_settings" })).unwrap()["connectors"][0]["name"], "Bugs");
    assert_eq!(link.call_hub(HubCall::GetSettings), Err("a machine may only forward a workflow session's callbacks".into()));
    node.kill();
}

struct Through<'a>(&'a Link);

impl sync::Hub for Through<'_> {
    fn list(&self, project: &str) -> Result<Vec<sync::Entry>, String> {
        Ok(serde_json::from_value(self.0.call_hub(HubCall::SyncList { project: project.into() })?).unwrap())
    }
    fn push(&self, project: &str, files: Vec<sync::File>) -> Result<(), String> {
        self.0.call_hub(HubCall::SyncPush { project: project.into(), files }).map(drop)
    }
    fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<sync::File>, String> {
        Ok(serde_json::from_value(self.0.call_hub(HubCall::SyncPull { project: project.into(), paths })?).unwrap())
    }
}

#[test]
fn machines_share_a_projects_memory_through_the_hub_wherever_they_keep_it() {
    let hub = TestHub::start("sync");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let start = |name: &str| {
        let dir = temp(&format!("sync-{name}"));
        let (machine, machine_token) = account::register_machine(&url, &token, name, "macos").unwrap();
        let creds = Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token };
        creds.save(&dir).unwrap();
        let node = Node::start(&dir, creds);
        wait_for("the machine online", || node.online());
        (node, dir)
    };
    let ((mac, mac_home), (mini, mini_home)) = (start("mac"), start("mini"));
    let project = sync::Project { key: "github.com/acme/app".into(), name: "app".into(), repo: "git@github.com:acme/app.git".into() };
    let checkout = |home: &Path, folder: &str| sync::Checkout { project: project.clone(), path: home.join(folder), history: None };
    let (on_mac, on_mini) = (checkout(&mac_home, "projetos/app"), checkout(&mini_home, "pinned/app"));
    let memory = |home: &Path, c: &sync::Checkout| sync::claude_dir(home, &c.path).join("memory/MEMORY.md");
    std::fs::create_dir_all(memory(&mac_home, &on_mac).parent().unwrap()).unwrap();
    std::fs::write(memory(&mac_home, &on_mac), "- deploys go through the VPS").unwrap();

    let (mac_link, mini_link) = (mac.link.as_ref().unwrap(), mini.link.as_ref().unwrap());
    let now = std::time::SystemTime::now();
    let pushed = sync::sync(&Through(mac_link), &mut sync::State::default(), &mac_home, &on_mac, now).unwrap().pushed;
    assert_eq!(pushed, vec!["claude/memory/MEMORY.md"]);
    let pulled = sync::sync(&Through(mini_link), &mut sync::State::default(), &mini_home, &on_mini, now).unwrap().pulled;
    assert_eq!(pulled, vec!["claude/memory/MEMORY.md"]);
    assert_eq!(std::fs::read_to_string(memory(&mini_home, &on_mini)).unwrap(), "- deploys go through the VPS");

    let outside = sync::File { entry: sync::Entry { path: "../.ssh/authorized_keys".into(), sha: "x".into(), modified: 1, size: 1 }, data: Some("eA==".into()) };
    assert_eq!(
        mac_link.call_hub(HubCall::SyncPush { project: project.key.clone(), files: vec![outside] }),
        Err("../.ssh/authorized_keys is not a file a project shares".into())
    );
    mac_link.call_hub(HubCall::ShareProject { project: project.clone() }).unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    assert_eq!(client.hub(json!({ "cmd": "shared_projects" })), Ok(json!([{ "key": "github.com/acme/app", "name": "app", "repo": "git@github.com:acme/app.git" }])));
    let (stranger, _) = account::signup(&url, "bruno@acme.dev", "correct horse").unwrap();
    let mut other = Client::connect(&url, &stranger);
    other.wait("hello", |f| f["type"] == "hello");
    assert_eq!(other.hub(json!({ "cmd": "shared_projects" })), Ok(json!([])), "another account sees none of it");
    let (mut mac, mut mini) = (mac, mini);
    mac.kill();
    mini.kill();
}

#[test]
fn a_session_too_big_for_one_websocket_frame_syncs_without_dropping_the_link() {
    let hub = TestHub::start("big_sync");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let start = |name: &str| {
        let dir = temp(&format!("big-sync-{name}"));
        let (machine, machine_token) = account::register_machine(&url, &token, name, "macos").unwrap();
        let creds = Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token };
        creds.save(&dir).unwrap();
        let node = Node::start(&dir, creds);
        wait_for("the machine online", || node.online());
        (node, dir)
    };
    let ((mut mac, mac_home), (mut mini, mini_home)) = (start("mac"), start("mini"));
    let project = sync::Project { key: "github.com/acme/app".into(), name: "app".into(), repo: "git@github.com:acme/app.git".into() };
    let checkout = |home: &Path| sync::Checkout { project: project.clone(), path: home.join("app"), history: None };
    let session = |home: &Path| sync::claude_dir(home, &home.join("app")).join("session.jsonl");
    // Random bytes don't compress, so the push carries about 19 MB of base64, past tungstenite's 16 MB default frame.
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let big: Vec<u8> = (0..14 << 20)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect();
    std::fs::create_dir_all(session(&mac_home).parent().unwrap()).unwrap();
    std::fs::write(session(&mac_home), &big).unwrap();

    let now = std::time::SystemTime::now();
    let pushed = sync::sync(&Through(mac.link.as_ref().unwrap()), &mut sync::State::default(), &mac_home, &checkout(&mac_home), now);
    assert_eq!(pushed.map(|s| s.pushed), Ok(vec!["claude/session.jsonl".to_owned()]));
    let pulled = sync::sync(&Through(mini.link.as_ref().unwrap()), &mut sync::State::default(), &mini_home, &checkout(&mini_home), now);
    assert_eq!(pulled.map(|s| s.pulled), Ok(vec!["claude/session.jsonl".to_owned()]));
    assert!(std::fs::read(session(&mini_home)).unwrap() == big, "the mini got the session byte for byte");
    assert!(mac.online() && mini.online(), "both links stay up");
    mac.kill();
    mini.kill();
}

#[test]
fn a_file_a_client_attaches_lands_in_the_nodes_private_uploads_folder() {
    let hub = TestHub::start("upload");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let dir = temp("upload-node");
    let creds = pair(&url, &mut client, "vps", &dir);
    let machine = creds.machine_id.clone();
    let node = Node::start(&dir, creds);
    wait_for("the node online", || node.online());
    // 20 MB in base64 is past tungstenite's 16 MB default frame, on the client's socket and the node's.
    let photo: Vec<u8> = (0..20 << 20).map(|i| (i % 251) as u8).collect();
    let upload = |name: &str, data: &[u8]| json!({ "cmd": "upload", "name": name, "data": base64::engine::general_purpose::STANDARD.encode(data) });

    let path = client.call(&machine, upload("../../.ssh/Foto da praia (1).JPG", &photo)).unwrap();
    let uploads = dir.join("uploads");
    assert_eq!(path, json!(uploads.join("Foto_da_praia__1_.JPG").display().to_string()));
    assert!(std::fs::read(uploads.join("Foto_da_praia__1_.JPG")).unwrap() == photo, "the node holds the photo byte for byte");
    assert_eq!(std::fs::metadata(&uploads).unwrap().permissions().mode() & 0o777, 0o700);
    let again = client.call(&machine, upload("Foto da praia (1).JPG", b"second")).unwrap();
    assert_eq!(again, json!(uploads.join("Foto_da_praia__1_-2.JPG").display().to_string()));

    let too_big = vec![0u8; (25 << 20) + 1];
    assert_eq!(client.call(&machine, upload("clip.mov", &too_big)), Err("clip.mov is over 25 MB, the largest file pinned attaches".into()));
    assert!(!uploads.join("clip.mov").exists());
    assert!(node.online(), "the link stays up");
}

#[test]
fn a_client_that_connects_later_sees_the_usage_a_machine_reported_before() {
    let hub = TestHub::start("usage");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let dir = temp("usage-node");
    let creds = pair(&url, &mut client, "vps", &dir);
    let mut node = Node::start(&dir, creds.clone());
    node.machine.listen(|_| Err("unexpected".into())).unwrap();
    client.wait_machines(&[("vps", true)]);
    let report = r#"{"kind":"usage","five_hour":{"used_percentage":23.5,"resets_at":4102444800}}"#;
    std::io::Write::write_all(&mut std::os::unix::net::UnixStream::connect(dir.join("sock")).unwrap(), format!("{report}\n").as_bytes()).unwrap();
    let expected = json!({ "type": "event", "machine": creds.machine_id, "event": { "kind": "usage", "usage": { "five_hour": { "used_percentage": 23.5, "resets_at": 4102444800u64 } } } });
    client.wait("the usage", |f| *f == expected);

    let mut later = Client::connect(&url, &token);
    later.wait("the usage the hub kept", |f| *f == expected);

    node.link.take().unwrap().stop();
    client.wait_machines(&[("vps", false)]);
    node.link = Some(link::spawn(creds, node.machine.clone(), "0.0.0-test"));
    client.wait_machines(&[("vps", true)]);
    let mut after_reconnect = Client::connect(&url, &token);
    after_reconnect.wait("the usage the machine sent again", |f| *f == expected);
}

#[test]
fn sign_in_rejects_wrong_passwords_duplicates_and_logged_out_tokens() {
    let hub = TestHub::start("auth");
    let url = hub.url.clone();
    assert_eq!(account::signup(&url, "ana@acme.dev", "short"), Err("the password needs at least 8 characters".into()));
    assert_eq!(account::signup(&url, "not an email", "correct horse"), Err("that is not an email address".into()));
    account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    assert_eq!(account::signup(&url, "ANA@acme.dev", "another horse"), Err("an account with that email already exists".into()));
    assert_eq!(account::login(&url, "ana@acme.dev", "wrong horse"), Err("wrong email or password".into()));
    assert_eq!(account::login(&url, "bruno@acme.dev", "correct horse"), Err("wrong email or password".into()));
    assert_eq!(account::register_machine(&url, "not-a-token", "vps", "linux"), Err("sign in first".into()));

    let (token, _) = account::login(&url, " Ana@acme.dev ", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    account::logout(&url, &token).unwrap();
    wait_for("the signed-out client to close", || !client.pump());
    assert_eq!(client.closed, Some(4001));
    let mut again = Client::connect(&url, &token);
    wait_for("the stale token to be refused", || !again.pump());
    assert_eq!((again.closed, again.frames.len()), (Some(4001), 0));
    assert_eq!(account::register_machine(&url, &token, "vps", "linux"), Err("sign in first".into()));

    let (idle, _) = account::login(&url, "ana@acme.dev", "correct horse").unwrap();
    assert!(account::register_machine(&url, &idle, "vps", "linux").is_ok());
    admin_on(&hub.database, "update logins set last_used = now() - interval '91 days'");
    assert_eq!(account::register_machine(&url, &idle, "vps", "linux"), Err("sign in first".into()), "a login unused for 90 days expired");

    for _ in 0..8 {
        assert_eq!(account::login(&url, "ana@acme.dev", "wrong horse"), Err("wrong email or password".into()));
    }
    assert_eq!(account::login(&url, "ana@acme.dev", "correct horse"), Err("too many failed sign ins; try again in 15 minutes".into()));
}

#[test]
fn a_closed_hub_takes_only_its_first_account_whose_password_can_change() {
    let hub = TestHub::start_with("closed", false);
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    assert_eq!(account::signup(&url, "bruno@acme.dev", "correct horse"), Err("this hub already has its account, so sign in with it instead".into()));
    assert_eq!(account::signup(&url, "ana@acme.dev", "another horse"), Err("this hub already has its account, so sign in with it instead".into()), "a closed hub doesn't say which emails it has");

    assert_eq!(account::change_password(&url, &token, "wrong horse", "battery staple"), Err("the current password is wrong".into()));
    assert_eq!(account::change_password(&url, &token, "correct horse", "short"), Err("the password needs at least 8 characters".into()));
    assert_eq!(account::change_password(&url, "not-a-token", "correct horse", "battery staple"), Err("sign in first".into()));
    account::change_password(&url, &token, "correct horse", "battery staple").unwrap();
    assert_eq!(account::login(&url, "ana@acme.dev", "correct horse"), Err("wrong email or password".into()));
    assert!(account::login(&url, "ana@acme.dev", "battery staple").is_ok());
    assert!(account::register_machine(&url, &token, "vps", "linux").is_ok(), "the login that changed the password stays signed in");
}

#[test]
fn a_burst_of_parallel_sign_ins_counts_against_the_limit() {
    let hub = TestHub::start("burst");
    let url = hub.url.clone();
    account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let answers: Vec<_> = (0..30)
        .map(|_| {
            let url = url.clone();
            std::thread::spawn(move || account::login(&url, "ana@acme.dev", "wrong horse"))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect();
    let refused = answers.iter().filter(|a| **a == Err("too many failed sign ins; try again in 15 minutes".to_owned())).count();
    assert_eq!(refused, 20);
}

#[test]
fn the_hub_reconnects_when_postgres_drops_it() {
    let hub = TestHub::start("reconnect");
    account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    admin(&format!("select pg_terminate_backend(pid) from pg_stat_activity where datname = '{}'", hub.database));
    std::thread::sleep(Duration::from_millis(200));
    assert!(account::login(&hub.url, "ana@acme.dev", "correct horse").is_ok());
}

#[test]
fn integration_keys_reach_postgres_sealed() {
    let hub = TestHub::start("sealed");
    let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&hub.url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    client.hub(json!({ "cmd": "import", "store": { "connectors": [] }, "linear_key": "lin_api_secret", "slack_key": null })).unwrap();
    let sealed = || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let (db, connection) = tokio_postgres::connect(&format!("{POSTGRES}/{}", hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.query_one("select secrets::text from accounts", &[]).await.unwrap().get::<_, String>(0)
        })
    };
    wait_for("the keys to be written", || sealed().contains("sealed:"));
    assert!(!sealed().contains("lin_api_secret"));
}
