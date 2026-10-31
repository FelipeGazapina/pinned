use base64::Engine;
use pinned_core::account::{self, Credentials};
use pinned_core::accounts::Provider;
use pinned_core::agent::{self, Stage, Subject};
use pinned_core::connectors::{BoxSize, BoxStep, Delivery, Run, RunsOn, Work};
use pinned_core::link::{self, Link};
use pinned_core::machine::{Launch, Machine};
use pinned_core::sync;
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
}

impl TestHub {
    fn start(name: &str) -> TestHub {
        TestHub::start_with(name, true)
    }

    fn start_with(name: &str, open_signup: bool) -> TestHub {
        TestHub::start_with_boxes(name, open_signup, Policy::default())
    }

    fn start_with_boxes(name: &str, open_signup: bool, policy: Policy) -> TestHub {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let database = format!("pinned_test_{name}_{}", std::process::id());
        admin(&format!("drop database if exists {database} with (force)"));
        admin(&format!("create database {database}"));
        let dir = temp(&format!("hub-{name}"));
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::write(dir.join("ui/index.html"), "<title>pinned</title>").unwrap();
        std::fs::create_dir_all(dir.join("downloads")).unwrap();
        std::fs::write(dir.join("downloads/pinned-server-x86_64-unknown-linux-musl"), "ELF").unwrap();
        let boxes = FakeBoxes::new(dir.join("boxes"));
        let mut hub = TestHub { runtime: tokio::runtime::Runtime::new().unwrap(), running: None, url: String::new(), database, dir, boxes, policy };
        hub.serve("127.0.0.1:0", open_signup);
        hub
    }

