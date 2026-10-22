use base64::Engine;
use pinned_core::account::{self, Credentials};
use pinned_core::agent::{self, Stage, Subject};
use pinned_core::connectors::{Delivery, Run, Work};
use pinned_core::link::{self, Link};
use pinned_core::machine::{Launch, Machine};
use serde_json::{json, Value};
use std::net::TcpStream;
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

    assert_eq!(
        client.call(b, json!({ "cmd": "format_disk" })),
        Err("this hub runs pinned 0.5.1, which can't answer that: unknown variant `format_disk`, expected one of `tabs`, `open_tab`, `start_claude`, `write_input`, `resize_pty`, `close_tab`, `set_folder`, `list_projects`, `project_history`, `git_remote`, `run`, `deliver`".replace("0.5.1", env!("CARGO_PKG_VERSION")))
    );
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
    let run = |subject: Subject| Work::Run(Run { cwd: repo.clone(), repo: None, args: vec![], connector: id.clone(), subject });
    let open = |work| hub.runtime.block_on(hub.hub().open(account_id, work));
    let connector_tabs = |node: &Node| node.machine.tabs().into_iter().filter(|t| t.connector.as_deref() == Some(&id) && t.running).count();

    let mut node_b = Node::start(&dir_b, creds_b);
    wait_for("vps online", || node_b.online());
    client.wait_machines(&[("mac-mini", false), ("vps", true)]);
    assert!(open(run(Subject::LinearIssue { issue: "uuid-1".into() })), "the preferred machine is offline, so the other one runs it");
    assert_eq!(connector_tabs(&node_b), 1);
    assert_eq!(node_b.machine.tabs()[0].cwd, repo.display().to_string());

    let mut node_a = Node::start(&dir_a, creds_a);
    client.wait_machines(&[("mac-mini", true), ("vps", true)]);
    assert!(open(run(Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() })));
    assert_eq!((connector_tabs(&node_a), connector_tabs(&node_b)), (1, 1), "the preferred machine runs it once online");

    let issue = Subject::LinearIssue { issue: "uuid-1".into() };
    assert!(open(run(issue.clone())), "a run a session already works on counts as taken");
    let Work::Run(again) = run(Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() }) else { unreachable!() };
    assert_eq!(node_a.machine.run(&again.spec()), Ok("tab-1".into()), "a machine answers a run sent again with the session it started");
    assert_eq!((connector_tabs(&node_a), connector_tabs(&node_b)), (1, 1));

    let forward = |node: &Node, subject: Subject| node.link.as_ref().unwrap().forward(agent::Message::Stage { connector: id.clone(), subject, stage: Stage::Coding });
    assert_eq!(
        forward(&node_b, issue.clone()),
        Ok("this connector has no column for that stage, so the issue stays where it is".into()),
        "vps reports on the issue its session works on"
    );
    assert_eq!(
        forward(&node_a, issue),
        Err("this machine runs no session of that connector on that subject".into()),
        "mac-mini can't report on vps's issue"
    );

    let delivery = |thread: &str| {
        Work::Delivery(Delivery { connector: id.clone(), channel: "C0123ABCD".into(), thread: thread.into(), messages: vec!["yes, file it".into()], cursor: "1792400090.000000".into() })
    };
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = seen.clone();
    node_a.machine.attach("tab-1", "test".into(), Box::new(move |b: &[u8]| sink.lock().unwrap().extend_from_slice(b))).unwrap();
    assert!(open(delivery("1792400050.000200")), "the thread's live session takes its new messages");
    wait_for("the delivered prompt", || String::from_utf8_lossy(&seen.lock().unwrap()).contains("yes, file it"));
    assert!(!open(delivery("1792499999.000100")), "no session answers that thread");

    node_a.kill();
    node_b.kill();
    client.wait_machines(&[("mac-mini", false), ("vps", false)]);
    assert!(!open(run(Subject::LinearIssue { issue: "uuid-2".into() })), "with no machine online the run is unclaimed, for the next check");
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
    assert_eq!(account::signup(&url, "bruno@acme.dev", "correct horse"), Err("this hub takes no new accounts".into()));
    assert_eq!(account::signup(&url, "ana@acme.dev", "another horse"), Err("this hub takes no new accounts".into()), "a closed hub doesn't say which emails it has");

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
