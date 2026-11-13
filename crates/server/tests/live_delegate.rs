//! Ignored. Run it by hand: `cargo test -p pinned-server --test live_delegate -- --ignored --nocapture`
//! A real Machine socket and the real `pinned-server mcp` process, then a real Cursor child and a real Claude child, which runs as a worker thread.

use pinned_core::agent::{SOCK_ENV, TAB_ENV, TASK_ENV};
use pinned_core::space::Space;
use pinned_core::machine::{Launch, Machine};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

#[test]
#[ignore]
fn live_cursor_and_claude_through_pinned_mcp() {
    let exe = server_bin();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("pinned-live-delegate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let launch = Launch::prepare(&exe).expect("prepare");
    let machine = Machine::new(launch, Space::at(&dir), Some(root), |_| {});
    machine
        .listen(|_| Err("not a connector callback".into()))
        .unwrap();
    let sock = machine.launch().sock.clone();
    let tab = machine.open_tab(Some(dir.display().to_string()), false).unwrap();
    for (account, model) in [("cursor", "composer-2.5"), ("claude", "claude-haiku-5-5")] {
        let status = ask(&exe, &sock, &tab, account, model);
        eprintln!("LIVE {account} {model}: {}", status);
        let summary = status["summary"].as_str().unwrap_or("");
        assert!(
            summary.contains("PINEAPPLE"),
            "{account}/{model} returned {status}"
        );
    }
    machine.quit();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn live_claude_child_is_a_worker_thread_that_ends_its_task_on_its_first_reply() {
    let exe = server_bin();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("pinned-live-thread-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let launch = Launch::prepare(&exe).expect("prepare");
    let machine = Machine::new(launch, Space::at(&dir), Some(root), |_| {});
    machine.listen(|_| Err("not a connector callback".into())).unwrap();
    let sock = machine.launch().sock.clone();
    let tab = machine.open_tab(Some(dir.display().to_string()), false).unwrap();

    let (reply, thread) = start(&exe, &sock, &tab, "Reply with exactly the word WAITING and nothing else.");
    let view = machine.view(&thread).unwrap();
    assert_eq!((view.role.map(|r| r.name()), view.spawned_by.as_deref(), view.running), (Some("worker"), Some(tab.as_str()), true), "{view:?}");
    let last = wait_until(&exe, &sock, &tab, &reply, |status| status["workState"] != "working");
    eprintln!("LIVE reply: {last}");
    assert_eq!(last["workState"], "result_available", "{last}");
    assert!(last["summary"].as_str().unwrap_or("").contains("WAITING"), "{last}");
    assert_eq!(last["tabId"], thread.as_str(), "{last}");
    assert!(machine.view(&thread).unwrap().running, "the thread stays open once its task ends");

    let (slow, slow_thread) = start(&exe, &sock, &tab, "Use the Bash tool to run exactly: sleep 120. Wait for it, then reply DONE.");
    std::thread::sleep(Duration::from_secs(15));
    let cancelled = mcp(&exe, &sock, &tab, json!({ "name": "task_cancel", "arguments": { "taskId": slow } }));
    assert_eq!(cancelled["workState"], "cancelled", "{cancelled}");
    std::thread::sleep(Duration::from_secs(2));
    assert!(!machine.view(&slow_thread).unwrap().running, "task_cancel left the thread's session running");
    machine.quit();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Starts a Claude Code child and returns its task id and its thread's tab id.
fn start(exe: &Path, sock: &Path, tab: &str, task: &str) -> (String, String) {
    let started = mcp(
        exe,
        sock,
        tab,
        json!({
            "name": "delegate_task",
            "arguments": { "task": task, "target": { "providerInstanceId": "claude", "model": "claude-haiku-5-5" }, "mode": "async" }
        }),
    );
    let id = started["taskId"].as_str().unwrap_or("").to_owned();
    assert!(!id.is_empty(), "delegate_task returned {started}");
    (id, started["tabId"].as_str().unwrap_or("").to_owned())
}

fn wait_until(exe: &Path, sock: &Path, tab: &str, id: &str, done: impl Fn(&Value) -> bool) -> Value {
    let begun = Instant::now();
    loop {
        let status = mcp(exe, sock, tab, json!({ "name": "task_status", "arguments": { "taskId": id } }));
        if done(&status) {
            return status;
        }
        assert!(begun.elapsed() < Duration::from_secs(180), "gave up waiting: {status}");
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn server_bin() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_pinned_server").map(std::path::PathBuf::from) {
        if path.is_file() {
            return path;
        }
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/pinned-server");
    assert!(built.is_file(), "build pinned-server before the live check: {}", built.display());
    built
}

fn ask(exe: &Path, sock: &Path, tab: &str, account: &str, model: &str) -> Value {
    let started = mcp(
        exe,
        sock,
        tab,
        json!({
            "name": "delegate_task",
            "arguments": {
                "task": "Reply with exactly the word PINEAPPLE and nothing else.",
                "target": { "providerInstanceId": account, "model": model },
                "mode": "wait",
                "timeoutMs": 180000,
            }
        }),
    );
    let id = started["taskId"].as_str().unwrap_or("").to_owned();
    assert!(!id.is_empty(), "delegate_task returned {started}");
    mcp(
        exe,
        sock,
        tab,
        json!({ "name": "task_status", "arguments": { "taskId": id } }),
    )
}

fn mcp(exe: &Path, sock: &Path, tab: &str, params: Value) -> Value {
    let mut child = pinned_core::process::command(exe)
        .arg("mcp")
        .env(SOCK_ENV, sock)
        .env(TAB_ENV, tab)
        .env_remove(TASK_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params})
    )
    .unwrap();
    drop(stdin);
    let mut stdout = String::new();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    reader.read_line(&mut stdout).unwrap();
    let response: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("bad mcp answer {stdout:?}: {error}"));
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert_eq!(response["result"]["isError"], false, "{text}");
    let _ = child.wait();
    serde_json::from_str(text).unwrap_or_else(|error| panic!("bad tool text {text:?}: {error}"))
}
