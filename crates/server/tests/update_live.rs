use base64::Engine;
use pinned_core::account::{self, Credentials};
use pinned_server::boxes::{Policy, Upstash};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const BIN: &str = env!("CARGO_BIN_EXE_pinned-server");
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn postgres() -> String {
    std::env::var("PINNED_TEST_POSTGRES").unwrap_or_else(|_| "postgres://pinned@127.0.0.1:5499".into())
}

fn admin(database: &str, sql: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let url = format!("{}/{database}", postgres());
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        client.batch_execute(sql).await.unwrap();
    });
}

struct Hub {
    url: String,
    addr: SocketAddr,
    database: String,
    dir: PathBuf,
    _runtime: tokio::runtime::Runtime,
    server: tokio::task::JoinHandle<()>,
}

impl Hub {
    fn start(name: &str) -> Hub {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let database = format!("pinned_live_{name}_{}", std::process::id());
        admin("pinned", &format!("drop database if exists {database} with (force)"));
        admin("pinned", &format!("create database {database}"));
        let dir = std::env::temp_dir().join(format!("pinned-live-hub-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::write(dir.join("ui/index.html"), "<title>pinned</title>").unwrap();
        std::fs::create_dir_all(dir.join("downloads")).unwrap();
        let config = pinned_server::Config {
            db: format!("{}/{database}", postgres()),
            listen: "127.0.0.1:0".into(),
            ui: dir.join("ui"),
            downloads: dir.join("downloads"),
            public_url: None,
            key_file: dir.join("hub.key"),
            open_signup: true,
            box_api: Arc::new(Upstash),
            box_policy: Policy::default(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let running = runtime.block_on(pinned_server::start(config)).unwrap();
        Hub { url: format!("http://{}", running.addr), addr: running.addr, database, dir, _runtime: runtime, server: running.server }
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.server.abort();
        admin("pinned", &format!("drop database if exists {} with (force)", self.database));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Gate {
    url: String,
    open: Arc<AtomicBool>,
    live: Arc<Mutex<Vec<TcpStream>>>,
}

impl Gate {
    fn to(upstream: SocketAddr) -> Gate {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let open = Arc::new(AtomicBool::new(true));
        let live = Arc::new(Mutex::new(Vec::new()));
        let (accepting, tracked) = (open.clone(), live.clone());
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(near) = incoming else { continue };
                if !accepting.load(Ordering::SeqCst) {
                    continue;
                }
                let Ok(far) = TcpStream::connect(upstream) else { continue };
                for (mut from, mut to) in [(near.try_clone().unwrap(), far.try_clone().unwrap()), (far.try_clone().unwrap(), near.try_clone().unwrap())] {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut from, &mut to);
                        let _ = to.shutdown(Shutdown::Write);
                    });
                }
                tracked.lock().unwrap().extend([near, far]);
            }
        });
        Gate { url, open, live }
    }

    fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
        for stream in self.live.lock().unwrap().drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn reopen(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
}

struct Client {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    frames: Vec<Value>,
    next: u64,
}

impl Client {
    fn connect(hub: &str, token: &str) -> Client {
        let (mut ws, _) = tungstenite::connect(format!("{}/ws/client", hub.replace("http://", "ws://"))).unwrap();
        if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
            stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        }
        ws.send(Message::text(json!({ "type": "auth", "token": token }).to_string())).unwrap();
        let mut client = Client { ws, frames: vec![], next: 1 };
        client.until("hello", Duration::from_secs(10), |frame| frame["type"] == "hello");
        client
    }

