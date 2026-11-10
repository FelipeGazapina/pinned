use base64::Engine;
use pinned_core::account::{self, Credentials, DictationEngine, DictationFeedback, DictationRating, Signup};
use pinned_core::accounts::Provider;
use pinned_core::agent::{self, Role, Stage, Subject};
use pinned_core::connectors::{BoxSize, BoxStep, Delivery, Run, RunsOn, Work};
use pinned_core::global_orchestrator::{CaptainReport, RelayTo};
use pinned_core::link::{self, Link};
use pinned_core::machine::{Launch, Machine};
use pinned_core::sync;
use pinned_core::taste::{TastePatch, TasteScope, Tastes};
use pinned_core::upstash::{BoxId, Exec, Listed, NewBox};
use pinned_core::sessions::Saved;
use pinned_core::wire::{Call, HubCall};
use pinned_server::boxes::{BoxApi, Policy};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use futures_util::future::BoxFuture;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// The test Postgres, where the role `pinned` owns a database `pinned`. Override with PINNED_TEST_POSTGRES.
fn postgres() -> String {
    std::env::var("PINNED_TEST_POSTGRES").unwrap_or_else(|_| "postgres://pinned@127.0.0.1:5499".into())
}
const WAIT: Duration = Duration::from_secs(10);

struct TestHub {
    runtime: tokio::runtime::Runtime,
    running: Option<pinned_server::Running>,
    url: String,
    database: String,
    dir: PathBuf,
    boxes: Arc<FakeBoxes>,
    policy: Policy,
    /// The learning database's name, when the hub runs with one.
    learning: Option<String>,
    signup: Signup,
}

impl TestHub {
    fn start(name: &str) -> TestHub {
        TestHub::start_with(name, Signup::Open)
    }

    fn start_with(name: &str, signup: Signup) -> TestHub {
        TestHub::start_with_boxes(name, signup, Policy::default())
    }

    fn start_with_boxes(name: &str, signup: Signup, policy: Policy) -> TestHub {
        TestHub::start_full(name, signup, policy, false)
    }

    /// A hub with a second, learning database next to its own.
    fn start_learning(name: &str) -> TestHub {
        TestHub::start_full(name, Signup::Open, Policy::default(), true)
    }

    fn start_full(name: &str, signup: Signup, policy: Policy, with_learning: bool) -> TestHub {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let database = format!("pinned_test_{name}_{}", std::process::id());
        admin(&format!("drop database if exists {database} with (force)"));
        admin(&format!("create database {database}"));
        let learning = with_learning.then(|| format!("pinned_learning_{name}_{}", std::process::id()));
        if let Some(learning) = &learning {
            admin(&format!("drop database if exists {learning} with (force)"));
            admin(&format!("create database {learning}"));
        }
        let dir = temp(&format!("hub-{name}"));
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::write(dir.join("ui/index.html"), "<title>pinned</title>").unwrap();
        std::fs::create_dir_all(dir.join("downloads")).unwrap();
        std::fs::write(dir.join("downloads/pinned-server-x86_64-unknown-linux-musl"), "ELF").unwrap();
        let boxes = FakeBoxes::new(dir.join("boxes"));
        let mut hub = TestHub { runtime: tokio::runtime::Runtime::new().unwrap(), running: None, url: String::new(), database, dir, boxes, policy, learning, signup };
        hub.serve("127.0.0.1:0");
        hub
    }

    fn serve(&mut self, listen: &str) {
        let config = || pinned_server::Config {
            db: format!("{}/{}", postgres(), self.database),
            listen: listen.into(),
            ui: self.dir.join("ui"),
            downloads: self.dir.join("downloads"),
            public_url: None,
            key_file: self.dir.join("hub.key"),
            signup: self.signup,
            box_api: self.boxes.clone(),
            box_policy: self.policy.clone(),
            learning_db: self.learning.as_ref().map(|learning| format!("{}/{learning}", postgres())),
        };
        let start = Instant::now();
        // A restarted hub takes its old address, which the dying runtime may not have let go of yet.
        let running = loop {
            match self.runtime.block_on(pinned_server::start(config())) {
                Ok(running) => break running,
                Err(e) if e.contains("could not listen") && start.elapsed() < WAIT => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => panic!("{e}"),
            }
        };
        self.url = format!("http://{}", running.addr);
        self.running = Some(running);
    }

