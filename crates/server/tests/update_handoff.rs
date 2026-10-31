use base64::Engine;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

const BIN: &str = env!("CARGO_BIN_EXE_pinned-server");
const SESSION: &str = "sess-handoff";

struct Node {
    pid: u32,
    dir: PathBuf,
    dest: PathBuf,
    original: Vec<u8>,
    script: Vec<u8>,
    fake: PathBuf,
    log: PathBuf,
    stderr: PathBuf,
    go: mpsc::Sender<()>,
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{}", self.pid)])
            .output();
        let _ = Command::new("kill")
            .args(["-KILL", &self.pid.to_string()])
            .output();
        let _ = Command::new("pkill")
            .args(["-KILL", "-f", &self.fake.display().to_string()])
            .output();
        let _ = std::fs::remove_dir_all(&self.dir);
        let prefix = format!("pinned-{}-", self.pid);
        if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
    }
}

impl Node {
    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn signed_release(asset_name: &str, asset: &[u8]) -> (String, String) {
    let latest = json!({ "version": "9.9.9" }).to_string().into_bytes();
    let sums = format!(
        "{}  latest.json\n{}  {asset_name}\n",
        sha256(&latest),
        sha256(asset)
    );
    let minisign::KeyPair { pk, sk } = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let signature = minisign::sign(
        Some(&pk),
        &sk,
        std::io::Cursor::new(sums.as_bytes()),
        None,
        None,
    )
    .unwrap()
    .into_string();
    let pubkey =
        base64::engine::general_purpose::STANDARD.encode(pk.to_box().unwrap().into_string());
    let mut files = std::collections::HashMap::new();
    files.insert("/latest.json".into(), latest);
    files.insert(format!("/{asset_name}"), asset.to_vec());
    files.insert("/SHA256SUMS".into(), sums.into_bytes());
    files.insert("/SHA256SUMS.minisig".into(), signature.into_bytes());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut socket) = incoming else { continue };
            let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0; 8192];
            let Ok(n) = socket.read(&mut buf) else {
                continue;
            };
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let Some(body) = files.get(path) else {
                let _ = socket.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                continue;
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(header.as_bytes());
            let _ = socket.write_all(body);
        }
    });
    (format!("http://{address}"), pubkey)
}

fn hello() -> String {
    json!({
        "type": "hello",
        "machine": { "id": "m1", "name": "vps", "os": "linux", "online": true, "last_seen": 0 }
    })
    .to_string()
}

fn set_timeout(ws: &mut WebSocket<std::net::TcpStream>) {
    let _ = ws
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(200)));
}

fn read_some(ws: &mut WebSocket<std::net::TcpStream>) {
    match ws.read() {
        Ok(_) => {}
        Err(tungstenite::Error::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) => {}
        Err(_) => {}
    }
}

fn parent_socket(mut ws: WebSocket<std::net::TcpStream>, go: mpsc::Receiver<()>) {
    let _ = ws.read();
    ws.send(Message::text(hello())).unwrap();
    set_timeout(&mut ws);
    let mut sent = false;
    loop {
        read_some(&mut ws);
        if !sent && go.try_recv().is_ok() {
            ws.send(Message::text(
                json!({ "type": "call", "id": 7, "call": { "cmd": "update_self" } }).to_string(),
            ))
            .unwrap();
            sent = true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn probe_socket(stream: std::net::TcpStream) {
    let Ok(mut ws) = tungstenite::accept(stream) else {
        return;
    };
    let _ = ws.read();
    let _ = ws.send(Message::text(hello()));
    set_timeout(&mut ws);
    loop {
        match ws.read() {
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
        }
    }
}

fn start_hub(accept_probe: bool) -> (String, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (go, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        if !accept_probe {
            drop(listener);
            let ws = tungstenite::accept(stream).unwrap();
            parent_socket(ws, rx);
            return;
        }
        let ws = tungstenite::accept(stream).unwrap();
        let parent = std::thread::spawn(move || parent_socket(ws, rx));
        for incoming in listener.incoming() {
            let Ok(stream) = incoming else { continue };
            std::thread::spawn(move || probe_socket(stream));
        }
        let _ = parent.join();
    });
    (format!("http://{address}"), go)
}

fn wait_until(limit: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    pred()
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

fn claudes(fake: &Path) -> Vec<(u32, u32, String)> {
    let out = Command::new("ps")
        .args(["-ax", "-o", "pid=,ppid=,command="])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = fake.display().to_string();
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.contains(&needle) {
                return None;
            }
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let ppid = parts.next()?.parse().ok()?;
            Some((pid, ppid, line.to_owned()))
        })
        .collect()
}

fn sockets(pid: u32) -> Vec<PathBuf> {
    let prefix = format!("pinned-{pid}-");
    let mut found = vec![];
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return found;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let sock = entry.path().join("pinned.sock");
            if sock.exists() {
                found.push(sock);
            }
        }
    }
    found
}

