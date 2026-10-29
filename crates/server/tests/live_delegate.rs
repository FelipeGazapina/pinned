//! Ignored. Run it by hand: `cargo test -p pinned-server --test live_delegate -- --ignored --nocapture`
//! A real Machine socket and the real `pinned-server mcp` process, then a real Cursor child and a real Claude child.

use pinned_core::agent::{SOCK_ENV, TAB_ENV, TASK_ENV};
use pinned_core::machine::{Launch, Machine};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Stdio;

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
    for (account, model) in [("cursor", "composer-2.5"), ("claude", "claude-haiku-5-5")] {
        let status = ask(&exe, &sock, account, model);
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

fn ask(exe: &Path, sock: &Path, account: &str, model: &str) -> Value {
    let started = mcp(
        exe,
        sock,
        json!({
            "name": "delegate_task",
            "arguments": {
                "task": "Reply with exactly the word PINEAPPLE and nothing else.",
                "target": { "providerInstanceId": account, "model": model },
                "timeoutMs": 180000,
            }
        }),
    );
    let id = started["taskId"].as_str().unwrap_or("").to_owned();
    assert!(!id.is_empty(), "delegate_task returned {started}");
    mcp(
        exe,
        sock,
        json!({ "name": "task_status", "arguments": { "taskId": id } }),
    )
}

fn mcp(exe: &Path, sock: &Path, params: Value) -> Value {
    let mut child = pinned_core::process::command(exe)
        .arg("mcp")
        .env(SOCK_ENV, sock)
        .env_remove(TAB_ENV)
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