    fn serve(&mut self, listen: &str, open_signup: bool) {
        let config = || pinned_server::Config {
            db: format!("{}/{}", postgres(), self.database),
            listen: listen.into(),
            ui: self.dir.join("ui"),
            downloads: self.dir.join("downloads"),
            public_url: None,
            key_file: self.dir.join("hub.key"),
            open_signup,
            box_api: self.boxes.clone(),
            box_policy: self.policy.clone(),
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
        self.serve(&addr.to_string(), true);
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
    client.wait("the open tab", |frame| {
        frame["type"] == "event" && frame["machine"] == id && frame["event"]["kind"] == "tabs" && frame["event"]["tabs"].as_array().is_some_and(|tabs| tabs.iter().any(|tab| tab["id"] == tab_id))
    });
    node.machine.quit();
    let empty_at = wait_frame(&mut client, 0, "the empty tab list", |frame| {
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
        let (dir, creds) = {
            let mut state = self.state.lock().unwrap();
            if let Some(failing) = state.failing.take() {
                return Ok(failing);
            }
            let fake = state.boxes.get(&id.0).ok_or("no such box")?;
            let creds: Credentials = serde_json::from_str(&fake.files[&format!("{BOX_DIR}/account.json")]).unwrap();
            assert!(fake.files[&format!("{BOX_DIR}/start.sh")].contains("pinned-server\" node --config-dir"));
            (self.root.join(&id.0), creds)
        };
        std::fs::create_dir_all(&dir).unwrap();
        let node = tokio::task::spawn_blocking(move || {
            let node = Node::start(&dir, creds);
            // A delivery is a prompt, which the node holds until claude takes the terminal, as this stand-in does.
            std::fs::write(dir.join("claude"), "#!/bin/sh\nstty raw -echo; exec cat\n").unwrap();
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
        let (token, account) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
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
    let hub = TestHub::start_with_boxes("box_run", true, fast());
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
    let hub = TestHub::start_with_boxes("box_pool", true, fast());
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
    let hub = TestHub::start_with_boxes("box_fail", true, fast());
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
    let mut hub = TestHub::start_with_boxes("box_restart", true, fast());
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
    let hub = TestHub::start_with_boxes("box_sweep", true, fast());
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
    let hub = TestHub::start_with_boxes("box_remove", true, fast());
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
    let hub = TestHub::start_with_boxes("box_done", true, Policy { done: Duration::from_millis(300), ..fast() });
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
    let hub = TestHub::start_with_boxes("box_unreadable", true, fast());
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
    let hub = TestHub::start_with_boxes("box_fresh", true, fast());
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
    let hub = TestHub::start_with_boxes("box_fixed", true, fast());
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
    let hub = TestHub::start_with_boxes("box_ask", true, fast());
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
    let hub = TestHub::start_with_boxes("box_off", true, fast());
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
    let mut hub = TestHub::start_with_boxes("box_legacy", true, fast());
    let ana = BoxAccount::new(&hub);
    let account = ana.id;
    admin_on(&hub.database, "update accounts set store = store - 'boxes' - 'box_asks' - 'box_grants'");
    hub.restart();
    assert_eq!(hub.runtime.block_on(hub.hub().open(account, ana.box_run(issue("uuid-1")))), Ok(()));
    assert_eq!(created(&hub), 1);
}

#[test]
fn a_denied_ask_opens_nothing() {
    let hub = TestHub::start_with_boxes("box_deny", true, fast());
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
    let hub = TestHub::start_with_boxes("box_fixed_stays", true, Policy { done: Duration::from_millis(300), ..fast() });
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
    let hub = TestHub::start_with_boxes("box_cap", true, Policy { max_open: 1, ..fast() });
    let mut ana = BoxAccount::new(&hub);
    let account = ana.id;
    let open = |work| hub.runtime.block_on(hub.hub().open(account, work));
    set_workflow(&mut ana.client, "fresh");
    assert_eq!(open(ana.box_run(issue("uuid-1"))), Ok(()));
    assert_eq!(open(ana.box_run(issue("uuid-2"))), Err("Upstash Box: 1 box is already open, and this account's limit is 1".into()));
    assert_eq!(created(&hub), 1);
}

struct RawMachine {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl RawMachine {
    fn connect(hub: &str, token: &str) -> RawMachine {
        let (mut ws, _) = tungstenite::connect(format!("{}/ws/machine", hub.replace("http://", "ws://"))).unwrap();
        if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
            stream.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        }
        ws.send(Message::text(
            json!({ "type": "auth", "token": token, "name": "raw", "os": "linux", "version": "9.9.9" }).to_string(),
        ))
        .unwrap();
        let hello = loop {
            match ws.read() {
                Ok(Message::Text(text)) => break text,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                Err(e) => panic!("machine hello failed: {e}"),
            }
        };
        assert!(serde_json::from_str::<Value>(&hello).unwrap()["type"] == "hello");
        RawMachine { ws }
    }

    fn send(&mut self, frame: Value) {
        self.ws.send(Message::text(frame.to_string())).unwrap();
    }

    fn wait(&mut self, pred: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        loop {
            match self.ws.read() {
                Ok(Message::Text(text)) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if pred(&value) {
                        return value;
                    }
                }
                Ok(Message::Close(close)) => panic!("the machine socket closed: {close:?}"),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) => panic!("the machine socket failed: {e}"),
            }
            assert!(start.elapsed() < WAIT, "timed out waiting on the machine socket");
        }
    }
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
    let mut machine = RawMachine::connect(&url, &creds.machine_token);
    watcher.wait_machines(&[("watch-node", true)]);
    let mut other = Client::connect(&url, &token);
    other.wait("hello", |f| f["type"] == "hello" && f["machines"][0]["online"] == true);

    machine.send(json!({ "type": "event", "event": { "kind": "from-the-future", "tab": "tab-9" } }));
    machine.send(json!({ "type": "event", "event": { "kind": "tabs", "tabs": [] } }));
    watcher.wait("tabs after an unknown event", |f| f["type"] == "event" && f["event"]["kind"] == "tabs");
    assert!(other.wait("tabs for the other client", |f| f["type"] == "event" && f["event"]["kind"] == "tabs")["event"]["tabs"].as_array().unwrap().is_empty());

    assert_eq!(watcher.call(&creds.machine_id, json!({ "cmd": "watch_threads", "tabs": ["tab-1"] })), Ok(Value::Null));
    let told = machine.wait(|frame| frame["type"] == "watch_threads");
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
    node.machine.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into() }).unwrap();
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
        }],
    );
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
fn a_dropped_machine_link_replays_the_watch_without_another_client_call() {
    let (_hub, mut client, machine, dir, mut node, tab, file) = node_with_a_transcript("watch_replay");
    watch(&mut client, &machine, &tab);
    append_line(&file, "visible");
    client.wait("the first nudge", |frame| frame["event"]["kind"] == "thread");

    node.link.take().unwrap().stop();
    client.wait_machines(&[("watch-node", false)]);
    node.link = Some(link::spawn(Credentials::load(&dir).unwrap(), node.machine.clone(), "0.0.0-test"));
    client.wait_machines(&[("watch-node", true)]);
    assert_eq!(node.machine.view(&tab).unwrap().id, tab, "the same process kept {tab}");
    client.pump();
    client.frames.clear();
    let before = std::fs::metadata(&file).unwrap().len() as usize;
    append_line(&file, "replayed");
    client.wait("the nudge after the link returns", |frame| frame["event"]["kind"] == "thread");
    assert_eq!(read_texts(&mut client, &machine, &tab, before), vec!["replayed"]);
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
    assert_eq!(*nudges.lock().unwrap(), 1, "the machine still looked at the tab for a client that left");
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