    /// Kills the hub mid-whatever, as a deploy does, and starts another on its database and address.
    fn restart(&mut self) {
        let addr = self.running.take().unwrap().addr;
        let old = std::mem::replace(&mut self.runtime, tokio::runtime::Runtime::new().unwrap());
        old.shutdown_background();
        self.serve(&addr.to_string());
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
        if let Some(learning) = &self.learning {
            admin(&format!("drop database if exists {learning} with (force)"));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn admin(sql: &str) {
    admin_on("pinned", sql);
}

fn admin_on(database: &str, sql: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{database}", postgres()), tokio_postgres::NoTls).await.unwrap();
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

fn launched(dir: &Path) -> Arc<Machine> {
    let claude = dir.join("claude");
    std::fs::write(&claude, "#!/bin/sh\n[ \"$1 $2\" = \"auth status\" ] && { echo '{\"loggedIn\":true}'; exit 0; }\nexec cat\n").unwrap();
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let launch = Launch {
        exe: "/opt/pinned/bin/pinned-server".into(),
        bins: BTreeMap::from([(Provider::Claude, claude)]),
        path_env: None,
        settings: dir.join("settings.json"),
        mcp_config: None,
        reminders: true,
        plugin_dir: dir.join("plugin"),
        tutorial_helper: dir.join("plugin/tutorial.mjs"),
        sock: dir.join("sock"),
        vector_search: false,
    };
    Machine::new(launch, Some(dir.to_owned()), Some(dir.to_owned()), |_| {})
}

impl Node {
    fn start(dir: &Path, creds: Credentials) -> Node {
        let machine = launched(dir);
        let link = link::spawn(creds, machine.clone(), "0.0.0-test");
        Node { machine, link: Some(link) }
    }

    /// The production node: the link is up, then saved sessions are restored and started.
    fn resume(dir: &Path, creds: Credentials) -> Node {
        let machine = launched(dir);
        let link = link::spawn(creds, machine.clone(), "0.0.0-test");
        for tab in machine.restore_tabs() {
            machine.start_claude(&tab, 80, 24).unwrap();
        }
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

    let tab_id = client.call(b, json!({ "cmd": "open_tab", "path": null, "bypass": false })).unwrap().as_str().unwrap().to_owned();
    let cwd = client.call(b, json!({ "cmd": "start_claude", "tab": &tab_id, "cols": 80, "rows": 24 })).unwrap();
    assert_eq!(cwd, dir_b.display().to_string());
    client.wait("node-b's running tab", |f| {
        f["type"] == "event" && f["machine"] == b && f["event"]["kind"] == "tabs" && f["event"]["tabs"][0]["running"] == true
    });
    client.send(json!({ "type": "attach", "machine": b, "tab": &tab_id }));
    let typed: String = (0..300).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    for key in typed.chars().chain(['\r']) {
        client.send(json!({ "type": "call", "id": 1000, "machine": b, "call": { "cmd": "write_input", "tab": &tab_id, "data": key.to_string() } }));
    }
    let screen = format!("{typed}\r\n{typed}\r\n");
    let echoed = client.wait_output(b, &tab_id, "\r\n");
    assert_eq!(echoed.get(..typed.len()), Some(typed.as_str()), "keys echo in the order they were typed");
    assert_eq!(client.wait_output(b, &tab_id, &screen), screen);

    let mut late = Client::connect(&url, &token);
    let hello = late.wait("hello", |f| f["type"] == "hello");
    assert_eq!(hello["tabs"][b][0]["id"], tab_id);
    late.send(json!({ "type": "attach", "machine": b, "tab": &tab_id }));
    assert_eq!(late.wait_output(b, &tab_id, &screen), screen, "a late client gets the screen");
    client.call(b, json!({ "cmd": "write_input", "tab": &tab_id, "data": "again\r" })).unwrap();
    late.wait_output(b, &tab_id, "again\r\nagain\r\n");
    assert_eq!(client.wait_output(b, &tab_id, "again\r\nagain\r\n"), format!("{screen}again\r\nagain\r\n"), "the first client never sees the screen twice");

    let refused = client.call(b, json!({ "cmd": "format_disk" })).unwrap_err();
    let says = format!("this hub runs pinned {}, which can't answer that: unknown variant `format_disk`, expected one of `tabs`, `open_tab`", env!("CARGO_PKG_VERSION"));
    assert!(refused.starts_with(&says), "{refused}");
    assert!(client.hub(json!({ "cmd": "self_destruct" })).unwrap_err().contains("unknown variant `self_destruct`"));
    assert_eq!(client.call("not-a-machine", json!({ "cmd": "tabs" })), Err("this account has no machine not-a-machine".into()));

    client.call(b, json!({ "cmd": "close_tab", "tab": &tab_id })).unwrap();
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
fn a_restarted_node_does_not_reuse_tab_ids_and_the_client_gets_that_list() {
    let hub = TestHub::start("retab");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "Ana@Acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    let dir = temp("retab-node");
    let creds = pair(&url, &mut client, "box", &dir);
    let id = creds.machine_id.clone();
    client.wait_machines(&[("box", false)]);
    let mut node = Node::start(&dir, creds.clone());
    client.wait_machines(&[("box", true)]);
    let first = client.call(&id, json!({ "cmd": "open_tab", "path": null, "bypass": false })).unwrap();
    let first_id = first.as_str().unwrap().to_owned();
    node.kill();
    client.wait_machines(&[("box", false)]);
    drop(node);
    let _node = Node::start(&dir, creds);
    client.wait_machines(&[("box", true)]);
    let second = client.call(&id, json!({ "cmd": "open_tab", "path": null, "bypass": false })).unwrap();
    let second_id = second.as_str().unwrap().to_owned();
    assert_ne!(first_id, second_id, "the new process reused {first_id}");
    assert_eq!(
        client.call(&id, json!({ "cmd": "start_claude", "tab": first_id, "cols": 80, "rows": 24 })),
        Err(format!("no tab {first_id}")),
        "the id from before the restart drove the new tab"
    );
    client.pump();
    for frame in &client.frames {
        if frame["type"] != "machines" {
            continue;
        }
        let Some(listed) = frame["tabs"][&id].as_array() else { continue };
        assert!(listed.iter().all(|tab| tab["id"] != first_id), "the snapshot still listed {first_id}: {frame}");
    }
    let ended = ended_tabs(&client.frames, &id);
    assert!(ended.iter().all(|tab| tab["id"] != first_id), "the client still had {first_id}: {ended:?}");
    assert!(ended.iter().any(|tab| tab["id"] == second_id), "the client never got {second_id}: {ended:?}");
}

/// The last tab list a client was given for `machine`: a tabs event, or a machines
/// snapshot that includes that machine. A snapshot that omits the key leaves the previous list.
fn ended_tabs(frames: &[Value], machine: &str) -> Vec<Value> {
    let mut ended = vec![];
    for frame in frames {
        if frame["type"] == "event" && frame["machine"] == machine && frame["event"]["kind"] == "tabs" {
            ended = frame["event"]["tabs"].as_array().cloned().unwrap_or_default();
        } else if matches!(frame["type"].as_str(), Some("machines" | "hello")) && frame["tabs"][machine].is_array() {
            ended = frame["tabs"][machine].as_array().cloned().unwrap();
        }
    }
    ended
}

fn wait_frame(client: &mut Client, from: usize, what: &str, pred: impl Fn(&Value) -> bool) -> usize {
    let start = Instant::now();
    loop {
        if let Some(index) = client.frames.iter().enumerate().skip(from).find(|(_, frame)| pred(frame)).map(|(index, _)| index) {
            return index;
        }
        assert!(client.pump(), "the socket closed while waiting for {what}");
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}; got {:#?}", &client.frames[from..]);
    }
}

/// Drops the bytes between a node and the hub without stopping the node.
struct SocketCut {
    addr: std::net::SocketAddr,
    generation: Arc<AtomicU64>,
}

impl SocketCut {
    fn in_front_of(upstream: std::net::SocketAddr) -> SocketCut {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let generation = Arc::new(AtomicU64::new(1));
        let flag = generation.clone();
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(down) = incoming else { continue };
                let flag = flag.clone();
                std::thread::spawn(move || {
                    let Ok(up) = TcpStream::connect(upstream) else { return };
                    let mine = flag.load(Ordering::SeqCst);
                    let _ = down.set_nodelay(true);
                    let _ = up.set_nodelay(true);
                    let (down_out, up_out) = (down.try_clone().unwrap(), up.try_clone().unwrap());
                    let flag_back = flag.clone();
                    std::thread::spawn(move || copy_until(down_out, up_out, &flag_back, mine));
                    copy_until(up, down, &flag, mine);
                });
            }
        });
        SocketCut { addr, generation }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn cut(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

fn copy_until(mut from: TcpStream, mut to: TcpStream, generation: &AtomicU64, mine: u64) {
    let _ = from.set_read_timeout(Some(Duration::from_millis(50)));
    let mut buf = [0u8; 16 * 1024];
    loop {
        if generation.load(Ordering::SeqCst) != mine {
            let _ = to.shutdown(Shutdown::Both);
            let _ = from.shutdown(Shutdown::Both);
            return;
        }
        match from.read(&mut buf) {
            Ok(0) => {
                let _ = to.shutdown(Shutdown::Write);
                return;
            }
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return,
        }
    }
}

fn http_addr(url: &str) -> std::net::SocketAddr {
    url.trim_start_matches("http://").parse().unwrap()
}

#[test]
fn a_reconnected_node_does_not_wipe_the_tabs_the_client_already_has() {
    let hub = TestHub::start("reconn");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "Ana@Acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    let dir = temp("reconn-node");
    let mut creds = pair(&url, &mut client, "box", &dir);
    let id = creds.machine_id.clone();
    let cut = SocketCut::in_front_of(http_addr(&url));
    creds.hub = cut.url();
    client.wait_machines(&[("box", false)]);
    let node = Node::start(&dir, creds);
    client.wait_machines(&[("box", true)]);
    let tab_id = client.call(&id, json!({ "cmd": "open_tab", "path": null, "bypass": false })).unwrap().as_str().unwrap().to_owned();
    client.wait("the open tab", |frame| {
        frame["type"] == "event" && frame["machine"] == id && frame["event"]["kind"] == "tabs" && frame["event"]["tabs"].as_array().is_some_and(|tabs| tabs.iter().any(|tab| tab["id"] == tab_id))
    });
    let mark = client.frames.len();
    cut.cut();
    client.wait_machines(&[("box", false)]);
    client.wait_machines(&[("box", true)]);
    let reported = wait_frame(&mut client, mark, "tabs after the socket came back", |frame| {
        frame["type"] == "event" && frame["machine"] == id && frame["event"]["kind"] == "tabs" && frame["event"]["tabs"].as_array().is_some_and(|tabs| tabs.iter().any(|tab| tab["id"] == tab_id))
    });
    let wiped = client.frames[mark..reported].iter().find(|frame| {
        frame["type"] == "machines" && frame["tabs"][&id].as_array().is_some_and(|tabs| tabs.is_empty())
    });
    assert!(wiped.is_none(), "a machines frame listed {id} with no tabs before the node reported them: {wiped:?}");
    let ended: Vec<String> = ended_tabs(&client.frames, &id).iter().map(|tab| tab["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(ended, vec![tab_id]);
    drop(node);
}

#[test]
fn a_restarted_node_reports_the_restored_session() {
    let hub = TestHub::start("restore");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "Ana@Acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    let dir = temp("restore-node");
    let creds = pair(&url, &mut client, "box", &dir);
    let id = creds.machine_id.clone();
    client.wait_machines(&[("box", false)]);
    let mut node = Node::start(&dir, creds.clone());
    client.wait_machines(&[("box", true)]);
    let first_id = client.call(&id, json!({ "cmd": "open_tab", "path": null, "bypass": false })).unwrap().as_str().unwrap().to_owned();
    node.kill();
    client.wait_machines(&[("box", false)]);
    drop(node);
    std::fs::write(
        dir.join("sessions.json"),
        format!(r#"[{{"cwd": "{}", "session": "conv-1", "env": [], "bypass": false, "interrupted": false}}]"#, dir.display()),
    )
    .unwrap();
    let mark = client.frames.len();
    let _node = Node::resume(&dir, creds);
    client.wait_machines(&[("box", true)]);
    let reported = wait_frame(&mut client, mark, "the restored tab", |frame| {
        frame["type"] == "event" && frame["machine"] == id && frame["event"]["kind"] == "tabs" && frame["event"]["tabs"].as_array().is_some_and(|tabs| tabs.iter().any(|tab| tab["session"] == "conv-1"))
    });
    let wiped = client.frames[mark..reported].iter().find(|frame| {
        frame["type"] == "machines" && frame["tabs"][&id].as_array().is_some_and(|tabs| tabs.is_empty())
    });
    assert!(wiped.is_none(), "a machines frame listed {id} with no tabs before the restored session: {wiped:?}");
    let ended = ended_tabs(&client.frames, &id);
    assert_eq!(ended.len(), 1, "the client ended with {ended:?}");
    assert_eq!(ended[0]["session"], "conv-1");
    assert_ne!(ended[0]["id"], first_id);
}

#[test]
fn quitting_through_the_hub_sends_an_empty_tab_list_before_disconnect() {
    let hub = TestHub::start("quit");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "Ana@Acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    let dir = temp("quit-node");
    let creds = pair(&url, &mut client, "box", &dir);
    let id = creds.machine_id.clone();
    client.wait_machines(&[("box", false)]);
    let mut node = Node::start(&dir, creds);
    client.wait_machines(&[("box", true)]);
    let tab_id = client.call(&id, json!({ "cmd": "open_tab", "path": null, "bypass": false })).unwrap().as_str().unwrap().to_owned();
    let opened_at = wait_frame(&mut client, 0, "the open tab", |frame| {
        frame["type"] == "event" && frame["machine"] == id && frame["event"]["kind"] == "tabs" && frame["event"]["tabs"].as_array().is_some_and(|tabs| tabs.iter().any(|tab| tab["id"] == tab_id))
    });
    node.machine.quit();
    let empty_at = wait_frame(&mut client, opened_at + 1, "the empty tab list", |frame| {
        frame["type"] == "event" && frame["machine"] == id && frame["event"] == json!({ "kind": "tabs", "tabs": [] })
    });
    node.link.take().unwrap().stop();
    client.wait_machines(&[("box", false)]);
    let offline_at = client
        .frames
        .iter()
        .rposition(|frame| frame["type"] == "machines" && frame["machines"].as_array().is_some_and(|list| list.iter().any(|machine| machine["name"] == "box" && machine["online"] == false)))
        .unwrap();
    assert!(empty_at < offline_at, "empty tabs at {empty_at}, offline at {offline_at}");
    let ended = ended_tabs(&client.frames[..offline_at], &id);
    assert_eq!(ended, Vec::<Value>::new());
}

#[test]
fn a_remote_project_with_no_thread_is_still_in_list_projects() {
    let hub = TestHub::start("remote_projects");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let dir = temp("remote-projects");
    let (machine, machine_token) = account::register_machine(&url, &token, "vps", "linux").unwrap();
    let id = machine.id.clone();
    let creds = Credentials { hub: url.clone(), email: account.email, user_token: token.clone(), machine_id: machine.id, machine_token };
    creds.save(&dir).unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let node = Node::start(&dir, creds);
    client.wait_machines(&[("vps", true)]);

    let notes = dir.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let path = node.machine.open_project(notes).display().to_string();
    assert!(node.machine.tabs().iter().all(|tab| tab.cwd != path), "the folder has a thread, so the tab list would already carry it");
    let listed = client.call(&id, json!({ "cmd": "list_projects" })).unwrap();
    assert!(
        listed.as_array().unwrap().iter().any(|project| project["path"] == path),
        "list_projects omitted a folder that exists and has no thread: {listed}"
    );
    assert!(
        client.frames.iter().filter(|frame| frame["type"] == "event" && frame["machine"] == id && frame["event"]["kind"] == "tabs").all(|frame| {
            frame["event"]["tabs"].as_array().is_none_or(|tabs| tabs.iter().all(|tab| tab["cwd"] != path))
        }),
        "the tab snapshot the client was given includes {path}"
    );
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
    let mac_mini = creds_a.machine_id.clone();
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
    let run = |subject: Subject| {
        Work::Run(Run { cwd: repo.clone(), repo: None, args: vec![], connector: id.clone(), subject, mentioned: vec![], account: None, runs_on: RunsOn::Machine(mac_mini.clone()), input: String::new() })
    };
    let open = |work| hub.runtime.block_on(hub.hub().open(account_id, work));
    let connector_tabs = |node: &Node| node.machine.tabs().into_iter().filter(|t| t.connector.as_deref() == Some(&id) && t.running).count();

    std::fs::write(dir_b.join("accounts.json"), r#"[{"id": "work", "provider": "claude", "name": "Work"}]"#).unwrap();
    let mut node_b = Node::start(&dir_b, creds_b);
    wait_for("vps online", || node_b.online());
    client.wait_machines(&[("mac-mini", false), ("vps", true)]);
    assert_eq!(open(run(Subject::LinearIssue { issue: "uuid-1".into() })), Ok(()), "the preferred machine is offline, so the other one runs it");
    assert_eq!(connector_tabs(&node_b), 1);
    assert_eq!(node_b.machine.tabs()[0].cwd, repo.display().to_string());

    let mut node_a = Node::start(&dir_a, creds_a);
    // The delivery below is a prompt, which the node holds until claude takes the terminal, as this stand-in does.
    std::fs::write(dir_a.join("claude"), "#!/bin/sh\nstty raw -echo; exec cat\n").unwrap();
    client.wait_machines(&[("mac-mini", true), ("vps", true)]);
    assert_eq!(open(run(Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() })), Ok(()));
    assert_eq!((connector_tabs(&node_a), connector_tabs(&node_b)), (1, 1), "the preferred machine runs it once online");

    let issue = Subject::LinearIssue { issue: "uuid-1".into() };
    assert_eq!(open(run(issue.clone())), Ok(()), "a run a session already works on counts as taken");
    let Work::Run(again) = run(Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() }) else { unreachable!() };
    let started = node_a.machine.tabs().into_iter().find(|t| t.connector.as_deref() == Some(id.as_str())).unwrap().id;
    assert_eq!(node_a.machine.run(&again.spec()), Ok(started.clone()), "a machine answers a run sent again with the session it started");
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
    node_a.machine.attach(&started, "test".into(), Box::new(move |b: &[u8]| sink.lock().unwrap().extend_from_slice(b))).unwrap();
    assert_eq!(open(delivery("1792400050.000200")), Ok(()), "the thread's live session takes its new messages");
    wait_for("the delivered prompt", || String::from_utf8_lossy(&seen.lock().unwrap()).contains("yes, file it"));
    assert_eq!(open(delivery("1792499999.000100")), Err("no session answers that thread".into()));

    let work = Run {
        cwd: repo.clone(),
        repo: None,
        args: vec![],
        connector: id.clone(),
        subject: Subject::LinearIssue { issue: "uuid-3".into() },
        mentioned: vec![],
        account: Some("work".into()),
        runs_on: RunsOn::Machine(mac_mini.clone()),
        input: String::new(),
    };
    assert_eq!(open(Work::Run(work)), Ok(()), "mac-mini has no account work and refuses, so vps runs it");
    let on_work = |node: &Node| node.machine.tabs().into_iter().filter(|t| t.account == "work").count();
    assert_eq!((on_work(&node_a), on_work(&node_b)), (0, 1));

    node_a.kill();
    node_b.kill();
    client.wait_machines(&[("mac-mini", false), ("vps", false)]);
    assert_eq!(
        open(run(Subject::LinearIssue { issue: "uuid-2".into() })),
        Err("no machine of the account is online".into()),
        "with no machine online the run is unclaimed, for the next check"
    );
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
    assert_eq!(link.call_hub(HubCall::Import { store: local, linear_key: Some("lin_api_1".into()), slack_key: None, github_key: None, vercel_key: None }), Ok(json!(true)));
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    assert_eq!(client.hub(json!({ "cmd": "get_settings" })).unwrap()["connectors"][0]["name"], "Bugs");

    let other = HubCall::Import { store: json!({ "connectors": [] }), linear_key: None, slack_key: Some("xoxb-2".into()), github_key: None, vercel_key: None };
    assert_eq!(link.call_hub(other), Ok(json!(false)), "a later machine can't replace the account's connectors");
    assert_eq!(client.hub(json!({ "cmd": "get_settings" })).unwrap()["connectors"][0]["name"], "Bugs");
    assert_eq!(link.call_hub(HubCall::GetSettings), Err("a machine may only forward a workflow session's callbacks".into()));
    node.kill();
}

#[test]
fn the_accounts_taste_lives_on_the_hub_and_a_machines_old_style_seeds_an_empty_account_once() {
    let hub = TestHub::start("taste");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let get = || json!({ "cmd": "get_tastes" });
    assert_eq!(client.hub(get()).unwrap()["captains"]["strategy"], "escalate_on_failure", "an account with no taste reads the defaults");

    let dir = temp("taste-mac");
    std::fs::write(dir.join("orchestration.json"), r#"{"style":"quality","max_workers":4}"#).unwrap();
    let (machine, machine_token) = account::register_machine(&url, &token, "macbook", "macos").unwrap();
    let creds = Credentials { hub: url.clone(), email: account.email, user_token: token.clone(), machine_id: machine.id, machine_token };
    creds.save(&dir).unwrap();
    let node = Node::start(&dir, creds);
    wait_for("the machine's old cost style to reach the account", || client.hub(get()).unwrap()["captains"]["strategy"] == "strongest_first");
    let opus = json!({ "model": "claude-opus-5-5" });
    let aggressive = json!({
        "strategy": "strongest_first", "ladder": [opus], "hard_model": opus, "parallel": 4, "autonomy": "report_at_end",
        "review_diffs": false, "second_opinion": true, "skills": [], "context": [],
        "tier": "high", "default_effort": null, "max_effort": "max", "budget_cents": 0, "time_limit_min": 480, "box_budget_cents": 0,
    });
    assert_eq!(client.hub(get()).unwrap()["captains"], aggressive, "the old style's preset, with workers at once lowered to the old cap of 4");

    let site = json!({ "kind": "project", "repo": "github.com/acme/site" });
    let set = client.hub(json!({ "cmd": "set_taste", "scope": site, "patch": { "parallel": 5, "autonomy": "report_at_end" } })).unwrap();
    assert_eq!(set["projects"], json!({ "github.com/acme/site": { "parallel": 5 } }), "the Capitães already report at the end");
    let set = client.hub(json!({ "cmd": "set_taste", "scope": site, "patch": { "review_diffs": true } })).unwrap();
    assert_eq!(set["projects"], json!({ "github.com/acme/site": { "parallel": 5, "review_diffs": true } }));
    assert_eq!(client.hub(json!({ "cmd": "set_taste", "scope": { "kind": "captains" }, "patch": { "parallel": 101 } })), Err("parallel workers go from 1 to 100".into()));
    let reset = client.hub(json!({ "cmd": "reset_taste_field", "scope": site, "field": "parallel" })).unwrap();
    assert_eq!(reset["projects"], json!({ "github.com/acme/site": { "review_diffs": true } }));
    client.hub(json!({ "cmd": "set_taste", "scope": { "kind": "orchestrator" }, "patch": { "parallel": 1, "context": ["Releases go out on Fridays."] } })).unwrap();

    let mut later = Client::connect(&url, &token);
    later.wait("hello", |f| f["type"] == "hello");
    let stored = later.hub(get()).unwrap();
    assert_eq!((&stored["orchestrator"]["parallel"], &stored["orchestrator"]["context"], &stored["captains"], &stored["projects"]), (&json!(1), &json!(["Releases go out on Fridays."]), &aggressive, &json!({ "github.com/acme/site": { "review_diffs": true } })));

    let link = node.link.as_ref().unwrap();
    assert_eq!(link.call_hub(HubCall::GetTastes), Ok(stored.clone()), "a machine reads the account's taste");
    assert_eq!(link.call_hub(HubCall::ImportTastes { tastes: Tastes::default() }), Ok(json!(false)), "a later machine can't replace the account's taste");
    let change = HubCall::SetTaste { scope: TasteScope::Captains, patch: TastePatch { parallel: Some(2), ..TastePatch::default() } };
    assert_eq!(link.call_hub(change), Err("only a machine with a Chat to adjust thread open may change the way of working".into()));

    let folder = dir.join("app");
    std::fs::create_dir_all(&folder).unwrap();
    let captain = node.machine.call(Call::OpenOrchestrator { path: folder.display().to_string() }).unwrap();
    node.machine.start_claude(captain.as_str().unwrap(), 80, 24).unwrap();
    let kept = || std::fs::read(dir.join("taste.json")).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    assert_eq!(kept(), Some(stored), "the Capitão's start kept the account's taste for when the hub can't answer");
}

#[test]
fn a_taste_chat_changes_its_own_accounts_taste_live_and_never_another_accounts() {
    let hub = TestHub::start("taste_chat");
    let url = hub.url.clone();
    let start = |email: &str, name: &str| {
        let (token, account) = account::signup(&url, email, "correct horse").unwrap();
        let mut client = Client::connect(&url, &token);
        client.wait("hello", |f| f["type"] == "hello");
        let dir = temp(&format!("taste-chat-{name}"));
        let (machine, machine_token) = account::register_machine(&url, &token, name, "macos").unwrap();
        let creds = Credentials { hub: url.clone(), email: account.email, user_token: token, machine_id: machine.id, machine_token };
        creds.save(&dir).unwrap();
        let node = Node::start(&dir, creds);
        node.machine.listen(|_| Err("unexpected".into())).unwrap();
        client.wait_machines(&[(name, true)]);
        (client, node, dir)
    };
    let (mut ana, ana_node, ana_dir) = start("ana@acme.dev", "ana-mac");
    let (mut bob, bob_node, bob_dir) = start("bob@acme.dev", "bob-mac");
    let get = || json!({ "cmd": "get_tastes" });
    let site = TasteScope::Project { repo: "github.com/acme/site".into() };
    let open_chat = |node: &Node, client: &mut Client| {
        let tab = node.machine.call(Call::OpenTasteChat { scope: site.clone(), path: None }).unwrap().as_str().unwrap().to_owned();
        client.wait("the chat's tab at the hub", |f| f["type"] == "event" && f["event"]["kind"] == "tabs" && f["event"]["tabs"].as_array().unwrap().iter().any(|t| t["id"] == tab && t["taste_chat"] == true));
        tab
    };
    let set = |parallel: u32| json!({ "name": "taste_set", "scope": "project:github.com/acme/site", "patch": { "parallel": parallel } });
    let read = json!({ "name": "taste_get", "scope": "project:github.com/acme/site" });

    let plain = ana_node.machine.open_tab(Some(ana_dir.display().to_string()), false).unwrap();
    assert_eq!(session_tool(&ana_dir, &plain, set(2)), Err("only a thread opened with Chat to adjust on the Way of working page has this tool".into()));
    assert_eq!(ana_node.link.as_ref().unwrap().call_hub(HubCall::SetTaste { scope: site.clone(), patch: TastePatch { parallel: Some(2), ..TastePatch::default() } }), Err("only a machine with a Chat to adjust thread open may change the way of working".into()), "a machine with no chat open can't");

    let chat = open_chat(&ana_node, &mut ana);
    let after = session_tool(&ana_dir, &chat, set(2)).unwrap();
    assert_eq!((&after["own"], &after["effective"]["parallel"]), (&json!({ "parallel": 2 }), &json!(2)));
    ana.wait("the page's cue to read again", |f| f["type"] == "event" && f["event"] == json!({ "kind": "tastes-changed" }));
    assert_eq!(ana.hub(get()).unwrap()["projects"], json!({ "github.com/acme/site": { "parallel": 2 } }), "the page's own read sees the session's change");
    assert_eq!(session_tool(&ana_dir, &chat, read.clone()), Ok(after.clone()));

    let bobs_set = HubCall::SetTaste { scope: site.clone(), patch: TastePatch { parallel: Some(7), ..TastePatch::default() } };
    assert_eq!(bob_node.link.as_ref().unwrap().call_hub(bobs_set), Err("only a machine with a Chat to adjust thread open may change the way of working".into()), "ana's open chat lets no machine of bob's change a taste");
    let bob_chat = open_chat(&bob_node, &mut bob);
    let bobs = session_tool(&bob_dir, &bob_chat, read.clone()).unwrap();
    assert_eq!((&bobs["own"], &bobs["effective"]["parallel"]), (&json!({}), &json!(3)), "bob's session reads none of ana's taste");
    session_tool(&bob_dir, &bob_chat, set(7)).unwrap();
    assert_eq!(bob.hub(get()).unwrap()["projects"], json!({ "github.com/acme/site": { "parallel": 7 } }));
    assert_eq!(session_tool(&ana_dir, &chat, read.clone()), Ok(after.clone()), "bob's set changed ana's taste");
    let reset = session_tool(&bob_dir, &bob_chat, json!({ "name": "taste_reset", "scope": "project:github.com/acme/site", "field": "parallel" })).unwrap();
    assert_eq!((&reset["own"], &reset["effective"]["parallel"]), (&json!({}), &json!(3)));
    assert_eq!(session_tool(&ana_dir, &chat, read), Ok(after), "bob's reset changed ana's taste");
    assert_eq!(ana.hub(get()).unwrap()["projects"], json!({ "github.com/acme/site": { "parallel": 2 } }));
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
    fn mark_executable(&self, project: &str, repairs: Vec<sync::ModeRepair>) -> Result<(), String> {
        self.0.call_hub(HubCall::SyncMarkExecutable { project: project.into(), repairs }).map(drop)
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
    let checkout = |home: &Path, folder: &str| sync::Checkout { project: project.clone(), path: home.join(folder), history: None, claude_home: Some(home.join(".claude")) };
    let (on_mac, on_mini) = (checkout(&mac_home, "projetos/app"), checkout(&mini_home, "pinned/app"));
    let memory = |c: &sync::Checkout| sync::claude_dir(c.claude_home.as_ref().unwrap(), &c.path).join("memory/MEMORY.md");
    std::fs::create_dir_all(memory(&on_mac).parent().unwrap()).unwrap();
    std::fs::write(memory(&on_mac), "- deploys go through the VPS").unwrap();

    let (mac_link, mini_link) = (mac.link.as_ref().unwrap(), mini.link.as_ref().unwrap());
    let now = std::time::SystemTime::now();
    let pushed = sync::sync(&Through(mac_link), &mut sync::State::default(), &on_mac, now).unwrap().pushed;
    assert_eq!(pushed, vec!["claude/memory/MEMORY.md"]);
    let pulled = sync::sync(&Through(mini_link), &mut sync::State::default(), &on_mini, now).unwrap().pulled;
    assert_eq!(pulled, vec!["claude/memory/MEMORY.md"]);
    assert_eq!(std::fs::read_to_string(memory(&on_mini)).unwrap(), "- deploys go through the VPS");

    let outside = sync::File { entry: sync::Entry { path: "../.ssh/authorized_keys".into(), sha: "x".into(), modified: 1, size: 1, executable: None }, data: Some("eA==".into()) };
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
fn a_node_reads_an_older_hubs_refusal_of_a_later_call_as_an_unknown_command() {
    let hub = TestHub::start("global_orchestrator");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let (_, machine_token) = account::register_machine(&url, &token, "mini", "macos").unwrap();
    let older = machine_result(&url, &machine_token, json!({ "cmd": "relay_from_a_later_app" })).unwrap_err();
    assert!(older.starts_with(&format!("this hub runs pinned {}, which can't answer that: unknown variant `relay_from_a_later_app`", env!("CARGO_PKG_VERSION"))), "{older}");
    assert!(link::is_unknown_command(&older));
}

#[test]
fn a_plugin_tree_is_stored_without_settings_env_or_claude_json() {
    let hub = TestHub::start("plugins");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let start = |name: &str| {
        let dir = temp(&format!("plugins-{name}"));
        let (machine, machine_token) = account::register_machine(&url, &token, name, "macos").unwrap();
        let creds = Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token };
        creds.save(&dir).unwrap();
        let node = Node::start(&dir, creds);
        wait_for("the machine online", || node.online());
        (node, dir)
    };
    let ((mac, mac_home), (mini, mini_home)) = (start("mac"), start("mini"));
    let install = mac_home.join("plugins/toy");
    let skill = "---\nname: greet\ndescription: Say hello\n---\n";
    std::fs::create_dir_all(install.join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(install.join("skills/greet")).unwrap();
    std::fs::write(install.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#).unwrap();
    std::fs::write(install.join("skills/greet/SKILL.md"), skill).unwrap();
    std::fs::write(install.join("settings.json"), "token=1").unwrap();
    std::fs::write(install.join(".env"), "SECRET=1").unwrap();
    std::fs::write(install.join(".claude.json"), "{}").unwrap();
    let installed = serde_json::json!({ "plugins": { "toy@demo": [{ "installPath": install, "version": "1.2.3", "gitCommitSha": "abc123" }] } });
    std::fs::create_dir_all(mac_home.join(".claude/plugins")).unwrap();
    std::fs::write(mac_home.join(".claude/settings.json"), r#"{"enabledPlugins":{"toy@demo":true}}"#).unwrap();
    std::fs::write(mac_home.join(".claude/plugins/installed_plugins.json"), installed.to_string()).unwrap();
    let share = |home: &Path| sync::PluginShare { claude_home: home.join(".claude"), dir: home.join("synced-plugins") };
    let now = std::time::SystemTime::now();
    let (mac_link, mini_link) = (mac.link.as_ref().unwrap(), mini.link.as_ref().unwrap());
    sync::sync_plugins(&Through(mac_link), &mut sync::State::default(), &share(&mac_home), now).unwrap();
    let stored = stored_plugin_paths(&hub.database);
    assert_eq!(
        stored,
        vec![
            "plugins/toy@demo/.claude-plugin/plugin.json".to_owned(),
            "plugins/toy@demo/.pinned-plugin.json".to_owned(),
            "plugins/toy@demo/skills/greet/SKILL.md".to_owned(),
        ]
    );
    let mut pulled = sync::sync_plugins(&Through(mini_link), &mut sync::State::default(), &share(&mini_home), now).unwrap().pulled;
    pulled.sort();
    assert_eq!(pulled, stored);
    assert_eq!(std::fs::read_to_string(mini_home.join("synced-plugins/plugins/toy@demo/skills/greet/SKILL.md")).unwrap(), skill);
    let secret = |path: &str| sync::File { entry: sync::Entry { path: path.into(), sha: "x".into(), modified: 1, size: 1, executable: None }, data: Some("eA==".into()) };
    for path in [
        "plugins/toy@demo/settings.json",
        "plugins/toy@demo/.claude.json",
        "plugins/toy@demo/.env",
        "plugins/toy@demo/Settings.json",
        "plugins/toy@demo/.ENV",
        "plugins/toy@demo/.env.local",
        "plugins/toy@demo/settings.local.json",
        "plugins/toy@demo/cert.PEM",
        "plugins/toy@demo/Secrets/token.txt",
        "plugins/toy@demo/../../.ssh/id_rsa",
        "/etc/passwd",
    ] {
        assert_eq!(
            mac_link.call_hub(HubCall::SyncPush { project: sync::PLUGINS_KEY.into(), files: vec![secret(path)] }),
            Err(format!("{path} is not a file a project shares"))
        );
    }
    let raw = vec![7u8; sync::FILE_LIMIT as usize + 1];
    let oversized = sync::File {
        entry: sync::Entry { path: "plugins/toy@demo/skills/greet/blob.bin".into(), sha: "x".into(), modified: 1, size: raw.len() as u64, executable: None },
        data: Some(base64::engine::general_purpose::STANDARD.encode(&raw)),
    };
    assert_eq!(
        mac_link.call_hub(HubCall::SyncPush { project: sync::PLUGINS_KEY.into(), files: vec![oversized] }),
        Err("plugins/toy@demo/skills/greet/blob.bin is bigger than a synced file may be".into())
    );
    assert_eq!(stored_plugin_paths(&hub.database), stored);
    let (mut mac, mut mini) = (mac, mini);
    mac.kill();
    mini.kill();
}

#[test]
fn an_executable_hook_stays_executable_through_the_hub() {
    let hub = TestHub::start("hookmode");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let start = |name: &str| {
        let dir = temp(&format!("hookmode-{name}"));
        let (machine, machine_token) = account::register_machine(&url, &token, name, "linux").unwrap();
        let creds = Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token };
        creds.save(&dir).unwrap();
        let node = Node::start(&dir, creds);
        wait_for("the machine online", || node.online());
        (node, dir)
    };
    let ((mac, mac_home), (mini, mini_home)) = (start("mac"), start("mini"));
    let install = mac_home.join("plugins/pstack");
    let hook = install.join("hooks/session-start.sh");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::create_dir_all(install.join(".claude-plugin")).unwrap();
    std::fs::write(install.join(".claude-plugin/plugin.json"), r#"{"name":"pstack"}"#).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho session-start\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let installed = serde_json::json!({ "plugins": { "pstack@local": [{ "installPath": install, "version": "0.9.82", "gitCommitSha": "abc123" }] } });
    std::fs::create_dir_all(mac_home.join(".claude/plugins")).unwrap();
    std::fs::write(mac_home.join(".claude/settings.json"), r#"{"enabledPlugins":{"pstack@local":true}}"#).unwrap();
    std::fs::write(mac_home.join(".claude/plugins/installed_plugins.json"), installed.to_string()).unwrap();
    let share = |home: &Path| sync::PluginShare { claude_home: home.join(".claude"), dir: home.join("synced-plugins") };
    let now = std::time::SystemTime::now();
    let (mac_link, mini_link) = (mac.link.as_ref().unwrap(), mini.link.as_ref().unwrap());
    let mut mac_state = sync::State::default();
    let mut mini_state = sync::State::default();
    sync::sync_plugins(&Through(mac_link), &mut mac_state, &share(&mac_home), now).unwrap();
    sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    let dest = mini_home.join("synced-plugins/plugins/pstack@local/hooks/session-start.sh");
    let plain = mini_home.join("synced-plugins/plugins/pstack@local/.claude-plugin/plugin.json");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho session-start\n");
    assert_eq!(mode(&dest) & 0o111, 0o100, "receiver hook mode is {:o}", mode(&dest));
    assert_eq!(mode(&plain) & 0o111, 0, "plain file mode is {:o}", mode(&plain));
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(mode(&dest), 0o644);
    sync::sync_plugins(&Through(mac_link), &mut mac_state, &share(&mac_home), now).unwrap();
    sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    assert_eq!(mode(&dest) & 0o111, 0o100, "resync left the hook at {:o}", mode(&dest));
    let again = sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    assert_eq!(again.pushed, Vec::<String>::new());
    assert_eq!(again.pulled, Vec::<String>::new());
    let (mut mac, mut mini) = (mac, mini);
    mac.kill();
    mini.kill();
}

const HOOK_PATH: &str = "plugins/pstack@local/hooks/session-start.sh";
const STOP_PATH: &str = "plugins/pstack@local/hooks/stop.sh";

fn call_machine(hub: &str, token: &str, call: Value) -> Value {
    machine_result(hub, token, call).unwrap_or_else(|err| panic!("{err}"))
}

fn machine_result(hub: &str, token: &str, call: Value) -> Result<Value, String> {
    let (mut ws, _) = tungstenite::connect(format!("{}/ws/machine", hub.replace("http://", "ws://"))).unwrap();
    if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    }
    ws.send(Message::text(json!({ "type": "auth", "token": token, "name": "raw", "os": "linux", "version": "test" }).to_string())).unwrap();
    let hello = read_machine(&mut ws, |frame| frame["type"] == "hello");
    assert!(hello["machine"]["id"].is_string(), "{hello}");
    ws.send(Message::text(json!({ "type": "call", "id": 1, "call": call }).to_string())).unwrap();
    let reply = read_machine(&mut ws, |frame| frame["type"] == "reply" && frame["id"] == 1);
    match reply.get("err") {
        Some(err) => Err(err.as_str().unwrap_or("hub call failed").to_string()),
        None => Ok(reply["ok"].clone()),
    }
}

fn read_machine(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, pred: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        match ws.read() {
            Ok(Message::Text(text)) => {
                let frame: Value = serde_json::from_str(&text).unwrap();
                if pred(&frame) {
                    return frame;
                }
            }
            Ok(Message::Ping(payload)) => ws.send(Message::Pong(payload)).unwrap(),
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock || error.kind() == std::io::ErrorKind::TimedOut => {
                assert!(start.elapsed() < WAIT, "timed out waiting for the machine socket");
            }
            Err(error) => panic!("{error}"),
        }
    }
}

fn gzip_bytes(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hook_entry(link: &Link) -> Value {
    let listed = link.call_hub(HubCall::SyncList { project: sync::PLUGINS_KEY.into() }).unwrap();
    listed.as_array().unwrap().iter().find(|row| row["path"] == HOOK_PATH).unwrap().clone()
}

#[test]
fn an_omitted_executable_field_keeps_the_stored_bit_through_the_hub() {
    let hub = TestHub::start("hookomit");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let (_, raw_token) = account::register_machine(&url, &token, "raw", "linux").unwrap();
    let start = |name: &str| {
        let dir = temp(&format!("hookomit-{name}"));
        let (machine, machine_token) = account::register_machine(&url, &token, name, "linux").unwrap();
        let creds = Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token };
        creds.save(&dir).unwrap();
        let node = Node::start(&dir, creds);
        wait_for("the machine online", || node.online());
        (node, dir)
    };
    let ((mac, mac_home), (mini, mini_home)) = (start("mac"), start("mini"));
    let install = mac_home.join("plugins/pstack");
    let hook = install.join("hooks/session-start.sh");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::create_dir_all(install.join(".claude-plugin")).unwrap();
    std::fs::write(install.join(".claude-plugin/plugin.json"), r#"{"name":"pstack"}"#).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho session-start\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let installed = serde_json::json!({ "plugins": { "pstack@local": [{ "installPath": install, "version": "0.9.82", "gitCommitSha": "abc123" }] } });
    std::fs::create_dir_all(mac_home.join(".claude/plugins")).unwrap();
    std::fs::write(mac_home.join(".claude/settings.json"), r#"{"enabledPlugins":{"pstack@local":true}}"#).unwrap();
    std::fs::write(mac_home.join(".claude/plugins/installed_plugins.json"), installed.to_string()).unwrap();
    let share = |home: &Path| sync::PluginShare { claude_home: home.join(".claude"), dir: home.join("synced-plugins") };
    let now = std::time::SystemTime::now();
    let (mac_link, mini_link) = (mac.link.as_ref().unwrap(), mini.link.as_ref().unwrap());
    let mut mac_state = sync::State::default();
    let mut mini_state = sync::State::default();
    sync::sync_plugins(&Through(mac_link), &mut mac_state, &share(&mac_home), now).unwrap();
    sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    let dest = mini_home.join("synced-plugins").join(HOOK_PATH);
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dest) & 0o100, 0o100, "receiver hook mode is {:o}", mode(&dest));

    let pulled = sync::Hub::pull(&Through(mac_link), sync::PLUGINS_KEY, vec![HOOK_PATH.into()]).unwrap();
    let mut same = serde_json::to_value(&pulled[0]).unwrap();
    same.as_object_mut().unwrap().remove("executable");
    assert!(same.get("executable").is_none());
    call_machine(&url, &raw_token, json!({ "cmd": "sync_push", "project": sync::PLUGINS_KEY, "files": [same] }));
    assert_eq!(hook_entry(mac_link)["executable"], json!(true));
    let quiet = sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    assert_eq!((quiet.pushed.as_slice(), quiet.pulled.as_slice()), (&[][..], &[][..]));
    assert_eq!(mode(&dest) & 0o100, 0o100, "same bytes without the field left mode {:o}", mode(&dest));

    let body = b"#!/bin/sh\necho session-start\necho again\n";
    let packed = gzip_bytes(body);
    let modified = hook_entry(mac_link)["modified"].as_u64().unwrap() + 60_000;
    call_machine(
        &url,
        &raw_token,
        json!({
            "cmd": "sync_push",
            "project": sync::PLUGINS_KEY,
            "files": [{
                "path": HOOK_PATH,
                "sha": sha256_hex(body),
                "modified": modified,
                "size": packed.len(),
                "data": base64::engine::general_purpose::STANDARD.encode(&packed),
            }],
        }),
    );
    assert_eq!(hook_entry(mac_link)["executable"], json!(true));
    sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert_eq!(mode(&dest) & 0o100, 0o100, "new bytes without the field left mode {:o}", mode(&dest));
    let again = sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    assert_eq!((again.pushed.as_slice(), again.pulled.as_slice()), (&[][..], &[][..]));
    let (mut mac, mut mini) = (mac, mini);
    mac.kill();
    mini.kill();
}

#[test]
fn a_new_sha_with_executable_false_clears_the_bit_on_the_receiver() {
    let hub = TestHub::start("hookclear");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let start = |name: &str| {
        let dir = temp(&format!("hookclear-{name}"));
        let (machine, machine_token) = account::register_machine(&url, &token, name, "linux").unwrap();
        let creds = Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token };
        creds.save(&dir).unwrap();
        let node = Node::start(&dir, creds);
        wait_for("the machine online", || node.online());
        (node, dir)
    };
    let ((mac, mac_home), (mini, mini_home)) = (start("mac"), start("mini"));
    let install = mac_home.join("plugins/pstack");
    let hook = install.join("hooks/session-start.sh");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::create_dir_all(install.join(".claude-plugin")).unwrap();
    std::fs::write(install.join(".claude-plugin/plugin.json"), r#"{"name":"pstack"}"#).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho session-start\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let installed = serde_json::json!({ "plugins": { "pstack@local": [{ "installPath": install, "version": "0.9.82", "gitCommitSha": "abc123" }] } });
    std::fs::create_dir_all(mac_home.join(".claude/plugins")).unwrap();
    std::fs::write(mac_home.join(".claude/settings.json"), r#"{"enabledPlugins":{"pstack@local":true}}"#).unwrap();
    std::fs::write(mac_home.join(".claude/plugins/installed_plugins.json"), installed.to_string()).unwrap();
    let share = |home: &Path| sync::PluginShare { claude_home: home.join(".claude"), dir: home.join("synced-plugins") };
    let now = std::time::SystemTime::now();
    let (mac_link, mini_link) = (mac.link.as_ref().unwrap(), mini.link.as_ref().unwrap());
    let mut mac_state = sync::State::default();
    let mut mini_state = sync::State::default();
    sync::sync_plugins(&Through(mac_link), &mut mac_state, &share(&mac_home), now).unwrap();
    sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    let dest = mini_home.join("synced-plugins").join(HOOK_PATH);
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(hook_entry(mac_link)["executable"], json!(true));
    assert_eq!(mode(&dest) & 0o100, 0o100, "receiver hook mode is {:o}", mode(&dest));
    let changed = "#!/bin/sh\necho changed\n";
    std::fs::write(&hook, changed).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o644)).unwrap();
    stamp_newer(&hook);
    sync::sync_plugins(&Through(mac_link), &mut mac_state, &share(&mac_home), now).unwrap();
    assert_eq!(hook_entry(mac_link)["executable"], json!(false));
    assert_eq!(hook_entry(mac_link)["sha"], json!(sha256_hex(changed.as_bytes())));
    sync::sync_plugins(&Through(mini_link), &mut mini_state, &share(&mini_home), now).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), changed.as_bytes());
    assert_eq!(mode(&dest) & 0o111, 0, "cleared hook mode is {:o}", mode(&dest));
    let (mut mac, mut mini) = (mac, mini);
    mac.kill();
    mini.kill();
}

#[test]
fn a_mode_repair_keeps_a_newer_edit_and_marks_only_the_matching_sha() {
    let hub = TestHub::start("moderepair");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let (_, raw_token) = account::register_machine(&url, &token, "raw", "linux").unwrap();
    let older = b"#!/bin/sh\necho session-start\n";
    let newer = b"#!/bin/sh\necho edited\n";
    push_plugin_file(&url, &raw_token, older, 100, false);
    push_plugin_file(&url, &raw_token, newer, 200, false);
    let stale = machine_result(
        &url,
        &raw_token,
        json!({
            "cmd": "sync_mark_executable",
            "project": sync::PLUGINS_KEY,
            "repairs": [{ "path": HOOK_PATH, "sha": sha256_hex(older) }],
        }),
    );
    assert_eq!(stale, Ok(Value::Null));
    let newer_gz = gzip_bytes(newer);
    let row = stored_hook_row(&hub.database);
    assert_eq!(row.sha, sha256_hex(newer));
    assert_eq!(row.modified, 200);
    assert_eq!(row.executable, false);
    assert_eq!(row.data, newer_gz);
    let marked = machine_result(
        &url,
        &raw_token,
        json!({
            "cmd": "sync_mark_executable",
            "project": sync::PLUGINS_KEY,
            "repairs": [{ "path": HOOK_PATH, "sha": sha256_hex(newer) }],
        }),
    );
    assert_eq!(marked, Ok(Value::Null));
    let row = stored_hook_row(&hub.database);
    assert_eq!(row.sha, sha256_hex(newer));
    assert_eq!(row.modified, 200);
    assert_eq!(row.data, newer_gz);
    assert_eq!(row.executable, true);
    let listed = machine_result(&url, &raw_token, json!({ "cmd": "sync_list", "project": sync::PLUGINS_KEY })).unwrap();
    let entry = listed.as_array().unwrap().iter().find(|item| item["path"] == HOOK_PATH).unwrap();
    assert_eq!(entry["executable"], json!(true));
    assert_eq!(entry["modified"], json!(200));
    assert_eq!(entry["sha"], json!(sha256_hex(newer)));
}

struct RacingLink<'a> {
    link: &'a Link,
    ahead: Mutex<Option<sync::File>>,
}

impl sync::Hub for RacingLink<'_> {
    fn list(&self, project: &str) -> Result<Vec<sync::Entry>, String> {
        sync::Hub::list(self.link, project)
    }
    fn push(&self, project: &str, files: Vec<sync::File>) -> Result<(), String> {
        sync::Hub::push(self.link, project, files)
    }
    fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<sync::File>, String> {
        sync::Hub::pull(self.link, project, paths)
    }
    fn mark_executable(&self, project: &str, repairs: Vec<sync::ModeRepair>) -> Result<(), String> {
        if let Some(newer) = self.ahead.lock().unwrap().take() {
            sync::Hub::push(self.link, project, vec![newer])?;
        }
        sync::Hub::mark_executable(self.link, project, repairs)
    }
}

fn plugin_hook(body: &[u8], modified: u64, executable: bool) -> sync::File {
    let packed = gzip_bytes(body);
    sync::File {
        entry: sync::Entry { path: HOOK_PATH.into(), sha: sha256_hex(body), modified, size: packed.len() as u64, executable: Some(executable) },
        data: Some(base64::engine::general_purpose::STANDARD.encode(packed)),
    }
}

#[test]
fn a_node_marks_the_executable_bit_and_keeps_a_newer_edit() {
    let hub = TestHub::start("noderepair");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let start = |name: &str| {
        let dir = temp(&format!("noderepair-{name}"));
        let (machine, machine_token) = account::register_machine(&url, &token, name, "linux").unwrap();
        let creds = Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token };
        creds.save(&dir).unwrap();
        let node = Node::start(&dir, creds);
        wait_for("the machine online", || node.online());
        (node, dir)
    };
    let ((mac, mac_home), (mini, mini_home)) = (start("mac"), start("mini"));
    let body = b"#!/bin/sh\necho session-start\n";
    let install = |home: &Path, mode: u32, secs: u64| {
        let plugin = home.join("plugins/pstack");
        std::fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        std::fs::write(plugin.join(".claude-plugin/plugin.json"), r#"{"name":"pstack"}"#).unwrap();
        for rel in ["hooks/session-start.sh", "hooks/stop.sh"] {
            let hook = plugin.join(rel);
            std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
            std::fs::write(&hook, body).unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(mode)).unwrap();
            let when = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
            std::fs::File::options().write(true).open(&hook).unwrap().set_modified(when).unwrap();
        }
        let installed = serde_json::json!({ "plugins": { "pstack@local": [{ "installPath": plugin, "version": "0.9.82", "gitCommitSha": "abc123" }] } });
        std::fs::create_dir_all(home.join(".claude/plugins")).unwrap();
        std::fs::write(home.join(".claude/settings.json"), r#"{"enabledPlugins":{"pstack@local":true}}"#).unwrap();
        std::fs::write(home.join(".claude/plugins/installed_plugins.json"), installed.to_string()).unwrap();
    };
    install(&mac_home, 0o644, 1_700_000_000);
    install(&mini_home, 0o755, 1_600_000_000);
    let share = |home: &Path| sync::PluginShare { claude_home: home.join(".claude"), dir: home.join("synced-plugins") };
    let now = std::time::SystemTime::now();
    let (mac_link, mini_link) = (mac.link.as_ref().unwrap(), mini.link.as_ref().unwrap());
    let mut mac_state = sync::State::default();
    let mut mini_state = sync::State::default();
    sync::sync_plugins(mac_link, &mut mac_state, &share(&mac_home), now).unwrap();
    let repaired = sync::sync_plugins(mini_link, &mut mini_state, &share(&mini_home), now).unwrap();
    assert!(!repaired.pushed.iter().any(|path| path == HOOK_PATH), "a mode repair sends no file bytes");
    let row = stored_hook_row(&hub.database);
    assert_eq!(row.sha, sha256_hex(body));
    assert_eq!(row.modified, 1_700_000_000_000);
    assert_eq!(row.data, gzip_bytes(body));
    assert_eq!(row.executable, true);
    admin_on(
        &hub.database,
        "update project_files set executable = false where project = 'pinned:plugins' and path in ('plugins/pstack@local/hooks/session-start.sh', 'plugins/pstack@local/hooks/stop.sh')",
    );
    let newer = b"#!/bin/sh\necho edited\n";
    let racing = RacingLink { link: mini_link, ahead: Mutex::new(Some(plugin_hook(newer, 1_800_000_000_000, false))) };
    sync::sync_plugins(&racing, &mut mini_state, &share(&mini_home), now).unwrap();
    let row = stored_hook_row(&hub.database);
    assert_eq!(row.sha, sha256_hex(newer));
    assert_eq!(row.modified, 1_800_000_000_000);
    assert_eq!(row.data, gzip_bytes(newer));
    assert_eq!(row.executable, false);
    let stop = stored_plugin_row(&hub.database, STOP_PATH);
    assert_eq!(stop.sha, sha256_hex(body));
    assert_eq!(stop.modified, 1_700_000_000_000);
    assert_eq!(stop.data, gzip_bytes(body));
    assert_eq!(stop.executable, true);
    let (mut mac, mut mini) = (mac, mini);
    mac.kill();
    mini.kill();
}

fn push_plugin_file(hub: &str, token: &str, body: &[u8], modified: u64, executable: bool) {
    let packed = gzip_bytes(body);
    call_machine(
        hub,
        token,
        json!({
            "cmd": "sync_push",
            "project": sync::PLUGINS_KEY,
            "files": [{
                "path": HOOK_PATH,
                "sha": sha256_hex(body),
                "modified": modified,
                "size": packed.len(),
                "executable": executable,
                "data": base64::engine::general_purpose::STANDARD.encode(&packed),
            }],
        }),
    );
}

fn stamp_newer(path: &Path) {
    let later = std::fs::metadata(path).unwrap().modified().unwrap() + Duration::from_secs(120);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
}

struct HookRow {
    sha: String,
    modified: i64,
    executable: bool,
    data: Vec<u8>,
}

fn stored_hook_row(database: &str) -> HookRow {
    stored_plugin_row(database, HOOK_PATH)
}

fn stored_plugin_row(database: &str, path: &str) -> HookRow {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{database}", postgres()), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        let row = client
            .query_one("select sha, modified, executable, data from project_files where project = $1 and path = $2", &[&sync::PLUGINS_KEY, &path])
            .await
            .unwrap();
        HookRow { sha: row.get(0), modified: row.get(1), executable: row.get(2), data: row.get(3) }
    })
}

fn stored_plugin_paths(database: &str) -> Vec<String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{database}", postgres()), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        let rows = client.query("select path from project_files where project = $1 order by path", &[&sync::PLUGINS_KEY]).await.unwrap();
        rows.iter().map(|row| row.get(0)).collect()
    })
}

fn dictation(raw: &str) -> DictationFeedback {
    DictationFeedback {
        rating: DictationRating::Bad,
        raw: raw.into(),
        cleaned: raw.replace("comet", "commit"),
        final_text: format!("please {raw}"),
        locale: "en-US".into(),
        engine: DictationEngine::Native,
        app_version: "0.17.15".into(),
        platform: "macos".into(),
        duration_ms: 4200,
    }
}

fn learning_rows(database: &str) -> Vec<Value> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{database}", postgres()), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        let rows = client
            .query("select account::text, rating, raw, cleaned, final_text, locale, engine, app_version, platform, duration_ms, created_at > now() - interval '1 minute' from dictation_feedback", &[])
            .await
            .unwrap();
        rows.iter()
            .map(|r| {
                json!({ "account": r.get::<_, String>(0), "rating": r.get::<_, String>(1), "raw": r.get::<_, String>(2), "cleaned": r.get::<_, String>(3),
                    "final_text": r.get::<_, String>(4), "locale": r.get::<_, String>(5), "engine": r.get::<_, String>(6), "app_version": r.get::<_, String>(7),
                    "platform": r.get::<_, String>(8), "duration_ms": r.get::<_, i64>(9), "just_now": r.get::<_, bool>(10) })
            })
            .collect()
    })
}

#[test]
fn a_shared_dictation_lands_in_the_learning_database_under_its_account() {
    let hub = TestHub::start_learning("dictation");
    let url = hub.url.clone();
    let learning = hub.learning.clone().unwrap();
    let (token, ana) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();

    assert_eq!(account::dictation_feedback(&url, "not-a-token", &dictation("add a comet")), Err("sign in first".into()));
    account::dictation_feedback(&url, &token, &dictation("add a comet\nto the branch")).unwrap();
    assert_eq!(
        learning_rows(&learning),
        [json!({ "account": ana.id, "rating": "bad", "raw": "add a comet\nto the branch", "cleaned": "add a commit\nto the branch",
            "final_text": "please add a comet\nto the branch", "locale": "en-US", "engine": "native", "app_version": "0.17.15",
            "platform": "macos", "duration_ms": 4200, "just_now": true })]
    );

    let refused = [
        (DictationFeedback { raw: "x".repeat(20_001), ..dictation("") }, "raw must be text of up to 20000 characters"),
        (DictationFeedback { final_text: "a\0b".into(), ..dictation("hi") }, "final_text must be text of up to 20000 characters"),
        (dictation("  "), "raw is empty: there is no dictation to keep"),
        (DictationFeedback { locale: "en-US\nDROP".into(), ..dictation("hi") }, "locale must be one line of 1 to 40 characters"),
        (DictationFeedback { platform: String::new(), ..dictation("hi") }, "platform must be one line of 1 to 40 characters"),
        (DictationFeedback { duration_ms: 86_400_001, ..dictation("hi") }, "duration_ms is longer than a day"),
    ];
    for (feedback, why) in refused {
        assert_eq!(account::dictation_feedback(&url, &token, &feedback), Err(why.into()));
    }
    let mut unknown = serde_json::to_value(dictation("hi")).unwrap();
    unknown["rating"] = json!("meh");
    let answer = reqwest::blocking::Client::new().post(format!("{url}/api/dictation/feedback")).bearer_auth(&token).json(&unknown).send().unwrap();
    assert_eq!(answer.status(), 422, "a rating other than good or bad is refused");
    assert_eq!(learning_rows(&learning).len(), 1, "nothing refused was kept");
}

#[test]
fn a_hub_without_a_learning_database_keeps_no_dictation() {
    let hub = TestHub::start("no_learning");
    let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    assert_eq!(
        account::dictation_feedback(&hub.url, &token, &dictation("add a comet")),
        Err("this hub keeps no dictation feedback: it runs without a learning database".into())
    );
    let answer = reqwest::blocking::Client::new().post(format!("{}/api/dictation/feedback", hub.url)).bearer_auth(&token).json(&dictation("hi")).send().unwrap();
    assert_eq!(answer.status(), 503);
    let anonymous = reqwest::blocking::Client::new().post(format!("{}/api/dictation/feedback", hub.url)).json(&dictation("hi")).send().unwrap();
    assert_eq!(anonymous.status(), 401);
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
    let checkout = |home: &Path| sync::Checkout { project: project.clone(), path: home.join("app"), history: None, claude_home: Some(home.join(".claude")) };
    let session = |home: &Path| sync::claude_dir(&home.join(".claude"), &home.join("app")).join("session.jsonl");
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
    let pushed = sync::sync(&Through(mac.link.as_ref().unwrap()), &mut sync::State::default(), &checkout(&mac_home), now);
    assert_eq!(pushed.map(|s| s.pushed), Ok(vec!["claude/session.jsonl".to_owned()]));
    let pulled = sync::sync(&Through(mini.link.as_ref().unwrap()), &mut sync::State::default(), &checkout(&mini_home), now);
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
    let expected = json!({ "type": "event", "machine": creds.machine_id, "event": { "kind": "usage", "usage": { "five_hour": { "used_percentage": 23.5, "resets_at": 4102444800u64 } }, "account": "claude" } });
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
    let hub = TestHub::start_with("closed", Signup::First);
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

/// `POST /api/signup` as a browser behind the hub's proxy at `address` sends it: the status and the body.
fn signup_from(url: &str, address: &str, body: Value) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .post(format!("{url}/api/signup"))
        .header("content-type", "application/json")
        .header("x-forwarded-for", address)
        .body(body.to_string())
        .send()
        .unwrap();
    let status = response.status().as_u16();
    (status, serde_json::from_str(&response.text().unwrap()).unwrap())
}

