use crate::history;
use crate::reminders;
use crate::agent::{
    AgentEvent, AgentStatus, Message, Stage, Subject, CONNECTOR_ENV, ISSUE_ENV, SLACK_CHANNEL_ENV, SLACK_THREAD_ENV, SOCK_ENV, TAB_ENV,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const SUBCOMMAND: &str = "hook";
pub const STAGE_SUBCOMMAND: &str = "stage";
pub const SLACK_SUBCOMMAND: &str = "slack";
pub const LINEAR_SUBCOMMAND: &str = "linear";

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
    std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({
        "hooks": hooks,
        "permissions": { "allow": reminders::permissions() },
    }))?)
}

/// Runs inside the hook process Claude Code spawns: read the hook payload from stdin and
/// forward it to the tab that owns this session. Never fails loudly, so it can't
/// disturb the agent.
pub fn forward(arg: Option<&str>) {
    let Some(status) = AgentStatus::ALL.iter().find(|(_, a, _)| Some(*a) == arg).map(|(s, _, _)| *s)
    else {
        eprintln!("usage: pinned {SUBCOMMAND} <working|done|needs-input|failed>");
        return;
    };
    let Ok(sock) = std::env::var(SOCK_ENV) else { return };

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let hook: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let field = |k: &str| hook[k].as_str().map(str::to_owned);
    let event = AgentEvent {
        status,
        cwd: field("cwd"),
        message: field("message"),
        tab: std::env::var(TAB_ENV).ok(),
        session: field("session_id"),
        title: field("transcript_path")
            .and_then(|path| std::fs::File::open(path).ok())
            .and_then(|file| title_from_transcript(BufReader::new(file)))
            .or_else(|| hook["prompt"].as_str().and_then(title_from_prompt)),
        resume_at: (status == AgentStatus::Failed)
            .then(|| field("transcript_path"))
            .flatten()
            .and_then(|path| std::fs::File::open(path).ok())
            .and_then(|file| limit_reset_from_transcript(BufReader::new(file))),
        prompt: (status == AgentStatus::Working).then(|| hook["prompt"].as_str().map(history::clip_prompt)).flatten(),
        // Taken here, in the hook Claude Code waits on, so the agent can't edit before the baseline exists.
        snapshot: matches!(status, AgentStatus::Working | AgentStatus::Done | AgentStatus::Failed)
            .then(|| field("cwd").and_then(|cwd| history::snapshot(Path::new(&cwd))))
            .flatten(),
    };

    if let Ok(mut stream) = UnixStream::connect(sock) {
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let _ = writeln!(stream, "{}", serde_json::to_string(&Message::Agent(event)).unwrap());
    }
}

const LIMIT_ERROR: &[u8] = br#""apiError":"usage_limit_reached""#;

/// When the usage limit that last stopped the session lifts, from the error Claude Code
/// wrote to its transcript. Only lines carrying that error are parsed.
fn limit_reset_from_transcript(transcript: impl BufRead) -> Option<u64> {
    transcript
        .split(b'\n')
        .map_while(Result::ok)
        .filter(|line| line.windows(LIMIT_ERROR.len()).any(|w| w == LIMIT_ERROR))
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(&line).ok()?["apiErrorParams"]["rate_limit_info"]["resetsAt"].as_u64())
        .last()
}

const AI_TITLE: &[u8] = br#""type":"ai-title""#;
const PROMPT_TITLE_CHARS: usize = 60;

/// The last title Claude Code gave the session in its transcript. Transcripts grow to many
/// megabytes, so only lines that mention a title are parsed.
fn title_from_transcript(transcript: impl BufRead) -> Option<String> {
    transcript
        .split(b'\n')
        .map_while(Result::ok)
        .filter(|line| line.windows(AI_TITLE.len()).any(|w| w == AI_TITLE))
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(&line).ok()?["aiTitle"].as_str().map(str::to_owned))
        .last()
}

/// Stands in for the title until Claude Code names the session: the prompt's first line.
fn title_from_prompt(prompt: &str) -> Option<String> {
    let line = prompt.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(match line.char_indices().nth(PROMPT_TITLE_CHARS) {
        Some((cut, _)) => format!("{}…", line[..cut].trim_end()),
        None => line.to_owned(),
    })
}

