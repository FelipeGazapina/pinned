use crate::agent::{AgentEvent, AgentStatus, SOCK_ENV};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SUBCOMMAND: &str = "hook";

/// Claude Code settings layered onto the harnessed session via `--settings`, so the user's
/// own ~/.claude/settings.json stays untouched.
pub fn write_settings(path: &Path, exe: &Path) -> std::io::Result<()> {
    let mut hooks = serde_json::Map::new();
    for (_, arg, hook_event) in AgentStatus::ALL {
        let command = format!("'{}' {SUBCOMMAND} {arg}", exe.display());
        hooks.insert(
            hook_event.into(),
            serde_json::json!([{ "hooks": [{ "type": "command", "command": command }] }]),
        );
    }
    std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({ "hooks": hooks }))?)
}

/// Runs inside the hook process Claude Code spawns: read the hook payload from stdin and
/// forward it to the window that owns this session. Never fails loudly, so it can't
/// disturb the agent.
pub fn forward(arg: Option<&str>) {
    let Some(status) = AgentStatus::ALL.iter().find(|(_, a, _)| Some(*a) == arg).map(|(s, _, _)| *s)
    else {
        eprintln!("usage: pinned {SUBCOMMAND} <working|done|needs-input>");
        return;
    };
    let Ok(sock) = std::env::var(SOCK_ENV) else { return };

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let hook: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let field = |k: &str| hook[k].as_str().map(str::to_owned);
    let event = AgentEvent { status, cwd: field("cwd"), message: field("message") };

    if let Ok(mut stream) = UnixStream::connect(sock) {
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let _ = writeln!(stream, "{}", serde_json::to_string(&event).unwrap());
    }
}

pub fn listen(sock: PathBuf, on_event: impl Fn(AgentEvent) + Send + 'static) -> std::io::Result<()> {
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                match serde_json::from_str(&line) {
                    Ok(event) => on_event(event),
                    Err(e) => eprintln!("pinned: bad hook event {line:?}: {e}"),
                }
            }
        }
    });
    Ok(())
}
