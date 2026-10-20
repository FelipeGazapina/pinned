use crate::agent::{AgentEvent, AgentStatus, Message, Stage, CONNECTOR_ENV, ISSUE_ENV, SOCK_ENV, TAB_ENV};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const SUBCOMMAND: &str = "hook";
pub const STAGE_SUBCOMMAND: &str = "stage";

/// What the harness answers to each socket line.
type Reply = Result<String, String>;

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
/// forward it to the tab that owns this session. Never fails loudly, so it can't
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
    let event = AgentEvent { status, cwd: field("cwd"), message: field("message"), tab: std::env::var(TAB_ENV).ok() };

    if let Ok(mut stream) = UnixStream::connect(sock) {
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let _ = writeln!(stream, "{}", serde_json::to_string(&Message::Agent(event)).unwrap());
    }
}

/// Runs when a connector's session calls `pinned stage <stage>`: asks the harness to move the
/// session's issue to that stage's column and returns what the harness answered.
pub fn stage(arg: Option<&str>) -> Reply {
    let stage: Stage = arg
        .and_then(|a| serde_json::from_value(serde_json::json!(a)).ok())
        .ok_or(format!("usage: pinned {STAGE_SUBCOMMAND} <picked|coding|review>"))?;
    let env = |k: &str| std::env::var(k).map_err(|_| format!("{k} is not set: run this inside a session a pinned connector started"));
    let message = Message::Stage { connector: env(CONNECTOR_ENV)?, issue: env(ISSUE_ENV)?, stage };

    let mut stream = UnixStream::connect(env(SOCK_ENV)?).map_err(|e| format!("pinned is not running: {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    writeln!(stream, "{}", serde_json::to_string(&message).unwrap()).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).map_err(|e| format!("pinned did not answer: {e}"))?;
    serde_json::from_str::<Reply>(&reply).map_err(|e| format!("pinned sent a bad answer {reply:?}: {e}"))?
}

/// Answers every line on the socket with `on_message`'s reply. Each connection gets its own
/// thread, so a slow Linear call for one session doesn't hold up another session's hooks.
pub fn listen(sock: PathBuf, on_message: impl Fn(Message) -> Reply + Send + Sync + 'static) -> std::io::Result<()> {
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    let on_message = Arc::new(on_message);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let on_message = on_message.clone();
            std::thread::spawn(move || {
                let Ok(reader) = stream.try_clone() else { return };
                for line in BufReader::new(reader).lines().map_while(Result::ok) {
                    let reply = match serde_json::from_str(&line) {
                        Ok(message) => on_message(message),
                        Err(e) => Err(format!("bad message {line:?}: {e}")),
                    };
                    // Hook forwarders hang up without reading the reply.
                    let _ = writeln!(stream, "{}", serde_json::to_string(&reply).unwrap());
                }
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_reaches_the_harness_and_returns_its_answer() {
        let sock = std::env::temp_dir().join(format!("pinned-test-{}.sock", std::process::id()));
        listen(sock.clone(), |message| match message {
            Message::Stage { connector, issue, stage: Stage::Coding } => Ok(format!("{connector} moved {issue}")),
            other => Err(format!("unexpected {other:?}")),
        })
        .unwrap();
        std::env::set_var(SOCK_ENV, &sock);
        std::env::set_var(CONNECTOR_ENV, "c1");
        std::env::set_var(ISSUE_ENV, "issue-uuid");

        assert_eq!(stage(Some("coding")), Ok("c1 moved issue-uuid".to_owned()));
        assert_eq!(stage(Some("review")), Err("unexpected Stage { connector: \"c1\", issue: \"issue-uuid\", stage: Review }".to_owned()));
        assert_eq!(stage(Some("shipping")), Err("usage: pinned stage <picked|coding|review>".to_owned()));
        let _ = std::fs::remove_file(sock);
    }
}
