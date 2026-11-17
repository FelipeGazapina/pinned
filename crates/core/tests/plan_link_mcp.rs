//! A Captain's plan through the MCP server: `plan_set`, then `delegate_task` with `planStep`, to fake
//! Cursor children as delegate_mcp.rs runs them. The plan's glance reads each step from the task
//! linked to it, and a refused link leaves no task behind.
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
use std::time::{Duration, Instant};

fn dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!("pinned-plan-link-mcp-{}", std::process::id()));
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

fn rpc(name: &str, arguments: Value) -> Value {
    let (mut client, server) = UnixStream::pair().unwrap();
    client.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let reader = server.try_clone().unwrap();
    std::thread::spawn(move || {
        let _ = reminders::serve_io(true, BufReader::new(reader), server);
    });
    let params = json!({ "name": name, "arguments": arguments });
    writeln!(client, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params})).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut line = String::new();
    BufReader::new(&client).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

/// The tool's answer: its text, or its refusal text.
fn call(name: &str, arguments: Value) -> Result<String, String> {
    let response = rpc(name, arguments);
    let text = response["result"]["content"][0]["text"].as_str().unwrap_or("").to_owned();
    if response["result"]["isError"] == true {
        Err(text)
    } else {
        Ok(text)
    }
}

/// A tool answer that is a JSON body.
fn body(name: &str, arguments: Value) -> Value {
    let text = call(name, arguments).unwrap_or_else(|error| panic!("{name} refused: {error}"));
    serde_json::from_str(&text).unwrap()
}