const INVITE_NEEDED: &str = "this hub needs a valid invite code to create an account";

fn refused_signup() -> (u16, Value) {
    (403, json!({ "error": INVITE_NEEDED }))
}

fn signed_in_client(url: &str, token: &str) -> Client {
    let mut client = Client::connect(url, token);
    client.wait("hello", |f| f["type"] == "hello");
    client
}

#[test]
fn an_invite_hub_takes_its_first_account_then_only_single_use_codes() {
    let hub = TestHub::start_with("invite", Signup::Invite);
    let url = hub.url.clone();
    let info = reqwest::blocking::get(format!("{url}/api/signup")).unwrap();
    assert_eq!(info.headers()["access-control-allow-origin"], "*", "the app's sign-in form reads it from another origin");
    assert_eq!(serde_json::from_str::<Value>(&info.text().unwrap()).unwrap(), json!({ "signup": "invite" }));

    let (status, ana) = signup_from(&url, "10.0.0.1", json!({ "email": "ana@acme.dev", "password": "correct horse" }));
    assert_eq!(status, 200, "the first account needs no code: {ana}");
    let ana_token = ana["token"].as_str().unwrap().to_owned();
    let ana_id = ana["account"]["id"].as_str().unwrap().to_owned();

    assert_eq!(signup_from(&url, "10.0.0.2", json!({ "email": "bruno@acme.dev", "password": "correct horse" })), refused_signup());
    assert_eq!(signup_from(&url, "10.0.0.2", json!({ "email": "ana@acme.dev", "password": "correct horse" })), refused_signup(), "a taken email gets the same answer");
    assert_eq!(signup_from(&url, "10.0.0.2", json!({ "email": "bruno@acme.dev", "password": "correct horse", "invite": "AAAA-BBBB-CCCC-DDDD" })), refused_signup());
    assert_eq!(signup_from(&url, "10.0.0.2", json!({ "email": "bruno@acme.dev", "password": "correct horse", "invite": "  " })), refused_signup());

    let mut ana_client = signed_in_client(&url, &ana_token);
    assert_eq!(ana_client.hub(json!({ "cmd": "invites" })), Ok(json!({ "signup": "invite", "pending": [] })));
    let invite = ana_client.hub(json!({ "cmd": "create_invite" })).unwrap();
    let code = invite["code"].as_str().unwrap().to_owned();
    assert_eq!((code.len(), code.matches('-').count()), (19, 3), "four groups of four: {code}");
    let week = invite["expires_at"].as_i64().unwrap() - std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    assert!((7 * 86400 - 60..=7 * 86400 + 60).contains(&week), "a code lasts 7 days, not {week} s");
    let pending = ana_client.hub(json!({ "cmd": "invites" })).unwrap();
    assert_eq!(pending["pending"].as_array().unwrap().len(), 1);
    assert_eq!(pending["pending"][0]["id"], invite["id"]);
    assert_eq!(pending["pending"][0]["hint"], code[..4]);
    assert_eq!(pending["pending"][0]["expires_at"], invite["expires_at"]);

    assert_eq!(
        signup_from(&url, "10.0.0.3", json!({ "email": "ana@acme.dev", "password": "correct horse", "invite": code })),
        (409, json!({ "error": "an account with that email already exists" })),
    );
    let typed = format!(" {} ", code.replace('-', " ").to_lowercase());
    let (status, bruno) = signup_from(&url, "10.0.0.3", json!({ "email": "bruno@acme.dev", "password": "correct horse", "invite": typed }));
    assert_eq!(status, 200, "a taken email left the code unused, and case and spacing don't matter: {bruno}");
    let bruno_token = bruno["token"].as_str().unwrap().to_owned();
    assert_eq!(bruno["account"]["email"], "bruno@acme.dev");
    assert_ne!(bruno["account"]["id"], ana_id, "the code makes a new account, not a login on Ana's");
    assert_eq!(account::login(&url, "bruno@acme.dev", "correct horse").unwrap().1.id, bruno["account"]["id"].as_str().unwrap());
    assert_eq!(account::login(&url, "ana@acme.dev", "correct horse").unwrap().1.id, ana_id);
    assert_eq!(signup_from(&url, "10.0.0.4", json!({ "email": "carla@acme.dev", "password": "correct horse", "invite": code })), refused_signup(), "a code works once");
    assert_eq!(ana_client.hub(json!({ "cmd": "invites" })), Ok(json!({ "signup": "invite", "pending": [] })), "a used code is no longer pending");

    let expired = ana_client.hub(json!({ "cmd": "create_invite" })).unwrap();
    admin_on(&hub.database, &format!("update invites set expires_at = now() - interval '1 second' where id = '{}'", expired["id"].as_str().unwrap()));
    assert_eq!(signup_from(&url, "10.0.0.4", json!({ "email": "carla@acme.dev", "password": "correct horse", "invite": expired["code"] })), refused_signup(), "an expired code fails");
    assert_eq!(ana_client.hub(json!({ "cmd": "invites" })).unwrap()["pending"], json!([]));

    let revoked = ana_client.hub(json!({ "cmd": "create_invite" })).unwrap();
    let kept = ana_client.hub(json!({ "cmd": "create_invite" })).unwrap();
    let mut bruno_client = signed_in_client(&url, &bruno_token);
    let bruno_invite = bruno_client.hub(json!({ "cmd": "create_invite" })).unwrap();
    let revoke = |id: &Value| json!({ "cmd": "revoke_invite", "id": id });
    assert_eq!(bruno_client.hub(revoke(&revoked["id"])), Err(format!("this account has no unused invite {}", revoked["id"].as_str().unwrap())), "Bruno can't revoke Ana's code");
    assert_eq!(ana_client.hub(revoke(&revoked["id"])), Ok(Value::Null));
    assert_eq!(ana_client.hub(revoke(&revoked["id"])), Err(format!("this account has no unused invite {}", revoked["id"].as_str().unwrap())));
    assert_eq!(signup_from(&url, "10.0.0.5", json!({ "email": "carla@acme.dev", "password": "correct horse", "invite": revoked["code"] })), refused_signup(), "a revoked code fails");

    let ids = |list: Value| list["pending"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(ids(ana_client.hub(json!({ "cmd": "invites" })).unwrap()), [kept["id"].as_str().unwrap()], "Ana's list never shows Bruno's code");
    assert_eq!(ids(bruno_client.hub(json!({ "cmd": "invites" })).unwrap()), [bruno_invite["id"].as_str().unwrap()]);
    let (status, carla) = signup_from(&url, "10.0.0.5", json!({ "email": "carla@acme.dev", "password": "correct horse", "invite": bruno_invite["code"] }));
    assert_eq!(status, 200, "{carla}");
    assert_eq!(ids(bruno_client.hub(json!({ "cmd": "invites" })).unwrap()), Vec::<String>::new());
    assert_eq!(ids(ana_client.hub(json!({ "cmd": "invites" })).unwrap()), [kept["id"].as_str().unwrap()]);
}

#[test]
fn guessing_invite_codes_hits_the_sign_up_limit() {
    let hub = TestHub::start_with("invite_limit", Signup::Invite);
    let url = hub.url.clone();
    account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    for _ in 0..5 {
        assert_eq!(signup_from(&url, "10.0.0.9", json!({ "email": "eve@acme.dev", "password": "correct horse", "invite": "AAAA-BBBB-CCCC-DDDD" })), refused_signup());
    }
    assert_eq!(
        signup_from(&url, "10.0.0.9", json!({ "email": "eve@acme.dev", "password": "correct horse", "invite": "AAAA-BBBB-CCCC-DDDE" })),
        (429, json!({ "error": "too many sign ups; try again in 15 minutes" })),
    );
}

#[test]
fn only_an_invite_hub_makes_invite_codes() {
    for (name, signup, mode) in [("invite_open", Signup::Open, "open"), ("invite_first", Signup::First, "first")] {
        let hub = TestHub::start_with(name, signup);
        let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
        assert_eq!(reqwest::blocking::get(format!("{}/api/signup", hub.url)).unwrap().text().unwrap(), format!(r#"{{"signup":"{mode}"}}"#));
        let mut client = signed_in_client(&hub.url, &token);
        assert_eq!(client.hub(json!({ "cmd": "create_invite" })), Err("this hub takes no invites: anyone may sign up, or only its first account".into()));
        assert_eq!(client.hub(json!({ "cmd": "invites" })), Ok(json!({ "signup": mode, "pending": [] })));
    }
}

#[test]
fn two_invited_accounts_see_nothing_of_each_others_machines_projects_or_memory() {
    let hub = TestHub::start_with("invite_apart", Signup::Invite);
    let url = hub.url.clone();
    let (ana_token, ana) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut ana_client = signed_in_client(&url, &ana_token);
    let code = ana_client.hub(json!({ "cmd": "create_invite" })).unwrap()["code"].as_str().unwrap().to_owned();
    let (bruno_token, bruno) = account::signup_invited(&url, "bruno@acme.dev", "correct horse", Some(&code)).unwrap();
    assert_ne!(bruno.id, ana.id);
    let mut bruno_client = signed_in_client(&url, &bruno_token);

    let start = |client: &mut Client, token: &str, email: &str, name: &str| {
        let dir = temp(&format!("invite-apart-{name}"));
        let (machine, machine_token) = account::register_machine(&url, token, name, "linux").unwrap();
        let creds = Credentials { hub: url.clone(), email: email.into(), user_token: token.into(), machine_id: machine.id.clone(), machine_token };
        let node = Node::start(&dir, creds);
        client.wait_machines(&[(name, true)]);
        (node, dir, machine.id)
    };
    let (ana_node, ana_home, ana_machine) = start(&mut ana_client, &ana_token, "ana@acme.dev", "ana-mac");
    let (bruno_node, _bruno_home, bruno_machine) = start(&mut bruno_client, &bruno_token, "bruno@acme.dev", "bruno-mac");

    let secret = ana_home.join("secret-project");
    std::fs::create_dir_all(&secret).unwrap();
    let path = ana_node.machine.open_project(secret).display().to_string();
    let project = sync::Project { key: "github.com/ana/secret".into(), name: "secret".into(), repo: "git@github.com:ana/secret.git".into() };
    ana_node.link.as_ref().unwrap().call_hub(HubCall::ShareProject { project }).unwrap();
    let projects = ana_client.call(&ana_machine, json!({ "cmd": "list_projects" })).unwrap();
    assert!(projects.as_array().unwrap().iter().any(|p| p["path"] == path), "{projects}");
    assert_eq!(ana_client.hub(json!({ "cmd": "shared_projects" })).unwrap().as_array().unwrap().len(), 1);

    let names = |client: &mut Client| client.hub(json!({ "cmd": "machines" })).unwrap().as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(names(&mut bruno_client), ["bruno-mac"]);
    assert_eq!(names(&mut ana_client), ["ana-mac"]);
    assert_eq!(bruno_client.hub(json!({ "cmd": "shared_projects" })), Ok(json!([])), "Bruno sees none of Ana's projects");
    assert!(ana_client.call(&ana_machine, json!({ "cmd": "memory_status", "path": path })).is_ok(), "Ana reads her own project's memory");
    let refused = Err(format!("this account has no machine {ana_machine}"));
    assert_eq!(bruno_client.call(&ana_machine, json!({ "cmd": "list_projects" })), refused);
    assert_eq!(bruno_client.call(&ana_machine, json!({ "cmd": "tabs" })), refused);
    assert_eq!(bruno_client.call(&ana_machine, json!({ "cmd": "memory_status", "path": path })), refused, "Ana's memory lives on her machine, which Bruno can't reach");
    assert_eq!(ana_client.call(&bruno_machine, json!({ "cmd": "tabs" })), Err(format!("this account has no machine {bruno_machine}")));
    bruno_client.pump();
    let leaked: Vec<&Value> = bruno_client.frames.iter().filter(|f| f["type"] != "reply" && (f.to_string().contains(&ana_machine) || f.to_string().contains("secret"))).collect();
    assert!(leaked.is_empty(), "Bruno's socket carried Ana's machine or project: {leaked:?}");
    let (mut ana_node, mut bruno_node) = (ana_node, bruno_node);
    ana_node.kill();
    bruno_node.kill();
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
            let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            db.query_one("select secrets::text from accounts", &[]).await.unwrap().get::<_, String>(0)
        })
    };
    wait_for("the keys to be written", || sealed().contains("sealed:"));
    assert!(!sealed().contains("lin_api_secret"));
}

const BOX_KEY: &str = "box_testkey";
const CLAUDE_TOKEN: &str = "sk-ant-oat01-testtoken";
const BOX_DIR: &str = "/workspace/home/.pinned";

struct FakeBox {
    labels: Vec<String>,
    files: BTreeMap<String, String>,
    node: Option<Arc<Node>>,
}

#[derive(Default)]
struct FakeState {
    boxes: BTreeMap<String, FakeBox>,
    created: Vec<NewBox>,
    scripts: Vec<String>,
    /// Every account.json the hub wrote, oldest first.
    accounts: Vec<Credentials>,
    /// What the next bootstrap answers instead of booting a node.
    failing: Option<Exec>,
    /// The next bootstrap never answers, as when the hub dies during it. Other scripts answer at once.
    hanging: bool,
    /// A checkout each booted node already knows, as `(folder name, git origin)`, so the node finds
    /// the repo a worker or a Captain needs without cloning from the network.
    checkout: Option<(String, String)>,
}

/// Upstash's Box API in memory. Running the bootstrap starts a node from the account.json the hub
/// wrote, so a box boots only when what the hub writes is enough to sign in with.
struct FakeBoxes {
    root: PathBuf,
    state: Mutex<FakeState>,
}

impl FakeBoxes {
    fn new(root: PathBuf) -> Arc<FakeBoxes> {
        Arc::new(FakeBoxes { root, state: Mutex::default() })
    }

    fn live(&self) -> Vec<String> {
        self.state.lock().unwrap().boxes.keys().cloned().collect()
    }

    /// Adds a box as `box create --label …` would, outside pinned.
    fn add(&self, id: &str, labels: &[&str]) {
        let made = FakeBox { labels: labels.iter().map(|l| l.to_string()).collect(), files: BTreeMap::new(), node: None };
        self.state.lock().unwrap().boxes.insert(id.into(), made);
    }

    /// Runs `f` on the node a box runs, without holding the fake, which the hub may be waiting on.
    fn with_node<T>(&self, f: impl FnOnce(&Node) -> T) -> T {
        let node = self.state.lock().unwrap().boxes.values().find_map(|b| b.node.clone()).expect("a box runs a node");
        f(&node)
    }

    fn file(&self, path: &str) -> Option<String> {
        self.state.lock().unwrap().boxes.values().find_map(|b| b.files.get(path).cloned())
    }

    fn refuse(key: &str) -> Result<(), String> {
        (key == BOX_KEY).then_some(()).ok_or_else(|| "Upstash Box refused the key: Invalid box API key".into())
    }

    async fn boot(&self, id: &BoxId) -> Result<Exec, String> {
        let (dir, creds, checkout) = {
            let mut state = self.state.lock().unwrap();
            if let Some(failing) = state.failing.take() {
                return Ok(failing);
            }
            let fake = state.boxes.get(&id.0).ok_or("no such box")?;
            let creds: Credentials = serde_json::from_str(&fake.files[&format!("{BOX_DIR}/account.json")]).unwrap();
            assert!(fake.files[&format!("{BOX_DIR}/start.sh")].contains("pinned-server\" node --config-dir"));
            (self.root.join(&id.0), creds, state.checkout.clone())
        };
        std::fs::create_dir_all(&dir).unwrap();
        let node = tokio::task::spawn_blocking(move || {
            let node = Node::start(&dir, creds);
            // A delivery is a prompt, which the node holds until claude takes the terminal, as this stand-in does.
            std::fs::write(dir.join("claude"), "#!/bin/sh\n[ \"$1 $2\" = \"auth status\" ] && { echo '{\"loggedIn\":true}'; exit 0; }\nstty raw -echo; exec cat\n").unwrap();
            if let Some((name, remote)) = checkout {
                origin(&dir.join(&name), &remote);
                node.machine.open_project(dir.join(&name));
            }
            node
        })
        .await
        .unwrap();
        let mut state = self.state.lock().unwrap();
        match state.boxes.get_mut(&id.0) {
            Some(fake) => fake.node = Some(Arc::new(node)),
            None => drop(state),
        }
        Ok(Exec { exit_code: 0, output: "booted\n".into(), error: String::new() })
    }
}

impl BoxApi for FakeBoxes {
    fn create<'a>(&'a self, key: &'a str, new: NewBox) -> BoxFuture<'a, Result<BoxId, String>> {
        Box::pin(async move {
            Self::refuse(key)?;
            let mut state = self.state.lock().unwrap();
            let id = format!("box_{}", state.created.len() + 1);
            state.boxes.insert(id.clone(), FakeBox { labels: new.labels.clone(), files: BTreeMap::new(), node: None });
            state.created.push(new);
            Ok(BoxId(id))
        })
    }

    fn list<'a>(&'a self, key: &'a str, label: &'a str) -> BoxFuture<'a, Result<Vec<Listed>, String>> {
        Box::pin(async move {
            Self::refuse(key)?;
            let state = self.state.lock().unwrap();
            let found = state.boxes.iter().filter(|(_, b)| b.labels.iter().any(|l| l == label));
            Ok(found.map(|(id, b)| Listed { id: BoxId(id.clone()), labels: b.labels.clone() }).collect())
        })
    }

    fn write<'a>(&'a self, key: &'a str, id: &'a BoxId, path: &'a str, content: String) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            Self::refuse(key)?;
            let mut state = self.state.lock().unwrap();
            if path == format!("{BOX_DIR}/account.json") {
                state.accounts.push(serde_json::from_str(&content).unwrap());
            }
            state.boxes.get_mut(&id.0).ok_or("no such box")?.files.insert(path.into(), content);
            Ok(())
        })
    }

    fn exec<'a>(&'a self, key: &'a str, id: &'a BoxId, script: String) -> BoxFuture<'a, Result<Exec, String>> {
        Box::pin(async move {
            Self::refuse(key)?;
            let hanging = {
                let mut state = self.state.lock().unwrap();
                state.scripts.push(script.clone());
                script.contains("start.sh") && std::mem::take(&mut state.hanging)
            };
            if !script.contains("start.sh") {
                return Ok(Exec { exit_code: 0, output: String::new(), error: String::new() });
            }
            if hanging {
                std::future::pending::<()>().await;
            }
            self.boot(id).await
        })
    }

    fn delete<'a>(&'a self, key: &'a str, id: &'a BoxId) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            Self::refuse(key)?;
            let gone = self.state.lock().unwrap().boxes.remove(&id.0);
            tokio::task::spawn_blocking(move || drop(gone)).await.unwrap();
            Ok(())
        })
    }
}

fn fast() -> Policy {
    let ms = Duration::from_millis;
    Policy {
        boot_wait: ms(8000),
        idle: Duration::from_secs(3600),
        done: Duration::from_secs(3600),
        exited: Duration::from_secs(3600),
        tab_gone: ms(400),
        offline: ms(1000),
        max_life: Duration::from_secs(3600),
        cooldown: Duration::from_secs(3600),
        tick: ms(50),
        sweep: ms(100),
        max_open: 8,
    }
}

/// A signed-in account with Upstash connected and a workflow whose sessions run in a box.
struct BoxAccount {
    client: Client,
    id: uuid::Uuid,
    connector: String,
    repo: PathBuf,
}

impl BoxAccount {
    fn new(hub: &TestHub) -> BoxAccount {
        BoxAccount::named(hub, "ana@acme.dev")
    }

    fn named(hub: &TestHub, email: &str) -> BoxAccount {
        let (token, account) = account::signup(&hub.url, email, "correct horse").unwrap();
        let mut client = Client::connect(&hub.url, &token);
        client.wait("hello", |f| f["type"] == "hello");
        assert_eq!(
            client.hub(json!({ "cmd": "connect_upstash", "key": "box_wrongkey", "claude_token": CLAUDE_TOKEN })),
            Err("Upstash Box refused the key: Invalid box API key".into())
        );
        let connected = client.hub(json!({ "cmd": "connect_upstash", "key": format!(" {BOX_KEY} "), "claude_token": CLAUDE_TOKEN })).unwrap();
        assert_eq!(connected, json!({ "key_hint": "…tkey", "claude_hint": "…oken" }));
        let repo = temp(&format!("box-repo-{}", hub.database));
        let connector = client
            .hub(json!({ "cmd": "save_connector", "connector": {
                "name": "Bugs", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
                "folder": repo, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
                "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
            } }))
            .unwrap();
        assert_eq!(connector["machine"], json!({ "kind": "upstash-box", "size": "small" }));
        let settings = client.wait("settings with Upstash", |f| f["type"] == "settings" && !f["settings"]["upstash"].is_null());
        assert!(!settings.to_string().contains(BOX_KEY) && !settings.to_string().contains(CLAUDE_TOKEN), "clients never see the keys");
        BoxAccount { client, id: uuid::Uuid::parse_str(&account.id).unwrap(), connector: connector["id"].as_str().unwrap().into(), repo }
    }

    fn run(&self, subject: Subject, runs_on: RunsOn) -> Work {
        let args = vec!["--name".into(), "ENG-7 Fix login".into()];
        Work::Run(Run { cwd: self.repo.clone(), repo: None, args, connector: self.connector.clone(), subject, mentioned: vec![], account: None, runs_on, input: String::new() })
    }

    fn box_run(&self, subject: Subject) -> Work {
        self.run(subject, RunsOn::FreshBox(BoxStep { size: BoxSize::Small }))
    }

    fn rows(&self, hub: &TestHub) -> usize {
        hub.runtime.block_on(hub.hub().db.boxes(self.id)).unwrap().len()
    }

    fn box_machine(&mut self) -> String {
        self.client.pump();
        let latest = self.client.frames.iter().rev().find(|f| f["type"] == "machines").unwrap();
        let machine = latest["machines"].as_array().unwrap().iter().find(|m| m["upstash_box"].is_string()).expect("a box machine");
        machine["id"].as_str().unwrap().to_owned()
    }
}

fn issue(id: &str) -> Subject {
    Subject::LinearIssue { issue: id.into() }
}

#[test]
fn a_box_run_boots_its_own_node_and_the_box_goes_when_its_tab_closes() {
    let hub = TestHub::start_with_boxes("box_run", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));

    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()));
    let created = hub.boxes.state.lock().unwrap().created.clone();
    let label = pinned_server::boxes::hub_label(&hub.url);
    assert_eq!(created.len(), 1);
    let row = created[0].labels[1].strip_prefix("p:").unwrap().to_owned();
    assert_eq!((created[0].labels[0].as_str(), row.len()), (label.as_str(), 16));
    assert_eq!(created[0].name, format!("pinned-{row}"));
    assert_eq!((created[0].size, created[0].init_command.as_str()), (BoxSize::Small, "sh /workspace/home/.pinned/start.sh || true"));
    ana.client.wait_machines(&[("Box · ENG-7 Fix login", true)]);
    let machine = ana.box_machine();
    assert_eq!(hub.boxes.file(&format!("{BOX_DIR}/box.env")), Some(format!("CLAUDE_CODE_OAUTH_TOKEN='{CLAUDE_TOKEN}'\n")), "no GitHub token without GitHub");
    let token = hub.boxes.state.lock().unwrap().accounts[0].machine_token.clone();
    for script in &hub.boxes.state.lock().unwrap().scripts {
        for secret in [BOX_KEY, CLAUDE_TOKEN, token.as_str()] {
            assert!(!script.contains(secret), "a secret on an exec line: {script}");
        }
    }
    let tabs = hub.boxes.with_node(|node| node.machine.tabs());
    assert_eq!((tabs.len(), tabs[0].connector.as_deref(), tabs[0].subject.clone()), (1, Some(ana.connector.as_str()), Some(issue("uuid-1"))));

    let forward = |subject| hub.boxes.with_node(|node| node.link.as_ref().unwrap().forward(agent::Message::Stage { connector: ana.connector.clone(), subject, stage: Stage::Coding }));
    assert_eq!(
        forward(issue("uuid-1")),
        Ok("this workflow has no column for that stage, so the issue stays where it is".into()),
        "the box reports on its own issue"
    );
    assert_eq!(forward(issue("uuid-2")), Err("this machine runs no session of that workflow on that subject".into()));
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()), "a run its box already works on counts as taken");
    assert_eq!(hub.boxes.state.lock().unwrap().created.len(), 1);

    ana.client.call(&machine, json!({ "cmd": "close_tab", "tab": &tabs[0].id })).unwrap();
    wait_for("the box to be deleted", || hub.boxes.live().is_empty());
    ana.client.wait_machines(&[]);
    wait_for("the row to go", || ana.rows(&hub) == 0);
}

#[test]
fn other_workflows_never_run_on_a_box_but_its_thread_reaches_it() {
    let hub = TestHub::start_with_boxes("box_pool", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    let token = ana.client.hub(json!({ "cmd": "pair_code" })).unwrap();
    let dir = temp("box-pool-vps");
    let creds = account::pair(&hub.url, token["code"].as_str().unwrap(), "vps", "linux").unwrap();
    let mut vps = Node::start(&dir, creds);
    wait_for("vps online", || vps.online());

    let thread = Subject::SlackThread { channel: "C0123ABCD".into(), thread: "1792400050.000200".into() };
    assert_eq!(open(ana.box_run(thread.clone())), Ok(()));
    ana.client.wait_machines(&[("vps", true), ("Box · ENG-7 Fix login", true)]);
    assert_eq!(open(ana.run(issue("uuid-9"), RunsOn::Any)), Ok(()), "the box connected last, yet an any-machine run goes to the vps");
    assert_eq!(vps.machine.tabs().len(), 1);
    let box_machine = ana.box_machine();
    assert_eq!(open(ana.run(issue("uuid-10"), RunsOn::Machine(box_machine))), Ok(()), "naming a box's machine still skips it");
    assert_eq!((vps.machine.tabs().len(), hub.boxes.with_node(|n| n.machine.tabs().len())), (2, 1));

    let delivery = Work::Delivery(Delivery {
        connector: ana.connector.clone(),
        channel: "C0123ABCD".into(),
        thread: "1792400050.000200".into(),
        messages: vec!["yes, file it".into()],
        cursor: "1792400090.000000".into(),
        mentioned: vec![],
    });
    vps.kill();
    assert_eq!(open(delivery), Ok(()), "a later message in the box's thread reaches its session");
    assert_eq!(open(ana.run(issue("uuid-11"), RunsOn::Any)), Err("no machine of the account is online".into()), "only the box is online");
}

#[test]
fn a_failed_boot_cleans_up_and_cools_down() {
    let hub = TestHub::start_with_boxes("box_fail", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    hub.boxes.state.lock().unwrap().failing = Some(Exec { exit_code: 22, output: "Downloading pinned-server\n".into(), error: "curl: (22) The requested URL returned error: 404\n".into() });
    assert_eq!(
        open(ana.box_run(issue("uuid-1"))),
        Err("Upstash Box: the bootstrap failed: Downloading pinned-server\ncurl: (22) The requested URL returned error: 404".into())
    );
    assert_eq!((hub.boxes.live(), ana.rows(&hub)), (vec![], 0), "nothing is left behind");
    ana.client.wait_machines(&[]);
    assert_eq!(open(ana.box_run(issue("uuid-2"))), Err("Upstash Box: this workflow's last box failed to boot, so pinned tries again in 60 min".into()));
    assert_eq!(hub.boxes.state.lock().unwrap().created.len(), 1, "the cooldown creates no box");
}

#[test]
fn a_hub_restarted_mid_boot_finishes_the_run_with_one_box() {
    let mut hub = TestHub::start_with_boxes("box_restart", Signup::Open, fast());
    let ana = BoxAccount::new(&hub);
    hub.boxes.state.lock().unwrap().hanging = true;
    let (dying, id, work) = (hub.hub().clone(), ana.id, ana.box_run(issue("uuid-1")));
    hub.runtime.spawn(async move { dying.open(id, work).await });
    wait_for("the bootstrap to start", || hub.boxes.state.lock().unwrap().scripts.iter().any(|s| s.contains("start.sh")));
    let first = hub.boxes.state.lock().unwrap().accounts[0].clone();

    hub.restart();
    wait_for("the run to land in the box", || hub.boxes.state.lock().unwrap().boxes.values().any(|b| b.node.as_ref().is_some_and(|n| !n.machine.tabs().is_empty())));
    assert_eq!(hub.boxes.state.lock().unwrap().created.len(), 1, "the new hub found the box the old one created");
    let second = hub.boxes.state.lock().unwrap().accounts.last().unwrap().clone();
    assert_eq!(second.machine_id, first.machine_id, "the same machine, booted again");
    assert_ne!(second.machine_token, first.machine_token);
    assert_eq!(hub.runtime.block_on(hub.hub().db.machine_connected(&first.machine_token, "linux", "0.0.0")).unwrap().map(|m| m.id), None, "the old token no longer signs in");
    let phases = || hub.runtime.block_on(hub.hub().db.boxes(ana.id)).unwrap().into_iter().map(|r| r.unwrap().phase).collect::<Vec<_>>();
    wait_for("the row to run", || phases() == [pinned_server::boxes::Phase::Running]);
}

#[test]
fn the_sweep_deletes_this_hubs_unrecorded_boxes_only() {
    let hub = TestHub::start_with_boxes("box_sweep", Signup::Open, fast());
    let _ana = BoxAccount::new(&hub);
    let label = pinned_server::boxes::hub_label(&hub.url);
    hub.boxes.add("box_orphan", &[&label, "p:ffffffffffffffff"]);
    hub.boxes.add("box_users", &[]);
    hub.boxes.add("box_other_hub", &["hub:000000000000", "p:eeeeeeeeeeeeeeee"]);
    wait_for("the orphan to go", || !hub.boxes.live().contains(&"box_orphan".to_owned()));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(hub.boxes.live(), ["box_other_hub", "box_users"]);
}

#[test]
fn removing_a_box_machine_or_disconnecting_upstash_deletes_its_boxes() {
    let hub = TestHub::start_with_boxes("box_remove", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()));
    ana.client.wait_machines(&[("Box · ENG-7 Fix login", true)]);
    let machine = ana.box_machine();
    ana.client.hub(json!({ "cmd": "remove_machine", "id": machine })).unwrap();
    assert_eq!((hub.boxes.live(), ana.rows(&hub)), (vec![], 0));
    ana.client.wait_machines(&[]);

    assert_eq!(open(ana.box_run(issue("uuid-2"))), Ok(()));
    ana.client.wait_machines(&[("Box · ENG-7 Fix login", true)]);
    assert_eq!(ana.client.hub(json!({ "cmd": "disconnect_upstash" })), Ok(Value::Null));
    assert_eq!((hub.boxes.live(), ana.rows(&hub)), (vec![], 0));
    ana.client.wait_machines(&[]);
    ana.client.wait("settings without Upstash", |f| f["type"] == "settings" && f["settings"]["upstash"].is_null());
    assert_eq!(open(ana.box_run(issue("uuid-3"))), Err("Upstash Box is not connected: connect it in pinned's Connectors settings".into()));
}

#[test]
fn a_box_goes_once_its_session_opened_its_pull_request_and_stopped() {
    let hub = TestHub::start_with_boxes("box_done", Signup::Open, Policy { done: Duration::from_millis(300), ..fast() });
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    assert_eq!(hub.runtime.block_on(hub.hub().open(account, ana.box_run(issue("uuid-1")))), Ok(()));
    ana.client.wait_machines(&[("Box · ENG-7 Fix login", true)]);
    let stage = |stage| hub.boxes.with_node(|node| node.link.as_ref().unwrap().forward(agent::Message::Stage { connector: ana.connector.clone(), subject: issue("uuid-1"), stage }));
    stage(Stage::Coding).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(hub.boxes.live().len(), 1, "a quiet session without a pull request keeps its box until idle");

    stage(Stage::Review).unwrap();
    wait_for("the box to be deleted", || hub.boxes.live().is_empty());
    ana.client.wait_machines(&[]);
    wait_for("the row to go", || ana.rows(&hub) == 0);
}

#[test]
fn a_box_whose_row_this_hub_cannot_read_is_still_deleted() {
    let hub = TestHub::start_with_boxes("box_unreadable", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    assert_eq!(hub.runtime.block_on(hub.hub().open(ana.id, ana.box_run(issue("uuid-1")))), Ok(()));
    ana.client.wait_machines(&[("Box · ENG-7 Fix login", true)]);
    // As a hub whose runs have another shape would have written it.
    admin_on(&hub.database, "update boxes set spec = '{\"shape\": \"newer\"}'");
    wait_for("the box to be deleted", || hub.boxes.live().is_empty());
    ana.client.wait_machines(&[]);
    wait_for("the row to go", || ana.rows(&hub) == 0);
}

fn created(hub: &TestHub) -> usize {
    hub.boxes.state.lock().unwrap().created.len()
}

fn set_workflow(client: &mut Client, mode: &str) {
    client.hub(json!({ "cmd": "set_box_access", "access": { "workflow": { "mode": mode } } })).unwrap();
}

#[test]
fn a_fresh_workflow_permission_opens_one_box_per_run() {
    let hub = TestHub::start_with_boxes("box_fresh", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    set_workflow(&mut ana.client, "fresh");
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()));
    assert_eq!(open(ana.box_run(issue("uuid-2"))), Ok(()));
    assert_eq!(created(&hub), 2);
    assert_eq!(ana.client.hub(json!({ "cmd": "get_settings" })).unwrap()["connectors"][0]["machine"]["kind"], "upstash-box");
}

#[test]
fn a_fixed_workflow_permission_reuses_one_box_across_two_runs() {
    let hub = TestHub::start_with_boxes("box_fixed", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    set_workflow(&mut ana.client, "fixed");
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()));
    ana.client.wait_machines(&[("Box · ENG-7 Fix login", true)]);
    assert_eq!(open(ana.box_run(issue("uuid-2"))), Ok(()));
    assert_eq!(created(&hub), 1, "the second run uses the box the first run opened");
}

