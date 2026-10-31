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
fn live_claude_task_waits_for_its_background_shell_then_ends_on_its_last_reply() {
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

    let finite = start(&exe, &sock, &tab, "Use the Bash tool with run_in_background set to true to run exactly: sleep 20 && echo bgdone. Do not wait for it. Reply right away with exactly the word WAITING. Later, when you are notified that the background command finished, reply with FINAL-REPORT followed by its output.");
    let first = wait_until(&exe, &sock, &tab, &finite, |status| status["summary"].is_string());
    eprintln!("LIVE first reply: {first}");
    assert_eq!(first["workState"], "working", "{first}");
    assert_eq!(first["background"].as_array().map(Vec::len), Some(1), "{first}");
    let last = wait_until(&exe, &sock, &tab, &finite, |status| status["workState"] != "working");
    eprintln!("LIVE last reply: {last}");
    assert_eq!(last["workState"], "result_available", "{last}");
    assert!(last["summary"].as_str().unwrap_or("").contains("FINAL-REPORT") && last["summary"].as_str().unwrap_or("").contains("bgdone"), "{last}");

    let server = start(&exe, &sock, &tab, &format!("Use the Bash tool with run_in_background set to true to run exactly: sh -c 'while true; do echo {marker}; sleep 2; done'. Do not wait for it, do not stop it, and do not read its output. Then reply with exactly the word SERVER-UP."));
    wait_until(&exe, &sock, &tab, &server, |status| status["summary"].is_string());
    std::thread::sleep(Duration::from_secs(3));
    let up = mcp(&exe, &sock, &tab, json!({ "name": "task_status", "arguments": { "taskId": server } }));
    eprintln!("LIVE server reply: {up}");
    assert_eq!(up["workState"], "working", "{up}");
    assert!(!pgrep(&marker).is_empty(), "pinned stopped the pending shell");
    let cancelled = mcp(&exe, &sock, &tab, json!({ "name": "task_cancel", "arguments": { "taskId": server } }));
    assert_eq!(cancelled["workState"], "cancelled", "{cancelled}");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(pgrep(&marker), "", "the shell outlived task_cancel");
    machine.quit();
    let _ = std::fs::remove_dir_all(&dir);
}

fn start(exe: &Path, sock: &Path, tab: &str, task: &str) -> String {
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
    id
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

fn pgrep(marker: &str) -> String {
    let found = pinned_core::process::command("pgrep").args(["-fl", marker]).output().unwrap();
    String::from_utf8_lossy(&found.stdout).trim().to_owned()
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