fn task_state(id: &str) -> String {
    body("task_status", json!({ "taskId": id }))["workState"].as_str().unwrap().to_owned()
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_plan_step_reads_the_state_of_the_task_linked_to_it_through_mcp() {
    let dir = dir();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/delegate/cursor.jsonl");
    let bin = script(
        &dir,
        "cursor-agent",
        &format!(
            r#"if [ "$1" = "status" ]; then printf '%s\n' 'Logged in as ana@acme.dev'; exit 0; fi
if [ "$1" = "--list-models" ]; then printf '%s\n' 'composer-2.5 - Composer'; exit 0; fi
case " $* " in
  *" SLEEP "*) printf '%s\n' '{{"type":"system","subtype":"init","session_id":"s1"}}'; sleep 30 ;;
  *" FAIL "*) exit 1 ;;
  *) cat '{}' ;;
esac"#,
            fixture.display(),
        ),
    );
    let claude = script(&dir, "claude", "exit 0");

    // Two Captains, each on its own project, and no plan in either yet.
    let config = dir.join("config");
    let project = dir.join("app");
    let other = dir.join("other");
    for folder in [&config, &project, &other] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let (path, other_path) = (project.display().to_string(), other.display().to_string());
    let captain_env = |folder: &str| json!([["PINNED_ROLE", "orchestrator"], ["PINNED_PROJECT", folder]]);
    std::fs::write(config.join("projects.json"), json!([{ "path": path }, { "path": other_path }]).to_string()).unwrap();
    std::fs::write(
        config.join("sessions.json"),
        json!([
            { "cwd": path, "session": "cap-1", "env": captain_env(&path) },
            { "cwd": other_path, "session": "cap-2", "env": captain_env(&other_path) },
        ])
        .to_string(),
    )
    .unwrap();

    let launch = Launch {
        exe: bin.clone(),
        bins: BTreeMap::from([(Provider::Cursor, bin), (Provider::Claude, claude)]),
        path_env: None,
        settings: dir.join("settings.json"),
        mcp_config: None,
        reminders: true,
        plugin_dir: dir.join("plugin"),
        tutorial_helper: dir.join("helper"),
        sock: dir.join("pinned.sock"),
        vector_search: false,
        first_run_wait: Duration::from_secs(60),
        starting_limit: Duration::from_secs(600),
        claude_home: None,
    };
    let machine = Machine::new(launch, Some(config), None, |_| {});
    machine.listen(|_| Err("not a connector callback".into())).unwrap();
    machine.restore_tabs();
    let tab_of = |session: &str| machine.tabs().into_iter().find(|t| t.session.as_deref() == Some(session)).unwrap().id;
    let captain = tab_of("cap-1");
    let other_captain = tab_of("cap-2");
    let thread = machine.open_tab(Some(path.clone()), false).unwrap();
    std::env::set_var(SOCK_ENV, machine.launch().sock.clone());
    std::env::remove_var(TASK_ENV);
    std::env::set_var(TAB_ENV, &captain);
    let glance = |tab: &str| serde_json::to_value(machine.view(tab).unwrap().plans).unwrap();
    let cursor = json!({ "providerInstanceId": "cursor", "model": "composer-2.5" });

    assert_eq!(
        call("plan_set", json!({ "title": "Ship the form", "lists": [{ "name": "Build", "steps": [{ "name": "Parser" }, { "name": "Driver" }, { "name": "Docs" }] }] })),
        Ok("p1 rev 1: Ship the form\nl1 Build: s1 pending, s2 pending, s3 pending".to_string())
    );

    // A child that finishes: s1 reads done, and the captain is idle.
    let first = body("delegate_task", json!({ "task": "DONE", "title": "Parser run", "planStep": "s1", "target": cursor, "mode": "wait", "timeoutMs": 15000 }));
    assert_eq!((first["workState"].as_str(), first["planStep"].as_str()), (Some("result_available"), Some("s1")));
    assert_eq!(glance(&captain), json!([{ "id": "p1", "title": "Ship the form", "state": "idle", "done": 1, "total": 3, "strips": ["dpp"] }]));

    // A child that runs: s2 reads running, and a report on it is refused with the task's name.
    let sleeping = body("delegate_task", json!({ "task": "SLEEP", "title": "Driver run", "planStep": "s2", "target": cursor, "mode": "async" }));
    let sleep_id = sleeping["taskId"].as_str().unwrap().to_owned();
    assert_eq!(glance(&captain), json!([{ "id": "p1", "title": "Ship the form", "state": "working", "done": 1, "total": 3, "now": "Driver", "strips": ["drp"] }]));
    assert_eq!(
        call("plan_step", json!({ "step": "s2", "state": "done" })),
        Err(format!("{sleep_id} runs this step. Wait for it, or cancel {sleep_id} first. Nothing changed."))
    );

    // A second child on s2 replaces the link: s2 reads failed when that child fails, and the older task stays in the registry.
    let failing = body("delegate_task", json!({ "task": "FAIL", "title": "Driver fails", "planStep": "s2", "target": cursor, "mode": "async" }));
    let fail_id = failing["taskId"].as_str().unwrap().to_owned();
    wait_until("the failing child never failed", || task_state(&fail_id) == "failed");
    assert_eq!(glance(&captain), json!([{ "id": "p1", "title": "Ship the form", "state": "fixing", "done": 1, "total": 3, "strips": ["dfp"] }]));
    assert_eq!(task_state(&sleep_id), "working", "the replaced task keeps running in the registry");

    // A retry on the failed s2 opens one fix, and its success reads x.
    let retry = body("delegate_task", json!({ "task": "DONE", "title": "Driver retry", "planStep": "s2", "target": cursor, "mode": "wait", "timeoutMs": 15000 }));
    assert_eq!((retry["workState"].as_str(), retry["planStep"].as_str()), (Some("result_available"), Some("s4")));
    assert_eq!(glance(&captain), json!([{ "id": "p1", "title": "Ship the form", "state": "idle", "done": 2, "total": 3, "strips": ["dxp"] }]));

    // Refusals: a step no plan holds, a plain thread, and another Captain. Each starts no task.
    let refused = |tab: &str, step: &str| {
        std::env::set_var(TAB_ENV, tab);
        let answer = call("delegate_task", json!({ "task": "DONE", "planStep": step, "target": cursor, "mode": "async" }));
        std::env::set_var(TAB_ENV, &captain);
        answer
    };
    assert_eq!(refused(&captain, "s99"), Err("no step or list s99 in any plan of this Captain. Nothing changed.".to_string()));
    assert_eq!(refused(&thread, "s3"), Err("only a project's Captain keeps a plan. Nothing changed.".to_string()));
    assert_eq!(refused(&other_captain, "s3"), Err("no step or list s3 in any plan of this Captain. Nothing changed.".to_string()));

    // Once the plan is closed, its steps are frozen to a link too.
    assert_eq!(body("task_cancel", json!({ "taskId": sleep_id, "reason": "stop" }))["workState"], "cancelled");
    assert!(call("plan_close", json!({ "plan": "p1", "reason": "Shipped." })).is_ok());
    assert_eq!(refused(&captain, "s3"), Err("p1 is closed, so its steps are frozen. Nothing changed.".to_string()));

    machine.quit();
    let _ = std::fs::remove_dir_all(&dir);
}