#[test]
fn an_ask_workflow_permission_opens_nothing_until_it_is_allowed() {
    let hub = TestHub::start_with_boxes("box_ask", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    set_workflow(&mut ana.client, "ask");
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Err("Upstash Box: waiting for a yes before it opens a box for this".into()));
    assert_eq!(created(&hub), 0);
    let settings = ana.client.hub(json!({ "cmd": "get_settings" })).unwrap();
    let id = settings["box_asks"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(settings["connectors"][0]["machine"], json!({ "kind": "upstash-box", "size": "small" }));
    ana.client.hub(json!({ "cmd": "answer_box", "id": id, "allow": true })).unwrap();
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()));
    assert_eq!(created(&hub), 1);
    assert_eq!(open(ana.box_run(issue("uuid-2"))), Err("Upstash Box: waiting for a yes before it opens a box for this".into()));
    assert_eq!(created(&hub), 1, "a different run still waits for its own yes");
}

#[test]
fn workflow_boxes_that_are_off_or_disconnected_open_nothing() {
    let hub = TestHub::start_with_boxes("box_off", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    set_workflow(&mut ana.client, "off");
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Err("Upstash Box is off for workflows: choose a box permission in Connectors settings".into()));
    assert_eq!(created(&hub), 0);
    set_workflow(&mut ana.client, "fresh");
    assert_eq!(ana.client.hub(json!({ "cmd": "disconnect_upstash" })), Ok(Value::Null));
    assert_eq!(open(ana.box_run(issue("uuid-2"))), Err("Upstash Box is not connected: connect it in pinned's Connectors settings".into()));
    assert_eq!(created(&hub), 0);
}

#[test]
fn an_account_without_a_stored_box_permission_still_opens_a_box() {
    let mut hub = TestHub::start_with_boxes("box_legacy", Signup::Open, fast());
    let ana = BoxAccount::new(&hub);
    let account = ana.id;
    admin_on(&hub.database, "update accounts set store = store - 'boxes' - 'box_asks' - 'box_grants'");
    hub.restart();
    assert_eq!(hub.runtime.block_on(hub.hub().open(account, ana.box_run(issue("uuid-1")))), Ok(()));
    assert_eq!(created(&hub), 1);
}

#[test]
fn a_denied_ask_opens_nothing() {
    let hub = TestHub::start_with_boxes("box_deny", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    set_workflow(&mut ana.client, "ask");
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Err("Upstash Box: waiting for a yes before it opens a box for this".into()));
    let settings = ana.client.hub(json!({ "cmd": "get_settings" })).unwrap();
    let id = settings["box_asks"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(settings["box_asks"][0]["subject"]["issue"], "uuid-1");
    ana.client.hub(json!({ "cmd": "answer_box", "id": id, "allow": false })).unwrap();
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Err("Upstash Box: you said no to a box for this, so none was opened".into()));
    assert_eq!(created(&hub), 0);
    let after = ana.client.hub(json!({ "cmd": "get_settings" })).unwrap();
    assert_eq!(after["box_asks"].as_array().map(|asks| asks.len()).unwrap_or(0), 0);
}

#[test]
fn a_fixed_box_stays_after_its_session_ends_so_the_next_run_reuses_it() {
    let hub = TestHub::start_with_boxes("box_fixed_stays", Signup::Open, Policy { done: Duration::from_millis(300), ..fast() });
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    set_workflow(&mut ana.client, "fixed");
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()));
    ana.client.wait_machines(&[("Box · ENG-7 Fix login", true)]);
    let machine = ana.box_machine();
    let stage = |stage| {
        hub.boxes.with_node(|node| {
            node.link.as_ref().unwrap().forward(agent::Message::Stage { connector: ana.connector.clone(), subject: issue("uuid-1"), stage })
        })
    };
    stage(Stage::Review).unwrap();
    let tab = hub.boxes.with_node(|node| node.machine.tabs().into_iter().next().unwrap().id);
    ana.client.call(&machine, json!({ "cmd": "close_tab", "tab": tab })).unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1500) {
        assert_eq!(hub.boxes.live().len(), 1, "fixed box reaped");
        assert_eq!(ana.rows(&hub), 1, "fixed row reaped");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(open(ana.box_run(issue("uuid-2"))), Ok(()));
    assert_eq!(created(&hub), 1);
}

#[test]
fn a_fresh_permission_stops_at_the_accounts_box_limit() {
    let hub = TestHub::start_with_boxes("box_cap", Signup::Open, Policy { max_open: 1, ..fast() });
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    set_workflow(&mut ana.client, "fresh");
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()));
    assert_eq!(open(ana.box_run(issue("uuid-2"))), Err("Upstash Box: 1 box is already open, and this account's limit is 1".into()));
    assert_eq!(created(&hub), 1);
}

#[test]
fn an_unknown_event_keeps_the_machine_socket_and_a_thread_reaches_only_its_watcher() {
    let hub = TestHub::start("watch");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut watcher = Client::connect(&url, &token);
    watcher.wait("hello", |f| f["type"] == "hello");
    let dir = temp("watch-node");
    let creds = pair(&url, &mut watcher, "watch-node", &dir);
    let mut machine = MachineSock::connect(&url, &creds.machine_token, "9.9.9");
    watcher.wait_machines(&[("watch-node", true)]);
    let mut other = Client::connect(&url, &token);
    other.wait("hello", |f| f["type"] == "hello" && f["machines"][0]["online"] == true);

    machine.send(json!({ "type": "event", "event": { "kind": "from-the-future", "tab": "tab-9" } }));
    machine.send(json!({ "type": "event", "event": { "kind": "tabs", "tabs": [] } }));
    watcher.wait("tabs after an unknown event", |f| f["type"] == "event" && f["event"]["kind"] == "tabs");
    assert!(other.wait("tabs for the other client", |f| f["type"] == "event" && f["event"]["kind"] == "tabs")["event"]["tabs"].as_array().unwrap().is_empty());

    assert_eq!(watcher.call(&creds.machine_id, json!({ "cmd": "watch_threads", "tabs": ["tab-1"] })), Ok(Value::Null));
    let told = std::iter::repeat_with(|| machine.read_json()).find(|frame| frame["type"] == "watch_threads").unwrap();
    assert_eq!(told["tabs"], json!(["tab-1"]));
    assert!(told["viewer"].as_str().unwrap().starts_with("client-"));

    machine.send(json!({ "type": "event", "event": { "kind": "thread", "tab": "tab-1" } }));
    machine.send(json!({ "type": "event", "event": { "kind": "thread", "tab": "tab-2" } }));
    let got = watcher.wait("the watched tab's nudge", |f| f["event"]["kind"] == "thread");
    assert_eq!(got["event"], json!({ "kind": "thread", "tab": "tab-1" }));
    std::thread::sleep(Duration::from_millis(300));
    assert!(watcher.pump());
    assert!(other.pump());
    assert_eq!(nudged(&watcher), vec!["tab-1".to_string()]);
    assert_eq!(nudged(&other), Vec::<String>::new());
}

fn assistant_line(text: &str) -> String {
    format!(
        "{{\"type\":\"assistant\",\"uuid\":\"{text}\",\"timestamp\":\"2020-01-01T00:00:00Z\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
    )
}

fn nudged(client: &Client) -> Vec<String> {
    client
        .frames
        .iter()
        .filter(|frame| frame["type"] == "event" && frame["event"]["kind"] == "thread")
        .map(|frame| frame["event"]["tab"].as_str().unwrap().to_owned())
        .collect()
}

fn read_texts(client: &mut Client, machine: &str, tab: &str, from: usize) -> Vec<String> {
    let page = client.call(machine, json!({ "cmd": "thread", "tab": tab, "from": from })).unwrap();
    page["items"].as_array().unwrap().iter().map(|item| item["text"].as_str().unwrap_or_default().to_owned()).collect()
}

fn append_line(file: &Path, text: &str) {
    std::fs::OpenOptions::new().append(true).open(file).unwrap().write_all(assistant_line(text).as_bytes()).unwrap();
}

fn node_with_a_transcript(name: &str) -> (TestHub, Client, String, PathBuf, Node, String, PathBuf) {
    let hub = TestHub::start(name);
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |frame| frame["type"] == "hello");
    let dir = temp(&format!("{name}-node"));
    let creds = pair(&url, &mut client, "watch-node", &dir);
    let machine = creds.machine_id.clone();
    let node = Node::start(&dir, creds);
    client.wait_machines(&[("watch-node", true)]);
    node.machine.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
    let app = dir.join("app");
    std::fs::create_dir_all(&app).unwrap();
    let projects = dir.join("accounts/work/projects/demo");
    std::fs::create_dir_all(&projects).unwrap();
    let file = projects.join("s1.jsonl");
    std::fs::write(&file, assistant_line("ready")).unwrap();
    pinned_core::sessions::save(
        &dir.join("sessions.json"),
        &[Saved {
            cwd: app,
            session: "s1".into(),
            env: vec![],
            bypass: false,
            interrupted: false,
            resume_at: None,
            held: false,
            account: "work".into(),
            model: None,
            permission_mode: None,
            spawned_by: None,
        }],
    )
    .unwrap();
    let tabs = node.machine.restore_tabs();
    assert_eq!(tabs.len(), 1, "the saved session did not reopen");
    let tab = tabs[0].clone();
    (hub, client, machine, dir, node, tab, file)
}

fn watch(client: &mut Client, machine: &str, tab: &str) {
    assert_eq!(client.call(machine, json!({ "cmd": "watch_threads", "tabs": [tab] })), Ok(Value::Null));
    client.wait("the first look", |frame| frame["event"]["kind"] == "thread" && frame["event"]["tab"] == tab);
    std::thread::sleep(Duration::from_millis(200));
    client.pump();
    client.frames.clear();
}

#[test]
fn a_watched_tab_is_nudged_and_again_after_the_machine_restarts() {
    let (_hub, mut client, machine, dir, node, tab, file) = node_with_a_transcript("watch_push");
    watch(&mut client, &machine, &tab);
    let started = Instant::now();
    append_line(&file, "visible");
    client.wait("the nudge", |frame| frame["event"]["kind"] == "thread");
    assert!(started.elapsed() < Duration::from_millis(800), "the nudge took {:?}, the poll is 1200 ms", started.elapsed());
    assert_eq!(read_texts(&mut client, &machine, &tab, assistant_line("ready").len()), vec!["visible"]);
    std::thread::sleep(Duration::from_millis(200));
    client.pump();
    assert_eq!(nudged(&client), vec![tab.clone()], "one line, one nudge");

    let quiet = Arc::new(Mutex::new(0u32));
    let counted = quiet.clone();
    node.machine.subscribe(Box::new(move |event| {
        if serde_json::to_value(event).unwrap()["kind"] == "thread" {
            *counted.lock().unwrap() += 1;
        }
    }));
    let mut node = node;
    node.link.take().unwrap().stop();
    append_line(&file, "quiet");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(*quiet.lock().unwrap(), 0, "a dropped link forgets remote watches");
    node.machine.quit();
    client.wait_machines(&[("watch-node", false)]);
    let mut node = Node::start(&dir, Credentials::load(&dir).unwrap());
    client.wait_machines(&[("watch-node", true)]);
    let restored = node.machine.restore_tabs();
    assert_eq!(restored.len(), 1, "the saved session did not reopen");
    assert_ne!(restored[0], tab, "a new process reused {tab}");
    std::thread::sleep(Duration::from_millis(300));
    append_line(&file, "stale");
    std::thread::sleep(Duration::from_millis(250));
    client.pump();
    assert!(!nudged(&client).contains(&restored[0]), "the watch from the previous process named the new tab");
    watch(&mut client, &machine, &restored[0]);
    let before = std::fs::metadata(&file).unwrap().len() as usize;
    let started = Instant::now();
    append_line(&file, "again");
    client.wait("the nudge after the machine restarts", |frame| frame["event"]["kind"] == "thread");
    assert!(started.elapsed() < Duration::from_millis(800), "after the restart the nudge took {:?}, the poll is 1200 ms", started.elapsed());
    assert_eq!(read_texts(&mut client, &machine, &restored[0], before), vec!["again"]);
    node.kill();
}

#[test]
fn a_dropped_machine_link_is_nudged_again_once_the_client_names_its_tabs() {
    let (_hub, mut client, machine, dir, mut node, tab, file) = node_with_a_transcript("watch_rewatch");
    watch(&mut client, &machine, &tab);
    append_line(&file, "visible");
    client.wait("the first nudge", |frame| frame["event"]["kind"] == "thread");

    node.link.take().unwrap().stop();
    client.wait_machines(&[("watch-node", false)]);
    assert_eq!(client.call(&machine, json!({ "cmd": "watch_threads", "tabs": [] })), Ok(Value::Null));
    node.link = Some(link::spawn(Credentials::load(&dir).unwrap(), node.machine.clone(), "0.0.0-test"));
    client.wait_machines(&[("watch-node", true)]);
    assert_eq!(node.machine.view(&tab).unwrap().id, tab, "the same process kept {tab}");
    watch(&mut client, &machine, &tab);
    let before = std::fs::metadata(&file).unwrap().len() as usize;
    append_line(&file, "again");
    client.wait("the nudge after the link returns", |frame| frame["event"]["kind"] == "thread");
    assert_eq!(read_texts(&mut client, &machine, &tab, before), vec!["again"]);
    node.kill();
}

#[test]
fn a_client_that_disconnects_ends_its_watch_on_the_machine() {
    let (_hub, mut client, machine, _dir, mut node, tab, file) = node_with_a_transcript("watch_gone");
    watch(&mut client, &machine, &tab);
    let nudges = Arc::new(Mutex::new(0u32));
    let counted = nudges.clone();
    node.machine.subscribe(Box::new(move |event| {
        if serde_json::to_value(event).unwrap()["kind"] == "thread" {
            *counted.lock().unwrap() += 1;
        }
    }));
    append_line(&file, "watched");
    client.wait("the nudge", |frame| frame["event"]["kind"] == "thread");
    assert_eq!(*nudges.lock().unwrap(), 1);
    drop(client);
    std::thread::sleep(Duration::from_millis(300));
    append_line(&file, "nobody");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(*nudges.lock().unwrap(), 1, "the machine stops looking once the only client watching leaves");
    node.kill();
}

struct MachineSock {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl MachineSock {
    fn connect(hub: &str, token: &str, version: &str) -> MachineSock {
        let (mut ws, _) = tungstenite::connect(format!("{}/ws/machine", hub.replace("http://", "ws://"))).unwrap();
        if let MaybeTlsStream::Plain(s) = ws.get_mut() {
            s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        }
        ws.send(Message::text(json!({ "type": "auth", "token": token, "name": "vps", "os": "linux", "version": version }).to_string())).unwrap();
        let mut sock = MachineSock { ws };
        let hello = sock.read_json();
        assert_eq!(hello["type"], "hello");
        sock
    }

    fn try_text(&mut self) -> Option<Value> {
        match self.ws.read() {
            Ok(Message::Text(text)) => Some(serde_json::from_str(&text).unwrap()),
            Ok(_) => self.try_text(),
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => None,
            Err(e) => panic!("machine socket: {e}"),
        }
    }

    fn read_json(&mut self) -> Value {
        let start = Instant::now();
        loop {
            if let Some(value) = self.try_text() {
                return value;
            }
            assert!(start.elapsed() < WAIT, "timed out waiting for a machine frame");
        }
    }

    fn send(&mut self, frame: Value) {
        self.ws.send(Message::text(frame.to_string())).unwrap();
    }
}

fn listed(frame: &Value, id: &str) -> Value {
    frame["machines"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap().clone()
}

fn latest_row(client: &Client, id: &str) -> Value {
    let latest = client.frames.iter().rev().find(|f| f["type"] == "machines" || f["type"] == "hello").unwrap();
    listed(latest, id)
}

fn wait_row(client: &mut Client, id: &str, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let frame = client.wait(what, |f| {
        (f["type"] == "machines" || f["type"] == "hello") && f["machines"].as_array().is_some_and(|rows| rows.iter().any(|m| m["id"] == id && pred(m)))
    });
    listed(&frame, id)
}

fn settle(client: &mut Client) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        client.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_update_self_is_delivered_and_the_list_shows_the_phase() {
    let hub = TestHub::start("upd_relay");
    let url = hub.url.clone();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let (machine, machine_token) = account::register_machine(&url, &token, "vps", "linux").unwrap();
    let id = machine.id.clone();
    let mut node = MachineSock::connect(&url, &machine_token, "0.15.0");
    client.wait_machines(&[("vps", true)]);
    assert_eq!(latest_row(&client, &id)["version"], "0.15.0");

    node.send(json!({ "type": "event", "event": { "kind": "update", "status": { "state": "updating", "to": "0.15.0" } } }));
    assert_eq!(wait_row(&mut client, &id, "updating to the running version", |m| m["update"]["state"] == "updating")["update"], json!({ "state": "updating", "to": "0.15.0" }));
    node.send(json!({ "type": "event", "event": { "kind": "update", "status": null } }));
    assert_eq!(wait_row(&mut client, &id, "up to date", |m| m["update"]["state"] == "up_to_date")["update"], json!({ "state": "up_to_date", "version": "0.15.0" }));

    node.send(json!({ "type": "event", "event": { "kind": "update", "status": { "state": "restarting", "to": "0.16.0" } } }));
    assert_eq!(wait_row(&mut client, &id, "restarting", |m| m["update"]["state"] == "restarting")["update"], json!({ "state": "restarting", "to": "0.16.0" }));
    node.send(json!({ "type": "event", "event": { "kind": "update", "status": null } }));
    let failed = json!({ "state": "failed", "to": "0.16.0", "reason": "came back on 0.15.0" });
    assert_eq!(wait_row(&mut client, &id, "came back", |m| m["update"]["state"] == "failed")["update"], failed);

    client.send(json!({ "type": "call", "id": 4, "machine": id, "call": { "cmd": "tabs" } }));
    let frame = node.read_json();
    assert_eq!(frame["call"], json!({ "cmd": "tabs" }));
    let call_id = frame["id"].as_u64().unwrap();
    node.send(json!({ "type": "reply", "id": call_id, "err": "this machine runs pinned 0.15.0, which can't answer that: tabs" }));
    let reply = client.wait("tabs reply", |f| f["type"] == "reply" && f["id"] == 4);
    assert_eq!(reply["err"], "this machine runs pinned 0.15.0, which can't answer that: tabs");
    settle(&mut client);
    assert_eq!(latest_row(&client, &id)["update"], failed);

    client.send(json!({ "type": "call", "id": 5, "machine": id, "call": { "cmd": "update_self" } }));
    let frame = node.read_json();
    assert_eq!(frame["call"], json!({ "cmd": "update_self" }));
    let call_id = frame["id"].as_u64().unwrap();
    node.send(json!({ "type": "reply", "id": call_id, "ok": { "state": "updating", "to": "0.16.0" } }));
    let reply = client.wait("update reply", |f| f["type"] == "reply" && f["id"] == 5);
    assert_eq!(reply["ok"], json!({ "state": "updating", "to": "0.16.0" }));
    assert_eq!(wait_row(&mut client, &id, "reply on the list", |m| m["update"]["to"] == "0.16.0" && m["update"]["state"] == "updating")["update"], json!({ "state": "updating", "to": "0.16.0" }));

    let sentence = "this machine runs pinned 0.15.0, which can't answer that: unknown variant `update_self`";
    client.send(json!({ "type": "call", "id": 6, "machine": id, "call": { "cmd": "update_self" } }));
    let frame = node.read_json();
    assert_eq!(frame["call"], json!({ "cmd": "update_self" }));
    let call_id = frame["id"].as_u64().unwrap();
    node.send(json!({ "type": "reply", "id": call_id, "err": sentence }));
    let reply = client.wait("hand reply", |f| f["type"] == "reply" && f["id"] == 6);
    assert_eq!(reply["err"], sentence);
    assert_eq!(wait_row(&mut client, &id, "update by hand", |m| m["update"]["reason"] == "update by hand")["update"], json!({ "state": "cannot", "reason": "update by hand" }));
}

#[test]
fn an_update_for_another_account_is_refused() {
    let hub = TestHub::start("upd_other");
    let url = hub.url.clone();
    let (token_a, _) = account::signup(&url, "a@acme.dev", "correct horse").unwrap();
    let (token_b, _) = account::signup(&url, "b@acme.dev", "correct horse").unwrap();
    let mut b = Client::connect(&url, &token_b);
    b.wait("hello", |f| f["type"] == "hello");
    let code = b.hub(json!({ "cmd": "pair_code" })).unwrap();
    let creds = account::pair(&url, code["code"].as_str().unwrap(), "node-b", "linux").unwrap();
    let mut a = Client::connect(&url, &token_a);
    a.wait("hello", |f| f["type"] == "hello");
    assert_eq!(a.call(&creds.machine_id, json!({ "cmd": "update_self" })), Err(format!("this account has no machine {}", creds.machine_id)));
}

#[test]
fn an_online_upstash_box_is_not_asked_to_update() {
    let hub = TestHub::start("upd_box");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let (machine, machine_token) = account::register_machine(&url, &token, "vps", "linux").unwrap();
    admin_on(&hub.database, &format!(
        "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase) values ('box-1', '{}', '{}', 'box_123', 'c1', '{{\"issue\":\"i1\"}}', '{{}}', 'small', 'running')",
        account.id, machine.id,
    ));
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    assert_eq!(client.call(&machine.id, json!({ "cmd": "update_self" })), Err("vps is offline".into()));

    let mut node = MachineSock::connect(&url, &machine_token, "0.15.0");
    client.wait_machines(&[("vps", true)]);
    assert_eq!(
        client.call(&machine.id, json!({ "cmd": "update_self" })),
        Ok(json!({ "state": "cannot", "reason": "an Upstash box is replaced by the hub" })),
    );
    assert!(node.try_text().is_none());
    assert_eq!(
        wait_row(&mut client, &machine.id, "box refusal", |m| m["update"]["reason"] == "an Upstash box is replaced by the hub")["update"],
        json!({ "state": "cannot", "reason": "an Upstash box is replaced by the hub" }),
    );
}

// External MCP agents: OAuth through the hub, then the MCP endpoint, against a real hub, Postgres and node.

/// A browser and an MCP client in one: it follows no redirect, so each step's answer is visible.
fn agent_http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
}

fn with_query(base: &str, params: &[(&str, &str)]) -> String {
    url::Url::parse_with_params(base, params).unwrap().to_string()
}

fn pkce() -> (String, String) {
    use sha2::Digest;
    let verifier = format!("verifier-{}-{}", std::process::id(), Instant::now().elapsed().as_nanos()).repeat(3);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

const AGENT_REDIRECT: &str = "https://agent.example/oauth/callback";

fn register_agent(url: &str, http: &reqwest::blocking::Client) -> String {
    let registered = http.post(format!("{url}/oauth/register")).json(&json!({ "redirect_uris": [AGENT_REDIRECT], "client_name": "Grok test agent" })).send().unwrap();
    assert_eq!(registered.status(), 201);
    let body: Value = registered.json().unwrap();
    assert_eq!(body["token_endpoint_auth_method"], "none");
    body["client_id"].as_str().unwrap().to_owned()
}

/// Starts an authorization as the client's redirect of the browser would, and answers the pending request's id.
fn authorize(url: &str, http: &reqwest::blocking::Client, client_id: &str, challenge: &str, state: &str) -> String {
    let response = http
        .get(with_query(&format!("{url}/oauth/authorize"), &[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", AGENT_REDIRECT),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("scope", "sessions orchestrator"),
            ("resource", &format!("{url}/mcp")),
        ]))
        .send()
        .unwrap();
    assert_eq!(response.status(), 303);
    let location = response.headers()["location"].to_str().unwrap().to_owned();
    location.strip_prefix("/connect-agent?request=").unwrap_or_else(|| panic!("{location}")).to_owned()
}

fn query_of(redirect: &str) -> HashMap<String, String> {
    let at = redirect.find('?').unwrap();
    redirect[at + 1..].split('&').map(|pair| {
        let (k, v) = pair.split_once('=').unwrap();
        (k.to_owned(), v.replace('+', " ").replace("%3A", ":").replace("%2F", "/"))
    }).collect()
}

fn exchange(url: &str, http: &reqwest::blocking::Client, form: &[(&str, &str)]) -> (u16, Value) {
    let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
    let response = http.post(format!("{url}/oauth/token")).header("content-type", "application/x-www-form-urlencoded").body(body).send().unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status().as_u16();
    (status, response.json().unwrap())
}

/// Lets the client in for `scopes` on `projects` and trades the code for tokens: (access, refresh).
fn connect_agent(url: &str, http: &reqwest::blocking::Client, client: &mut Client, client_id: &str, scopes: &[&str], projects: Value) -> (String, String) {
    let (verifier, challenge) = pkce();
    let request = authorize(url, http, client_id, &challenge, "st-1");
    let approved = client.hub(json!({ "cmd": "approve_agent", "request": request, "scopes": scopes, "projects": projects })).unwrap();
    let back = query_of(approved["redirect"].as_str().unwrap());
    assert_eq!((back["state"].as_str(), back["iss"].as_str()), ("st-1", url));
    let (status, tokens) = exchange(url, http, &[("grant_type", "authorization_code"), ("client_id", client_id), ("code", &back["code"]), ("code_verifier", &verifier), ("redirect_uri", AGENT_REDIRECT)]);
    assert_eq!(status, 200, "{tokens}");
    assert_eq!(tokens["token_type"], "Bearer");
    (tokens["access_token"].as_str().unwrap().to_owned(), tokens["refresh_token"].as_str().unwrap().to_owned())
}

fn mcp(url: &str, http: &reqwest::blocking::Client, token: &str, method: &str, params: Value) -> (u16, Value) {
    let response = http
        .post(format!("{url}/mcp"))
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }))
        .send()
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().unwrap_or(Value::Null))
}

/// A tool's structured answer, or the error text it returned.
fn tool(url: &str, http: &reqwest::blocking::Client, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let (status, body) = mcp(url, http, token, "tools/call", json!({ "name": name, "arguments": args }));
    assert_eq!(status, 200, "{body}");
    let result = &body["result"];
    match result["isError"].as_bool() {
        Some(true) => Err(result["content"][0]["text"].as_str().unwrap().to_owned()),
        _ => Ok(result["structuredContent"].clone()),
    }
}

/// Sends a hook event to the node's socket, as claude's hooks do.
fn hook(dir: &Path, event: Value) {
    let mut stream = std::os::unix::net::UnixStream::connect(dir.join("sock")).unwrap();
    writeln!(stream, "{event}").unwrap();
    let mut reply = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stream), &mut reply).unwrap();
}

fn tabs_with(machine: &Machine, connector: &str) -> Vec<pinned_core::wire::TabView> {
    machine.tabs().into_iter().filter(|t| t.connector.as_deref() == Some(connector)).collect()
}

use std::collections::HashMap;