fn hook_reply(pid: u32) -> Result<String, String> {
    let socks = sockets(pid);
    if socks.is_empty() {
        return Err(format!("no hook socket for {pid}"));
    }
    let mut errors = vec![];
    for sock in socks {
        match UnixStream::connect(&sock) {
            Ok(mut stream) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                if writeln!(stream, "{{").is_err() {
                    errors.push(format!("{} closed on write", sock.display()));
                    continue;
                }
                let mut line = String::new();
                match BufReader::new(stream).read_line(&mut line) {
                    Ok(0) => errors.push(format!("{} closed", sock.display())),
                    Ok(_) => return Ok(line),
                    Err(error) => errors.push(format!("{}: {error}", sock.display())),
                }
            }
            Err(error) => errors.push(format!("{}: {error}", sock.display())),
        }
    }
    Err(errors.join("; "))
}

fn resumes(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("--resume") && line.contains(SESSION))
        .count()
}

fn boot(accept_probe: bool) -> Node {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let real = PathBuf::from(BIN);
    assert!(
        !real.display().to_string().contains([' ', '\'']),
        "{}",
        real.display()
    );
    let script = format!("#!/bin/sh\nexec {} \"$@\"\n", real.display()).into_bytes();
    let asset = "pinned-server-aarch64-apple-darwin";
    let (base, pubkey) = signed_release(asset, &script);
    let (hub, go) = start_hub(accept_probe);
    let dir = std::env::temp_dir().join(format!(
        "pinned-handoff-{}-{}",
        std::process::id(),
        accept_probe
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("project")).unwrap();
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let log = dir.join("claude.log");
    let fake = dir.join("bin").join("claude");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nprintf '%s %s %s\\n' \"$$\" \"$PPID\" \"$*\" >> '{}'\nsleep 600\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let shell = dir.join("shell");
    std::fs::write(
        &shell,
        "#!/bin/sh\nprintf '\\n__PINNED_PATH__%s__PINNED_PATH__\\n' \"$FAKE_BIN:/bin:/usr/bin\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = dir.join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("account.json"),
        json!({
            "hub": hub,
            "email": "ana@acme.dev",
            "user_token": "user",
            "machine_id": "m1",
            "machine_token": "machine-token"
        })
        .to_string(),
    )
    .unwrap();
    let project = dir.join("project");
    std::fs::write(
        config.join("sessions.json"),
        json!([{
            "cwd": project,
            "session": SESSION,
            "env": [],
            "bypass": false,
            "interrupted": false,
            "account": "claude"
        }])
        .to_string(),
    )
    .unwrap();
    let dest = dir.join("pinned-server");
    std::fs::copy(&real, &dest).unwrap();
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _ = Command::new("codesign")
        .args(["--force", "--sign", "-", dest.to_str().unwrap()])
        .status();
    let original = std::fs::read(&dest).unwrap();
    let stderr = dir.join("stderr");
    let err = std::fs::File::create(&stderr).unwrap();
    let mut cmd = Command::new(&dest);
    cmd.args(["node", "--config-dir", config.to_str().unwrap()])
        .env("HOME", &dir)
        .env("SHELL", &shell)
        .env("FAKE_BIN", dir.join("bin"))
        .env(
            "PATH",
            format!("{}:/bin:/usr/bin", dir.join("bin").display()),
        )
        .env("PINNED_FAKE_CLAUDE_LOG", &log)
        .env("PINNED_RELEASE_BASE", &base)
        .env("PINNED_RELEASE_PUBKEY", &pubkey)
        .env("PINNED_RELEASE_TRIPLE", "aarch64-apple-darwin")
        .env_remove("PINNED_POST_UPDATE")
        .env_remove("PINNED_UPDATE_READY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(err))
        .process_group(0);
    let child = cmd.spawn().unwrap();
    let pid = child.id();
    let node = Node {
        pid,
        dir,
        dest,
        original,
        script,
        fake,
        log,
        stderr,
        go,
    };
    let up = wait_until(Duration::from_secs(20), || {
        hook_reply(pid).is_ok() && claudes(&node.fake).len() == 1 && resumes(&node.log) == 1
    });
    assert!(
        up,
        "the node did not resume its saved session\nlog:\n{}\nprocesses:\n{:?}\n{}",
        std::fs::read_to_string(&node.log).unwrap_or_default(),
        claudes(&node.fake),
        node.stderr()
    );
    node
}

#[test]
fn a_hub_that_is_down_restores_the_binary_and_keeps_the_hook_socket() {
    let node = boot(false);
    node.go.send(()).unwrap();
    assert!(
        wait_until(Duration::from_secs(30), || std::fs::read(&node.dest)
            .unwrap()
            != node.original),
        "the update never replaced the binary\n{}",
        node.stderr()
    );
    assert!(
        wait_until(Duration::from_secs(75), || std::fs::read(&node.dest)
            .unwrap()
            == node.original),
        "the old binary was not restored\n{}",
        node.stderr()
    );
    assert!(
        alive(node.pid),
        "the parent {} exited\n{}",
        node.pid,
        node.stderr()
    );
    let running = claudes(&node.fake);
    assert_eq!(
        running.len(),
        1,
        "claude processes left after rollback: {running:?}\n{}",
        node.stderr()
    );
    assert_eq!(
        running[0].1, node.pid,
        "the session is not parented by the node: {running:?}"
    );
    assert_eq!(
        resumes(&node.log),
        1,
        "the probe resumed the session\n{}",
        std::fs::read_to_string(&node.log).unwrap_or_default()
    );
    let reply = hook_reply(node.pid).unwrap_or_else(|error| panic!("{error}\n{}", node.stderr()));
    assert!(reply.contains("err") || reply.starts_with('{'), "{reply}");
}

#[test]
fn a_ready_probe_keeps_the_pid_and_resumes_the_session_once() {
    let node = boot(true);
    node.go.send(()).unwrap();
    assert!(
        wait_until(Duration::from_secs(45), || resumes(&node.log) >= 2
            || !alive(node.pid)),
        "the session never came back\n{}",
        node.stderr()
    );
    assert!(
        alive(node.pid),
        "parent {} exited instead of becoming the new binary\n{}",
        node.pid,
        node.stderr()
    );
    assert_eq!(
        std::fs::read(&node.dest).unwrap(),
        node.script,
        "the process is not running the staged bytes"
    );
    let running = claudes(&node.fake);
    assert_eq!(
        running.len(),
        1,
        "claude processes: {running:?}\nlog:\n{}",
        std::fs::read_to_string(&node.log).unwrap_or_default()
    );
    assert_eq!(
        running[0].1, node.pid,
        "resumed session parent: {running:?}"
    );
    assert_eq!(
        resumes(&node.log),
        2,
        "session starts:\n{}",
        std::fs::read_to_string(&node.log).unwrap_or_default()
    );
    let reply = hook_reply(node.pid).unwrap_or_else(|error| panic!("{error}\n{}", node.stderr()));
    assert!(reply.contains("err") || reply.starts_with('{'), "{reply}");
}
