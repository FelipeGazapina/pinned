//! A budgeted worker's PostToolUse hook, run as Claude Code runs it: the `budget` subcommand of the
//! binary, with the socket and the tab in its environment.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_pinned-server");

fn hook(sock: &std::path::Path) -> (Output, Duration) {
    let started = Instant::now();
    let out = Command::new(BIN)
        .arg("budget")
        .env("PINNED_SOCK", sock)
        .env("PINNED_TAB", "tab-7")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    (out, started.elapsed())
}

#[test]
fn the_budget_hook_against_a_socket_that_does_not_exist_prints_nothing_and_exits_0_within_its_bound() {
    let dir = tempfile::tempdir().unwrap();
    let (out, took) = hook(&dir.path().join("missing.sock"));
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "", "nothing is injected");
    assert!(took < Duration::from_millis(500), "the hook took {took:?}");
}

#[test]
fn the_budget_hook_prints_exactly_one_post_tool_use_object_for_the_line_the_machine_hands_it() {
    let line = "[pinned] You have used US$ 0.42 of your US$ 0.50 budget. Push your work now, write your proof and PR text as you go, and plan to report soon.";
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("pinned.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let machine = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut asked = String::new();
        BufReader::new(stream.try_clone().unwrap()).read_line(&mut asked).unwrap();
        writeln!(stream, "{}", serde_json::json!({ "Ok": line })).unwrap();
        asked
    });
    let (out, took) = hook(&sock);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "one object: {stdout:?}");
    let printed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(printed, serde_json::json!({ "hookSpecificOutput": { "hookEventName": "PostToolUse", "additionalContext": line } }));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&machine.join().unwrap()).unwrap(), serde_json::json!({ "kind": "budget", "tab": "tab-7" }));
    assert!(took < Duration::from_secs(2), "the hook took {took:?}");
}

#[test]
fn the_budget_hook_prints_nothing_when_no_line_waits() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("pinned.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let machine = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut asked = String::new();
        BufReader::new(stream.try_clone().unwrap()).read_line(&mut asked).unwrap();
        writeln!(stream, r#"{{"Ok":""}}"#).unwrap();
    });
    let (out, _) = hook(&sock);
    machine.join().unwrap();
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned()), (Some(0), String::new()));
}