#[test]
fn an_external_agent_connects_through_oauth_and_works_only_where_it_was_let_in() {
    let hub = TestHub::start("agent_e2e");
    let url = hub.url.clone();
    let http = agent_http();

    // Discovery: an unauthenticated call is told where the protected resource's metadata is.
    let (status, _) = mcp(&url, &http, "", "initialize", json!({}));
    assert_eq!(status, 401);
    let denied = http.post(format!("{url}/mcp")).json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).send().unwrap();
    assert_eq!(denied.status(), 401);
    let challenge = denied.headers()["www-authenticate"].to_str().unwrap().to_owned();
    assert!(challenge.contains(&format!("resource_metadata=\"{url}/.well-known/oauth-protected-resource/mcp\"")), "{challenge}");
    let bad = http.post(format!("{url}/mcp")).bearer_auth("not-a-token").json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).send().unwrap();
    assert_eq!(bad.status(), 401);
    assert!(bad.headers()["www-authenticate"].to_str().unwrap().contains("error=\"invalid_token\""));
    let resource: Value = http.get(format!("{url}/.well-known/oauth-protected-resource/mcp")).send().unwrap().json().unwrap();
    assert_eq!((resource["resource"].clone(), resource["authorization_servers"].clone()), (json!(format!("{url}/mcp")), json!([url])));
    let server: Value = http.get(format!("{url}/.well-known/oauth-authorization-server")).send().unwrap().json().unwrap();
    assert_eq!(server["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(server["registration_endpoint"], format!("{url}/oauth/register"));

    // Registration refuses a redirect a code could leak through.
    let refused = http.post(format!("{url}/oauth/register")).json(&json!({ "redirect_uris": ["http://evil.example/cb"] })).send().unwrap();
    assert_eq!(refused.status(), 400);
    let client_id = register_agent(&url, &http);

    // An authorization with an unregistered redirect never redirects; one without PKCE goes back with an error.
    let unregistered = http.get(with_query(&format!("{url}/oauth/authorize"), &[("response_type", "code"), ("client_id", client_id.as_str()), ("redirect_uri", "https://evil.example/cb"), ("code_challenge", &pkce().1), ("code_challenge_method", "S256")])).send().unwrap();
    assert_eq!(unregistered.status(), 400);
    assert!(unregistered.headers().get("location").is_none());
    let plain = http.get(with_query(&format!("{url}/oauth/authorize"), &[("response_type", "code"), ("client_id", client_id.as_str()), ("redirect_uri", AGENT_REDIRECT), ("state", "s")])).send().unwrap();
    let back = plain.headers()["location"].to_str().unwrap().to_owned();
    assert!(back.starts_with(AGENT_REDIRECT) && back.contains("error=invalid_request") && back.contains("state=s"), "{back}");

    // The account, its node and a project on it.
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let dir = temp("agent-node");
    let creds = pair(&url, &mut client, "mac", &dir);
    let node = Node::start(&dir, creds.clone());
    // Like claude's TUI, it reads keys raw, so the node types prompts into it rather than holding them.
    std::fs::write(dir.join("claude"), "#!/bin/sh\n[ \"$1 $2\" = \"auth status\" ] && { echo '{\"loggedIn\":true}'; exit 0; }\nstty raw -echo 2>/dev/null\nexec cat\n").unwrap();
    client.wait_machines(&[("mac", true)]);
    node.machine.listen(|_| Err("not in this test".into())).unwrap();
    let project = dir.join("app");
    std::fs::create_dir_all(&project).unwrap();
    let project = node.machine.open_project(project).display().to_string();
    let elsewhere = dir.join("other");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let m = creds.machine_id.as_str();

    // The consent card reads the pending request; the approval checks every project against its machine.
    let (verifier, challenge) = pkce();
    let request = authorize(&url, &http, &client_id, &challenge, "xyz");
    let shown = client.hub(json!({ "cmd": "agent_request", "request": request })).unwrap();
    assert_eq!((shown["client"].as_str(), shown["redirect_uri"].as_str()), (Some("Grok test agent"), Some(AGENT_REDIRECT)));
    let approve = |client: &mut Client, projects: Value, scopes: Value| client.hub(json!({ "cmd": "approve_agent", "request": request, "scopes": scopes, "projects": projects }));
    let stranger = uuid::Uuid::new_v4().to_string();
    assert!(approve(&mut client, json!([{ "machine": stranger, "path": project }]), json!(["sessions"])).unwrap_err().contains("this account has no machine"));
    assert!(approve(&mut client, json!([{ "machine": m, "path": elsewhere.display().to_string() }]), json!(["sessions"])).unwrap_err().contains("is not a project on mac"));
    assert!(approve(&mut client, json!([{ "machine": m, "path": project }]), json!(["root"])).unwrap_err().contains("Pick what the agent may do"));
    let approved = approve(&mut client, json!([{ "machine": m, "path": project }]), json!(["sessions", "orchestrator"])).unwrap();
    assert!(approve(&mut client, json!([{ "machine": m, "path": project }]), json!(["sessions"])).unwrap_err().contains("expired or was already answered"), "a request is approved once");
    let back = query_of(approved["redirect"].as_str().unwrap());
    assert_eq!(back["state"], "xyz");
    let code = back["code"].clone();
    let (status, err) = exchange(&url, &http, &[("grant_type", "authorization_code"), ("client_id", "pinned-agent-other"), ("code", &code), ("code_verifier", &verifier)]);
    assert_eq!((status, err["error"].as_str()), (400, Some("invalid_grant")), "a code is only its own client's");
    // That first use spent the code, so a second exchange is a replay, which ends the grant.
    let (status, err) = exchange(&url, &http, &[("grant_type", "authorization_code"), ("client_id", &client_id), ("code", &code), ("code_verifier", &verifier)]);
    assert_eq!((status, err["error"].as_str()), (400, Some("invalid_grant")));
    assert_eq!(client.hub(json!({ "cmd": "external_agents" })).unwrap()["agents"], json!([]), "a replayed code ended its grant");

    let (access, refresh) = connect_agent(&url, &http, &mut client, &client_id, &["sessions", "orchestrator"], json!([{ "machine": m, "path": project }]));
    let listed = client.hub(json!({ "cmd": "external_agents" })).unwrap();
    assert_eq!(listed["endpoint"], format!("{url}/mcp"));
    assert_eq!(listed["agents"][0]["name"], "Grok test agent");
    assert_eq!(listed["agents"][0]["projects"], json!([{ "machine": m, "path": project, "name": "app" }]));

    // MCP: initialize, the tools its scopes reach, and the one project it was let into.
    let (status, init) = mcp(&url, &http, &access, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }));
    assert_eq!((status, init["result"]["protocolVersion"].as_str()), (200, Some("2025-06-18")));
    let notified = http.post(format!("{url}/mcp")).bearer_auth(&access).json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).send().unwrap();
    assert_eq!(notified.status(), 202);
    let foreign = http.post(format!("{url}/mcp")).bearer_auth(&access).header("origin", "https://evil.example").json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })).send().unwrap();
    assert_eq!(foreign.status(), 403, "a page on another origin can't call the endpoint");
    let (_, listed) = mcp(&url, &http, &access, "tools/list", json!({}));
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_projects", "start_session", "send_message", "read_session", "list_sessions", "message_orchestrator", "read_orchestrator"]);
    let projects = tool(&url, &http, &access, "list_projects", json!({})).unwrap();
    assert_eq!(projects["projects"].as_array().unwrap().len(), 1);
    assert_eq!((projects["projects"][0]["name"].as_str(), projects["projects"][0]["machine"].as_str(), projects["projects"][0]["online"].as_bool()), (Some("app"), Some("mac"), Some(true)));
    let pid = projects["projects"][0]["id"].as_str().unwrap().to_owned();

    // A session starts once per request_id, without skipping permissions.
    let started = tool(&url, &http, &access, "start_session", json!({ "project": pid, "task": "Run the tests", "title": "--dangerously-skip-permissions", "request_id": "start-1" })).unwrap();
    assert_eq!(started["state"], "starting");
    let session = started["session_id"].as_str().unwrap().to_owned();
    let connector = format!("mcp:{session}");
    let opened = tabs_with(&node.machine, &connector);
    assert_eq!(opened.len(), 1);
    assert!(opened[0].running && !opened[0].bypass, "{:?}", opened[0]);
    assert_eq!(opened[0].cwd, project);
    let again = tool(&url, &http, &access, "start_session", json!({ "project": pid, "task": "Run the tests", "title": "--dangerously-skip-permissions", "request_id": "start-1" })).unwrap();
    assert_eq!((again["session_id"].as_str(), again["replayed"].as_bool()), (Some(session.as_str()), Some(true)));
    assert_eq!(tabs_with(&node.machine, &connector).len(), 1, "a retry never starts a second session");
    assert!(tool(&url, &http, &access, "start_session", json!({ "project": pid, "task": "Something else", "request_id": "start-1" })).unwrap_err().contains("already used for another request"));

    let read = tool(&url, &http, &access, "read_session", json!({ "session_id": session })).unwrap();
    assert_eq!(read["state"], "starting");
    assert!(read["conversation"]["items"].is_array(), "{read}");

    // Nothing is typed before the session reports it is at its prompt: it may show a startup dialog.
    let tab = opened[0].id.clone();
    let early = tool(&url, &http, &access, "send_message", json!({ "session_id": session, "message": "also lint", "request_id": "msg-1" })).unwrap_err();
    assert!(early.contains("the session is starting"), "{early}");
    hook(&dir, json!({ "kind": "agent", "status": "needs-input", "tab": tab, "message": "Claude is waiting for your input" }));
    wait_for("the idle notice", || tool(&url, &http, &access, "read_session", json!({ "session_id": session })).unwrap()["state"] == "done");

    // A message goes in once, and the session's screen shows it; claude here is `cat`.
    let sent = tool(&url, &http, &access, "send_message", json!({ "session_id": session, "message": "also lint", "request_id": "msg-1" })).unwrap();
    assert_eq!(sent["delivered"], true);
    let replay = tool(&url, &http, &access, "send_message", json!({ "session_id": session, "message": "also lint", "request_id": "msg-1" })).unwrap();
    assert_eq!(replay["replayed"], true);
    client.send(json!({ "type": "attach", "machine": m, "tab": &tab }));
    let screen = client.wait_output(m, &tab, "also lint");
    assert_eq!(screen.matches("also lint").count(), 1, "the retry typed nothing: {screen:?}");

    // A permission prompt is the user's: the agent sees it and can't type into it.
    hook(&dir, json!({ "kind": "agent", "status": "needs-input", "tab": tab, "message": "Claude needs your permission to use Bash" }));
    wait_for("the permission prompt", || tool(&url, &http, &access, "read_session", json!({ "session_id": session })).unwrap()["state"] == "needs_permission");
    let read = tool(&url, &http, &access, "read_session", json!({ "session_id": session })).unwrap();
    assert_eq!(read["question"], "Claude needs your permission to use Bash");
    let refused = tool(&url, &http, &access, "send_message", json!({ "session_id": session, "message": "yes", "request_id": "msg-2" })).unwrap_err();
    assert!(refused.contains("needs_permission"), "{refused}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!client.output(m, &tab).contains("yes"), "nothing was typed into the prompt");
    hook(&dir, json!({ "kind": "agent", "status": "done", "tab": tab }));
    wait_for("the finished turn", || tool(&url, &http, &access, "read_session", json!({ "session_id": session })).unwrap()["state"] == "done");
    assert_eq!(tool(&url, &http, &access, "send_message", json!({ "session_id": session, "message": "now ship", "request_id": "msg-2" })).unwrap()["delivered"], true, "a refused key may try again once the session is free");
    let sessions = tool(&url, &http, &access, "list_sessions", json!({})).unwrap();
    assert_eq!((sessions["sessions"][0]["session_id"].as_str(), sessions["sessions"][0]["project"].as_str()), (Some(session.as_str()), Some(pid.as_str())));

    // The orchestrator: the first message starts it and types nothing; once it is idle, the same key delivers once.
    assert_eq!(tool(&url, &http, &access, "read_orchestrator", json!({ "project": pid })).unwrap()["state"], "not_started");
    let orchestrate = json!({ "project": pid, "message": "plan the release", "request_id": "orch-1" });
    let told = tool(&url, &http, &access, "message_orchestrator", orchestrate.clone()).unwrap();
    assert_eq!((told["delivered"].as_bool(), told["started"].as_bool(), told["state"].as_str()), (Some(false), Some(true), Some("starting")));
    let orchestrators: Vec<_> = node.machine.tabs().into_iter().filter(|t| t.role == Some(pinned_core::agent::Role::Orchestrator)).collect();
    assert_eq!(orchestrators.len(), 1);
    assert!(orchestrators[0].running && !orchestrators[0].bypass);
    let orchestrator = orchestrators[0].id.clone();
    assert_eq!(tool(&url, &http, &access, "read_orchestrator", json!({ "project": pid })).unwrap()["state"], "starting");
    assert!(tool(&url, &http, &access, "message_orchestrator", orchestrate.clone()).unwrap_err().contains("the orchestrator is starting"));
    client.send(json!({ "type": "attach", "machine": m, "tab": &orchestrator }));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!client.output(m, &orchestrator).contains("plan the release"), "nothing was typed while it started");
    hook(&dir, json!({ "kind": "agent", "status": "needs-input", "tab": orchestrator, "message": "Claude is waiting for your input" }));
    wait_for("the orchestrator's idle notice", || tool(&url, &http, &access, "read_orchestrator", json!({ "project": pid })).unwrap()["state"] == "done");
    assert_eq!(tool(&url, &http, &access, "message_orchestrator", orchestrate.clone()).unwrap()["delivered"], true);
    assert_eq!(tool(&url, &http, &access, "message_orchestrator", orchestrate).unwrap()["replayed"], true);
    assert_eq!(client.wait_output(m, &orchestrator, "plan the release").matches("plan the release").count(), 1);
    assert_eq!(node.machine.tabs().into_iter().filter(|t| t.role == Some(pinned_core::agent::Role::Orchestrator)).count(), 1, "one orchestrator, the project's own");

    // Another account's agent reaches none of this.
    let (bob_token, _) = account::signup(&url, "bob@acme.dev", "correct horse").unwrap();
    let mut bob = Client::connect(&url, &bob_token);
    bob.wait("hello", |f| f["type"] == "hello");
    let (bob_machine, bob_machine_token) = account::register_machine(&url, &bob_token, "bob-box", "linux").unwrap();
    let _raw = RawMachine::start(&url, &bob_machine_token, |call| match call["cmd"].as_str() {
        Some("list_projects") => RawReply::Ok(json!([{ "path": "/w/bob", "name": "bob", "account": "claude" }])),
        _ => RawReply::Ok(json!([])),
    });
    bob.wait_machines(&[("bob-box", true)]);
    assert!(bob.hub(json!({ "cmd": "approve_agent", "request": authorize(&url, &http, &client_id, &pkce().1, "b"), "scopes": ["sessions"], "projects": [{ "machine": m, "path": project }] })).unwrap_err().contains("this account has no machine"), "bob can't grant ana's machine");
    let (bob_access, _) = connect_agent(&url, &http, &mut bob, &client_id, &["sessions"], json!([{ "machine": bob_machine.id, "path": "/w/bob" }]));
    assert!(tool(&url, &http, &bob_access, "read_session", json!({ "session_id": session })).unwrap_err().contains("no session"));
    assert!(tool(&url, &http, &bob_access, "send_message", json!({ "session_id": session, "message": "hi", "request_id": "b1" })).unwrap_err().contains("no session"));
    assert!(tool(&url, &http, &bob_access, "start_session", json!({ "project": pid, "task": "t", "request_id": "b2" })).unwrap_err().contains("no project"));
    assert!(tool(&url, &http, &bob_access, "message_orchestrator", json!({ "project": pid, "message": "m", "request_id": "b3" })).unwrap_err().contains("needs the orchestrator scope"));
    assert_eq!(tool(&url, &http, &bob_access, "list_sessions", json!({})).unwrap()["sessions"], json!([]));

    // Refresh rotates; the old refresh token coming back ends the grant.
    let (status, fresh) = exchange(&url, &http, &[("grant_type", "refresh_token"), ("client_id", &client_id), ("refresh_token", &refresh)]);
    assert_eq!(status, 200, "{fresh}");
    let access = fresh["access_token"].as_str().unwrap().to_owned();
    assert_eq!(tool(&url, &http, &access, "list_projects", json!({})).unwrap()["projects"][0]["id"], pid);
    let (status, err) = exchange(&url, &http, &[("grant_type", "refresh_token"), ("client_id", &client_id), ("refresh_token", &refresh)]);
    assert_eq!((status, err["error"].as_str()), (400, Some("invalid_grant")));
    assert_eq!(mcp(&url, &http, &access, "tools/list", json!({})).0, 401, "a replayed refresh token ended the grant's access");

    // Revoking from the Connectors page ends access at once.
    let (access, _) = connect_agent(&url, &http, &mut client, &client_id, &["orchestrator"], json!([{ "machine": m, "path": project }]));
    let (_, listed) = mcp(&url, &http, &access, "tools/list", json!({}));
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 3, "an orchestrator-only grant lists no session tools");
    assert!(tool(&url, &http, &access, "start_session", json!({ "project": pid, "task": "t", "request_id": "x" })).unwrap_err().contains("needs the sessions scope"));
    let id = client.hub(json!({ "cmd": "external_agents" })).unwrap()["agents"][0]["id"].as_str().unwrap().to_owned();
    assert!(bob.hub(json!({ "cmd": "revoke_agent", "id": id })).unwrap_err().contains("this account has no agent"), "bob can't revoke ana's agent");
    client.hub(json!({ "cmd": "revoke_agent", "id": id })).unwrap();
    assert_eq!(mcp(&url, &http, &access, "tools/list", json!({})).0, 401);
    drop(node);
}

enum RawReply {
    Ok(Value),
    Err(String),
    /// Reads the call, then drops the connection without answering, as a machine that crashed mid-call.
    Hangup,
}

/// A machine socket the test scripts, which reconnects after a hangup and logs every call it got.
/// It sends `events` after each sign in, and the hub calls `ask` queues, as a node's sessions do.
struct RawMachine {
    calls: Arc<Mutex<Vec<Value>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    outbox: Arc<Mutex<Vec<Value>>>,
    replies: Arc<Mutex<Vec<Value>>>,
    next: AtomicU64,
}

impl RawMachine {
    fn start(url: &str, token: &str, answer: impl Fn(&Value) -> RawReply + Send + 'static) -> RawMachine {
        RawMachine::start_with(url, token, vec![], answer)
    }