/// Runs when a connector's session calls `pinned stage <stage>`: asks the harness to move the
/// session's issue to that stage's column and returns what the harness answered. A Slack session
/// names its thread, and the harness moves the issue the thread filed.
pub fn stage(arg: Option<&str>) -> Reply {
    let stage: Stage = arg
        .and_then(|a| serde_json::from_value(serde_json::json!(a)).ok())
        .ok_or(format!("usage: pinned {STAGE_SUBCOMMAND} <picked|coding|review>"))?;
    ask(Message::Stage { connector: env(CONNECTOR_ENV)?, subject: subject()?, stage })
}

/// What the session works on, from the environment its connector started it with.
fn subject() -> Result<Subject, String> {
    match (env(ISSUE_ENV), env(SLACK_CHANNEL_ENV), env(SLACK_THREAD_ENV)) {
        (Ok(issue), ..) => Ok(Subject::LinearIssue { issue }),
        (_, Ok(channel), Ok(thread)) => Ok(Subject::SlackThread { channel, thread }),
        (Err(e), ..) => Err(e),
    }
}

/// Runs when a Slack connector's session calls `pinned slack reply <text>`: asks the harness to
/// post `text` in the session's thread and returns what the harness answered.
pub fn slack(args: &[String]) -> Reply {
    let text = match args {
        [verb, text @ ..] if verb == "reply" && !text.is_empty() => text.join(" "),
        _ => return Err(format!("usage: pinned {SLACK_SUBCOMMAND} reply <text>")),
    };
    ask(Message::SlackReply { connector: env(CONNECTOR_ENV)?, channel: env(SLACK_CHANNEL_ENV)?, thread: env(SLACK_THREAD_ENV)?, text })
}

/// Runs when a Slack connector's session calls `pinned linear create <title> <description>`: asks
/// the harness to file a Linear issue for the session's thread and returns its identifier and link.
pub fn linear(args: &[String]) -> Reply {
    let (title, description) = match args {
        [verb, title, description @ ..] if verb == "create" && !title.trim().is_empty() && !description.is_empty() => {
            (title.clone(), description.join(" "))
        }
        _ => return Err(format!("usage: pinned {LINEAR_SUBCOMMAND} create <title> <description>")),
    };
    ask(Message::LinearCreate {
        connector: env(CONNECTOR_ENV)?,
        channel: env(SLACK_CHANNEL_ENV)?,
        thread: env(SLACK_THREAD_ENV)?,
        title,
        description,
    })
}

fn env(k: &str) -> Result<String, String> {
    std::env::var(k).map_err(|_| format!("{k} is not set: run this inside a session a pinned connector started"))
}

