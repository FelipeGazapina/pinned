//! A load generator for a local hub, to watch its memory under traffic like production's: fake
//! accounts, each with fake machines that churn their tab lists, stream terminal output and answer
//! thread reads with 1–8 MB pages, and fake clients that watch threads, attach to tabs, read every
//! page they hear of and reconnect now and then.
//!
//! cargo run --release -p pinned-server --example hub_load -- --hub 127.0.0.1:8790 --pid <hub pid> --secs 540
//!
//! Every `--every` seconds it prints the hub's VmRSS and the bytes the hub sent to the fake peers
//! in that interval, by frame type. Nothing here touches a real account or hub.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

type Socket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

#[derive(Clone)]
struct Opts {
    hub: String,
    accounts: usize,
    machines: usize,
    clients: usize,
    tabs: usize,
    /// Seconds between a machine's Tabs events.
    tabs_every: f64,
    /// Seconds between a machine's Thread events.
    thread_every: f64,
    page_min: usize,
    page_max: usize,
    /// Seconds a client stays connected before it reconnects.
    client_life: f64,
    /// Seconds a machine stays connected before it reconnects.
    machine_life: f64,
    /// Output bytes per second a machine streams to each attached viewer.
    output_rate: usize,
    secs: u64,
    every: u64,
    pid: Option<u32>,
    tag: String,
}

/// Bytes the hub sent, by frame type, since the last report.
#[derive(Default)]
struct Tally {
    by_kind: Mutex<HashMap<String, (u64, u64, u64)>>,
}

impl Tally {
    fn add(&self, kind: String, bytes: usize) {
        let mut map = self.by_kind.lock().unwrap();
        let entry = map.entry(kind).or_default();
        entry.0 += 1;
        entry.1 += bytes as u64;
        entry.2 = entry.2.max(bytes as u64);
    }
    fn take(&self) -> HashMap<String, (u64, u64, u64)> {
        std::mem::take(&mut *self.by_kind.lock().unwrap())
    }
}

/// xorshift, so a run is the same each time.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() as usize) % (hi - lo + 1)
    }
    fn text(&mut self, n: usize) -> String {
        const WORDS: [&str; 12] = ["the", "hub", "reads", "a", "transcript", "page", "and", "sends", "it", "to", "every", "client"];
        let mut s = String::with_capacity(n + 16);
        while s.len() < n {
            s.push_str(WORDS[(self.next() % 12) as usize]);
            s.push(' ');
        }
        s
    }
}