    fn start_with(url: &str, token: &str, events: Vec<Value>, answer: impl Fn(&Value) -> RawReply + Send + 'static) -> RawMachine {
        let (calls, stop) = (Arc::new(Mutex::new(vec![])), Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let (outbox, replies): (Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<Value>>>) = (Arc::default(), Arc::default());
        let (log, stopped, url, token) = (calls.clone(), stop.clone(), url.to_owned(), token.to_owned());
        let (queued, answered) = (outbox.clone(), replies.clone());
        std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let (mut ws, _) = tungstenite::connect(format!("{}/ws/machine", url.replace("http://", "ws://"))).unwrap();
                if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
                    stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
                }
                ws.send(Message::text(json!({ "type": "auth", "token": token, "name": "raw", "os": "linux", "version": "test" }).to_string())).unwrap();
                for event in &events {
                    ws.send(Message::text(json!({ "type": "event", "event": event }).to_string())).unwrap();
                }
                loop {
                    if stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    for frame in std::mem::take(&mut *queued.lock().unwrap()) {
                        ws.send(Message::text(frame.to_string())).unwrap();
                    }
                    let frame: Value = match ws.read() {
                        Ok(Message::Text(text)) => serde_json::from_str(&text).unwrap(),
                        Ok(Message::Ping(payload)) => {
                            let _ = ws.send(Message::Pong(payload));
                            continue;
                        }
                        Ok(_) => continue,
                        Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                        Err(_) => break,
                    };
                    if frame["type"] == "reply" {
                        answered.lock().unwrap().push(frame);
                        continue;
                    }
                    if frame["type"] != "call" {
                        continue;
                    }
                    log.lock().unwrap().push(frame["call"].clone());
                    match answer(&frame["call"]) {
                        RawReply::Ok(value) => ws.send(Message::text(json!({ "type": "reply", "id": frame["id"], "ok": value }).to_string())).unwrap(),
                        RawReply::Err(err) => ws.send(Message::text(json!({ "type": "reply", "id": frame["id"], "err": err }).to_string())).unwrap(),
                        RawReply::Hangup => {
                            if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
                                let _ = stream.shutdown(Shutdown::Both);
                            }
                            break;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        RawMachine { calls, stop, outbox, replies, next: AtomicU64::new(1) }
    }

    fn calls(&self, cmd: &str) -> usize {
        self.calls.lock().unwrap().iter().filter(|c| c["cmd"] == cmd).count()
    }

    /// A hub call from this machine, as `Link::call_hub` sends it.
    fn ask(&self, call: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.outbox.lock().unwrap().push(json!({ "type": "call", "id": id, "call": call }));
        let start = Instant::now();
        loop {
            if let Some(reply) = self.replies.lock().unwrap().iter().find(|r| r["id"] == id) {
                return match reply.get("err") {
                    Some(e) => Err(e.as_str().unwrap().to_owned()),
                    None => Ok(reply["ok"].clone()),
                };
            }
            assert!(start.elapsed() < Duration::from_secs(40), "timed out waiting for the hub's answer to {call}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for RawMachine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[test]
fn a_lost_answer_is_reported_as_unknown_and_never_acted_on_twice() {
    let hub = TestHub::start("agent_lost");
    let url = hub.url.clone();
    let http = agent_http();
    let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    let (machine, machine_token) = account::register_machine(&url, &token, "flaky", "linux").unwrap();
    // The session's tab shows up only after the test says so, as a machine that started it but whose answer was lost.
    let tab_visible = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let session_env = Arc::new(Mutex::new(String::new()));
    let (visible, env) = (tab_visible.clone(), session_env.clone());
    let raw = RawMachine::start(&url, &machine_token, move |call| match call["cmd"].as_str() {
        Some("list_projects") => RawReply::Ok(json!([{ "path": "/w/app", "name": "app", "account": "claude" }])),
        Some("run") => {
            let connector = call["run"]["env"].as_array().unwrap().iter().find(|pair| pair[0] == "PINNED_CONNECTOR").unwrap()[1].as_str().unwrap().to_owned();
            *env.lock().unwrap() = connector;
            RawReply::Hangup
        }
        Some("prompt") => RawReply::Hangup,
        Some("tabs") => {
            let connector = env.lock().unwrap().clone();
            match visible.load(Ordering::SeqCst) {
                true => RawReply::Ok(json!([{ "id": "tab-1-1", "cwd": "/w/app", "bypass": false, "running": true, "status": "done", "title": null, "message": null, "resume_at": null, "connector": connector, "opened": 1 }])),
                false => RawReply::Ok(json!([])),
            }
        }
        _ => RawReply::Ok(Value::Null),
    });
    client.wait_machines(&[("flaky", true)]);
    let client_id = register_agent(&url, &http);
    let (access, _) = connect_agent(&url, &http, &mut client, &client_id, &["sessions"], json!([{ "machine": machine.id, "path": "/w/app" }]));
    let pid = tool(&url, &http, &access, "list_projects", json!({})).unwrap()["projects"][0]["id"].as_str().unwrap().to_owned();

    let start = json!({ "project": pid, "task": "Run the tests", "request_id": "start-1" });
    let lost = tool(&url, &http, &access, "start_session", start.clone()).unwrap();
    assert_eq!(lost["state"], "unknown", "{lost}");
    let session = lost["session_id"].as_str().unwrap().to_owned();
    assert_eq!(raw.calls("run"), 1);
    let back_online = || wait_for("the machine back online", || tool(&url, &http, &access, "list_projects", json!({})).unwrap()["projects"][0]["online"] == true);
    back_online();

    // Retries with the same key look for the session instead of starting another.
    let again = tool(&url, &http, &access, "start_session", start.clone()).unwrap();
    assert_eq!((again["state"].as_str(), again["session_id"].as_str()), (Some("unknown"), Some(session.as_str())));
    assert_eq!(raw.calls("run"), 1, "a retry after a lost answer never sends the run again");
    back_online();
    assert_eq!(tool(&url, &http, &access, "list_sessions", json!({})).unwrap()["sessions"][0]["state"], "not_found");
    tab_visible.store(true, Ordering::SeqCst);
    let found = tool(&url, &http, &access, "start_session", start.clone()).unwrap();
    assert_eq!((found["state"].as_str(), found["session_id"].as_str()), (Some("starting"), Some(session.as_str())), "the tab that appeared settles it");
    assert_eq!(tool(&url, &http, &access, "start_session", start).unwrap()["replayed"], true);
    assert_eq!(raw.calls("run"), 1);
    assert_eq!(tool(&url, &http, &access, "read_session", json!({ "session_id": session })).unwrap()["state"], "done");

    // A message whose answer is lost is unknown, and its retry types nothing.
    let message = json!({ "session_id": session, "message": "also lint", "request_id": "msg-1" });
    let unknown = tool(&url, &http, &access, "send_message", message.clone()).unwrap();
    assert_eq!(unknown["delivered"], "unknown", "{unknown}");
    back_online();
    assert_eq!(tool(&url, &http, &access, "send_message", message).unwrap()["delivered"], "unknown");
    assert_eq!(raw.calls("prompt"), 1, "the retry did not type the message again");
}

/// Two machines of one account: `mini` hosts the global orchestrator and has its tab open, `mac`
/// has a project `site`. Each records every claude launch.
struct Fleet {
    hub: TestHub,
    token: String,
    client: Client,
    mac: Node,
    mini: Node,
    mac_dir: PathBuf,
    mini_dir: PathBuf,
    mac_id: String,
    mini_id: String,
    site: String,
    global: String,
}

impl Fleet {
    fn start(name: &str) -> Fleet {
        let hub = TestHub::start(name);
        let url = hub.url.clone();
        let (token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
        let mut client = Client::connect(&url, &token);
        client.wait("hello", |f| f["type"] == "hello");
        let (mac_dir, mini_dir) = (temp(&format!("{name}-mac")), temp(&format!("{name}-mini")));
        let (mac_creds, mini_creds) = (pair(&url, &mut client, "mac", &mac_dir), pair(&url, &mut client, "mini", &mini_dir));
        let (mac_id, mini_id) = (mac_creds.machine_id.clone(), mini_creds.machine_id.clone());
        let (mac, mini) = (Node::start(&mac_dir, mac_creds), Node::start(&mini_dir, mini_creds));
        for dir in [&mac_dir, &mini_dir] {
            std::fs::write(dir.join("claude"), "#!/bin/sh\n[ \"$1 $2\" = \"auth status\" ] && { echo '{\"loggedIn\":true}'; exit 0; }\nprintf '%s\\n' \"$@\" >> \"$0.$PINNED_TAB.args\"\nexec cat\n").unwrap();
        }
        client.wait_machines(&[("mac", true), ("mini", true)]);
        mini.machine.listen(|_| Err("not in this test".into())).unwrap();
        client.call(&mini_id, json!({ "cmd": "set_global_orchestrator_settings", "settings": { "host": true } })).unwrap();
        let global = client.call(&mini_id, json!({ "cmd": "open_global_orchestrator" })).unwrap().as_str().unwrap().to_owned();
        std::fs::create_dir_all(mac_dir.join("site")).unwrap();
        let site = mac_dir.join("site").display().to_string();
        Fleet { hub, token, client, mac, mini, mac_dir, mini_dir, mac_id, mini_id, site, global }
    }

    /// What the global orchestrator's tab gets from `send_to_captain`, through mini's session socket.
    fn send_to_captain(&self, project: &str, message: &str, request_id: &str) -> Result<Value, String> {
        session_tool(&self.mini_dir, &self.global, json!({ "name": "send_to_captain", "project": project, "message": message, "request_id": request_id }))
    }

    fn set_host(&mut self, machine: &str, host: bool) {
        let machine = machine.to_owned();
        self.client.call(&machine, json!({ "cmd": "set_global_orchestrator_settings", "settings": { "host": host } })).unwrap();
    }
}

/// A session's MCP tool call on the node's socket, answered as the tool's text read as JSON.
fn session_tool(dir: &Path, tab: &str, tool: Value) -> Result<Value, String> {
    let mut stream = std::os::unix::net::UnixStream::connect(dir.join("sock")).unwrap();
    writeln!(stream, "{}", json!({ "kind": "project", "tab": tab, "tool": tool })).unwrap();
    let mut reply = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stream), &mut reply).unwrap();
    let reply: Result<String, String> = serde_json::from_str(&reply).unwrap();
    reply.map(|text| serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// Each argument of every claude launch in `tab`, one per line.
fn launches(dir: &Path, tab: &str) -> String {
    std::fs::read_to_string(dir.join(format!("claude.{tab}.args"))).unwrap_or_default()
}

fn wait_launch(dir: &Path, tab: &str, line: &str) {
    wait_for(&format!("{tab} to start on {line:?}"), || launches(dir, tab).lines().any(|l| l == line));
}

/// Makes `dir` a checkout whose origin is `remote`.
fn origin(dir: &Path, remote: &str) {
    std::fs::create_dir_all(dir).unwrap();
    for args in [&["init", "-q"][..], &["remote", "add", "origin", remote]] {
        assert!(pinned_core::process::command("git").arg("-C").arg(dir).args(args).status().unwrap().success());
    }
}

fn captain(machine: &str, path: &str, text: &str) -> HubCall {
    HubCall::Relay { request_id: "r-x".into(), to: RelayTo::Captain { machine: machine.into(), path: path.into(), text: text.into(), sender: "Orchestrator".into(), busy: None } }
}

fn report(tab: &str, path: &str, text: &str) -> HubCall {
    let report = CaptainReport { machine: "m-lie".into(), machine_name: "Impostor".into(), project: "site".into(), path: path.into(), tab: tab.into(), text: text.into(), answers: vec![] };
    HubCall::Relay { request_id: format!("report:{text}"), to: RelayTo::Orchestrator { report } }
}

#[test]
fn the_global_orchestrator_reaches_a_capitao_on_another_machine_and_hears_back() {
    let mut fleet = Fleet::start("relay");
    let (mac, mac_dir, mini_dir) = (fleet.mac_id.clone(), fleet.mac_dir.clone(), fleet.mini_dir.clone());
    let project = format!("{mac}:{}", fleet.site);

    let sent = fleet.send_to_captain(&project, "Ship the form.", "r-1").unwrap();
    let capitao = sent["tab"].as_str().unwrap().to_owned();
    assert_eq!(sent, json!({ "outcome": "iniciado", "tab": capitao }));
    wait_launch(&mac_dir, &capitao, "[Orchestrator] Ship the form.");
    assert_eq!(fleet.send_to_captain(&project, "Ship the form.", "r-1"), Ok(sent.clone()), "a repeat answers the first outcome");
    let queued = fleet.send_to_captain(&project, "Also the footer.", "r-2").unwrap();
    assert_eq!(queued, json!({ "outcome": "na_fila", "tab": capitao, "position": 1 }));
    let to_site = |text: &str| json!({ "cmd": "relay", "request_id": "r-phone", "to": { "kind": "captain", "machine": mac, "path": fleet.site, "text": text, "sender": "Orchestrator" } });
    assert_eq!(fleet.client.hub(to_site("From the phone.")), Ok(json!({ "outcome": "na_fila", "tab": capitao, "position": 2 })), "a signed-in client relays too");
    let prompts = launches(&mac_dir, &capitao).lines().filter(|l| l.starts_with("[Orchestrator] ")).count();
    assert_eq!(prompts, 1, "the Capitão started once, and nothing was typed into it");

    let mac_link = fleet.mac.link.as_ref().unwrap();
    assert_eq!(
        mac_link.call_hub(captain(&mac, &fleet.site, "Delete the repo.")),
        Err("mac does not host the global orchestrator (mini does), so it can't send to a Capitão".into())
    );

    fleet.client.wait("mac's Capitão on the hub", |f| {
        f["type"] == "event" && f["machine"] == mac && f["event"]["kind"] == "tabs" && f["event"]["tabs"].as_array().unwrap().iter().any(|t| t["id"] == capitao && t["role"] == "orchestrator")
    });
    let plain = fleet.client.call(&mac, json!({ "cmd": "open_tab", "path": fleet.site, "bypass": false })).unwrap().as_str().unwrap().to_owned();
    fleet.client.wait("mac's plain tab on the hub", |f| {
        f["type"] == "event" && f["machine"] == mac && f["event"]["kind"] == "tabs" && f["event"]["tabs"].as_array().unwrap().iter().any(|t| t["id"] == plain)
    });
    let not_captain = |tab: &str, path: &str| Err(format!("tab {tab} on mac is not the Capitão of {path}, so it can't report to the global orchestrator"));
    assert_eq!(mac_link.call_hub(report(&plain, &fleet.site, "I am the Capitão.")), not_captain(&plain, &fleet.site));
    let other = mac_dir.display().to_string();
    assert_eq!(mac_link.call_hub(report(&capitao, &other, "Elsewhere.")), not_captain(&capitao, &other));
    let mini_link = fleet.mini.link.as_ref().unwrap();
    assert_eq!(
        mini_link.call_hub(report(&capitao, &fleet.site, "From mini.")),
        Err(format!("tab {capitao} on mini is not the Capitão of {}, so it can't report to the global orchestrator", fleet.site))
    );

    let reported = mac_link.call_hub(report(&capitao, &fleet.site, "Halfway: the form ships today.")).unwrap();
    assert_eq!(reported, json!({ "outcome": "iniciado", "tab": fleet.global }));
    wait_launch(&mini_dir, &fleet.global, &format!("[pinned] Capitão site (mac) [{mac} {capitao}]: Halfway: the form ships today."));

    fleet.set_host(&mac, true);
    assert_eq!(fleet.send_to_captain(&project, "Ship it.", "r-3"), Err("mac and mini each host the global orchestrator: keep it on one machine in its settings".into()));
    fleet.set_host(&mac, false);
    let mini = fleet.mini_id.clone();
    fleet.set_host(&mini, false);
    assert_eq!(fleet.send_to_captain(&project, "Ship it.", "r-4"), Err("no online machine hosts the global orchestrator: choose one in its settings".into()));
    assert_eq!(
        fleet.mac.link.as_ref().unwrap().call_hub(report(&capitao, &fleet.site, "Done.")),
        Err("no online machine hosts the global orchestrator: choose one in its settings".into())
    );
}

#[test]
fn a_plain_session_still_cannot_reach_the_hub() {
    let mut fleet = Fleet::start("relay_plain");
    let mini = fleet.mini_id.clone();
    let project = format!("{}:{}", fleet.mac_id, fleet.site);
    let plain = fleet.client.call(&mini, json!({ "cmd": "open_tab", "path": null, "bypass": false })).unwrap().as_str().unwrap().to_owned();
    let send = json!({ "name": "send_to_captain", "project": project, "message": "Ship it.", "request_id": "r-1" });
    assert_eq!(session_tool(&fleet.mini_dir, &plain, send), Err("only the global orchestrator has this tool".into()));
    assert_eq!(
        session_tool(&fleet.mini_dir, &plain, json!({ "name": "tell_orchestrator", "text": "Done." })),
        Err("only a project's Capitão reports to the global orchestrator".into())
    );
    let mini_link = fleet.mini.link.as_ref().unwrap();
    assert_eq!(mini_link.call_hub(HubCall::Machines), Err("a machine may only forward a workflow session's callbacks".into()));
    let started = std::fs::read_dir(&fleet.mac_dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".args")).count();
    assert_eq!(started, 0, "nothing started on mac");
}

#[test]
fn a_machine_too_old_for_the_relay_is_named_for_an_update() {
    let mut fleet = Fleet::start("relay_old");
    let (machine, machine_token) = account::register_machine(&fleet.hub.url, &fleet.token, "old", "linux").unwrap();
    let old = RawMachine::start(&fleet.hub.url, &machine_token, |call| {
        RawReply::Err(format!("this machine runs pinned 0.17.0, which can't answer that: unknown variant `{}`", call["cmd"].as_str().unwrap()))
    });
    fleet.client.wait_machines(&[("mac", true), ("mini", true), ("old", true)]);
    let sent = fleet.send_to_captain(&format!("{}:/w/site", machine.id), "Ship it.", "r-1");
    assert_eq!(sent, Err("old precisa atualizar o pinned".into()));
    assert_eq!((old.calls("global_orchestrator_settings"), old.calls("enqueue")), (2, 1), "the overview and the relay each asked whether it hosts");
}

#[test]
fn the_overview_lists_a_repo_on_two_machines_once_and_an_old_or_offline_machine_without_failing() {
    let mut fleet = Fleet::start("overview");
    let (mac, mini) = (fleet.mac_id.clone(), fleet.mini_id.clone());
    let mini_site = fleet.mini_dir.join("site");
    origin(Path::new(&fleet.site), "git@github.com:acme/site.git");
    origin(&mini_site, "https://github.com/acme/site.git");
    fleet.mac.machine.open_project(PathBuf::from(&fleet.site));
    let mini_site = fleet.mini.machine.open_project(mini_site).display().to_string();
    let (old, old_token) = account::register_machine(&fleet.hub.url, &fleet.token, "old", "linux").unwrap();
    let raw = RawMachine::start(&fleet.hub.url, &old_token, |call| match call["cmd"].as_str().unwrap() {
        "list_projects" => RawReply::Ok(json!([{ "path": "/srv/notes", "name": "notes", "account": "claude" }])),
        "tabs" => RawReply::Ok(json!([])),
        cmd => RawReply::Err(format!("this machine runs pinned 0.17.0, which can't answer that: unknown variant `{cmd}`")),
    });
    fleet.client.wait_machines(&[("mac", true), ("mini", true), ("old", true)]);
    // A node lists its projects by when they were opened, which ties within a second.
    let by_key = |mut overview: Value| {
        overview["projects"].as_array_mut().unwrap().sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
        overview
    };
    let projects = || session_tool(&fleet.mini_dir, &fleet.global, json!({ "name": "projects" })).map(by_key);
    let checkout = |machine: &str, name: &str, online: bool, version: &str, path: &str| {
        json!({ "machine": machine, "machine_name": name, "online": online, "version": version, "path": path, "captain": "nenhum", "workers": 0 })
    };
    let site = |mac_online: bool| {
        json!({ "key": "github.com/acme/site", "name": "site", "repo": "github.com/acme/site", "checkouts": [
            checkout(&mac, "mac", mac_online, "0.0.0-test", &fleet.site),
            checkout(&mini, "mini", true, "0.0.0-test", &mini_site),
        ] })
    };
    let notes = json!({ "key": format!("{}:/srv/notes", old.id), "name": "notes", "checkouts": [checkout(&old.id, "old", true, "test", "/srv/notes")] });
    // Each node's own folder is a project too, with no origin.
    let home = |id: &str, name: &str, online: bool, dir: &Path| {
        let dir = dir.display().to_string();
        json!({ "key": format!("{id}:{dir}"), "name": Path::new(&dir).file_name().unwrap().to_str().unwrap(), "checkouts": [checkout(id, name, online, "0.0.0-test", &dir)] })
    };
    let machine = |id: &str, name: &str, online: bool, version: &str| json!({ "id": id, "name": name, "online": online, "version": version });
    let old_machine = json!({ "id": old.id, "name": "old", "online": true, "version": "test", "problem": "precisa atualizar o pinned" });

    assert_eq!(
        projects(),
        Ok(by_key(json!({
            "projects": [site(true), home(&mac, "mac", true, &fleet.mac_dir), home(&mini, "mini", true, &fleet.mini_dir), notes],
            "waiting": [],
            "machines": [machine(&mac, "mac", true, "0.0.0-test"), machine(&mini, "mini", true, "0.0.0-test"), old_machine],
        })))
    );
    assert_eq!((raw.calls("list_projects"), raw.calls("global_orchestrator_settings"), raw.calls("tabs")), (1, 1, 1), "the hub asked for the tabs only of the machine that never reported them");
    let mac_link = fleet.mac.link.as_ref().unwrap();
    assert_eq!(mac_link.call_hub(HubCall::Overview), Err("mac does not host the global orchestrator (mini does), so it can't list the account's projects".into()));
    let signed_in = fleet.client.hub(json!({ "cmd": "overview" })).unwrap();
    let signed_in_site = signed_in["projects"].as_array().unwrap().iter().find(|p| p["key"] == "github.com/acme/site");
    assert_eq!(signed_in_site, Some(&site(true)), "a signed-in client reads it too");

    drop(raw);
    fleet.mac.kill();
    fleet.client.wait_machines(&[("mac", false), ("mini", true), ("old", false)]);
    let offline = json!({ "key": format!("{}:/srv/notes", old.id), "name": "notes", "checkouts": [checkout(&old.id, "old", false, "test", "/srv/notes")] });
    assert_eq!(
        projects(),
        Ok(by_key(json!({
            "projects": [site(false), home(&mac, "mac", false, &fleet.mac_dir), home(&mini, "mini", true, &fleet.mini_dir), offline],
            "waiting": [],
            "machines": [machine(&mac, "mac", false, "0.0.0-test"), machine(&mini, "mini", true, "0.0.0-test"), machine(&old.id, "old", false, "test")],
        }))),
        "an offline machine keeps the projects it listed last, with no Capitão"
    );
}

#[test]
fn the_global_orchestrator_names_a_capitao_by_repo_and_machine_sees_it_working_at_once_and_reads_it() {
    let fleet = Fleet::start("reach");
    let mac = fleet.mac_id.clone();
    origin(Path::new(&fleet.site), "git@github.com:acme/site.git");
    origin(&fleet.mini_dir.join("site"), "git@github.com:acme/site.git");
    fleet.mac.machine.open_project(PathBuf::from(&fleet.site));
    fleet.mini.machine.open_project(fleet.mini_dir.join("site"));
    let tool = |tool: Value| session_tool(&fleet.mini_dir, &fleet.global, tool);

    let sent = tool(json!({ "name": "send_to_captain", "project": "github.com/acme/site", "machine": "@mac", "message": "Ship the form.", "request_id": "r-1" })).unwrap();
    let capitao = sent["tab"].as_str().unwrap().to_owned();
    assert_eq!(sent, json!({ "outcome": "iniciado", "tab": capitao }));
    wait_launch(&fleet.mac_dir, &capitao, "[Orchestrator] Ship the form.");
    let overview = tool(json!({ "name": "projects" })).unwrap();
    let site = overview["projects"].as_array().unwrap().iter().find(|p| p["key"] == "github.com/acme/site").unwrap();
    let on_mac = site["checkouts"].as_array().unwrap().iter().find(|c| c["machine"] == mac).unwrap();
    assert_eq!(
        (&on_mac["machine"], &on_mac["captain"], &on_mac["captain_tab"]),
        (&json!(mac), &json!("trabalhando"), &json!(capitao)),
        "before its first hook, the Capitão a message started is working"
    );
    let queued = tool(json!({ "name": "send_to_captain", "project": format!("mac:{}", fleet.site), "message": "Also the footer.", "request_id": "r-2" }));
    assert_eq!(queued, Ok(json!({ "outcome": "na_fila", "tab": capitao, "position": 1 })), "a send agrees with the overview");

    let read = tool(json!({ "name": "read_captain", "project": "github.com/acme/site", "machine": "mac" })).unwrap();
    assert_eq!(read, json!({ "state": "starting", "next": "it is starting, and may be showing the user a startup prompt; read again in a minute", "title": null, "conversation": { "items": [], "cursor": 0, "reset": false }, "project": fleet.site }));
    let mac_link = fleet.mac.link.as_ref().unwrap();
    assert_eq!(
        mac_link.call_hub(HubCall::ReadCaptain { machine: mac.clone(), path: fleet.site.clone() }),
        Err("mac does not host the global orchestrator (mini does), so it can't read a Capitão".into())
    );
    let elsewhere = tool(json!({ "name": "read_captain", "project": format!("{mac}:{}", fleet.mac_dir.display()) })).unwrap();
    assert_eq!(elsewhere, json!({ "state": "not_started", "next": "message it to start it", "project": fleet.mac_dir.display().to_string() }), "a folder with no Capitão, named by machine id");
    let unknown = tool(json!({ "name": "read_captain", "project": "/w/site", "machine": "Studio" }));
    assert_eq!(unknown, Err("this account has no machine Studio".into()), "a mistyped machine is not an offline one");
}

// Isolation between accounts: two accounts share one hub, and nothing of one reaches the other.

fn signed_in(url: &str, email: &str) -> (String, String, Client) {
    let (token, account) = account::signup(url, email, "correct horse").unwrap();
    let mut client = Client::connect(url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    (token, account.id, client)
}

/// Frames that name `needle` anywhere, after the client read all that came.
fn mentioning(client: &mut Client, needle: &str) -> Vec<Value> {
    settle(client);
    client.frames.iter().filter(|f| f.to_string().contains(needle)).cloned().collect()
}

/// Every frame the machine got within `wait`.
fn heard(sock: &mut MachineSock, wait: Duration) -> Vec<Value> {
    let start = Instant::now();
    let mut frames = vec![];
    while start.elapsed() < wait {
        frames.extend(sock.try_text());
    }
    frames
}

fn captain_tab(id: &str, path: &str) -> Value {
    json!({ "id": id, "cwd": path, "bypass": false, "running": true, "status": "done", "title": null, "message": null, "resume_at": null, "opened": 1, "role": "orchestrator" })
}

#[test]
fn another_accounts_machine_takes_no_call_attach_watch_or_change_from_this_one() {
    let hub = TestHub::start("iso_calls");
    let url = hub.url.clone();
    let (_, _, mut ana) = signed_in(&url, "ana@acme.dev");
    let (bob_token, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let (machine, machine_token) = account::register_machine(&url, &bob_token, "bob-mac", "linux").unwrap();
    let mut sock = MachineSock::connect(&url, &machine_token, "0.17.20");
    bob.wait_machines(&[("bob-mac", true)]);
    let id = machine.id.clone();
    let none = Err(format!("this account has no machine {id}"));

    for call in [json!({ "cmd": "tabs" }), json!({ "cmd": "prompt", "tab": "tab-1", "text": "rm -rf ~" }), json!({ "cmd": "close_tab", "tab": "tab-1" }), json!({ "cmd": "update_self" })] {
        assert_eq!(ana.call(&id, call.clone()), none, "{call}");
    }
    assert_eq!(ana.call(&id, json!({ "cmd": "watch_threads", "tabs": ["tab-1"] })), Ok(Value::Null), "a watch is answered by the hub, which passes it to no machine");
    ana.send(json!({ "type": "attach", "machine": id, "tab": "tab-1" }));
    ana.send(json!({ "type": "detach", "machine": id, "tab": "tab-1" }));
    assert_eq!(ana.hub(json!({ "cmd": "rename_machine", "id": id, "name": "mine now" })), none);
    assert_eq!(ana.hub(json!({ "cmd": "remove_machine", "id": id })), none);
    assert_eq!(ana.hub(json!({ "cmd": "clone", "machine": id, "repo": "github.com/acme/site" })), none);
    assert_eq!(ana.hub(json!({ "cmd": "box_status", "machine": id })), Err("this machine is no box".into()));
    assert_eq!(ana.hub(json!({ "cmd": "extend_box", "machine": id, "extra_usd": 5.0 })), Err("this machine is no box".into()));
    assert_eq!(heard(&mut sock, Duration::from_millis(400)), Vec::<Value>::new(), "bob's machine heard nothing of ana's");
    settle(&mut bob);
    assert_eq!(bob.machines(), vec![("bob-mac".to_owned(), true)], "still bob's, under its name");

    bob.send(json!({ "type": "attach", "machine": id, "tab": "tab-1" }));
    let subscribe = sock.read_json();
    assert_eq!((subscribe["type"].as_str(), subscribe["tab"].as_str()), (Some("subscribe"), Some("tab-1")), "bob's own attach does reach it");
}

#[test]
fn a_machines_tabs_usage_output_and_presence_reach_only_its_own_accounts_clients() {
    let hub = TestHub::start("iso_fanout");
    let url = hub.url.clone();
    let (ana_token, _, mut ana) = signed_in(&url, "ana@acme.dev");
    let (bob_token, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let (machine, machine_token) = account::register_machine(&url, &bob_token, "bob-mac", "linux").unwrap();
    let id = machine.id.clone();
    let mut sock = MachineSock::connect(&url, &machine_token, "0.17.20");
    bob.wait_machines(&[("bob-mac", true)]);
    bob.send(json!({ "type": "attach", "machine": id, "tab": "tab-1" }));
    assert_eq!(bob.call(&id, json!({ "cmd": "watch_threads", "tabs": ["tab-1"] })).err(), None);

    let mut tab = captain_tab("tab-1", "/b/secret-project");
    tab["question"] = json!("Ship the unreleased pricing page?");
    tab["orchestrator_turn"] = json!(true);
    tab["queued"] = json!(2);
    for event in [
        json!({ "kind": "tabs", "tabs": [tab] }),
        json!({ "kind": "usage", "usage": { "five_hour": { "used_percentage": 42.0, "resets_at": 1792400000 } }, "account": "claude" }),
        json!({ "kind": "thread", "tab": "tab-1" }),
        json!({ "kind": "exit", "tab": "tab-1" }),
        json!({ "kind": "update", "status": { "state": "updating", "to": "0.17.21" } }),
    ] {
        sock.send(json!({ "type": "event", "event": event }));
    }
    // A machine names its viewer, so it may name any client of the hub: it still reaches only its own account's.
    for n in 0..64 {
        sock.send(json!({ "type": "output", "viewer": format!("client-{n}"), "tab": "tab-1", "data": "c2VjcmV0" }));
    }
    bob.wait("bob's tabs", |f| f["type"] == "event" && f["event"]["kind"] == "tabs" && f.to_string().contains("secret-project"));
    bob.wait("bob's thread nudge", |f| f["type"] == "event" && f["event"]["kind"] == "thread");
    bob.wait("bob's output", |f| f["type"] == "event" && f["event"]["kind"] == "output");
    wait_row(&mut bob, &id, "bob's update", |m| m["update"]["state"] == "updating");

    assert_eq!(mentioning(&mut ana, &id), Vec::<Value>::new(), "ana got no frame about bob's machine");
    assert_eq!(mentioning(&mut ana, "c2VjcmV0"), Vec::<Value>::new(), "nor its output");
    assert_eq!(mentioning(&mut ana, "unreleased pricing"), Vec::<Value>::new(), "nor what its sessions ask");
    let mut late = Client::connect(&url, &ana_token);
    let hello = late.wait("hello", |f| f["type"] == "hello");
    assert_eq!((&hello["machines"], &hello["tabs"]), (&json!([]), &json!({})), "a client that connects later sees none of bob's machines or tabs");
    assert_eq!(mentioning(&mut late, &id), Vec::<Value>::new(), "nor bob's usage");
    assert_eq!(ana.hub(json!({ "cmd": "machines" })), Ok(json!([])));

    drop(sock);
    bob.wait_machines(&[("bob-mac", false)]);
    assert_eq!(mentioning(&mut ana, &id), Vec::<Value>::new(), "nor its going offline");

    bob.hub(json!({ "cmd": "set_box_access", "access": { "workflow": { "mode": "ask" } } })).unwrap();
    bob.wait("bob's new settings", |f| f["type"] == "settings");
    settle(&mut ana);
    assert_eq!(ana.frames.iter().filter(|f| f["type"] == "settings").count(), 0, "bob's settings reach only bob's clients");
}

#[test]
fn the_global_orchestrator_reaches_no_machine_project_or_capitao_of_another_account() {
    let hub = TestHub::start("iso_global");
    let url = hub.url.clone();
    let (ana_token, ana_id, mut ana) = signed_in(&url, "ana@acme.dev");
    let (bob_token, bob_id, mut bob) = signed_in(&url, "bob@acme.dev");
    let (ana_machine, ana_machine_token) = account::register_machine(&url, &ana_token, "ana-mini", "linux").unwrap();
    let (bob_machine, bob_machine_token) = account::register_machine(&url, &bob_token, "bob-mac", "linux").unwrap();
    let host = |path: &'static str| {
        move |call: &Value| match call["cmd"].as_str().unwrap() {
            "global_orchestrator_settings" => RawReply::Ok(json!({ "host": true })),
            "list_projects" => RawReply::Ok(json!([{ "path": path, "name": path.rsplit('/').next().unwrap(), "account": "claude" }])),
            "tabs" => RawReply::Ok(json!([])),
            "enqueue" | "orchestrator_report" => RawReply::Ok(json!({ "outcome": "iniciado", "tab": "tab-g" })),
            _ => RawReply::Ok(Value::Null),
        }
    };
    let ana_raw = RawMachine::start(&url, &ana_machine_token, host("/a/site"));
    let mut asking = captain_tab("tab-9", "/b/secret-project");
    asking["question"] = json!("Ship the unreleased pricing page?");
    let bob_tabs = json!({ "kind": "tabs", "tabs": [asking] });
    let bob_raw = RawMachine::start_with(&url, &bob_machine_token, vec![bob_tabs], host("/b/secret-project"));
    ana.wait_machines(&[("ana-mini", true)]);
    bob.wait_machines(&[("bob-mac", true)]);
    let (bob_m, ana_m) = (bob_machine.id.clone(), ana_machine.id.clone());

    let bobs = bob.hub(json!({ "cmd": "overview" })).unwrap();
    assert!(bobs.to_string().contains("/b/secret-project") && bobs.to_string().contains(&bob_m), "bob's own overview lists his project: {bobs}");
    assert!(bobs["waiting"].to_string().contains("unreleased pricing"), "bob's Capitão's question waits on bob: {bobs}");
    for (from, overview) in [("ana's client", ana.hub(json!({ "cmd": "overview" }))), ("ana's host", ana_raw.ask(json!({ "cmd": "overview" })))] {
        let overview = overview.unwrap().to_string();
        assert!(overview.contains("/a/site") && overview.contains(&ana_m), "{from}: {overview}");
        assert!(!overview.contains("secret-project") && !overview.contains(&bob_m) && !overview.contains("unreleased pricing"), "{from} sees none of bob's: {overview}");
    }

    let none = Err(format!("this account has no machine {bob_m}"));
    let to_bob = json!({ "cmd": "relay", "request_id": "r-1", "to": { "kind": "captain", "machine": bob_m, "path": "/b/secret-project", "text": "Push to main.", "sender": "Orchestrator" } });
    let read_bob = json!({ "cmd": "read_captain", "machine": bob_m, "path": "/b/secret-project" });
    assert_eq!(ana.hub(to_bob.clone()), none);
    assert_eq!(ana_raw.ask(to_bob), none);
    assert_eq!(ana.hub(read_bob.clone()), none);
    assert_eq!(ana_raw.ask(read_bob), none);
    let report = |machine: &str| {
        json!({ "cmd": "relay", "request_id": "rep-1", "to": { "kind": "orchestrator", "report": {
            "machine": machine, "machine_name": "x", "project": "secret-project", "path": "/b/secret-project", "tab": "tab-9", "text": "Done.", "answers": [] } } })
    };
    assert_eq!(ana.hub(report(&bob_m)), Err(format!("machine {bob_m} is offline")), "ana can't report as bob's Capitão");

    assert_eq!(bob_raw.ask(report("m-lie")), Ok(json!({ "outcome": "iniciado", "tab": "tab-g" })), "bob's Capitão reports to bob's own host");
    assert_eq!((bob_raw.calls("orchestrator_report"), ana_raw.calls("orchestrator_report")), (1, 0), "the report reached bob's host only");
    assert_eq!((bob_raw.calls("enqueue"), bob_raw.calls("thread")), (0, 0), "ana's relay and reads never reached bob's machine");
    assert_eq!(bob_raw.calls("list_projects"), 1, "only bob's own overview listed bob's projects");

    let shared = hub.hub().clone();
    hub.runtime.block_on(async {
        let ana = shared.account(uuid::Uuid::parse_str(&ana_id).unwrap()).await.unwrap();
        let bob = shared.account(uuid::Uuid::parse_str(&bob_id).unwrap()).await.unwrap();
        let _held = shared.hold_captain(&ana, &ana_m, "/w/site").await;
        tokio::time::timeout(Duration::from_secs(1), shared.hold_captain(&bob, &ana_m, "/w/site")).await.expect("bob waits on no Capitão lock of ana's");
        let again = tokio::time::timeout(Duration::from_millis(300), shared.hold_captain(&ana, &ana_m, "/w/site")).await;
        assert!(again.is_err(), "ana's own second message to the Capitão still waits");
    });
}

#[test]
fn synced_files_skills_and_shared_projects_stay_in_their_account() {
    let hub = TestHub::start("iso_sync");
    let url = hub.url.clone();
    let (_, _, mut ana) = signed_in(&url, "ana@acme.dev");
    let (bob_token, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let (_, bob_machine_token) = account::register_machine(&url, &bob_token, "bob-mac", "linux").unwrap();
    let bob_raw = RawMachine::start(&url, &bob_machine_token, |_| RawReply::Ok(Value::Null));
    bob.wait_machines(&[("bob-mac", true)]);
    let file = |path: &str, text: &str| {
        json!({ "path": path, "sha": sha256_hex(text.as_bytes()), "modified": 1, "data": base64::engine::general_purpose::STANDARD.encode(gzip_bytes(text.as_bytes())) })
    };
    let project = "github.com/acme/site";
    let memory = "claude/memory/notes.md";
    ana.hub(json!({ "cmd": "sync_push", "project": project, "files": [file(memory, "ana's plans")] })).unwrap();
    ana.hub(json!({ "cmd": "sync_push", "project": sync::PLUGINS_KEY, "files": [file(HOOK_PATH, "#!/bin/sh\necho ana")] })).unwrap();
    ana.hub(json!({ "cmd": "share_project", "project": { "key": project, "name": "site", "repo": "git@github.com:acme/site.git" } })).unwrap();

    let reads = [
        json!({ "cmd": "sync_list", "project": project }),
        json!({ "cmd": "sync_pull", "project": project, "paths": [memory] }),
        json!({ "cmd": "sync_list", "project": sync::PLUGINS_KEY }),
        json!({ "cmd": "shared_projects" }),
    ];
    for read in reads {
        assert_eq!(bob.hub(read.clone()), Ok(json!([])), "bob's client: {read}");
        let from_machine = match read["cmd"] == "shared_projects" {
            true => Err("a machine may only forward a workflow session's callbacks".to_owned()),
            false => Ok(json!([])),
        };
        assert_eq!(bob_raw.ask(read.clone()), from_machine, "bob's machine: {read}");
    }
    let ana_hook_sha = sha256_hex(b"#!/bin/sh\necho ana");
    bob.hub(json!({ "cmd": "sync_mark_executable", "project": sync::PLUGINS_KEY, "repairs": [{ "path": HOOK_PATH, "sha": ana_hook_sha }] })).unwrap();
    bob.hub(json!({ "cmd": "sync_push", "project": project, "files": [file(memory, "bob's plans")] })).unwrap();

    let listed = ana.hub(json!({ "cmd": "sync_list", "project": project })).unwrap();
    assert_eq!(listed[0]["sha"], sha256_hex(b"ana's plans"), "bob's push of the same path left ana's file alone");
    let hooks = ana.hub(json!({ "cmd": "sync_list", "project": sync::PLUGINS_KEY })).unwrap();
    assert_eq!((hooks[0]["sha"].as_str(), hooks[0]["executable"].as_bool()), (Some(ana_hook_sha.as_str()), Some(false)), "bob's repair named ana's sha and changed nothing");
    assert_eq!(bob.hub(json!({ "cmd": "sync_list", "project": project })).unwrap()[0]["sha"], sha256_hex(b"bob's plans"));
}

#[test]
fn a_pairing_code_pairs_into_the_account_that_made_it_and_a_logout_ends_only_that_login() {
    let hub = TestHub::start("iso_pair");
    let url = hub.url.clone();
    let (ana_token, _, mut ana) = signed_in(&url, "ana@acme.dev");
    let (bob_token, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let code = ana.hub(json!({ "cmd": "pair_code" })).unwrap()["code"].as_str().unwrap().to_owned();
    let creds = account::pair(&url, &code, "ana-mac", "linux").unwrap();
    assert_eq!(creds.email, "ana@acme.dev");
    ana.wait_machines(&[("ana-mac", false)]);
    assert_eq!(account::pair(&url, &code, "bob-mac", "linux"), Err("that pairing code is unknown, used or expired".into()), "a code pairs once");
    assert_eq!(bob.hub(json!({ "cmd": "machines" })), Ok(json!([])));
    assert_eq!(mentioning(&mut bob, &creds.machine_id), Vec::<Value>::new());

    account::logout(&url, &ana_token).unwrap();
    let start = Instant::now();
    while ana.pump() {
        assert!(start.elapsed() < WAIT, "ana's client stays open after her logout");
    }
    assert_eq!(bob.hub(json!({ "cmd": "machines" })), Ok(json!([])), "bob's login still works");
    let mut gone = Client::connect(&url, &ana_token);
    while gone.pump() {}
    assert_eq!(gone.closed, Some(u16::from(link::UNAUTHORIZED)));
    assert!(Client::connect(&url, &bob_token).wait("hello", |f| f["type"] == "hello")["account"]["email"] == "bob@acme.dev");
}

#[test]
fn an_agent_reaches_only_the_projects_and_orchestrators_of_the_account_that_let_it_in() {
    let hub = TestHub::start("iso_agent");
    let url = hub.url.clone();
    let http = agent_http();
    let (ana_token, _, mut ana) = signed_in(&url, "ana@acme.dev");
    let (bob_token, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let lists = |path: &'static str| {
        move |call: &Value| match call["cmd"].as_str().unwrap() {
            "list_projects" => RawReply::Ok(json!([{ "path": path, "name": "app", "account": "claude" }])),
            "tabs" => RawReply::Ok(json!([captain_tab("tab-o", path)])),
            "open_orchestrator" => RawReply::Ok(json!("tab-o")),
            _ => RawReply::Ok(Value::Null),
        }
    };
    let (ana_machine, ana_machine_token) = account::register_machine(&url, &ana_token, "ana-mac", "linux").unwrap();
    let (bob_machine, bob_machine_token) = account::register_machine(&url, &bob_token, "bob-mac", "linux").unwrap();
    let ana_raw = RawMachine::start(&url, &ana_machine_token, lists("/a/app"));
    let _bob_raw = RawMachine::start(&url, &bob_machine_token, lists("/b/app"));
    ana.wait_machines(&[("ana-mac", true)]);
    bob.wait_machines(&[("bob-mac", true)]);
    let client_id = register_agent(&url, &http);
    let scopes = ["sessions", "orchestrator"];
    let (ana_access, _) = connect_agent(&url, &http, &mut ana, &client_id, &scopes, json!([{ "machine": ana_machine.id, "path": "/a/app" }]));
    let (bob_access, _) = connect_agent(&url, &http, &mut bob, &client_id, &scopes, json!([{ "machine": bob_machine.id, "path": "/b/app" }]));
    let ana_pid = tool(&url, &http, &ana_access, "list_projects", json!({})).unwrap()["projects"][0]["id"].as_str().unwrap().to_owned();
    let bob_projects = tool(&url, &http, &bob_access, "list_projects", json!({})).unwrap();
    assert_eq!(bob_projects["projects"].as_array().map(Vec::len), Some(1));
    assert_ne!(bob_projects["projects"][0]["id"], json!(ana_pid));

    let refused = format!("no project {ana_pid} for this agent; call list_projects");
    assert_eq!(tool(&url, &http, &bob_access, "message_orchestrator", json!({ "project": ana_pid, "message": "drop the tables", "request_id": "b-1" })), Err(refused.clone()));
    assert_eq!(tool(&url, &http, &bob_access, "read_orchestrator", json!({ "project": ana_pid })), Err(refused.clone()));
    assert_eq!(tool(&url, &http, &bob_access, "start_session", json!({ "project": ana_pid, "task": "t", "request_id": "b-2" })), Err(refused));
    assert_eq!((ana_raw.calls("open_orchestrator"), ana_raw.calls("thread"), ana_raw.calls("run"), ana_raw.calls("prompt")), (0, 0, 0, 0), "bob's agent reached nothing on ana's machine");

    let ana_agents = ana.hub(json!({ "cmd": "external_agents" })).unwrap()["agents"].clone();
    let bob_agents = bob.hub(json!({ "cmd": "external_agents" })).unwrap()["agents"].clone();
    assert_eq!((ana_agents.as_array().map(Vec::len), bob_agents.as_array().map(Vec::len)), (Some(1), Some(1)), "each account lists its own grant only");
    let ana_grant = ana_agents[0]["id"].as_str().unwrap();
    assert_ne!(bob_agents[0]["id"].as_str(), Some(ana_grant));
    assert_eq!(bob.hub(json!({ "cmd": "revoke_agent", "id": ana_grant })), Err(format!("this account has no agent {ana_grant}")));
    assert_eq!(tool(&url, &http, &ana_access, "list_projects", json!({})).unwrap()["projects"][0]["id"], json!(ana_pid), "ana's agent still works");
    let request = authorize(&url, &http, &client_id, &pkce().1, "b");
    assert!(bob.hub(json!({ "cmd": "approve_agent", "request": request, "scopes": ["orchestrator"], "projects": [{ "machine": ana_machine.id, "path": "/a/app" }] })).unwrap_err().contains("this account has no machine"));
}

#[test]
fn a_dictation_is_kept_under_the_account_that_sent_it_and_no_route_reads_it_back() {
    let hub = TestHub::start_learning("iso_dictation");
    let url = hub.url.clone();
    let (ana_token, ana) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let (bob_token, bob) = account::signup(&url, "bob@acme.dev", "correct horse").unwrap();
    account::dictation_feedback(&url, &ana_token, &dictation("ana's comet")).unwrap();
    account::dictation_feedback(&url, &bob_token, &dictation("bob's comet")).unwrap();
    let mut kept: Vec<(String, String)> = learning_rows(&hub.learning.clone().unwrap()).iter().map(|r| (r["account"].as_str().unwrap().to_owned(), r["raw"].as_str().unwrap().to_owned())).collect();
    kept.sort();
    let mut expected = vec![(ana.id, "ana's comet".to_owned()), (bob.id, "bob's comet".to_owned())];
    expected.sort();
    assert_eq!(kept, expected, "each row names the account whose login sent it");
    let read = reqwest::blocking::Client::new().get(format!("{url}/api/dictation/feedback")).bearer_auth(&bob_token).send().unwrap();
    assert_eq!(read.status(), 405, "the hub only takes dictations in");
}

#[test]
fn a_box_permission_naming_another_accounts_box_opens_a_box_of_its_own() {
    let hub = TestHub::start_with_boxes("iso_box", Signup::Open, fast());
    let (bob_token, bob) = account::signup(&hub.url, "bob@acme.dev", "correct horse").unwrap();
    let (bob_machine, _) = account::register_machine(&hub.url, &bob_token, "bob-box", "linux").unwrap();
    admin_on(&hub.database, &format!(
        "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase) values ('bobs-box', '{}', '{}', 'box_bob', 'c-bob', '{{\"issue\":\"b1\"}}', '{{\"folder\":\"/w\",\"args\":[]}}', 'small', 'running')",
        bob.id, bob_machine.id,
    ));
    let mut ana = BoxAccount::new(&hub);
    ana.client.hub(json!({ "cmd": "set_box_access", "access": { "workflow": { "mode": "fixed", "row": "bobs-box" } } })).unwrap();
    let account = ana.id;
    assert_eq!(hub.runtime.block_on(hub.hub().open(account, ana.box_run(issue("uuid-1")))), Ok(()));
    assert_eq!(created(&hub), 1, "ana's run got a box of its own");
    ana.client.wait_machines(&[("Box · ENG-7 Fix login", true)]);
    assert_ne!(ana.box_machine(), bob_machine.id);
    let bobs = hub.runtime.block_on(hub.hub().db.boxes(uuid::Uuid::parse_str(&bob.id).unwrap())).unwrap();
    assert_eq!(bobs.len(), 1, "bob's box row is still there");
}

#[test]
fn an_accounts_tastes_are_read_written_and_imported_only_by_that_account() {
    let hub = TestHub::start("iso_taste");
    let url = hub.url.clone();
    let (ana_token, _, mut ana) = signed_in(&url, "ana@acme.dev");
    let (bob_token, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let (_, ana_machine_token) = account::register_machine(&url, &ana_token, "ana-mac", "linux").unwrap();
    let (_, bob_machine_token) = account::register_machine(&url, &bob_token, "bob-mac", "linux").unwrap();
    let ana_raw = RawMachine::start(&url, &ana_machine_token, |_| RawReply::Ok(Value::Null));
    let bob_raw = RawMachine::start(&url, &bob_machine_token, |_| RawReply::Ok(Value::Null));
    ana.wait_machines(&[("ana-mac", true)]);
    bob.wait_machines(&[("bob-mac", true)]);
    let defaults = bob.hub(json!({ "cmd": "get_tastes" })).unwrap();
    let site = |parallel: u32| json!({ "cmd": "set_taste", "scope": { "kind": "project", "repo": "github.com/acme/site" }, "patch": { "parallel": parallel } });

    let anas = ana.hub(site(5)).unwrap();
    assert_eq!(anas["projects"]["github.com/acme/site"]["parallel"], 5);
    assert_eq!(bob.hub(json!({ "cmd": "get_tastes" })), Ok(defaults.clone()), "bob's client reads none of ana's tastes");
    assert_eq!(bob_raw.ask(json!({ "cmd": "get_tastes" })), Ok(defaults.clone()), "nor does bob's machine");

    bob.hub(site(2)).unwrap();
    bob.hub(json!({ "cmd": "reset_taste_field", "scope": { "kind": "captains" }, "field": "hard_model" })).unwrap();
    assert_eq!(ana.hub(json!({ "cmd": "get_tastes" })), Ok(anas.clone()), "bob's writes left ana's tastes alone");
    assert_eq!(ana_raw.ask(json!({ "cmd": "get_tastes" })), Ok(anas.clone()));

    let imported = json!({ "cmd": "import_tastes", "tastes": { "projects": { "github.com/acme/other": { "parallel": 7 } } } });
    assert_eq!(ana_raw.ask(imported.clone()), Ok(json!(false)), "ana already has tastes, so her machine's import is refused");
    assert_eq!(ana.hub(json!({ "cmd": "get_tastes" })), Ok(anas));
    let (carol_token, _, mut carol) = signed_in(&url, "carol@acme.dev");
    let (_, carol_machine_token) = account::register_machine(&url, &carol_token, "carol-mac", "linux").unwrap();
    let carol_raw = RawMachine::start(&url, &carol_machine_token, |_| RawReply::Ok(Value::Null));
    carol.wait_machines(&[("carol-mac", true)]);
    assert_eq!(carol_raw.ask(imported), Ok(json!(true)), "an account with no tastes takes its machine's");
    assert_eq!(carol.hub(json!({ "cmd": "get_tastes" })).unwrap()["projects"]["github.com/acme/other"]["parallel"], 7);
    for (who, client) in [("ana", &mut ana), ("bob", &mut bob)] {
        assert!(!client.hub(json!({ "cmd": "get_tastes" })).unwrap().to_string().contains("github.com/acme/other"), "carol's import landed in {who}'s tastes");
    }
}

#[test]
fn an_accounts_box_budget_bounds_only_its_own_boxes_and_shows_only_in_its_tastes() {
    let hub = TestHub::start_with_boxes("iso_limits", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    let mut bob = BoxAccount::named(&hub, "bob@acme.dev");
    for client in [&mut ana.client, &mut bob.client] {
        client.hub(json!({ "cmd": "set_box_access", "access": { "session": { "mode": "fresh" } } })).unwrap();
    }
    let limits = json!({ "box_budget_cents": 100, "tier": "mid", "time_limit_min": 60 });
    let anas = ana.client.hub(json!({ "cmd": "set_taste", "scope": { "kind": "captains" }, "patch": limits })).unwrap();
    assert_eq!((anas["captains"]["box_budget_cents"].as_u64(), anas["captains"]["tier"].as_str()), (Some(100), Some("mid")));
    let bobs = bob.client.hub(json!({ "cmd": "get_tastes" })).unwrap();
    assert_eq!(
        (bobs["captains"]["box_budget_cents"].as_u64(), bobs["captains"]["tier"].as_str(), bobs["captains"]["time_limit_min"].as_u64()),
        (Some(0), Some("high"), Some(480)),
        "ana's limits never show in bob's tastes"
    );

    let open = |client: &mut Client, budget: Option<f64>| client.hub(json!({ "cmd": "open_box", "repo": "acme/site", "size": "small", "budget_usd": budget }));
    assert_eq!(open(&mut ana.client, Some(5.0)), Err("a box may spend at most US$ 1.00; name less, or raise Box budget in Settings › Way of working".into()));
    let anas_box = open(&mut ana.client, None).unwrap();
    assert_eq!(anas_box["budget_usd"].as_f64(), Some(1.0), "a box with no budget named gets the account's box budget");
    let bobs_box = open(&mut bob.client, Some(5.0)).unwrap();
    assert_eq!(bobs_box["budget_usd"].as_f64(), Some(5.0), "ana's box budget never bounds bob's box");
    let machine = anas_box["machine"].as_str().unwrap();
    assert_eq!(
        ana.client.hub(json!({ "cmd": "extend_box", "machine": machine, "extra_usd": 1.0 })),
        Err("a box may spend at most US$ 1.00; name less, or raise Box budget in Settings › Way of working".into()),
        "adding funds stops at the box budget too"
    );
    let bobs_machine = bobs_box["machine"].as_str().unwrap();
    assert_eq!(bob.client.hub(json!({ "cmd": "extend_box", "machine": bobs_machine, "extra_usd": 1.0 })).unwrap()["budget_usd"].as_f64(), Some(6.0));
}

/// Opens a tab in `path` on `machine` and starts its claude, which here is `cat`.
fn running_tab(client: &mut Client, machine: &str, path: &str) -> String {
    let tab = client.call(machine, json!({ "cmd": "open_tab", "path": path, "bypass": false })).unwrap().as_str().unwrap().to_owned();
    client.call(machine, json!({ "cmd": "start_claude", "tab": &tab, "cols": 80, "rows": 24 })).unwrap();
    tab
}

#[test]
fn the_global_orchestrator_lists_every_session_of_every_machine_and_account_and_reads_one_to_its_end() {
    let mut fleet = Fleet::start("sessions");
    let (mac, mini) = (fleet.mac_id.clone(), fleet.mini_id.clone());
    fleet.mac.machine.listen(|_| Err("not in this test".into())).unwrap();
    let apps = fleet.mac_dir.join("client-apps");
    origin(&apps, "git@github.com:acme/client-apps.git");
    let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"];
    assert!(pinned_core::process::command("git").arg("-C").arg(&apps).args(commit).status().unwrap().success());
    let apps = fleet.mac.machine.open_project(apps).display().to_string();
    fleet.mac.machine.call(Call::AddAccount { provider: Provider::Claude, name: "Bloom".into(), auth: None, key: None }).unwrap();
    fleet.client.call(&mac, json!({ "cmd": "set_project_account", "path": apps, "account": "bloom" })).unwrap();

    let slack = "Someone mentioned you in the Slack thread";
    let asking = running_tab(&mut fleet.client, &mac, &apps);
    hook(&fleet.mac_dir, json!({ "kind": "agent", "status": "working", "tab": asking, "title": slack }));
    hook(&fleet.mac_dir, json!({ "kind": "agent", "status": "needs-input", "tab": asking, "message": "Claude needs your permission to use Bash" }));
    // A transcript past the 8 MiB a read from its start reaches.
    let long = running_tab(&mut fleet.client, &mac, &apps);
    let transcripts = fleet.mac_dir.join("accounts/bloom/projects/client-apps");
    std::fs::create_dir_all(&transcripts).unwrap();
    let line = |kind: &str, content: Value, at: &str| json!({ "isSidechain": false, "gitBranch": "felipega/fix-and-ops", "type": kind, "message": { "role": kind, "content": content }, "uuid": format!("{kind}-{at}"), "timestamp": at }).to_string();
    let mut transcript = String::new();
    for i in 0..10 {
        transcript += &line("assistant", json!([{ "type": "text", "text": "x".repeat(1 << 20) }]), &format!("2026-11-09T11:0{i}:00.000Z"));
        transcript.push('\n');
    }
    transcript += &format!("{}\n", line("user", json!("what is running?"), "2026-11-09T12:00:00.000Z"));
    transcript += &format!("{}\n", json!({ "type": "pr-link", "prNumber": 92, "prUrl": "https://github.com/acme/client-apps/pull/92", "prRepository": "acme/client-apps", "timestamp": "2026-11-09T12:05:00.000Z" }));
    transcript += &format!("{}\n", line("assistant", json!([{ "type": "text", "text": "Two Slack threads wait on you. Should I answer them?" }]), "2026-11-09T12:10:00.000Z"));
    assert!(transcript.len() > 8 << 20);
    std::fs::write(transcripts.join("s-long.jsonl"), transcript).unwrap();
    hook(&fleet.mac_dir, json!({ "kind": "agent", "status": "done", "tab": long, "session": "s-long", "title": slack }));
    let new_thread = running_tab(&mut fleet.client, &mac, &apps);
    let captain = fleet.client.call(&mac, json!({ "cmd": "open_orchestrator", "path": apps })).unwrap().as_str().unwrap().to_owned();
    fleet.client.call(&mac, json!({ "cmd": "start_claude", "tab": &captain, "cols": 80, "rows": 24 })).unwrap();
    hook(&fleet.mac_dir, json!({ "kind": "agent", "status": "working", "tab": captain }));
    let request = pinned_core::delegate::parse_request(&json!({ "task": "Lint the app.", "mode": "async" })).unwrap();
    let delegate = pinned_core::agent::Message::Delegate { tab: Some(captain.clone()), task: None, tool: None, request: Box::new(request) };
    let mut stream = std::os::unix::net::UnixStream::connect(fleet.mac_dir.join("sock")).unwrap();
    writeln!(stream, "{}", serde_json::to_string(&delegate).unwrap()).unwrap();
    let mut reply = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stream), &mut reply).unwrap();
    let started: Value = serde_json::from_str(&serde_json::from_str::<Result<String, String>>(&reply).unwrap().unwrap()).unwrap();
    let worker = started["tabId"].as_str().unwrap_or_else(|| panic!("no worker thread: {started}")).to_owned();
    let stopped = fleet.client.call(&mac, json!({ "cmd": "open_tab", "path": fleet.site, "bypass": false })).unwrap().as_str().unwrap().to_owned();
    let failed = running_tab(&mut fleet.client, &mini, &fleet.mini_dir.display().to_string());
    hook(&fleet.mini_dir, json!({ "kind": "agent", "status": "failed", "tab": failed, "message": "API Error: 500" }));

    let tool = |tool: Value| session_tool(&fleet.mini_dir, &fleet.global, tool);
    let sessions = |project: Option<&str>| tool(json!({ "name": "sessions", "project": project })).unwrap();
    wait_for("mac's tabs and the long transcript's tally on the hub", || {
        let listed = sessions(None);
        let list = listed["sessions"].as_array().unwrap();
        list.len() == 7 && list.iter().any(|s| s["id"] == long && s["pr"]["number"] == 92 && s["state"] == "waiting_on_input") && list.iter().any(|s| s["id"] == worker)
    });
    let listed = sessions(Some("client-apps"));
    let list = listed["sessions"].as_array().unwrap().clone();
    let activity: Vec<u64> = list.iter().filter_map(|s| s["last_activity"].as_u64()).collect();
    assert!(activity.windows(2).all(|w| w[0] >= w[1]), "newest activity first: {activity:?}");
    let mut shapes: Vec<Value> = list
        .into_iter()
        .map(|mut s| {
            s.as_object_mut().unwrap().remove("last_activity");
            s
        })
        .collect();
    shapes.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    let project = json!({ "key": "github.com/acme/client-apps", "name": "client-apps", "path": apps });
    let on_mac = |id: &str, role: &str, state: &str, extra: Value| {
        let mut session = json!({ "id": id, "machine": mac, "machine_name": "mac", "account": "bloom", "project": project, "role": role, "state": state });
        session.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        session
    };
    let mut expected = vec![
        on_mac(&asking, "thread", "waiting_on_input", json!({ "title": slack, "message": "Claude needs your permission to use Bash" })),
        on_mac(&long, "thread", "waiting_on_input", json!({ "title": slack, "message": "Should I answer them?", "branch": "felipega/fix-and-ops", "pr": { "number": 92, "url": "https://github.com/acme/client-apps/pull/92" } })),
        on_mac(&new_thread, "thread", "idle", json!({})),
        on_mac(&captain, "captain", "running", json!({})),
        on_mac(&worker, "worker", "idle", json!({ "captain": captain, "title": "Lint the app." })),
    ];
    expected.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    assert_eq!(shapes, expected);
    let long_session = listed["sessions"].as_array().unwrap().iter().find(|s| s["id"] == long).unwrap().clone();
    assert_eq!(long_session["last_activity"], 1794226200, "the time of the transcript's last message");
    assert_eq!(listed["machines"].as_array().unwrap().len(), 2, "every machine of the account");

    let everything = sessions(None);
    let elsewhere: Vec<(&str, &str, &str, &str)> = everything["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["project"]["name"] != "client-apps")
        .map(|s| (s["id"].as_str().unwrap(), s["machine_name"].as_str().unwrap(), s["account"].as_str().unwrap(), s["state"].as_str().unwrap()))
        .collect();
    let mut elsewhere = elsewhere;
    elsewhere.sort();
    let mut expected_elsewhere = vec![(failed.as_str(), "mini", "claude", "error"), (stopped.as_str(), "mac", "claude", "settled")];
    expected_elsewhere.sort();
    assert_eq!(elsewhere, expected_elsewhere, "the global orchestrator's own tab is not a session it reports");
    assert_eq!(tool(json!({ "name": "sessions", "machine": "@MINI" })).unwrap()["sessions"].as_array().unwrap().len(), 1);

    let read = tool(json!({ "name": "read_session", "id": long, "items": 3 })).unwrap();
    assert_eq!(
        read,
        json!({
            "state": "done", "next": "it may take a message now", "title": slack,
            "conversation": { "items": [
                { "kind": "user", "at": "2026-11-09T12:00:00.000Z", "text": "what is running?" },
                { "kind": "pr", "at": "2026-11-09T12:05:00.000Z", "url": "https://github.com/acme/client-apps/pull/92" },
                { "kind": "text", "at": "2026-11-09T12:10:00.000Z", "text": "Two Slack threads wait on you. Should I answer them?" },
            ], "cursor": read["conversation"]["cursor"], "reset": false },
            "id": long, "machine": mac, "machine_name": "mac",
        }),
        "a loose thread on another machine, read to the true end of a transcript past 8 MiB"
    );
    let waiting = tool(json!({ "name": "waiting" })).unwrap();
    let mut waits: Vec<(&str, &str, &str)> = waiting.as_array().unwrap().iter().map(|w| (w["tab"].as_str().unwrap(), w["kind"].as_str().unwrap(), w["message"].as_str().unwrap_or(""))).collect();
    waits.sort();
    let mut expected_waits = vec![(asking.as_str(), "needs_input", "Claude needs your permission to use Bash"), (long.as_str(), "needs_input", "Should I answer them?"), (failed.as_str(), "failed", "")];
    expected_waits.sort();
    assert_eq!(waits, expected_waits, "waiting() is the sessions waiting on input or in error, the loose thread whose last reply asked included");
    let here = tool(json!({ "name": "read_session", "id": failed })).unwrap();
    assert_eq!(here, json!({ "state": "failed", "next": "it may take a message now", "title": null, "conversation": { "items": [], "cursor": 0, "reset": false }, "id": failed, "machine": mini, "machine_name": "mini" }));
    assert_eq!(tool(json!({ "name": "read_session", "id": "tab-0-0" })), Err("no online machine of this account has a session tab-0-0".into()));
    assert_eq!(tool(json!({ "name": "read_session", "id": long, "machine": "mini" })), Err("mini has no session ".to_owned() + &long));
    let mac_link = fleet.mac.link.as_ref().unwrap();
    assert_eq!(
        mac_link.call_hub(HubCall::ReadSession { tab: long.clone(), machine: None, items: None }),
        Err("mac does not host the global orchestrator (mini does), so it can't read a session".into())
    );
    assert_eq!(session_tool(&fleet.mac_dir, &captain, json!({ "name": "sessions" })), Err("only the global orchestrator has this tool".into()));
}

/// Pairs a node for `client`'s account into `dir`, hosting that account's global orchestrator, and
/// answers its machine id and the global orchestrator's tab.
fn hosting_node(url: &str, client: &mut Client, name: &str, dir: &Path) -> (Node, String, String) {
    let code = client.hub(json!({ "cmd": "pair_code" })).unwrap()["code"].as_str().unwrap().to_owned();
    account::pair(url, &code, name, "linux").unwrap().save(dir).unwrap();
    let creds = Credentials::load(dir).unwrap();
    let id = creds.machine_id.clone();
    let node = Node::start(dir, creds);
    client.wait_machines(&[(name, true)]);
    node.machine.listen(|_| Err("not in this test".into())).unwrap();
    client.call(&id, json!({ "cmd": "set_global_orchestrator_settings", "settings": { "host": true } })).unwrap();
    let global = client.call(&id, json!({ "cmd": "open_global_orchestrator" })).unwrap().as_str().unwrap().to_owned();
    (node, id, global)
}

#[test]
fn the_global_orchestrators_sessions_read_session_and_waiting_reach_nothing_of_another_account() {
    let hub = TestHub::start("iso_sessions");
    let url = hub.url.clone();
    let (_, _, mut ana) = signed_in(&url, "ana@acme.dev");
    let (_, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let (ana_dir, bob_dir) = (temp("iso_sessions-ana"), temp("iso_sessions-bob"));
    let (_ana_node, ana_m, ana_global) = hosting_node(&url, &mut ana, "ana-mini", &ana_dir);
    let (bob_node, bob_m, bob_global) = hosting_node(&url, &mut bob, "bob-mac", &bob_dir);
    for project in [ana_dir.join("secret-project"), bob_dir.join("own-project")] {
        std::fs::create_dir_all(project).unwrap();
    }
    let ana_thread = running_tab(&mut ana, &ana_m, &ana_dir.join("secret-project").display().to_string());
    hook(&ana_dir, json!({ "kind": "agent", "status": "working", "tab": ana_thread }));
    hook(&ana_dir, json!({ "kind": "agent", "status": "needs-input", "tab": ana_thread, "message": "Ship the unreleased pricing page?" }));
    let bob_thread = running_tab(&mut bob, &bob_m, &bob_dir.join("own-project").display().to_string());
    hook(&bob_dir, json!({ "kind": "agent", "status": "working", "tab": bob_thread }));
    hook(&bob_dir, json!({ "kind": "agent", "status": "needs-input", "tab": bob_thread, "message": "Rename the button?" }));

    let ana_tool = |tool: Value| session_tool(&ana_dir, &ana_global, tool);
    let bob_tool = |tool: Value| session_tool(&bob_dir, &bob_global, tool);
    let waits_in = |sessions: &Value, tab: &str| sessions["sessions"].as_array().unwrap().iter().any(|s| s["id"] == tab && s["state"] == "waiting_on_input");
    wait_for("ana's thread waiting on her in her own sessions()", || waits_in(&ana_tool(json!({ "name": "sessions" })).unwrap(), ana_thread.as_str()));
    wait_for("bob's thread waiting on him in his own sessions()", || waits_in(&bob_tool(json!({ "name": "sessions" })).unwrap(), bob_thread.as_str()));
    let anas = ana_tool(json!({ "name": "waiting" })).unwrap().to_string();
    assert!(anas.contains(&ana_thread) && anas.contains("unreleased pricing"), "ana's own waiting() has her thread: {anas}");

    let ana_marks = [ana_m.as_str(), "ana-mini", ana_thread.as_str(), ana_global.as_str(), "secret-project", "unreleased pricing"];
    let nothing_of_ana = |from: &str, answer: &Value| {
        let text = answer.to_string();
        for mark in ana_marks {
            assert!(!text.contains(mark), "{from} names ana's {mark:?}: {text}");
        }
    };
    for args in [json!({ "name": "sessions" }), json!({ "name": "sessions", "machine": ana_m }), json!({ "name": "sessions", "machine": "ana-mini" }), json!({ "name": "sessions", "project": "secret-project" })] {
        let listed = bob_tool(args.clone()).unwrap();
        nothing_of_ana(&format!("bob's {args}"), &listed);
        let machines: Vec<&str> = listed["machines"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(machines, [bob_m.as_str()], "bob's {args} lists his own machine alone");
    }
    let mine: Vec<Value> = bob_tool(json!({ "name": "sessions" })).unwrap()["sessions"].as_array().unwrap().iter().map(|s| s["id"].clone()).collect();
    assert_eq!(mine, [json!(bob_thread)], "bob's sessions() is his own thread, not his global orchestrator's tab nor anything of ana's");
    let waiting = bob_tool(json!({ "name": "waiting" })).unwrap();
    nothing_of_ana("bob's waiting()", &waiting);
    let waits: Vec<&str> = waiting.as_array().unwrap().iter().map(|w| w["tab"].as_str().unwrap()).collect();
    assert_eq!(waits, [bob_thread.as_str()], "bob's waiting() is his own question alone");

    // Ana's tabs and machine read as tabs and machines that do not exist, from the tool and from bob's machine straight to the hub.
    let unknown_tab = |tab: &str| Err(format!("no online machine of this account has a session {tab}"));
    let unknown_machine = |machine: &str| Err(format!("this account has no machine {machine}"));
    let made_up = uuid::Uuid::new_v4().to_string();
    assert_eq!(bob_tool(json!({ "name": "read_session", "id": "tab-0-0" })), unknown_tab("tab-0-0"));
    assert_eq!(bob_tool(json!({ "name": "read_session", "id": "tab-0-0", "machine": made_up })), unknown_machine(&made_up));
    assert_eq!(bob_tool(json!({ "name": "read_session", "id": "tab-0-0", "machine": "Studio" })), unknown_machine("Studio"));
    let bob_link = bob_node.link.as_ref().unwrap();
    for tab in [ana_thread.as_str(), ana_global.as_str()] {
        assert_eq!(bob_tool(json!({ "name": "read_session", "id": tab })), unknown_tab(tab));
        assert_eq!(bob_link.call_hub(HubCall::ReadSession { tab: tab.to_owned(), machine: None, items: None }), unknown_tab(tab));
        for machine in [ana_m.as_str(), "ana-mini", "@ana-mini"] {
            let named = machine.trim_start_matches('@');
            assert_eq!(bob_tool(json!({ "name": "read_session", "id": tab, "machine": machine })), unknown_machine(named), "{tab} on {machine}");
            assert_eq!(bob_link.call_hub(HubCall::ReadSession { tab: tab.to_owned(), machine: Some(named.to_owned()), items: Some(5) }), unknown_machine(named), "{tab} on {machine}");
        }
    }
    let overview = bob_link.call_hub(HubCall::Overview).unwrap();
    nothing_of_ana("bob's machine's overview", &overview);
    assert!(overview.to_string().contains(&bob_thread), "bob's overview has his own thread: {overview}");
    let own = bob_link.call_hub(HubCall::ReadSession { tab: bob_thread.clone(), machine: Some(bob_m.clone()), items: None }).unwrap();
    assert_eq!((own["id"].as_str(), own["machine_name"].as_str()), (Some(bob_thread.as_str()), Some("bob-mac")), "bob reads his own thread through the hub: {own}");
}

// delegate_task on another machine or in an Upstash Box.

const SITE: &str = "https://github.com/acme/site.git";

/// One line on a node's session socket, answered as the text it returns read as JSON.
fn on_sock(dir: &Path, line: Value) -> Result<Value, String> {
    let mut stream = std::os::unix::net::UnixStream::connect(dir.join("sock")).unwrap();
    writeln!(stream, "{line}").unwrap();
    let mut reply = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stream), &mut reply).unwrap();
    let reply: Result<String, String> = serde_json::from_str(&reply).unwrap();
    reply.map(|text| serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn request(args: Value) -> Value {
    serde_json::to_value(pinned_core::delegate::parse_request(&args).unwrap()).unwrap()
}

/// What `delegate_task` answers the session in `tab`.
fn delegate_from(dir: &Path, tab: &str, args: Value) -> Result<Value, String> {
    on_sock(dir, json!({ "kind": "delegate", "tab": tab, "request": request(args) }))
}

fn task_status(dir: &Path, tab: &str, task: &str) -> Value {
    on_sock(dir, json!({ "kind": "task-status", "tab": tab, "id": task })).unwrap()
}

/// How many workers `projects()` counts on `machine`.
fn workers_on(client: &mut Client, machine: &str) -> u64 {
    let overview = client.hub(json!({ "cmd": "overview" })).unwrap();
    let checkouts = overview["projects"].as_array().unwrap().iter().flat_map(|p| p["checkouts"].as_array().cloned().unwrap_or_default());
    checkouts.filter(|c| c["machine"] == machine).map(|c| c["workers"].as_u64().unwrap_or(0)).sum()
}

/// The tabs of the node a box runs, none while no box has booted.
fn box_tabs(boxes: &FakeBoxes) -> Vec<pinned_core::wire::TabView> {
    let node = boxes.state.lock().unwrap().boxes.values().find_map(|b| b.node.clone());
    node.map(|node| node.machine.tabs()).unwrap_or_default()
}

#[test]
fn a_captain_delegates_to_another_machine_and_task_status_hears_how_it_ended() {
    let mut fleet = Fleet::start("remote_machine");
    origin(Path::new(&fleet.site), SITE);
    origin(&fleet.mini_dir.join("site"), SITE);
    let there = fleet.mini.machine.open_project(fleet.mini_dir.join("site")).display().to_string();
    fleet.mini.machine.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
    fleet.mac.machine.listen(|_| Err("not in this test".into())).unwrap();
    let (mac, mini, mac_dir, mini_dir) = (fleet.mac_id.clone(), fleet.mini_id.clone(), fleet.mac_dir.clone(), fleet.mini_dir.clone());
    let captain = fleet.client.call(&mac, json!({ "cmd": "open_orchestrator", "path": fleet.site })).unwrap().as_str().unwrap().to_owned();

    let started = delegate_from(&mac_dir, &captain, json!({ "task": "Fix the footer.", "title": "Footer", "target": { "machine": "mini", "providerInstanceId": "work" } })).unwrap();
    let (task, child) = (started["taskId"].as_str().unwrap().to_owned(), started["tabId"].as_str().unwrap().to_owned());
    assert_eq!((started["workState"].as_str(), started["machine"].as_str()), (Some("working"), Some("mini")));
    let view = fleet.mini.machine.view(&child).unwrap();
    assert_eq!(
        (view.role, view.spawned_by.as_deref(), view.spawned_on.as_deref(), view.account.as_str(), view.title.as_deref(), view.cwd.as_str(), view.running),
        (Some(Role::Worker), Some(captain.as_str()), Some(mac.as_str()), "work", Some("Footer"), there.as_str(), true),
        "a running worker thread on mini, under mac's Captain, on mini's account and checkout"
    );
    wait_launch(&mini_dir, &child, "Fix the footer.");
    assert_eq!(fleet.mac.machine.tabs().len(), 1, "mac opened no thread of its own for it");
    wait_for("projects() to count the worker on mini", || workers_on(&mut fleet.client, &mini) == 1);
    assert_eq!(task_status(&mac_dir, &captain, &task)["workState"], "working");
    let more: Vec<Value> = ["Deps", "Docs"]
        .iter()
        .map(|title| delegate_from(&mac_dir, &captain, json!({ "task": format!("Update the {title}."), "title": title, "target": { "machine": mini, "providerInstanceId": "work" } })).unwrap())
        .collect();
    assert_eq!(
        delegate_from(&mac_dir, &captain, json!({ "task": "One too many.", "target": { "machine": "mini" } })),
        Err("3 workers already run in this project, the most it allows at once. Wait for one to finish, or raise Workers at once in Settings › Way of working.".into()),
        "workers on another machine count against the caller's Workers at once"
    );

    let transcripts = mini_dir.join("accounts/work/projects/site");
    std::fs::create_dir_all(&transcripts).unwrap();
    std::fs::write(transcripts.join("s-footer.jsonl"), assistant_line("Footer fixed; tests pass.")).unwrap();
    hook(&mini_dir, json!({ "kind": "agent", "status": "working", "tab": child, "session": "s-footer", "cwd": there }));
    hook(&mini_dir, json!({ "kind": "agent", "status": "done", "tab": child, "session": "s-footer" }));
    wait_for("mac to hear how the worker ended", || task_status(&mac_dir, &captain, &task)["workState"] == "result_available");
    let status = task_status(&mac_dir, &captain, &task);
    assert_eq!(
        (status["summary"].as_str(), status["error"].as_str(), status["machine"].as_str(), status["tabId"].as_str()),
        (Some("Footer fixed; tests pass."), None, Some("mini"), Some(child.as_str())),
        "task_status on mac reports the end and the child's last reply"
    );

    for second in &more {
        let cancel = on_sock(&mac_dir, json!({ "kind": "task-cancel", "tab": captain, "id": second["taskId"], "reason": "not now" })).unwrap();
        assert_eq!((cancel["workState"].as_str(), cancel["error"].as_str()), (Some("cancelled"), Some("not now")));
        let remote = fleet.mini.machine.view(second["tabId"].as_str().unwrap()).unwrap();
        assert_eq!(remote.spawned_on.as_deref(), Some(mac.as_str()));
    }
    let mut stats = || fleet.client.hub(json!({ "cmd": "delegation_stats", "days": 1 })).unwrap();
    wait_for("mini to record the workers it ran", || stats()["totals"]["runs"] == 3);
    std::thread::sleep(Duration::from_millis(1500));
    let stats = stats();
    let ran: Vec<(&str, &str)> = stats["slowest"].as_array().unwrap().iter().map(|t| (t["machine"].as_str().unwrap(), t["delegator"].as_str().unwrap())).collect();
    assert_eq!((stats["totals"]["runs"].as_u64(), ran), (Some(3), vec![(mini.as_str(), "Captain site"); 3]), "one record per worker, from mini that ran it, under mac's Captain: {stats}");

    let refused = |target: Value| delegate_from(&mac_dir, &captain, json!({ "task": "Anything.", "target": target }));
    assert_eq!(refused(json!({ "machine": "nowhere" })), Err("this account has no machine nowhere".into()));
    assert_eq!(refused(json!({ "machine": "@mac" })), Err("mac is this machine: leave target.machine out to run the worker here".into()));
    pair(&fleet.hub.url, &mut fleet.client, "laptop", &temp("remote_machine-laptop"));
    assert_eq!(refused(json!({ "machine": "laptop" })), Err("laptop is offline, so it can't take the worker; nothing ran elsewhere".into()));
    assert_eq!(refused(json!({ "box": true })), Err("Upstash Box is not connected: connect it in pinned's Connectors settings".into()));
    assert_eq!(fleet.mini.machine.tabs().iter().filter(|t| t.role == Some(Role::Worker)).count(), 3, "no refusal started a worker anywhere");
}

#[test]
fn a_box_worker_signs_in_with_the_named_token_nests_under_its_caller_and_goes_when_its_task_ends() {
    let hub = TestHub::start_with_boxes("rbox", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    hub.boxes.state.lock().unwrap().checkout = Some(("site".into(), SITE.into()));
    let (_, _, mut bob) = signed_in(&hub.url, "bob@acme.dev");
    let mac_dir = temp("rbox-mac");
    let creds = pair(&hub.url, &mut ana.client, "mac", &mac_dir);
    let mac_id = creds.machine_id.clone();
    let mac = Node::start(&mac_dir, creds);
    mac.machine.listen(|_| Err("not in this test".into())).unwrap();
    ana.client.wait_machines(&[("mac", true)]);
    origin(&mac_dir.join("site"), SITE);
    let site = mac.machine.open_project(mac_dir.join("site")).display().to_string();
    let captain = ana.client.call(&mac_id, json!({ "cmd": "open_orchestrator", "path": site })).unwrap().as_str().unwrap().to_owned();
    let to_box = json!({ "task": "Run the slow suite.", "title": "Slow suite", "target": { "box": { "size": "small" }, "providerInstanceId": "bloom" } });

    assert_eq!(
        delegate_from(&mac_dir, &captain, to_box.clone()),
        Err("Upstash Box is off for Workers: choose a box permission for Workers in Connectors settings".into())
    );
    let fresh = json!({ "workflow": { "mode": "fresh" }, "session": { "mode": "off" }, "worker": { "mode": "fresh" }, "orchestrator": { "mode": "off" } });
    ana.client.hub(json!({ "cmd": "set_box_access", "access": fresh })).unwrap();
    assert_eq!(
        delegate_from(&mac_dir, &captain, to_box.clone()),
        Err("no Claude token is kept for bloom: add one on the Upstash card in Connectors, or leave providerInstanceId out to use the default token".into())
    );
    assert_eq!(ana.client.hub(json!({ "cmd": "set_box_claude_token", "name": "bloom", "token": " sk-ant-oat01-bloomtoken " })), Ok(json!({ "bloom": "…oken" })));
    let settings = ana.client.wait("settings with bloom's token", |f| f["type"] == "settings" && f["settings"]["box_claude_tokens"]["bloom"] == "…oken");
    assert!(!settings.to_string().contains("bloomtoken"), "clients see only the hint");
    assert_eq!(hub.boxes.state.lock().unwrap().created.len(), 0, "no refusal opened a box");

    let started = delegate_from(&mac_dir, &captain, to_box).unwrap();
    let task = started["taskId"].as_str().unwrap().to_owned();
    assert_eq!((started["workState"].as_str(), started["machine"].as_str()), (Some("working"), Some("Box · Slow suite")), "{started}");
    assert_eq!(started["tabId"], Value::Null, "the box has not booted yet");
    wait_for("the worker thread running on the box", || box_tabs(&hub.boxes).iter().any(|t| t.role == Some(Role::Worker) && t.running));
    assert_eq!(hub.boxes.file(&format!("{BOX_DIR}/box.env")), Some("CLAUDE_CODE_OAUTH_TOKEN='sk-ant-oat01-bloomtoken'\n".into()), "bloom's token, and no GitHub token without GitHub");
    assert_eq!(hub.boxes.state.lock().unwrap().created[0].size, BoxSize::Small);
    let child = box_tabs(&hub.boxes).into_iter().find(|t| t.role == Some(Role::Worker)).unwrap();
    assert_eq!(
        (child.spawned_by.as_deref(), child.spawned_on.as_deref(), child.title.as_deref(), child.running),
        (Some(captain.as_str()), Some(mac_id.as_str()), Some("Slow suite"), true),
        "the box's worker thread nests under mac's Captain"
    );
    assert!(child.cwd.ends_with("/site"), "it works in the box's checkout of the repo: {}", child.cwd);
    ana.client.wait_machines(&[("mac", true), ("Box · Slow suite", true)]);
    let machine = ana.box_machine();
    assert_eq!(task_status(&mac_dir, &captain, &task)["workState"], "working");

    let box_dir = hub.boxes.root.join(&hub.boxes.live()[0]);
    hub.boxes.with_node(|node| node.machine.listen(|_| Err("not in this test".into())).unwrap());
    hook(&box_dir, json!({ "kind": "agent", "status": "working", "tab": child.id }));
    hook(&box_dir, json!({ "kind": "agent", "status": "done", "tab": child.id }));
    wait_for("the box to go once its task ended", || hub.boxes.live().is_empty());
    assert_eq!(task_status(&mac_dir, &captain, &task)["workState"], "result_available");
    wait_for("the row to go", || ana.rows(&hub) == 0);
    ana.client.wait_machines(&[("mac", true)]);
    let stats = ana.client.hub(json!({ "cmd": "delegation_stats", "days": 1 })).unwrap();
    let ran = &stats["slowest"][0];
    assert_eq!(
        (stats["totals"]["runs"].as_u64(), ran["machine"].as_str(), ran["delegator"].as_str(), ran["title"].as_str()),
        (Some(1), Some(machine.as_str()), Some("Captain site"), Some("Slow suite")),
        "the box recorded its worker before it went: {stats}"
    );
    assert!(stats["totals"]["box_ms"].as_u64().unwrap() > 0, "and was billed its box time: {stats}");

    let again = delegate_from(&mac_dir, &captain, json!({ "task": "Run it again.", "title": "Again", "target": { "box": true } })).unwrap();
    let cancel = on_sock(&mac_dir, json!({ "kind": "task-cancel", "tab": captain, "id": again["taskId"], "reason": "not now" })).unwrap();
    assert_eq!(cancel["workState"], "cancelled");
    wait_for("a cancelled task's box to go", || hub.boxes.live().is_empty() && ana.rows(&hub) == 0);

    for needle in [machine.as_str(), "Slow suite", "Box · "] {
        assert_eq!(mentioning(&mut bob, needle), Vec::<Value>::new(), "bob's lists never show ana's box worker: {needle}");
    }
}

#[test]
fn the_global_orchestrator_opens_a_captain_in_a_box_with_the_accounts_connection() {
    let hub = TestHub::start_with_boxes("box_captain", Signup::Open, fast());
    let mut ana = BoxAccount::new(&hub);
    hub.boxes.state.lock().unwrap().checkout = Some(("site".into(), SITE.into()));
    let mini_dir = temp("rcap-mini");
    let creds = pair(&hub.url, &mut ana.client, "mini", &mini_dir);
    let mini_id = creds.machine_id.clone();
    let mini = Node::start(&mini_dir, creds);
    mini.machine.listen(|_| Err("not in this test".into())).unwrap();
    ana.client.wait_machines(&[("mini", true)]);
    ana.client.call(&mini_id, json!({ "cmd": "set_global_orchestrator_settings", "settings": { "host": true } })).unwrap();
    let global = ana.client.call(&mini_id, json!({ "cmd": "open_global_orchestrator" })).unwrap().as_str().unwrap().to_owned();
    let send = |request_id: &str, text: &str| session_tool(&mini_dir, &global, json!({ "name": "send_to_captain", "project": "github.com/acme/site", "message": text, "request_id": request_id, "box": true }));

    assert_eq!(send("r-0", "Ship the form."), Err("Upstash Box is off for Captains: choose a box permission for Captains in Connectors settings".into()));
    let fresh = json!({ "workflow": { "mode": "fresh" }, "session": { "mode": "off" }, "worker": { "mode": "off" }, "orchestrator": { "mode": "fresh" } });
    ana.client.hub(json!({ "cmd": "set_box_access", "access": fresh })).unwrap();
    let sent = send("r-1", "Ship the form.").unwrap();
    assert_eq!((sent["outcome"].as_str(), sent["machineName"].as_str()), (Some("iniciado"), Some("Box · Captain site")), "{sent}");
    wait_for("a Captain on the box", || box_tabs(&hub.boxes).iter().any(|t| t.role == Some(Role::Orchestrator) && t.running));
    let captain = box_tabs(&hub.boxes).into_iter().find(|t| t.role == Some(Role::Orchestrator)).unwrap();
    assert!(captain.cwd.ends_with("/site"), "{}", captain.cwd);
    ana.client.wait_machines(&[("mini", true), ("Box · Captain site", true)]);
    assert_eq!(hub.boxes.file(&format!("{BOX_DIR}/box.env")), Some(format!("CLAUDE_CODE_OAUTH_TOKEN='{CLAUDE_TOKEN}'\n")));

    let again = send("r-2", "Also the footer.").unwrap();
    assert_eq!((again["box"].as_str(), again["tab"].as_str()), (sent["box"].as_str(), Some(captain.id.as_str())), "the box that runs the Captain takes the next message: {again}");
    assert_eq!(hub.boxes.state.lock().unwrap().created.len(), 1, "one box for the project");
}

#[test]
fn a_worker_reaches_no_machine_box_or_task_of_another_account() {
    let hub = TestHub::start_with_boxes("iso_delegate", Signup::Open, fast());
    let url = hub.url.clone();
    let mut ana = BoxAccount::new(&hub);
    let fresh = json!({ "workflow": { "mode": "fresh" }, "session": { "mode": "off" }, "worker": { "mode": "fresh" }, "orchestrator": { "mode": "fresh" } });
    ana.client.hub(json!({ "cmd": "set_box_access", "access": fresh })).unwrap();
    let code = ana.client.hub(json!({ "cmd": "pair_code" })).unwrap();
    let ana_creds = account::pair(&url, code["code"].as_str().unwrap(), "ana-mini", "linux").unwrap();
    let ana_raw = RawMachine::start(&url, &ana_creds.machine_token, |call| match call["cmd"].as_str().unwrap() {
        "delegate" => RawReply::Ok(json!({ "taskId": "task-a", "tabId": "tab-w", "workState": "working" })),
        "remote_task" => RawReply::Ok(json!({ "workState": "working" })),
        "remote_ended" => RawReply::Ok(json!(true)),
        _ => RawReply::Ok(Value::Null),
    });
    let (bob_token, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let (bob_machine, bob_machine_token) = account::register_machine(&url, &bob_token, "bob-mac", "linux").unwrap();
    let bob_raw = RawMachine::start(&url, &bob_machine_token, |_| RawReply::Ok(Value::Null));
    ana.client.wait_machines(&[("ana-mini", true)]);
    bob.wait_machines(&[("bob-mac", true)]);
    let ana_m = ana_creds.machine_id.clone();
    let none = Err(format!("this account has no machine {ana_m}"));
    let delegate = |target: Value| json!({ "cmd": "delegate", "tab": "tab-1", "task": "task-b", "request": request(json!({ "task": "Read the secrets.", "target": target })), "repo": SITE, "access": "full" });

    assert_eq!(bob_raw.ask(delegate(json!({ "machine": ana_m }))), none, "bob can't target ana's machine by id");
    assert_eq!(bob_raw.ask(delegate(json!({ "machine": "ana-mini" }))), Err("this account has no machine ana-mini".into()), "nor by name");
    assert_eq!(bob_raw.ask(delegate(json!({ "box": true }))), Err("Upstash Box is not connected: connect it in pinned's Connectors settings".into()), "bob's box can't use ana's connection");
    assert_eq!(bob_raw.ask(json!({ "cmd": "remote_task", "machine": ana_m, "tab": "tab-1", "task": "task-a" })), none, "bob can't read ana's remote task");
    assert_eq!(bob_raw.ask(json!({ "cmd": "remote_task", "machine": ana_m, "tab": "tab-1", "task": "task-a", "cancel": "mine now" })), none, "nor cancel it");
    assert_eq!(bob_raw.ask(json!({ "cmd": "remote_ended", "to": ana_m, "tab": "tab-1", "task": "task-a", "end": { "state": "failed", "error": "forged" } })), none, "nor end it");
    assert_eq!(bob.hub(delegate(json!({ "machine": bob_machine.id }))), Err("only a thread's machine delegates a task, asks after it or reports its end".into()), "a client delegates only through a thread");
    assert_eq!(bob.hub(json!({ "cmd": "set_box_claude_token", "name": "bloom", "token": "sk-ant-oat01-x" })), Err("connect Upstash Box first".into()));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!((ana_raw.calls("delegate"), ana_raw.calls("remote_task"), ana_raw.calls("remote_ended")), (0, 0, 0), "ana's machine heard none of bob's");
    assert_eq!(hub.boxes.state.lock().unwrap().created.len(), 0, "no box opened");
}

fn task_record(id: &str, model: &str, outcome: &str, minutes: u64, cost: Option<f64>, ended: u64) -> Value {
    let mut record = json!({
        "id": id, "machine": "spoofed", "title": format!("task {id}"),
        "delegator": { "kind": "captain", "tab": "tab-1", "project": "github.com/acme/site", "project_name": "site" },
        "provider": "claude", "account": "claude", "model": model, "started": ended - minutes * 60_000, "ended": ended,
        "usage": { "tokens_in": 1000, "tokens_out": 100, "wall_ms": minutes * 60_000 },
        "outcome": outcome,
    });
    if let Some(cost) = cost {
        record["usage"]["cost_usd"] = json!(cost);
    }
    if outcome == "limit" {
        record["limit"] = json!("time");
    }
    record
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

#[test]
fn an_accounts_delegated_task_records_and_totals_reach_only_that_account() {
    let hub = TestHub::start("iso_delegation");
    let url = hub.url.clone();
    let (ana_token, _, mut ana) = signed_in(&url, "ana@acme.dev");
    let (bob_token, _, mut bob) = signed_in(&url, "bob@acme.dev");
    let (ana_machine, ana_machine_token) = account::register_machine(&url, &ana_token, "ana-mac", "linux").unwrap();
    let (_, bob_machine_token) = account::register_machine(&url, &bob_token, "bob-mac", "linux").unwrap();
    let ana_raw = RawMachine::start(&url, &ana_machine_token, |_| RawReply::Ok(Value::Null));
    let bob_raw = RawMachine::start(&url, &bob_machine_token, |_| RawReply::Ok(Value::Null));
    ana.wait_machines(&[("ana-mac", true)]);
    bob.wait_machines(&[("bob-mac", true)]);
    let now = now_ms();
    let stats = |days: u32| json!({ "cmd": "delegation_stats", "days": days });

    let anas = json!([task_record("task-1", "claude-opus-5-5", "done", 10, Some(1.5), now), task_record("task-2", "claude-opus-5-5", "limit", 120, Some(6.0), now)]);
    assert_eq!(ana_raw.ask(json!({ "cmd": "record_tasks", "records": anas })), Ok(Value::Null));
    let bobs = json!([task_record("task-1", "glm-5.3", "failed", 3, None, now)]);
    assert_eq!(bob_raw.ask(json!({ "cmd": "record_tasks", "records": bobs })), Ok(Value::Null), "the same task id on bob's machine is bob's own");
    assert_eq!(ana.hub(json!({ "cmd": "record_tasks", "records": [] })), Err("only a machine records the tasks it ran".into()));

    let ana_stats = ana.hub(stats(7)).unwrap();
    assert_eq!(
        json!([ana_stats["totals"]["runs"], ana_stats["totals"]["limited"], ana_stats["totals"]["cost_usd"], ana_stats["by_model"][0]["key"], ana_stats["by_model"][0]["limits"]]),
        json!([2, 1, 7.5, "claude-opus-5-5", { "time": 1 }])
    );
    assert_eq!(
        json!([ana_stats["slowest"][0]["id"], ana_stats["slowest"][0]["machine"], ana_stats["slowest"][0]["machine_name"]]),
        json!(["task-2", ana_machine.id, "ana-mac"]),
        "the hub names the machine that sent it"
    );
    assert!(!ana_stats.to_string().contains("glm-5.3"), "bob's task is in ana's totals: {ana_stats}");
    let bob_stats = bob.hub(stats(30)).unwrap();
    assert_eq!((bob_stats["totals"]["runs"].as_u64(), bob_stats["totals"]["failed"].as_u64()), (Some(1), Some(1)));
    assert!(!bob_stats.to_string().contains("opus"), "ana's tasks are in bob's totals: {bob_stats}");
    assert_eq!(bob_raw.ask(stats(7)).unwrap()["totals"], bob.hub(stats(7)).unwrap()["totals"], "bob's machine reads bob's totals");
    assert_eq!(ana.hub(stats(2)), Err("days must be 1, 7 or 30, not 2".into()));

    let mut accepted = task_record("task-1", "claude-opus-5-5", "done", 10, Some(1.5), now);
    accepted["review"] = json!({ "verdict": "accepted", "at": now });
    assert_eq!(ana_raw.ask(json!({ "cmd": "record_tasks", "records": [accepted] })), Ok(Value::Null));
    let again = ana.hub(stats(7)).unwrap();
    assert_eq!((again["totals"]["runs"].as_u64(), again["totals"]["accepted"].as_u64()), (Some(2), Some(1)), "a record sent again replaces the first");
    assert_eq!(bob.hub(stats(7)).unwrap()["totals"]["accepted"], 0);
    let old = task_record("task-3", "claude-opus-5-5", "done", 10, None, now - 8 * 24 * 60 * 60 * 1000);
    ana_raw.ask(json!({ "cmd": "record_tasks", "records": [old] })).unwrap();
    assert_eq!((ana.hub(stats(7)).unwrap()["totals"]["runs"].as_u64(), ana.hub(stats(30)).unwrap()["totals"]["runs"].as_u64()), (Some(2), Some(3)));
}

#[test]
fn a_task_on_a_box_is_billed_the_box_time_since_the_previous_task_on_that_box() {
    let hub = TestHub::start("delegation_box");
    let url = hub.url.clone();
    let (token, ana_account, mut ana) = signed_in(&url, "ana@acme.dev");
    let (machine, machine_token) = account::register_machine(&url, &token, "Box · site", "linux").unwrap();
    admin_on(&hub.database, &format!(
        "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase, created_at) values ('anas-box', '{ana_account}', '{}', 'box_ana', 'interactive', '{{\"issue\":\"b1\"}}', '{{\"folder\":\"/w\",\"args\":[]}}', 'medium', 'running', now() - interval '30 minutes')",
        machine.id,
    ));
    let raw = RawMachine::start(&url, &machine_token, |_| RawReply::Ok(Value::Null));
    ana.wait_machines(&[("Box · site", true)]);
    let now = now_ms();
    let first = task_record("task-a", "claude-sonnet-5-5", "done", 5, Some(0.2), now - 20 * 60_000);
    let second = task_record("task-b", "claude-sonnet-5-5", "done", 15, Some(0.4), now);
    raw.ask(json!({ "cmd": "record_tasks", "records": [first.clone(), second] })).unwrap();
    raw.ask(json!({ "cmd": "record_tasks", "records": [first] })).unwrap();

    let stats = ana.hub(json!({ "cmd": "delegation_stats", "days": 1 })).unwrap();
    let minutes = |ms: &Value| (ms.as_f64().unwrap() / 60_000.0).round();
    let task = |id: &str| stats["slowest"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap().clone();
    assert_eq!(minutes(&stats["totals"]["box_ms"]), 30.0, "both tasks together are billed the box's whole 30 minutes: {stats}");
    let usd = stats["totals"]["box_usd"].as_f64().unwrap();
    assert!((usd - 0.5 * pinned_core::wire::BOX_USD_PER_HOUR).abs() < 0.01, "{usd}");
    let a_cost = task("task-a")["cost_usd"].as_f64().unwrap();
    assert!((a_cost - (0.2 + 10.0 / 60.0 * pinned_core::wire::BOX_USD_PER_HOUR)).abs() < 0.01, "task-a: its 10 box minutes plus its agent cost, sent twice and billed once: {a_cost}");
}

#[test]
fn overlapping_tasks_on_one_box_each_pay_for_their_own_span_however_their_records_arrive() {
    let hub = TestHub::start("delegation_box_overlap");
    let url = hub.url.clone();
    let (token, ana_account, mut ana) = signed_in(&url, "ana@acme.dev");
    let boxed = |name: &str, minutes_ago: u64| {
        let (machine, machine_token) = account::register_machine(&url, &token, name, "linux").unwrap();
        admin_on(&hub.database, &format!(
            "insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase, created_at) values ('{0}', '{ana_account}', '{0}', 'box_{0}', 'interactive', '{{\"issue\":\"{0}\"}}', '{{\"folder\":\"/w\",\"args\":[]}}', 'medium', 'running', now() - interval '{minutes_ago} minutes')",
            machine.id,
        ));
        RawMachine::start(&url, &machine_token, |_| RawReply::Ok(Value::Null))
    };
    let site = boxed("Box · site", 60);
    let api = boxed("Box · api", 30);
    ana.wait_machines(&[("Box · site", true), ("Box · api", true)]);
    let now = now_ms();

    // On site, task-a started first and ended last; task-b ran inside it. They arrive in start order.
    let a = task_record("task-a", "claude-sonnet-5-5", "done", 50, Some(0.0), now);
    let b = task_record("task-b", "claude-sonnet-5-5", "done", 20, Some(0.0), now - 20 * 60_000);
    site.ask(json!({ "cmd": "record_tasks", "records": [a, b] })).unwrap();
    // On api, six tasks end four minutes apart and each record is sent on its own, all at once.
    std::thread::scope(|s| {
        for k in 0..6u64 {
            let api = &api;
            s.spawn(move || {
                let record = task_record(&format!("task-api-{k}"), "glm-5.3", "done", 1, Some(0.0), now - k * 4 * 60_000);
                api.ask(json!({ "cmd": "record_tasks", "records": [record] })).unwrap();
            });
        }
    });

    let stats = ana.hub(json!({ "cmd": "delegation_stats", "days": 1 })).unwrap();
    let box_minutes = |id: &str| {
        let task = stats["slowest"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap_or_else(|| panic!("{id} in {stats}"));
        (task["cost_usd"].as_f64().unwrap() / pinned_core::wire::BOX_USD_PER_HOUR * 60.0).round()
    };
    assert_eq!(box_minutes("task-b"), 40.0, "task-b pays from the box's start to its own end");
    assert_eq!(box_minutes("task-a"), 20.0, "task-a pays only the span after task-b ended");
    let minutes = |ms: &Value| (ms.as_f64().unwrap() / 60_000.0).round();
    assert_eq!(minutes(&stats["totals"]["box_ms"]), 90.0, "each box's tasks add up to its life, with no span billed twice: {stats}");
}

/// What a session's `act` tool answers: the hub's `ActAnswer`, or the refusal.
fn act(dir: &Path, tab: &str, action: &str, args: Value) -> Result<Value, String> {
    session_tool(dir, tab, json!({ "name": "act", "action": action, "args": args }))
}

/// The context a session's Stop hook gets, which carries the notices queued for it.
fn turn_end_context(dir: &Path, tab: &str) -> String {
    let mut stream = std::os::unix::net::UnixStream::connect(dir.join("sock")).unwrap();
    writeln!(stream, "{}", json!({ "kind": "agent", "status": "done", "tab": tab, "hook_feedback": true })).unwrap();
    let mut reply = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stream), &mut reply).unwrap();
    serde_json::from_str::<Result<String, String>>(&reply).unwrap().unwrap()
}

struct Agent {
    client: Client,
    node: Node,
    dir: PathBuf,
    machine: String,
    tab: String,
}

/// An account's app, its machine `name` with a project `site` and a project `docs`, and an agent
/// session in `site`.
fn agent_of(url: &str, token: &str, email: &str, name: &str) -> Agent {
    let mut client = Client::connect(url, token);
    client.wait("hello", |f| f["type"] == "hello");
    let dir = temp(&format!("surface-{name}"));
    let (machine, machine_token) = account::register_machine(url, token, name, "macos").unwrap();
    let creds = Credentials { hub: url.to_owned(), email: email.to_owned(), user_token: token.to_owned(), machine_id: machine.id.clone(), machine_token };
    creds.save(&dir).unwrap();
    let node = Node::start(&dir, creds);
    node.machine.listen(|_| Err("unexpected".into())).unwrap();
    client.wait_machines(&[(name, true)]);
    for project in ["site", "docs"] {
        std::fs::create_dir_all(dir.join(project)).unwrap();
    }
    node.machine.open_tab(Some(dir.join("docs").display().to_string()), false).unwrap();
    let tab = node.machine.open_tab(Some(dir.join("site").display().to_string()), false).unwrap();
    client.wait("the agent's tab at the hub", |f| f["type"] == "event" && f["event"]["kind"] == "tabs" && f["event"]["tabs"].as_array().unwrap().iter().any(|t| t["id"] == tab));
    Agent { client, node, dir, machine: machine.id, tab }
}

fn names(list: &Value) -> Vec<String> {
    let mut names: Vec<String> = list.as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap().to_owned()).collect();
    names.sort();
    names
}

#[test]
fn an_agent_acts_as_its_own_account_audited_and_a_confirm_runs_once_its_owner_approves() {
    let hub = TestHub::start_with("agent_surface", Signup::Invite);
    let url = hub.url.clone();
    let (ana_token, _) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let mut ana = agent_of(&url, &ana_token, "ana@acme.dev", "ana-mac");
    let (spare, spare_token) = account::register_machine(&url, &ana_token, "ana-spare", "linux").unwrap();
    let docs = ana.dir.join("docs").canonicalize().unwrap().display().to_string();

    let found = session_tool(&ana.dir, &ana.tab, json!({ "name": "actions", "search": "machine" })).unwrap();
    let listed: Vec<(&str, &str)> = found["actions"].as_array().unwrap().iter().map(|a| (a["name"].as_str().unwrap(), a["class"].as_str().unwrap())).collect();
    assert!(listed.contains(&("machines", "read")) && listed.contains(&("rename_machine", "write")) && listed.contains(&("remove_machine", "confirm")), "{listed:?}");

    let machines = act(&ana.dir, &ana.tab, "machines", json!({})).unwrap();
    assert_eq!(machines["outcome"], "done");
    assert_eq!(names(&machines["result"]), ["ana-mac", "ana-spare"]);
    assert_eq!(act(&ana.dir, &ana.tab, "rename_machine", json!({ "id": ana.machine, "name": "Ana Mini" })).unwrap(), json!({ "outcome": "done", "result": null }));
    assert_eq!(names(&ana.client.hub(json!({ "cmd": "machines" })).unwrap()), ["Ana Mini", "ana-spare"], "the app sees the agent's rename");

    let saved = act(&ana.dir, &ana.tab, "memory_save", json!({ "path": docs, "title": "Deploys", "text": "Docs deploy from tags." })).unwrap();
    assert_eq!((&saved["outcome"], &saved["result"]["title"]), (&json!("done"), &json!("Deploys")), "{saved}");
    let read = act(&ana.dir, &ana.tab, "memory_search", json!({ "path": docs, "query": "deploy" })).unwrap();
    assert_eq!(read["result"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap()).collect::<Vec<_>>(), ["Docs deploy from tags."]);
    assert_eq!(session_tool(&ana.dir, &ana.tab, json!({ "name": "memory_search", "query": "deploy" })), Ok(json!("No memory matches that.")), "the memory went to docs, not the session's own project");

    assert_eq!(act(&ana.dir, &ana.tab, "write_input", json!({ "tab": ana.tab, "data": "y" })), Err("write_input is not open to agents: the app's terminal plumbing".into()));
    assert_eq!(act(&ana.dir, &ana.tab, "prompt", json!({ "tab": ana.tab, "text": "hi" })), Err("prompt is a write action that is not wired yet".into()));
    assert_eq!(act(&ana.dir, &ana.tab, "rename_machine", json!({ "id": ana.machine, "name": "x", "token": "abc" })), Err("an agent never sends a password, key or token: token. The user enters it in the app.".into()));
    assert_eq!(act(&ana.dir, &ana.tab, "nope", json!({})), Err("no action nope; call actions to list them".into()));
    assert_eq!(machine_result(&url, &spare_token, json!({ "cmd": "act", "act": { "tab": "tab-zz-1", "action": "machines" } })), Err("ana-spare runs no session tab-zz-1".into()), "a machine acts only for a session it reports");

    let remove = session_tool(&ana.dir, &ana.tab, json!({ "name": "act", "action": "remove_machine", "args": { "id": spare.id }, "request_id": "r-remove" })).unwrap();
    assert_eq!(remove["outcome"], "pending", "{remove}");
    let remove_id = remove["id"].as_str().unwrap().to_owned();
    let pushed = ana.client.wait("the confirmation in the app", |f| f["type"] == "confirmations" && f["items"].as_array().unwrap().iter().any(|c| c["id"] == remove_id.as_str()));
    let item = pushed["items"].as_array().unwrap().iter().find(|c| c["id"] == remove_id.as_str()).unwrap().clone();
    assert_eq!((&item["action"], &item["args"], &item["tab"], &item["role"], &item["machine_name"]), (&json!("remove_machine"), &json!({ "id": spare.id }), &json!(ana.tab), &json!("session"), &json!("Ana Mini")));
    let again = session_tool(&ana.dir, &ana.tab, json!({ "name": "act", "action": "remove_machine", "args": { "id": spare.id }, "request_id": "r-remove" })).unwrap();
    assert_eq!(again["id"], remove_id.as_str(), "the same request_id holds one confirmation");
    assert_eq!(names(&ana.client.hub(json!({ "cmd": "machines" })).unwrap()), ["Ana Mini", "ana-spare"], "nothing runs before the owner approves");

    let invite = act(&ana.dir, &ana.tab, "create_invite", json!({})).unwrap();
    let invite_id = invite["id"].as_str().unwrap().to_owned();
    let decided = ana.client.hub(json!({ "cmd": "decide_confirmation", "id": invite_id, "approve": true })).unwrap();
    assert_eq!((&decided["decision"], &decided["outcome"]), (&json!("approved"), &json!("done")));
    let code = decided["result"]["code"].as_str().unwrap().to_owned();
    wait_for("the invite's notice in the agent's queue", || ana.node.machine.view(&ana.tab).unwrap().queued >= 1);
    let told = turn_end_context(&ana.dir, &ana.tab);
    assert!(told.starts_with(&format!("[pinned] The user approved your create_invite request ({invite_id}): done. Result: ")), "{told}");
    assert!(told.contains(r#""code":"[hidden]""#) && !told.contains(&code), "the agent never gets the code: {told}");

    let (bob_token, _) = account::signup_invited(&url, "bob@acme.dev", "correct horse", Some(&code)).unwrap();
    let mut bob = agent_of(&url, &bob_token, "bob@acme.dev", "bob-mac");
    let (_, bob_spare_token) = account::register_machine(&url, &bob_token, "bob-spare", "linux").unwrap();
    assert_eq!(names(&act(&bob.dir, &bob.tab, "machines", json!({})).unwrap()["result"]), ["bob-mac", "bob-spare"]);
    assert_eq!(act(&bob.dir, &bob.tab, "rename_machine", json!({ "id": ana.machine, "name": "mine" })), Err(format!("this account has no machine {}", ana.machine)));
    assert_eq!(session_tool(&bob.dir, &bob.tab, json!({ "name": "act", "action": "memory_search", "args": { "path": docs, "query": "deploy" }, "machine": "ana-mac" })), Err("this account has no machine ana-mac".into()));
    let bobs_remove = act(&bob.dir, &bob.tab, "remove_machine", json!({ "id": spare.id })).unwrap();
    assert_eq!(bob.client.hub(json!({ "cmd": "decide_confirmation", "id": remove_id, "approve": true })), Err(format!("no confirmation {remove_id} waits")), "bob's app can't decide ana's");
    assert_eq!(machine_result(&url, &bob_spare_token, json!({ "cmd": "decide_confirmation", "id": remove_id, "approve": true })), Err("only the account's owner decides, in the app".into()));
    let bobs_list = machine_result(&url, &bob_spare_token, json!({ "cmd": "confirmations" })).unwrap();
    assert_eq!(bobs_list.as_array().unwrap().iter().map(|c| c["action"].as_str().unwrap()).collect::<Vec<_>>(), ["remove_machine"], "bob sees only his own: {bobs_list}");
    assert_eq!(bobs_list[0]["id"], bobs_remove["id"]);
    let denied = bob.client.hub(json!({ "cmd": "decide_confirmation", "id": bobs_remove["id"], "approve": false })).unwrap();
    assert_eq!((&denied["decision"], &denied["outcome"]), (&json!("denied"), &json!("denied")));
    let bobs_audit = act(&bob.dir, &bob.tab, "agent_audit", json!({})).unwrap()["result"].clone();
    let rows: Vec<(&str, &str)> = bobs_audit.as_array().unwrap().iter().map(|r| (r["action"].as_str().unwrap(), r["outcome"].as_str().unwrap())).collect();
    assert_eq!(rows, [("remove_machine", "denied"), ("remove_machine", "pending"), ("memory_search", "refused"), ("rename_machine", "failed"), ("machines", "done")], "bob's trail holds his acts only");
    assert!(bobs_audit.as_array().unwrap().iter().all(|r| r["machine"] == bob.machine.as_str()));

    assert_eq!(ana.client.hub(json!({ "cmd": "confirmations" })).unwrap().as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>(), [remove_id.clone(), invite_id.clone()], "ana's waiting one, then her decided one; none of bob's");
    ana.node.machine.call(Call::SetGlobalOrchestratorSettings { settings: pinned_core::global_orchestrator::Settings { host: true, ..Default::default() } }).unwrap();
    let global = ana.node.machine.call(Call::OpenGlobalOrchestrator).unwrap().as_str().unwrap().to_owned();
    ana.client.wait("the global tab at the hub", |f| f["type"] == "event" && f["event"]["kind"] == "tabs" && f["event"]["tabs"].as_array().unwrap().iter().any(|t| t["id"] == global.as_str()));
    let waiting = session_tool(&ana.dir, &global, json!({ "name": "waiting" })).unwrap();
    let confirm = waiting.as_array().unwrap().iter().find(|w| w["kind"] == "confirm").unwrap_or_else(|| panic!("{waiting}"));
    assert_eq!((&confirm["title"], &confirm["tab"]), (&json!("Approve remove_machine?"), &json!(ana.tab)));

    let approved = ana.client.hub(json!({ "cmd": "decide_confirmation", "id": remove_id, "approve": true })).unwrap();
    assert_eq!((&approved["decision"], &approved["outcome"]), (&json!("approved"), &json!("done")));
    assert_eq!(ana.client.hub(json!({ "cmd": "decide_confirmation", "id": remove_id, "approve": true })), Err(format!("no confirmation {remove_id} waits")), "a second click decides nothing");
    assert_eq!(names(&ana.client.hub(json!({ "cmd": "machines" })).unwrap()), ["Ana Mini"]);
    ana.client.wait("the emptied list in the app", |f| f["type"] == "confirmations" && f["items"] == json!([]));
    wait_for("the removal's notice in the agent's queue", || ana.node.machine.view(&ana.tab).unwrap().queued >= 1);
    assert_eq!(turn_end_context(&ana.dir, &ana.tab), format!("[pinned] The user approved your remove_machine request ({remove_id}): done."));

    let trail = ana.client.hub(json!({ "cmd": "agent_audit" })).unwrap();
    let rows: Vec<(&str, &str, &str)> = trail.as_array().unwrap().iter().map(|r| (r["action"].as_str().unwrap(), r["outcome"].as_str().unwrap(), r["detail"].as_str().unwrap())).collect();
    assert_eq!(
        rows,
        [
            ("remove_machine", "done", ""),
            ("remove_machine", "approved", ""),
            ("create_invite", "done", ""),
            ("create_invite", "approved", ""),
            ("create_invite", "pending", ""),
            ("remove_machine", "pending", ""),
            ("machines", "refused", "ana-spare runs no session tab-zz-1"),
            ("nope", "refused", "no action nope; call actions to list them"),
            ("rename_machine", "refused", "an agent never sends a password, key or token: token. The user enters it in the app."),
            ("prompt", "refused", "prompt is a write action that is not wired yet"),
            ("write_input", "refused", "write_input is not open to agents: the app's terminal plumbing"),
            ("memory_search", "done", ""),
            ("memory_save", "done", ""),
            ("rename_machine", "done", ""),
            ("machines", "done", ""),
        ],
        "ana's trail, newest first, with none of bob's"
    );
    let secret = trail.as_array().unwrap().iter().find(|r| r["detail"].as_str().unwrap().starts_with("an agent never")).unwrap();
    assert_eq!(secret["args"], format!(r#"{{"id":"{}","name":"x","token":"[hidden]"}}"#, ana.machine));
    assert_eq!((&trail[0]["role"], &trail[0]["tab"], &trail[0]["machine_name"]), (&json!("session"), &json!(ana.tab), &json!("Ana Mini")));
    assert_eq!(trail[6]["machine_name"], "", "a removed machine's rows stay, without its name");
    assert_eq!(act(&bob.dir, &bob.tab, "agent_audit", json!({ "limit": 200 })).unwrap()["result"].as_array().unwrap().len(), 6, "bob's trail never shows ana's");
    let ids = |page: &Value| page.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(ids(&ana.client.hub(json!({ "cmd": "agent_audit", "limit": 2 })).unwrap()), ids(&trail)[..2]);
    let oldest = &trail[14];
    let page = ana.client.hub(json!({ "cmd": "agent_audit", "before": oldest["at"] })).unwrap();
    assert!(ids(&page).contains(&oldest["id"].as_str().unwrap().to_owned()), "before keeps the rows of its own second: {page}");

    let keyed = |tab: &str, action: &str| session_tool(&ana.dir, tab, json!({ "name": "act", "action": action, "request_id": "r-same" })).unwrap()["outcome"].clone();
    assert_eq!(keyed(&ana.tab, "machines"), "done");
    assert_eq!(keyed(&ana.tab, "machines"), "repeat");
    assert_eq!(keyed(&global, "machines"), "done", "another session's request_id is its own");
    assert_eq!(keyed(&ana.tab, "confirmations"), "done", "and so is another action's");
    drop(bob);
}