/// Sends one message to the running harness and waits for its answer.
fn ask(message: Message) -> Reply {
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
    fn transcript_title_is_the_last_ai_title() {
        let transcript = concat!(
            r#"{"type":"user","message":{"content":"fix the build"}}"#, "\n",
            r#"{"type":"ai-title","aiTitle":"Build fix","sessionId":"s1"}"#, "\n",
            "not json, but \"type\":\"ai-title\" anyway\n",
            "\u{0}\u{1} garbage\n",
            r#"{"type":"assistant","message":{"content":"the word \"aiTitle\" in passing"}}"#, "\n",
            r#"{"type":"ai-title","aiTitle":"App update on opening","sessionId":"s1"}"#, "\n",
            r#"{"type":"user","message":{"content":"thanks"}}"#,
        );
        assert_eq!(title_from_transcript(transcript.as_bytes()), Some("App update on opening".to_owned()));
    }

    #[test]
    fn transcript_without_ai_title_has_no_title() {
        let transcript = concat!(r#"{"type":"user","message":{"content":"hi"}}"#, "\n", "{broken\n");
        assert_eq!(title_from_transcript(transcript.as_bytes()), None);
        assert_eq!(title_from_transcript("".as_bytes()), None);
    }

    #[test]
    fn limit_reset_is_the_last_limit_error_in_the_transcript() {
        let transcript = concat!(
            r#"{"type":"assistant","apiError":"usage_limit_reached","apiErrorParams":{"rate_limit_info":{"status":"rejected","resetsAt":1792382400}}}"#, "\n",
            r#"{"type":"user","message":{"content":"try again"}}"#, "\n",
            r#"{"type":"assistant","apiError":"usage_limit_reached","apiErrorParams":{"rate_limit_info":{"status":"rejected","resetsAt":1792400000}}}"#, "\n",
            r#"{"type":"assistant","message":{"content":"mentions usage_limit_reached in passing"}}"#,
        );
        assert_eq!(limit_reset_from_transcript(transcript.as_bytes()), Some(1792400000));
        assert_eq!(limit_reset_from_transcript(r#"{"type":"user"}"#.as_bytes()), None);
    }

    #[test]
    fn prompt_title_is_the_first_line_cut_short() {
        assert_eq!(title_from_prompt("\n  fix the login page  \nit 500s on submit"), Some("fix the login page".to_owned()));
        assert_eq!(
            title_from_prompt("make the pill show every open tab as a carousel that keeps scrolling forever"),
            Some("make the pill show every open tab as a carousel that keeps s…".to_owned()),
        );
        assert_eq!(title_from_prompt("  \n"), None);
    }

    /// One test, because the commands read the session's environment, which is process-wide.
    #[test]
    fn connector_commands_reach_the_harness_and_return_its_answer() {
        let sock = std::env::temp_dir().join(format!("pinned-test-{}.sock", std::process::id()));
        listen(sock.clone(), |message| match message {
            Message::Stage { connector, subject: Subject::LinearIssue { issue }, stage: Stage::Coding } => Ok(format!("{connector} moved {issue}")),
            Message::Stage { connector, subject: Subject::SlackThread { channel, thread }, stage } => {
                Ok(format!("{connector} moved the issue of {channel}:{thread} to {stage:?}"))
            }
            Message::SlackReply { connector, channel, thread, text } => Ok(format!("{connector} posted {text:?} in {channel}:{thread}")),
            Message::LinearCreate { connector, channel, thread, title, description } => {
                Ok(format!("{connector} filed {title:?} / {description:?} for {channel}:{thread}"))
            }
            other => Err(format!("unexpected {other:?}")),
        })
        .unwrap();
        std::env::set_var(SOCK_ENV, &sock);
        std::env::set_var(CONNECTOR_ENV, "c1");
        assert_eq!(stage(Some("coding")), Err("PINNED_ISSUE is not set: run this inside a session a pinned connector started".to_owned()));
        std::env::set_var(ISSUE_ENV, "issue-uuid");

        assert_eq!(stage(Some("coding")), Ok("c1 moved issue-uuid".to_owned()));
        assert_eq!(
            stage(Some("review")),
            Err("unexpected Stage { connector: \"c1\", subject: LinearIssue { issue: \"issue-uuid\" }, stage: Review }".to_owned())
        );
        assert_eq!(stage(Some("shipping")), Err("usage: pinned stage <picked|coding|review>".to_owned()));

        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            slack(&args(&["reply", "hi"])),
            Err("PINNED_SLACK_CHANNEL is not set: run this inside a session a pinned connector started".to_owned())
        );
        std::env::remove_var(ISSUE_ENV);
        std::env::set_var(SLACK_CHANNEL_ENV, "C0123ABCD");
        std::env::set_var(SLACK_THREAD_ENV, "1792400050.000200");
        assert_eq!(
            slack(&args(&["reply", "It's *fixed* in v2"])),
            Ok("c1 posted \"It's *fixed* in v2\" in C0123ABCD:1792400050.000200".to_owned())
        );
        assert_eq!(slack(&args(&["reply"])), Err("usage: pinned slack reply <text>".to_owned()));
        assert_eq!(slack(&args(&["post", "hi"])), Err("usage: pinned slack reply <text>".to_owned()));

        assert_eq!(stage(Some("review")), Ok("c1 moved the issue of C0123ABCD:1792400050.000200 to Review".to_owned()));
        assert_eq!(
            linear(&args(&["create", "Export 500s", "The export button", "fails"])),
            Ok("c1 filed \"Export 500s\" / \"The export button fails\" for C0123ABCD:1792400050.000200".to_owned())
        );
        assert_eq!(linear(&args(&["create", "Export 500s"])), Err("usage: pinned linear create <title> <description>".to_owned()));
        assert_eq!(linear(&args(&["file", "a", "b"])), Err("usage: pinned linear create <title> <description>".to_owned()));
        let _ = std::fs::remove_file(sock);
    }

    #[test]
    fn a_linear_stage_message_keeps_its_wire_shape() {
        let linear = r#"{"kind":"stage","connector":"c1","issue":"uuid","stage":"coding"}"#;
        let message: Message = serde_json::from_str(linear).unwrap();
        assert_eq!(serde_json::to_string(&message).unwrap(), linear);
        let slack = r#"{"kind":"stage","connector":"c2","channel":"C0123ABCD","thread":"1792400050.000200","stage":"review"}"#;
        let message: Message = serde_json::from_str(slack).unwrap();
        assert!(matches!(&message, Message::Stage { subject: Subject::SlackThread { channel, .. }, .. } if channel == "C0123ABCD"));
        assert_eq!(serde_json::to_string(&message).unwrap(), slack);
    }
}