    fn pump(&mut self) {
        loop {
            match self.ws.read() {
                Ok(Message::Text(text)) => self.frames.push(serde_json::from_str(&text).unwrap()),
                Ok(Message::Close(_)) => panic!("the hub closed the client socket"),
                Ok(_) => {}
                Err(tungstenite::Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return,
                Err(error) => panic!("the client socket failed: {error}"),
            }
        }
    }

    fn until(&mut self, what: &str, limit: Duration, pred: impl Fn(&Value) -> bool) -> Value {
        let started = Instant::now();
        loop {
            if let Some(found) = self.frames.iter().find(|frame| pred(frame)) {
                return found.clone();
            }
            self.pump();
            assert!(started.elapsed() < limit, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn request(&mut self, mut frame: Value) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        frame["id"] = json!(id);
        self.ws.send(Message::text(frame.to_string())).unwrap();
        let reply = self.until(&format!("reply {id}"), Duration::from_secs(30), |frame| frame["type"] == "reply" && frame["id"] == id);
        match reply.get("err") {
            Some(error) => Err(error.as_str().unwrap_or("err").to_owned()),
            None => Ok(reply["ok"].clone()),
        }
    }

    fn hub(&mut self, call: Value) -> Result<Value, String> {
        self.request(json!({ "type": "hub", "call": call }))
    }

    fn call(&mut self, machine: &str, call: Value) -> Result<Value, String> {
        self.request(json!({ "type": "call", "machine": machine, "call": call }))
    }

    fn row(&self, id: &str) -> Option<Value> {
        self.frames.iter().rev().find_map(|frame| row_of(frame, id))
    }

    fn version(&self, id: &str) -> String {
        self.row(id).and_then(|row| row["version"].as_str().map(str::to_owned)).unwrap_or_default()
    }

    fn online(&self, id: &str) -> bool {
        self.row(id).is_some_and(|row| row["online"] == true)
    }

    fn rows(&self) -> Vec<Value> {
        self.frames.iter().filter(|frame| frame["type"] == "machines" || frame["type"] == "hello").cloned().collect()
    }
}

fn row_of(frame: &Value, id: &str) -> Option<Value> {
    if frame["type"] != "machines" && frame["type"] != "hello" {
        return None;
    }
    frame["machines"].as_array()?.iter().find(|machine| machine["id"] == id).cloned()
}

fn offline_transitions(frames: &[Value], id: &str) -> usize {
    let mut transitions = 0;
    let mut online = false;
    for row in frames.iter().filter_map(|frame| row_of(frame, id)) {
        let now = row["online"] == true;
        if online && !now {
            transitions += 1;
        }
        online = now;
    }
    transitions
}

fn phases(frames: &[Value], id: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for row in frames.iter().filter_map(|frame| row_of(frame, id)) {
        if let Some(update) = row.get("update").filter(|update| !update.is_null()) {
            if out.last() != Some(update) {
                out.push(update.clone());
            }
        }
    }
    out
}

fn came_back_on_the_old_version() -> Value {
    json!({ "state": "failed", "to": "9.9.9", "reason": format!("came back on {VERSION}") })
}

fn sign(path: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let status = Command::new("codesign").args(["--force", "--sign", "-", path.to_str().unwrap()]).status().unwrap();
    assert!(status.success(), "codesign {}", path.display());
}

fn shell(dir: &Path) -> PathBuf {
    let path = dir.join("shell");
    std::fs::write(&path, "#!/bin/sh\nprintf '\\n__PINNED_PATH__%s__PINNED_PATH__\\n' \"${FAKE_BIN:-/usr/bin}:/bin:/usr/bin\"\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

struct Node {
    child: Child,
    stderr: PathBuf,
}

impl Node {
    fn spawn(dest: &Path, config: &Path, extra: &[(&str, &str)]) -> Node {
        let stderr_path = config.join("stderr");
        let stderr = std::fs::OpenOptions::new().create(true).append(true).open(&stderr_path).unwrap();
        let mut cmd = Command::new(dest);
        cmd.args(["node", "--config-dir", config.to_str().unwrap()])
            .env("HOME", config)
            .env("SHELL", shell(config))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .process_group(0);
        for (key, value) in extra {
            cmd.env(key, value);
        }
        Node { child: cmd.spawn().unwrap(), stderr: stderr_path }
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn output(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let pid = self.child.id();
        let _ = Command::new("kill").args(["-KILL", &format!("-{pid}")]).output();
        let _ = self.child.wait();
    }
}

struct Live {
    hub: Hub,
    gate: Gate,
    client: Client,
    dir: PathBuf,
    id: String,
    dest: PathBuf,
}

impl Live {
    fn start(name: &str) -> Live {
        let hub = Hub::start(name);
        let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
        let mut client = Client::connect(&hub.url, &token);
        let gate = Gate::to(hub.addr);
        let dir = std::env::temp_dir().join(format!("pinned-live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let creds = pair(&gate.url, &mut client, &dir);
        let dest = dir.join("pinned-server");
        std::fs::copy(BIN, &dest).unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
        sign(&dest);
        Live { hub, gate, client, dir, id: creds.machine_id, dest }
    }

    fn marker(&self) -> PathBuf {
        self.dir.join("pinned-server.unproven")
    }

    fn backup(&self) -> PathBuf {
        self.dir.join("pinned-server.bak")
    }

    fn wait_online(&mut self, limit: Duration) {
        let started = Instant::now();
        while !self.client.online(&self.id) {
            self.client.pump();
            assert!(started.elapsed() < limit, "the machine did not come online: {:?}", self.client.rows());
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn wait_offline_since(&mut self, mark: usize, node: &Node, limit: Duration) {
        let started = Instant::now();
        while !self.client.frames[mark..].iter().filter_map(|frame| row_of(frame, &self.id)).any(|row| row["online"] == false) {
            self.client.pump();
            assert!(started.elapsed() < limit, "the old process never left the hub\n{}", node.output());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_settled(&mut self, version: &str, phase: &Value, node: &Node, limit: Duration) {
        let started = Instant::now();
        loop {
            self.client.pump();
            let shown = phases(&self.client.frames, &self.id);
            if self.client.online(&self.id) && self.client.version(&self.id) == version && shown.last() == Some(phase) {
                return;
            }
            assert!(
                started.elapsed() < limit,
                "the page never showed {version} with {phase} (version {}, phases {shown:?})\n{}",
                self.client.version(&self.id),
                node.output()
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn refuse_sign_in(&self) {
        admin(&self.hub.database, &format!("update machines set token_hash = token_hash || '\\x00'::bytea where id = '{}'", self.id));
    }

    fn allow_sign_in(&self) {
        admin(&self.hub.database, &format!("update machines set token_hash = substring(token_hash from 1 for length(token_hash) - 1) where id = '{}'", self.id));
    }

    fn save_session(&self) {
        let saved = json!([{ "cwd": self.dir, "session": "s1", "env": [], "bypass": false, "interrupted": false, "account": "claude" }]);
        std::fs::write(self.dir.join("sessions.json"), serde_json::to_vec_pretty(&saved).unwrap()).unwrap();
    }

    fn claude_that_ignores_hangups(&self) -> String {
        let bin = self.dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let claude = bin.join("claude");
        std::fs::write(&claude, "#!/bin/sh\necho $$ >> \"$HOME/claude.pids\"\ntrap '' HUP\nexec sleep 300\n").unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin.display().to_string()
    }

    fn claudes(&self) -> (Vec<u32>, Vec<u32>) {
        let recorded: Vec<u32> = std::fs::read_to_string(self.dir.join("claude.pids")).unwrap_or_default().lines().filter_map(|line| line.trim().parse().ok()).collect();
        let alive = recorded.iter().copied().filter(|pid| Command::new("kill").args(["-0", &pid.to_string()]).status().is_ok_and(|status| status.success())).collect();
        (recorded, alive)
    }

    fn wait_claudes(&self, recorded: usize, node: &mut Node, limit: Duration) {
        let started = Instant::now();
        loop {
            let (all, alive) = self.claudes();
            if all.len() == recorded && alive.len() == 1 {
                return;
            }
            assert!(node.alive(), "the node exited while claude started\n{}", node.output());
            assert!(started.elapsed() < limit, "wanted {recorded} claudes with one alive, recorded {all:?} alive {alive:?}\n{}", node.output());
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn wait_restored(&self, old: &[u8], node: &mut Node, limit: Duration) {
        let started = Instant::now();
        while std::fs::read(&self.dest).unwrap() != old || self.marker().exists() {
            assert!(node.alive(), "the node exited instead of restoring\n{}", node.output());
            assert!(started.elapsed() < limit, "the old bytes were not put back\n{}", node.output());
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        for pid in self.claudes().1 {
            let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn pair(gate: &str, client: &mut Client, dir: &Path) -> Credentials {
    let code = client.hub(json!({ "cmd": "pair_code" })).unwrap();
    let text = code["code"].as_str().unwrap().to_lowercase();
    let creds = Credentials { hub: gate.to_owned(), ..account::pair(gate, &text, "box", std::env::consts::OS).unwrap() };
    creds.save(dir).unwrap();
    creds
}

fn release(script: &str) -> (String, String) {
    let asset = script.as_bytes().to_vec();
    let latest = json!({ "version": "9.9.9" }).to_string().into_bytes();
    let sums = format!("{}  latest.json\n{}  pinned-server-test-host\n", sha256(&latest), sha256(&asset));
    let minisign::KeyPair { pk, sk } = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let signature = minisign::sign(Some(&pk), &sk, std::io::Cursor::new(sums.as_bytes()), None, None).unwrap().into_string();
    let pubkey = base64::engine::general_purpose::STANDARD.encode(pk.to_box().unwrap().into_string());
    let mut files = std::collections::HashMap::new();
    files.insert("/latest.json".to_owned(), latest);
    files.insert("/pinned-server-test-host".to_owned(), asset);
    files.insert("/SHA256SUMS".to_owned(), sums.into_bytes());
    files.insert("/SHA256SUMS.minisig".to_owned(), signature.into_bytes());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut socket) = incoming else { continue };
            let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0; 8192];
            let Ok(n) = socket.read(&mut buf) else { continue };
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let Some(body) = files.get(path) else {
                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                continue;
            };
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = socket.write_all(header.as_bytes());
            let _ = socket.write_all(body);
        }
    });
    (format!("http://{address}"), pubkey)
}

fn script(before: &str) -> String {
    let real = Path::new(BIN);
    assert!(!real.display().to_string().contains([' ', '\'']), "{}", real.display());
    format!("#!/bin/sh\n{before}\nexport PINNED_UNPROVEN=\"$0.unproven\"\nexport PINNED_VERSION=9.9.9\nexec {} \"$@\"\n", real.display())
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn updating(live: &Live, script: &str, extra: &[(&str, &str)]) -> Node {
    let (base, pubkey) = release(script);
    let mut env = vec![("PINNED_RELEASE_BASE", base.as_str()), ("PINNED_RELEASE_PUBKEY", pubkey.as_str()), ("PINNED_RELEASE_TRIPLE", "test-host")];
    env.extend_from_slice(extra);
    Node::spawn(&live.dest, &live.dir, &env)
}

#[test]
fn a_good_update_keeps_one_offline_gap_and_reports_the_new_version() {
    let mut live = Live::start("good");
    let script = script("");
    let mut node = updating(&live, &script, &[]);
    live.wait_online(Duration::from_secs(20));
    let mark = live.client.frames.len();
    live.client.call(&live.id, json!({ "cmd": "update_self" })).unwrap_or_else(|error| panic!("{error}\n{}", node.output()));
    live.wait_settled("9.9.9", &json!({ "state": "up_to_date", "version": "9.9.9" }), &node, Duration::from_secs(60));
    let transitions = offline_transitions(&live.client.frames[mark..], &live.id);
    assert_eq!(transitions, 1, "a watching client saw {transitions} offline gaps, frames {:?}", live.client.rows());
    assert!(node.alive(), "the node exited\n{}", node.output());
    assert_eq!(std::fs::read(&live.dest).unwrap(), script.as_bytes(), "the staged bytes were not the release that connected");
    assert!(!live.marker().exists(), "the new process left the unproven marker");
    let shown = phases(&live.client.frames, &live.id);
    assert!(shown.iter().any(|phase| phase["state"] == "updating"), "the page never showed the update start: {shown:?}");
}

#[test]
fn a_restart_after_stage_puts_the_old_bytes_back_online() {
    let mut live = Live::start("death");
    std::fs::copy(&live.dest, live.backup()).unwrap();
    sign(&live.backup());
    std::fs::write(live.marker(), format!("{}\n", sha256(b"unproven bytes"))).unwrap();
    let mut node = Node::spawn(&live.dest, &live.dir, &[]);
    live.wait_online(Duration::from_secs(20));
    assert!(!live.backup().exists(), "the restart left the backup in place\n{}", node.output());
    assert!(!live.marker().exists(), "the restart left the unproven marker\n{}", node.output());
    assert_eq!(live.client.version(&live.id), VERSION, "{}", node.output());
    assert!(node.alive(), "the old binary did not stay up\n{}", node.output());
    assert_eq!(phases(&live.client.frames, &live.id), Vec::<Value>::new(), "a restart that was not an update showed a phase");
}

#[test]
fn a_hub_that_is_down_past_the_deadline_keeps_the_new_version() {
    let mut live = Live::start("outage");
    let script = script("sleep 2");
    let mut node = updating(&live, &script, &[("PINNED_PROOF_WAIT", "2")]);
    live.wait_online(Duration::from_secs(20));
    let mark = live.client.frames.len();
    live.client.call(&live.id, json!({ "cmd": "update_self" })).unwrap_or_else(|error| panic!("{error}\n{}", node.output()));
    live.wait_offline_since(mark, &node, Duration::from_secs(30));
    live.gate.close();
    let down = Instant::now();
    while down.elapsed() < Duration::from_secs(8) {
        live.client.pump();
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(node.alive(), "the new process exited while the hub was down\n{}", node.output());
    assert!(live.marker().exists(), "the marker went away while the hub was down\n{}", node.output());
    assert_eq!(std::fs::read(&live.dest).unwrap(), script.as_bytes(), "a hub outage put the old bytes back\n{}", node.output());
    live.gate.reopen();
    live.wait_settled("9.9.9", &json!({ "state": "up_to_date", "version": "9.9.9" }), &node, Duration::from_secs(40));
    let shown = phases(&live.client.frames, &live.id);
    assert!(!shown.iter().any(|phase| phase["state"] == "failed"), "the page showed a failure: {shown:?}");
    assert!(!live.marker().exists(), "the new process left the unproven marker\n{}", node.output());
    assert_eq!(std::fs::read(&live.dest).unwrap(), script.as_bytes(), "the bytes after the hub returned were not the release");
    assert!(node.alive(), "the new process exited\n{}", node.output());
}

#[test]
fn a_binary_the_hub_refuses_rolls_back_and_keeps_one_session() {
    let mut live = Live::start("refused");
    live.save_session();
    let fake = live.claude_that_ignores_hangups();
    let old = std::fs::read(&live.dest).unwrap();
    let mut node = updating(&live, &script("sleep 2"), &[("FAKE_BIN", fake.as_str())]);
    live.wait_online(Duration::from_secs(20));
    live.wait_claudes(1, &mut node, Duration::from_secs(20));
    let mark = live.client.frames.len();
    live.client.call(&live.id, json!({ "cmd": "update_self" })).unwrap_or_else(|error| panic!("{error}\n{}", node.output()));
    live.wait_offline_since(mark, &node, Duration::from_secs(30));
    live.gate.close();
    live.wait_claudes(2, &mut node, Duration::from_secs(20));
    assert!(live.marker().exists(), "the new process was proven before the hub saw it\n{}", node.output());
    live.refuse_sign_in();
    live.gate.reopen();
    live.wait_restored(&old, &mut node, Duration::from_secs(30));
    live.wait_claudes(3, &mut node, Duration::from_secs(20));
    live.allow_sign_in();
    live.wait_settled(VERSION, &came_back_on_the_old_version(), &node, Duration::from_secs(60));
    let (recorded, alive) = live.claudes();
    assert_eq!(alive.len(), 1, "the rollback left claude running twice: alive {alive:?}, recorded {recorded:?}\n{}", node.output());
    assert_eq!(recorded.len(), 3, "each process should have started the saved session once: {recorded:?}");
    assert!(node.alive(), "the restored process exited\n{}", node.output());
}

#[test]
fn a_binary_that_exits_before_it_is_ready_is_replaced_by_the_old_one() {
    let mut live = Live::start("crash");
    let old = std::fs::read(&live.dest).unwrap();
    let mut node = updating(&live, &script("[ -n \"$PINNED_PROOF_BOOT\" ] && exit 42"), &[]);
    live.wait_online(Duration::from_secs(20));
    live.client.call(&live.id, json!({ "cmd": "update_self" })).unwrap_or_else(|error| panic!("{error}\n{}", node.output()));
    let started = Instant::now();
    while node.alive() {
        live.client.pump();
        assert!(started.elapsed() < Duration::from_secs(30), "the new binary did not exit\n{}", node.output());
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(node);
    let node = Node::spawn(&live.dest, &live.dir, &[]);
    live.wait_settled(VERSION, &came_back_on_the_old_version(), &node, Duration::from_secs(30));
    assert_eq!(std::fs::read(&live.dest).unwrap(), old, "the crashed bytes stayed on disk");
    assert!(!live.marker().exists(), "the next start left the marker\n{}", node.output());
}

#[test]
fn a_binary_that_hangs_before_it_is_ready_is_replaced_by_the_old_one() {
    let mut live = Live::start("hang");
    live.save_session();
    let fake = live.claude_that_ignores_hangups();
    let old = std::fs::read(&live.dest).unwrap();
    let mut node = updating(&live, &script(""), &[("PINNED_PROOF_WAIT", "2"), ("FAKE_BIN", fake.as_str())]);
    live.wait_online(Duration::from_secs(20));
    live.wait_claudes(1, &mut node, Duration::from_secs(20));
    let account = live.dir.join("account.json");
    let creds = std::fs::read(&account).unwrap();
    std::fs::remove_file(&account).unwrap();
    assert!(Command::new("mkfifo").arg(&account).status().unwrap().success());
    live.client.call(&live.id, json!({ "cmd": "update_self" })).unwrap_or_else(|error| panic!("{error}\n{}", node.output()));
    live.wait_restored(&old, &mut node, Duration::from_secs(30));
    let feeding = Arc::new(AtomicBool::new(true));
    let feeder = {
        let (feeding, fifo) = (feeding.clone(), account.clone());
        std::thread::spawn(move || {
            use std::os::unix::fs::OpenOptionsExt;
            while feeding.load(Ordering::SeqCst) {
                match std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(&fifo) {
                    Ok(mut reader) => {
                        let _ = reader.write_all(&creds);
                        drop(reader);
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            }
        })
    };
    live.wait_settled(VERSION, &came_back_on_the_old_version(), &node, Duration::from_secs(30));
    feeding.store(false, Ordering::SeqCst);
    feeder.join().unwrap();
    live.wait_claudes(2, &mut node, Duration::from_secs(20));
}

#[test]
fn a_reconnect_while_the_marker_is_unproven_leaves_it_in_place() {
    let mut live = Live::start("marker");
    let bytes = std::fs::read(&live.dest).unwrap();
    let mut node = Node::spawn(&live.dest, &live.dir, &[]);
    live.wait_online(Duration::from_secs(20));
    let pending = format!("{}\n", sha256(b"staged bytes"));
    std::fs::write(live.marker(), &pending).unwrap();
    let mark = live.client.frames.len();
    live.gate.close();
    live.wait_offline_since(mark, &node, Duration::from_secs(20));
    let mark = live.client.frames.len();
    live.gate.reopen();
    let started = Instant::now();
    while !live.client.frames[mark..].iter().any(|frame| frame["type"] == "event" && frame["machine"] == live.id && frame["event"]["kind"] == "tabs") {
        live.client.pump();
        assert!(started.elapsed() < Duration::from_secs(30), "the node never signed in again\n{}", node.output());
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(std::fs::read_to_string(live.marker()).unwrap_or_default(), pending, "the running process changed the marker on hello\n{}", node.output());
    assert_eq!(std::fs::read(&live.dest).unwrap(), bytes, "the reconnect replaced the bytes");
    assert_eq!(live.client.version(&live.id), VERSION);
    assert_eq!(phases(&live.client.frames, &live.id), Vec::<Value>::new(), "a reconnect showed an update phase");
    assert!(node.alive(), "the node exited\n{}", node.output());
}
