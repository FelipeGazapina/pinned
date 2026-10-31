//! A node update against the real hub, which allows one socket per machine.
//! These tests fail while a second process signs in as the live machine, and while
//! `node` ignores a binary that was replaced and never connected.

use base64::Engine;
use pinned_core::account::{self, Credentials};
use pinned_server::boxes::{Policy, Upstash};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
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
    database: String,
    dir: PathBuf,
    runtime: tokio::runtime::Runtime,
    server: Option<tokio::task::JoinHandle<()>>,
    addr: std::net::SocketAddr,
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
        let mut hub = Hub {
            url: String::new(),
            database,
            dir,
            runtime: tokio::runtime::Runtime::new().unwrap(),
            server: None,
            addr: "127.0.0.1:0".parse().unwrap(),
        };
        hub.listen("127.0.0.1:0");
        hub
    }

    fn listen(&mut self, listen: &str) {
        let started = Instant::now();
        let running = loop {
            let config = pinned_server::Config {
                db: format!("{}/{}", postgres(), self.database),
                listen: listen.into(),
                ui: self.dir.join("ui"),
                downloads: self.dir.join("downloads"),
                public_url: None,
                key_file: self.dir.join("hub.key"),
                open_signup: true,
                box_api: Arc::new(Upstash),
                box_policy: Policy::default(),
            };
            match self.runtime.block_on(pinned_server::start(config)) {
                Ok(running) => break running,
                Err(error) if error.contains("could not listen") && started.elapsed() < Duration::from_secs(5) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => panic!("{error}"),
            }
        };
        self.addr = running.addr;
        self.url = format!("http://{}", running.addr);
        self.server = Some(running.server);
    }

    fn stop_listener(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if TcpListener::bind(self.addr).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
        admin("pinned", &format!("drop database if exists {} with (force)", self.database));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Client {
    ws: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
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

    fn pump(&mut self) -> bool {
        loop {
            match self.ws.read() {
                Ok(Message::Text(text)) => self.frames.push(serde_json::from_str(&text).unwrap()),
                Ok(Message::Close(_)) => return false,
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                {
                    return true
                }
                Err(_) => return false,
            }
        }
    }

    fn until(&mut self, what: &str, limit: Duration, pred: impl Fn(&Value) -> bool) -> Value {
        let started = Instant::now();
        loop {
            if let Some(found) = self.frames.iter().find(|frame| pred(frame)) {
                return found.clone();
            }
            assert!(self.pump(), "the socket closed while waiting for {what}");
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
}

fn machine_online(frame: &Value, id: &str) -> Option<bool> {
    if frame["type"] != "machines" && frame["type"] != "hello" {
        return None;
    }
    frame["machines"].as_array()?.iter().find(|machine| machine["id"] == id).and_then(|machine| machine["online"].as_bool())
}

fn machine_version(frame: &Value, id: &str) -> Option<String> {
    if frame["type"] != "machines" && frame["type"] != "hello" {
        return None;
    }
    frame["machines"]
        .as_array()?
        .iter()
        .find(|machine| machine["id"] == id)
        .and_then(|machine| machine["version"].as_str())
        .map(str::to_owned)
}

fn offline_transitions(frames: &[Value], id: &str) -> usize {
    let mut transitions = 0;
    let mut online = false;
    for frame in frames {
        let Some(now) = machine_online(frame, id) else { continue };
        if online && !now {
            transitions += 1;
        }
        online = now;
    }
    transitions
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
        let stderr = std::fs::File::create(&stderr_path).unwrap();
        let mut cmd = Command::new(dest);
        cmd.args(["node", "--config-dir", config.to_str().unwrap()])
            .env("HOME", config)
            .env("SHELL", shell(config))
            .env_remove("PINNED_POST_UPDATE")
            .env_remove("PINNED_UPDATE_READY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .process_group(0);
        for (key, value) in extra {
            cmd.env(key, value);
        }
        let child = cmd.spawn().unwrap();
        Node { child, stderr: stderr_path }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
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

fn install_copy(dir: &Path) -> PathBuf {
    let dest = dir.join("pinned-server");
    std::fs::copy(BIN, &dest).unwrap();
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
    sign(&dest);
    dest
}

fn backup(dest: &Path) {
    let bak = dest.with_file_name(format!("{}.bak", dest.file_name().unwrap().to_string_lossy()));
    std::fs::copy(dest, &bak).unwrap();
    sign(&bak);
}

fn marker(dest: &Path) -> PathBuf {
    let name = dest.file_name().unwrap().to_string_lossy();
    dest.with_file_name(format!("{name}.unproven"))
}

fn pair(hub: &str, client: &mut Client, name: &str, dir: &Path) -> Credentials {
    let code = client.hub(json!({ "cmd": "pair_code" })).unwrap();
    let text = code["code"].as_str().unwrap().to_lowercase();
    let creds = account::pair(hub, &text, name, std::env::consts::OS).unwrap();
    creds.save(dir).unwrap();
    creds
}

fn wait_online(client: &mut Client, id: &str, limit: Duration) {
    let started = Instant::now();
    loop {
        if client.frames.iter().rev().find_map(|frame| machine_online(frame, id)) == Some(true) {
            return;
        }
        assert!(client.pump(), "the socket closed while waiting for the machine");
        assert!(started.elapsed() < limit, "the machine did not come online: {:?}", machines(client));
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn machines(client: &Client) -> Vec<Value> {
    client.frames.iter().filter(|frame| frame["type"] == "machines" || frame["type"] == "hello").cloned().collect()
}

fn signed_script(real: &Path) -> (String, String, String) {
    let script = format!("#!/bin/sh\nexport PINNED_VERSION=9.9.9\nexec {} \"$@\"\n", real.display());
    let asset = script.clone().into_bytes();
    let latest = json!({ "version": "9.9.9" }).to_string().into_bytes();
    let sums = format!("{}  latest.json\n{}  pinned-server-test-host\n", sha256(&latest), sha256(&asset));
    let minisign::KeyPair { pk, sk } = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let signature = minisign::sign(Some(&pk), &sk, std::io::Cursor::new(sums.as_bytes()), None, None).unwrap().into_string();
    let pubkey = base64::engine::general_purpose::STANDARD.encode(pk.to_box().unwrap().into_string());
    let files = std::sync::Mutex::new({
        let mut map = std::collections::HashMap::new();
        map.insert("/latest.json".to_owned(), latest);
        map.insert("/pinned-server-test-host".to_owned(), asset);
        map.insert("/SHA256SUMS".to_owned(), sums.into_bytes());
        map.insert("/SHA256SUMS.minisig".to_owned(), signature.into_bytes());
        map
    });
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
            let files = files.lock().unwrap();
            let Some(body) = files.get(path) else {
                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                continue;
            };
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = socket.write_all(header.as_bytes());
            let _ = socket.write_all(body);
        }
    });
    (format!("http://{address}"), pubkey, script)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn a_good_update_keeps_one_offline_gap_and_reports_the_new_version() {
    let real = PathBuf::from(BIN);
    assert!(!real.display().to_string().contains([' ', '\'']), "{}", real.display());
    let hub = Hub::start("good");
    let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&hub.url, &token);
    let dir = std::env::temp_dir().join(format!("pinned-live-good-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let creds = pair(&hub.url, &mut client, "box", &dir);
    let id = creds.machine_id.clone();
    let dest = install_copy(&dir);
    let (base, pubkey, script) = signed_script(&real);
    let mut node = Node::spawn(
        &dest,
        &dir,
        &[
            ("PINNED_RELEASE_BASE", base.as_str()),
            ("PINNED_RELEASE_PUBKEY", pubkey.as_str()),
            ("PINNED_RELEASE_TRIPLE", "test-host"),
        ],
    );
    wait_online(&mut client, &id, Duration::from_secs(20));
    let mark = client.frames.len();
    client.call(&id, json!({ "cmd": "update_self" })).unwrap_or_else(|error| panic!("{error}\n{}", node.output()));
    let started = Instant::now();
    let mut saw = String::new();
    loop {
        client.pump();
        if let Some(version) = client.frames.iter().rev().find_map(|frame| machine_version(frame, &id)) {
            saw = version;
        }
        let online = client.frames.iter().rev().find_map(|frame| machine_online(frame, &id));
        if saw == "9.9.9" && online == Some(true) {
            break;
        }
        if started.elapsed() > Duration::from_secs(60) {
            let transitions = offline_transitions(&client.frames[mark..], &id);
            panic!(
                "the client never saw 9.9.9 online (last version {saw}, offline transitions {transitions}) pid {} alive {}\n{}\n{:?}",
                node.pid(),
                node.alive(),
                node.output(),
                machines(&client)
            );
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let transitions = offline_transitions(&client.frames[mark..], &id);
    assert_eq!(transitions, 1, "a watching client saw {transitions} offline gaps, frames {:?}", machines(&client));
    assert!(node.alive(), "the node pid {} exited\n{}", node.pid(), node.output());
    assert_eq!(std::fs::read(&dest).unwrap(), script.as_bytes(), "the staged bytes were not the release that connected");
    assert!(!marker(&dest).exists(), "the new process left the unproven marker");
    let _ = dir;
}

#[test]
fn a_restart_after_stage_puts_the_old_bytes_back_online() {
    let hub = Hub::start("death");
    let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&hub.url, &token);
    let dir = std::env::temp_dir().join(format!("pinned-live-death-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let creds = pair(&hub.url, &mut client, "box", &dir);
    let id = creds.machine_id.clone();
    let dest = install_copy(&dir);
    backup(&dest);
    std::fs::write(marker(&dest), b"0").unwrap();
    let bak = dest.with_file_name("pinned-server.bak");
    let mut node = Node::spawn(&dest, &dir, &[]);
    wait_online(&mut client, &id, Duration::from_secs(20));
    assert!(!bak.exists(), "the restart left the backup in place\n{}", node.output());
    assert!(!marker(&dest).exists(), "the restart left the unproven marker\n{}", node.output());
    let version = client.frames.iter().rev().find_map(|frame| machine_version(frame, &id)).unwrap_or_default();
    assert_eq!(version, VERSION, "the restarted node reported {version}\n{}", node.output());
    assert!(node.alive(), "the old binary did not stay up\n{}", node.output());
}

#[test]
fn a_proof_that_never_says_hello_puts_the_old_bytes_back() {
    let hub = Hub::start("hang");
    let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&hub.url, &token);
    let dir = std::env::temp_dir().join(format!("pinned-live-hang-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let creds = pair(&hub.url, &mut client, "box", &dir);
    let id = creds.machine_id.clone();
    let dest = install_copy(&dir);
    backup(&dest);
    let marker_path = marker(&dest);
    std::fs::write(&marker_path, b"0").unwrap();
    let addr = hub.addr;
    // The proof boot must not be able to finish Hello. The same address comes back afterwards.
    drop(client);
    // Hub is dropped at the end. Stop only the listener.
    let mut hub = hub;
    hub.stop_listener();
    let marker_env = marker_path.display().to_string();
    let mut node = Node::spawn(
        &dest,
        &dir,
        &[("PINNED_PROOF_BOOT", "1"), ("PINNED_PROOF_WAIT", "2"), ("PINNED_UNPROVEN", marker_env.as_str())],
    );
    let started = Instant::now();
    while marker_path.exists() {
        assert!(started.elapsed() < Duration::from_secs(12), "the marker is still there after a missed Hello\n{}", node.output());
        assert!(node.alive(), "the proof boot exited instead of restoring\n{}", node.output());
        std::thread::sleep(Duration::from_millis(50));
    }
    let bak = dest.with_file_name("pinned-server.bak");
    assert!(!bak.exists(), "the backup is still beside the binary\n{}", node.output());
    hub.listen(&addr.to_string());
    let mut client = Client::connect(&hub.url, &token);
    wait_online(&mut client, &id, Duration::from_secs(20));
    let version = client.frames.iter().rev().find_map(|frame| machine_version(frame, &id)).unwrap_or_default();
    assert_eq!(version, VERSION, "came back as {version}\n{}", node.output());
    assert!(node.alive(), "the restored process exited\n{}", node.output());
}

#[test]
fn a_proof_that_exits_is_rolled_back_by_the_next_start() {
    let hub = Hub::start("exit");
    let (token, _) = account::signup(&hub.url, "ana@acme.dev", "correct horse").unwrap();
    let mut client = Client::connect(&hub.url, &token);
    let dir = std::env::temp_dir().join(format!("pinned-live-exit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let creds = pair(&hub.url, &mut client, "box", &dir);
    let id = creds.machine_id.clone();
    let dest = install_copy(&dir);
    backup(&dest);
    let marker_path = marker(&dest);
    std::fs::write(&marker_path, b"0").unwrap();
    let addr = hub.addr;
    drop(client);
    let mut hub = hub;
    hub.stop_listener();
    let marker_env = marker_path.display().to_string();
    let mut node = Node::spawn(
        &dest,
        &dir,
        &[("PINNED_PROOF_BOOT", "1"), ("PINNED_PROOF_WAIT", "60"), ("PINNED_UNPROVEN", marker_env.as_str())],
    );
    let started = Instant::now();
    loop {
        let body = std::fs::read(&marker_path).unwrap_or_default();
        if body == b"1" {
            break;
        }
        assert!(node.alive(), "the proof boot exited before recording the try: {}\n{}", String::from_utf8_lossy(&body), node.output());
        assert!(started.elapsed() < Duration::from_secs(8), "the boot was not counted, marker is {}\n{}", String::from_utf8_lossy(&body), node.output());
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(node);
    hub.listen(&addr.to_string());
    let mut client = Client::connect(&hub.url, &token);
    let node = Node::spawn(&dest, &dir, &[]);
    wait_online(&mut client, &id, Duration::from_secs(20));
    assert!(!marker_path.exists(), "the next start left the marker\n{}", node.output());
    assert!(!dest.with_file_name("pinned-server.bak").exists(), "the next start left the backup\n{}", node.output());
    let version = client.frames.iter().rev().find_map(|frame| machine_version(frame, &id)).unwrap_or_default();
    assert_eq!(version, VERSION, "came back as {version}\n{}", node.output());
}
