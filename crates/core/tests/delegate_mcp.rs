//! `pinned mcp` to the machine socket to a fake provider, then back.
//! The binary's `mcp` subcommand is `reminders::serve_io`.
#![cfg(unix)]

use pinned_core::accounts::Provider;
use pinned_core::agent::{SOCK_ENV, TAB_ENV, TASK_ENV};
use pinned_core::machine::{Launch, Machine};
use pinned_core::reminders;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!("pinned-delegate-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    path
}

fn rpc(reminders_on: bool, params: Value) -> Value {
    let (mut client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let reader = server.try_clone().unwrap();
    std::thread::spawn(move || {
        let _ = reminders::serve_io(reminders_on, BufReader::new(reader), server);
    });
    writeln!(
        client,
        "{}",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params})
    )
    .unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut line = String::new();
    BufReader::new(&client).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn tool_text(response: &Value) -> String {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(response["result"]["isError"], false, "{text}");
    text
}

#[test]
fn pinned_mcp_runs_a_fake_child_and_cancel_kills_it() {
    let dir = dir();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/delegate/cursor.jsonl");
    let pid_file = dir.join("child.pid");
    let bin = script(
        &dir,
        "cursor-agent",
        &format!(
            r#"if [ "$1" = "status" ]; then printf '%s\n' 'Logged in as ana@acme.dev'; exit 0; fi
if [ "$1" = "--list-models" ]; then printf '%s\n' 'composer-2.5 - Composer'; exit 0; fi
case " $* " in
  *" SLEEP "*) printf '%s\n' "$$" > '{}'; printf '%s\n' '{{"type":"system","subtype":"init","session_id":"s1"}}'; sleep 30 ;;
  *) cat '{}' ;;
esac"#,
            pid_file.display(),
            fixture.display(),
        ),
    );
    let launch = Launch {
        exe: bin.clone(),
        bins: BTreeMap::from([(Provider::Cursor, bin)]),
        path_env: None,
        settings: dir.join("settings.json"),
        mcp_config: None,
        reminders: true,
        plugin_dir: dir.join("plugin"),
        tutorial_helper: dir.join("helper"),
        sock: dir.join("pinned.sock"),
        vector_search: false,
    };
    let machine = Machine::new(launch, Some(dir.clone()), Some(dir.clone()), |_| {});
    machine
        .listen(|_| Err("not a connector callback".into()))
        .unwrap();
    std::env::set_var(SOCK_ENV, machine.launch().sock.clone());
    std::env::remove_var(TAB_ENV);
    std::env::remove_var(TASK_ENV);
    let refused = rpc(
        true,
        json!({ "name": "delegate_task", "arguments": { "task": "say done" } }),
    );
    let refused_text = refused["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert_eq!(refused["result"]["isError"], true, "{refused_text}");
    assert!(
        refused_text.contains("PINNED_TAB") && refused_text.contains("PINNED_TASK"),
        "{refused_text}"
    );
    let tab = machine.open_tab(Some(dir.display().to_string()), false).unwrap();
    std::env::set_var(TAB_ENV, &tab);

    let listed = {
        let (mut client, server) = UnixStream::pair().unwrap();
        let reader = server.try_clone().unwrap();
        std::thread::spawn(move || {
            let _ = reminders::serve_io(false, BufReader::new(reader), server);
        });
        writeln!(
            client,
            "{}",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})
        )
        .unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(!names.contains(&"remind_me"));
    assert!(names.contains(&"delegate_task"));

    let started = rpc(
        true,
        json!({
            "name": "delegate_task",
            "arguments": { "task": "say done", "target": { "providerInstanceId": "cursor", "model": "composer-2.5" }, "mode": "wait", "timeoutMs": 15000 },
            "_meta": { "claudecode/toolUseId": "toolu_e2e" },
        }),
    );
    let body: Value = serde_json::from_str(&tool_text(&started)).unwrap();
    assert_eq!(body["workState"], "result_available");
    assert_eq!(body["summary"], "DONE");
    assert_eq!(body["providerInstanceId"], "cursor");
    let id = body["taskId"].as_str().unwrap();

    let status = rpc(
        true,
        json!({ "name": "task_status", "arguments": { "taskId": id } }),
    );
    let status: Value = serde_json::from_str(&tool_text(&status)).unwrap();
    assert_eq!(status["summary"], "DONE");
    assert_eq!(
        status["conversationId"],
        "9fa2e973-3c1c-4a8e-9ab7-9790584c944c"
    );

    let sleeping = rpc(
        true,
        json!({
            "name": "delegate_task",
            "arguments": { "task": "SLEEP", "target": { "providerInstanceId": "cursor" }, "mode": "async", "timeoutMs": 600 },
        }),
    );
    let sleeping: Value = serde_json::from_str(&tool_text(&sleeping)).unwrap();
    assert_eq!(sleeping["workState"], "working");
    let sleep_id = sleeping["taskId"].as_str().unwrap();
    let start = std::time::Instant::now();
    while !pid_file.is_file() {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "the child never wrote its pid"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let cancelled = rpc(
        true,
        json!({ "name": "task_cancel", "arguments": { "taskId": sleep_id, "reason": "stop" } }),
    );
    let cancelled: Value = serde_json::from_str(&tool_text(&cancelled)).unwrap();
    assert_eq!(cancelled["workState"], "cancelled");
    let start = std::time::Instant::now();
    while pinned_core::process::command("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "cancel left pid {pid} running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    machine.quit();
    let _ = std::fs::remove_dir_all(&dir);
}
