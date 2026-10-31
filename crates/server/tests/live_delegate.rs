//! Ignored. Run it by hand: `cargo test -p pinned-server --test live_delegate -- --ignored --nocapture`
//! A real Machine socket and the real `pinned-server mcp` process, then a real Cursor child and a real Claude child.

use pinned_core::agent::{SOCK_ENV, TAB_ENV, TASK_ENV};
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
    let machine = Machine::new(launch, Some(dir.clone()), Some(root), |_| {});
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
fn live_claude_task_ends_on_its_reply_and_its_background_shell_stops() {
    let exe = server_bin();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("pinned-live-background-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let launch = Launch::prepare(&exe).expect("prepare");
    let machine = Machine::new(launch, Some(dir.clone()), Some(root), |_| {});
    machine.listen(|_| Err("not a connector callback".into())).unwrap();
    let sock = machine.launch().sock.clone();
    let tab = machine.open_tab(Some(dir.display().to_string()), false).unwrap();
    let marker = format!("pinned_live_tick_{}", std::process::id());
    let ran = dir.join("shell-ran");
    let task = format!(
        "Use the Bash tool with run_in_background set to true to run exactly: sh -c 'touch {}; while true; do echo {marker}; sleep 1; done'. Do not wait for it, do not stop it, and do not read its output. Then reply with exactly the word PINEAPPLE.",
        ran.display()
    );
    let started = mcp(
        &exe,
        &sock,
        &tab,
        json!({
            "name": "delegate_task",
            "arguments": { "task": task, "target": { "providerInstanceId": "claude", "model": "claude-haiku-5-5" }, "mode": "async" }
        }),
    );
    let id = started["taskId"].as_str().unwrap_or("").to_owned();
    assert!(!id.is_empty(), "delegate_task returned {started}");
    let begun = Instant::now();
    let status = loop {
        let status = mcp(&exe, &sock, &tab, json!({ "name": "task_status", "arguments": { "taskId": id } }));
        if status["workState"] != "working" {
            break status;
        }
        assert!(begun.elapsed() < Duration::from_secs(180), "the task never finished: {status}");
        std::thread::sleep(Duration::from_secs(2));
    };
    eprintln!("LIVE claude background shell: {status} after {:?}", begun.elapsed());
    assert_eq!(status["workState"], "result_available", "{status}");
    assert!(status["summary"].as_str().unwrap_or("").contains("PINEAPPLE"), "{status}");
    assert!(ran.is_file(), "the child never started its background shell");
    std::thread::sleep(Duration::from_secs(1));
    let left = pinned_core::process::command("pgrep").args(["-fl", &marker]).output().unwrap();
    let left = String::from_utf8_lossy(&left.stdout).into_owned();
    assert!(left.trim().is_empty(), "the background shell kept running: {left}");
    let conversation = status["conversationId"].as_str().unwrap_or("");
    let home = std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(&std::env::var("HOME").unwrap()).join(".claude"));
    let transcript = std::fs::read_dir(home.join("projects"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|project| project.path().join(format!("{conversation}.jsonl")))
        .find(|path| path.is_file())
        .expect("the stopped child left no transcript");
    let text = std::fs::read_to_string(&transcript).unwrap();
    assert!(text.contains("PINEAPPLE"), "the transcript lost the reply: {}", transcript.display());
    eprintln!("LIVE transcript kept the reply: {}", transcript.display());
    machine.quit();
    let _ = std::fs::remove_dir_all(&dir);
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