fn http_post(hub: &str, path: &str, bearer: Option<&str>, body: &Value) -> Value {
    let body = body.to_string();
    let mut stream = TcpStream::connect(hub).expect("connect to the hub");
    let auth = bearer.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    write!(stream, "POST {path} HTTP/1.1\r\nHost: {hub}\r\nContent-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    let (head, body) = answer.split_once("\r\n\r\n").expect("an HTTP answer");
    assert!(head.starts_with("HTTP/1.1 200"), "{path}: {head} {body}");
    serde_json::from_str(body).unwrap_or_else(|e| panic!("{path}: {e}: {body}"))
}

fn connect(hub: &str, path: &str) -> Socket {
    let (mut socket, _) = tungstenite::connect(format!("ws://{hub}{path}")).expect("websocket");
    if let MaybeTlsStream::Plain(tcp) = socket.get_mut() {
        tcp.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        tcp.set_nodelay(true).unwrap();
    }
    socket
}

/// The next frame, or None when none arrived in the read timeout. Panics on a closed socket.
fn read(socket: &mut Socket) -> Result<Option<String>, String> {
    match socket.read() {
        Ok(Message::Text(text)) => Ok(Some(text.as_str().to_owned())),
        Ok(Message::Close(frame)) => Err(format!("closed: {frame:?}")),
        Ok(_) => Ok(None),
        Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn send(socket: &mut Socket, frame: &Value) -> Result<(), String> {
    socket.send(Message::text(frame.to_string())).map_err(|e| e.to_string())
}

fn tab_view(rng: &mut Rng, n: usize, round: u64) -> Value {
    let status = ["working", "done", "needs-input", "failed"][((n as u64 + round) % 4) as usize];
    json!({
        "id": format!("tab-{n}"),
        "cwd": format!("/home/dev/projects/repo-{}/src/feature-{n}", n % 7),
        "bypass": false,
        "running": true,
        "status": status,
        "title": rng.text(60),
        "message": if status == "needs-input" { Value::String(rng.text(300)) } else { Value::Null },
        "resume_at": null,
        "connector": null,
        "opened": n,
        "updated": 1_700_000_000 + round,
        "session": format!("{:032x}", rng.next()),
        "branch": format!("feature/branch-{n}"),
        "last_message_at": 1_700_000_000 + round,
        "prompt_title": rng.text(80),
        "question": if n % 3 == 0 { Value::String(rng.text(500)) } else { Value::Null },
        "ask_detail": if n % 5 == 0 { Value::String(rng.text(300)) } else { Value::Null },
        "alive": true,
    })
}

/// A thread page about `bytes` long, of many small items as a transcript has, sometimes with a picture.
fn page(rng: &mut Rng, bytes: usize) -> Value {
    let mut items = Vec::new();
    let mut size = 0;
    if rng.next() % 4 == 0 {
        let picture = format!("data:image/png;base64,{}", "A".repeat(2 << 20));
        size += picture.len();
        items.push(json!({"kind": "user", "id": "u0", "at": "2026-11-14T00:00:00Z", "text": "look", "images": 1, "pictures": [picture]}));
    }
    let mut i = 0;
    while size < bytes {
        let len = rng.range(200, 4000);
        let item = match i % 3 {
            0 => json!({"kind": "text", "id": format!("t{i}"), "at": "2026-11-14T00:00:00Z", "text": rng.text(len)}),
            1 => json!({"kind": "tool", "id": format!("c{i}"), "at": "2026-11-14T00:00:00Z", "name": "Bash", "summary": rng.text(60), "detail": rng.text(len)}),
            _ => json!({"kind": "result", "tool": format!("c{}", i - 1), "at": "2026-11-14T00:00:00Z", "error": false, "text": rng.text(len)}),
        };
        size += len + 100;
        items.push(item);
        i += 1;
    }
    json!({"session": "s", "items": items, "next": size, "reset": false, "agents": []})
}

fn machine(o: Opts, token: String, name: String, seed: u64, stop: Arc<AtomicBool>, sent: Arc<AtomicU64>) {
    let mut rng = Rng(seed);
    let mut round = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let mut socket = connect(&o.hub, "/ws/machine");
        send(&mut socket, &json!({"type": "auth", "token": token, "name": name, "os": "linux", "version": "0.18.3"})).unwrap();
        let born = Instant::now();
        let (mut last_tabs, mut last_thread, mut last_output) = (Instant::now() - Duration::from_secs(60), Instant::now(), Instant::now());
        // viewer -> tabs it subscribed to.
        let mut viewers: HashMap<String, Vec<String>> = HashMap::new();
        let mut watched: HashMap<String, Vec<String>> = HashMap::new();
        let life = Duration::from_secs_f64(o.machine_life * (0.5 + (rng.next() % 1000) as f64 / 1000.0));
        let result: Result<(), String> = (|| {
            while !stop.load(Ordering::Relaxed) && born.elapsed() < life {
                while let Some(text) = read(&mut socket)? {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    match frame["type"].as_str() {
                        Some("call") => {
                            let id = frame["id"].as_u64().unwrap();
                            let answer = match frame["call"]["cmd"].as_str() {
                                Some("thread") => {
                                    let bytes = rng.range(o.page_min, o.page_max);
                                    page(&mut rng, bytes)
                                }
                                _ => Value::Null,
                            };
                            let reply = json!({"type": "reply", "id": id, "ok": answer}).to_string();
                            sent.fetch_add(reply.len() as u64, Ordering::Relaxed);
                            socket.send(Message::text(reply)).map_err(|e| e.to_string())?;
                        }
                        Some("subscribe") => {
                            let (tab, viewer) = (frame["tab"].as_str().unwrap().to_owned(), frame["viewer"].as_str().unwrap().to_owned());
                            // The screen first, as a real machine replays it.
                            let screen = base64(&vec![b'x'; 64 << 10]);
                            send(&mut socket, &json!({"type": "output", "viewer": viewer, "tab": tab, "data": screen}))?;
                            viewers.entry(viewer).or_default().push(tab);
                        }
                        Some("unsubscribe") => {
                            let (tab, viewer) = (frame["tab"].as_str().unwrap(), frame["viewer"].as_str().unwrap());
                            if let Some(tabs) = viewers.get_mut(viewer) {
                                tabs.retain(|t| t != tab);
                            }
                        }
                        Some("watch_threads") => {
                            let viewer = frame["viewer"].as_str().unwrap().to_owned();
                            let tabs: Vec<String> = frame["tabs"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_owned()).collect();
                            if tabs.is_empty() {
                                watched.remove(&viewer);
                                viewers.remove(&viewer);
                            } else {
                                watched.insert(viewer, tabs);
                            }
                        }
                        _ => {}
                    }
                }
                if last_tabs.elapsed().as_secs_f64() >= o.tabs_every {
                    last_tabs = Instant::now();
                    round += 1;
                    let tabs: Vec<Value> = (0..o.tabs).map(|n| tab_view(&mut rng, n, round)).collect();
                    send(&mut socket, &json!({"type": "event", "event": {"kind": "tabs", "tabs": tabs}}))?;
                    if round % 20 == 0 {
                        send(&mut socket, &json!({"type": "event", "event": {"kind": "usage", "usage": {}, "account": "default"}})).ok();
                    }
                }
                if last_thread.elapsed().as_secs_f64() >= o.thread_every {
                    last_thread = Instant::now();
                    let tabs: Vec<&String> = watched.values().flatten().collect();
                    if !tabs.is_empty() {
                        let tab = tabs[(rng.next() as usize) % tabs.len()].clone();
                        send(&mut socket, &json!({"type": "event", "event": {"kind": "thread", "tab": tab}}))?;
                    }
                }
                if o.output_rate > 0 && last_output.elapsed() >= Duration::from_millis(100) {
                    last_output = Instant::now();
                    let chunk = base64(&vec![b'y'; o.output_rate / 10]);
                    for (viewer, tabs) in &viewers {
                        for tab in tabs {
                            send(&mut socket, &json!({"type": "output", "viewer": viewer, "tab": tab, "data": chunk}))?;
                        }
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[{}] machine {name}: {e}", o.tag);
        }
        let _ = socket.close(None);
        let _ = socket.flush();
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn client(o: Opts, token: String, machines: Vec<String>, seed: u64, stop: Arc<AtomicBool>, tally: Arc<Tally>) {
    let mut rng = Rng(seed);
    while !stop.load(Ordering::Relaxed) {
        let mut socket = connect(&o.hub, "/ws/client");
        send(&mut socket, &json!({"type": "auth", "token": token})).unwrap();
        let born = Instant::now();
        let life = Duration::from_secs_f64(o.client_life * (0.5 + (rng.next() % 1000) as f64 / 1000.0));
        let mut next_id = 1u64;
        let mut setup = false;
        let mut last_settings = Instant::now();
        let result: Result<(), String> = (|| {
            while !stop.load(Ordering::Relaxed) && born.elapsed() < life {
                while let Some(text) = read(&mut socket)? {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    let kind = match frame["type"].as_str().unwrap_or("?") {
                        "event" => format!("event:{}", frame["event"]["kind"].as_str().unwrap_or("?")),
                        other => other.to_owned(),
                    };
                    tally.add(kind, text.len());
                    if frame["type"] == "event" && frame["event"]["kind"] == "thread" {
                        let tab = frame["event"]["tab"].clone();
                        let machine = frame["machine"].clone();
                        next_id += 1;
                        send(&mut socket, &json!({"type": "call", "id": next_id, "machine": machine, "call": {"cmd": "thread", "tab": tab, "from": 0}}))?;
                    }
                }
                if !setup && born.elapsed() > Duration::from_millis(300) {
                    setup = true;
                    for machine in &machines {
                        let tabs: Vec<String> = (0..4).map(|_| format!("tab-{}", rng.range(0, o.tabs - 1))).collect();
                        next_id += 1;
                        send(&mut socket, &json!({"type": "call", "id": next_id, "machine": machine, "call": {"cmd": "watch_threads", "tabs": tabs}}))?;
                        send(&mut socket, &json!({"type": "attach", "machine": machine, "tab": tabs[0]}))?;
                    }
                }
                if last_settings.elapsed() > Duration::from_secs(30) {
                    last_settings = Instant::now();
                    next_id += 1;
                    send(&mut socket, &json!({"type": "hub", "id": next_id, "call": {"cmd": "get_settings"}}))?;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[{}] client: {e}", o.tag);
        }
        let _ = socket.close(None);
        let _ = socket.flush();
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn rss_kib(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    status.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.trim().strip_suffix("kB")).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).map(|i| args[i + 1].clone());
    let num = |name: &str, default: f64| flag(name).map_or(default, |v| v.parse().unwrap());
    let o = Opts {
        hub: flag("--hub").expect("--hub host:port"),
        accounts: num("--accounts", 3.0) as usize,
        machines: num("--machines", 1.0) as usize,
        clients: num("--clients", 1.0) as usize,
        tabs: num("--tabs", 40.0) as usize,
        tabs_every: num("--tabs-every", 1.0),
        thread_every: num("--thread-every", 2.0),
        page_min: (num("--page-min-mb", 1.0) * 1048576.0) as usize,
        page_max: (num("--page-max-mb", 8.0) * 1048576.0) as usize,
        client_life: num("--client-life", 60.0),
        machine_life: num("--machine-life", 300.0),
        output_rate: num("--output-rate", 40960.0) as usize,
        secs: num("--secs", 300.0) as u64,
        every: num("--every", 30.0) as u64,
        pid: flag("--pid").map(|p| p.parse().unwrap()),
        tag: flag("--tag").unwrap_or_else(|| "load".into()),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let tally = Arc::new(Tally::default());
    let machine_sent = Arc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    let run = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
    for a in 0..o.accounts {
        let email = format!("load-{run}-{a}@example.test");
        let session = http_post(&o.hub, "/api/signup", None, &json!({"email": email, "password": "load-test-password-1"}));
        let token = session["token"].as_str().unwrap().to_owned();
        let mut ids = Vec::new();
        for m in 0..o.machines {
            let name = format!("fake-{a}-{m}");
            let registered = http_post(&o.hub, "/api/machines", Some(&token), &json!({"name": name, "os": "linux"}));
            ids.push(registered["machine"]["id"].as_str().unwrap().to_owned());
            let (o, t, s, sent) = (o.clone(), registered["token"].as_str().unwrap().to_owned(), stop.clone(), machine_sent.clone());
            threads.push(std::thread::spawn(move || machine(o, t, name, 7 + (a * 31 + m) as u64, s, sent)));
        }
        for c in 0..o.clients {
            let (o, t, ids, s, tally) = (o.clone(), token.clone(), ids.clone(), stop.clone(), tally.clone());
            threads.push(std::thread::spawn(move || client(o, t, ids, 1000 + (a * 31 + c) as u64, s, tally)));
        }
    }
    let start = Instant::now();
    let mut last = Instant::now();
    while start.elapsed() < Duration::from_secs(o.secs) {
        std::thread::sleep(Duration::from_millis(250));
        if last.elapsed() >= Duration::from_secs(o.every) {
            let secs = last.elapsed().as_secs_f64();
            last = Instant::now();
            let tallies = tally.take();
            let total: u64 = tallies.values().map(|t| t.1).sum();
            let mut kinds: Vec<_> = tallies.into_iter().collect();
            kinds.sort_by_key(|(_, t)| std::cmp::Reverse(t.1));
            let kinds: Vec<String> = kinds.iter().map(|(k, (n, b, max))| format!("{k}={n}/{}KB/max{}KB", b >> 10, max >> 10)).collect();
            let rss = o.pid.map(rss_kib).unwrap_or(0);
            println!(
                "[{}] t={:>5}s rss_mb={} to_clients_mb_per_h={:.0} machine_up_mb={} {}",
                o.tag,
                start.elapsed().as_secs(),
                rss >> 10,
                total as f64 / secs * 3600.0 / 1048576.0,
                machine_sent.swap(0, Ordering::Relaxed) >> 20,
                kinds.join(" ")
            );
        }
    }
    stop.store(true, Ordering::Relaxed);
    for t in threads {
        let _ = t.join();
    }
}
